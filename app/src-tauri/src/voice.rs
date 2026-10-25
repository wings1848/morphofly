//! 语音会话管理：唤醒 → 连云端 → 推流 → 播放 → 汇报。
//!
//! 这是 App 里唯一碰音频和网络的地方。
//!
//! ## 为什么单独一个模块
//!
//! `main.rs` 只该管「窗口 + IPC」，音频/网络全在这里。
//! 这样阶段 5 打磨时可以单独替换（比如加 AEC、换 provider），不碰 UI。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::audio::playback::AudioPlayer;
use morphofly_core::dispatch::Dispatcher;
use morphofly_core::realtime::{RealtimeClient, RealtimeEvent};
use morphofly_core::session::{OrbState, SessionMachine, Stimulus};
use morphofly_core::spectrum::Spectrum;
use morphofly_core::tasks::TaskStore;
use morphofly_core::tools::tool_defs;
use morphofly_core::Config;
use tokio::sync::Mutex;

/// 语音会话（一次唤醒 = 一个会话）
pub struct VoiceSession {
    /// 4 工具分发器（派活/查进度/取消/追加）
    dispatcher: Arc<Dispatcher>,
    /// 任务表（主动汇报要看它的状态）
    store: Arc<Mutex<TaskStore>>,
    client: Arc<RealtimeClient>,
    player: Arc<AudioPlayer>,
    mic: Arc<AudioCapture>,
    /// 当前会话的取消开关
    stop: Arc<AtomicBool>,
    /// 统计：播放了多少帧 / 打断几次 / 首音频延迟
    pub played_frames: Arc<AtomicU64>,
    pub interruptions: Arc<AtomicU64>,
    pub first_audio_ms: Arc<AtomicU64>,
    /// 是否「本地 VAD + 手动 commit」模式（= 关掉了服务端 VAD）。
    ///
    /// 为 false 时服务端自己判定回合结束并自动触发推理，
    /// 我们**不能**再发 `input_audio_buffer.commit` / `response.create`，
    /// 否则会重复触发（两轮回复/`ongoing response` 报错）。
    local_vad: bool,
}

/// 「这一轮什么时候该回话」的看门狗状态（commit 之后等首音频）。
///
/// 2026-10-25 真机：用户「说完了但它不回复」的根因就是这条路径没有任何
/// 超时兜底 —— commit + response.create 发出去之后，如果服务端因为
/// `ongoing response`、空缓冲区、网络抖动没回复，就永久静默。
/// 开源框架的通用做法（LiveKit/pipecat）：1.5–2 秒没首包就 cancel + 重发一次。
#[derive(Debug, Default)]
struct TurnWait {
    /// 本轮 commit/触发推理的时刻
    since: Option<std::time::Instant>,
    /// 本轮是否已经收到过音频
    audio_seen: bool,
    /// 已经重试了几次
    retries: u8,
}

/// 打断去重窗口：本地 VAD 和云端 SpeechStarted 会**同时**报同一次打断，
/// 同一个「用户开口」只该在统计里算一次。
const INTERRUPT_DEDUPE_MS: u64 = 2000;

/// 记一次打断（去重）。
///
/// 为什么需要去重：一次真打断会被两条路径各看见一遍 ——
/// 本地 VAD（我们自己的检测）和云端 `input_audio_buffer.speech_started`。
/// 以前只有本地路径计数，所以「数字看着正常」；现在两条路径都计，
/// 不去重就会一次用户开口记两次（统计就变假了）。
fn count_interruption(counter: &AtomicU64, last_ms: &AtomicU64) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let prev = last_ms.swap(now, Ordering::Relaxed);
    if now.saturating_sub(prev) < INTERRUPT_DEDUPE_MS {
        return false;
    }
    counter.fetch_add(1, Ordering::Relaxed);
    true
}

/// 打断后告诉服务端「用户只听到了前 N 毫秒」（`conversation.item.truncate`）。
///
/// 为什么要做：不截断的话，服务端上下文里 assistant 那条是**完整**的，
/// 模型会以为「我已经说过了」，下一轮回复就会跳过或复述用户没听到的内容
/// （表现是「答非所问 / 突然不吭声」）。官方开源控制台
/// （stepfun-ai/Step-Realtime-Console）在打断时正是这套顺序：
///   停播 → response.cancel → conversation.item.truncate(audio_end_ms=已播时长)。
///
/// `assist` 里存的是 (item_id, 这条回复开始时的累计已播采样数)；
/// 已播时长用 `frames_pulled`（真正被声卡取走的样本）而不是 `played_frames`
/// （那是「收进来多少」，会把还没播的也算进去）。
fn truncate_played(
    client: &RealtimeClient,
    assist: &std::sync::Mutex<Option<(String, u64)>>,
    player: &AudioPlayer,
) {
    let taken = {
        let mut g = match assist.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.take()
    };
    let Some((item_id, base)) = taken else {
        return;
    };
    let now = player.frames_pulled();
    let ms = now.saturating_sub(base).saturating_mul(1000)
        / u64::from(morphofly_core::audio::DOWNLINK_SAMPLE_RATE);
    if ms == 0 {
        return;
    }
    match client.truncate_item(&item_id, ms) {
        Ok(()) => {
            tracing::info!(item = %item_id, audio_end_ms = ms, "打断：已截断上下文（truncate）")
        }
        Err(e) => tracing::debug!(error = %e, "truncate 失败（可忽略）"),
    }
}

impl VoiceSession {
    /// 建立会话：连云端 + 开麦克风 + 开播放器
    ///
    /// ⚠️ `store` / `dispatcher` 必须由外部传进来（**不能在这里 new 一个**）。
    ///
    /// 2026-10-25 真机踩到的坑：这里原来自己 `TaskStore::open(同一个路径)` 建了一个
    /// **独立实例**，于是会话里派出去的任务只写进了这个实例（和磁盘），
    /// 主线程 that 个「主动汇报守卫」手里的实例永远看不到新任务 ——
    /// DSH 干完了、任务表里 `reported=false`，却一条汇报都不发，
    /// 只有重启 App（重新读盘）才会补报。查了很久。
    pub async fn open(
        cfg: &Config,
        store: Arc<Mutex<TaskStore>>,
        dispatcher: Arc<Dispatcher>,
    ) -> anyhow::Result<Self> {
        let client = Arc::new(RealtimeClient::connect(cfg).await?);
        client.send_tools(&tool_defs())?;
        let player = Arc::new(AudioPlayer::open()?);
        let mic = Arc::new(AudioCapture::start_default()?);
        morphofly_core::config::ensure_data_dir(cfg)?;
        Ok(Self {
            dispatcher,
            store,
            client,
            player,
            mic,
            stop: Arc::new(AtomicBool::new(false)),
            played_frames: Arc::new(AtomicU64::new(0)),
            interruptions: Arc::new(AtomicU64::new(0)),
            first_audio_ms: Arc::new(AtomicU64::new(u64::MAX)),
            local_vad: cfg.realtime.use_local_vad,
        })
    }

    #[allow(dead_code)] // 供后续「手动触发一轮」等场景
    pub fn client(&self) -> Arc<RealtimeClient> {
        self.client.clone()
    }

    /// 跑会话主循环，直到静音超时 / 用户点击退出 / 连接断开。
    ///
    /// 返回本次会话的统计（供 UI 显示与验收）。
    pub async fn run(
        &self,
        machine: Arc<Mutex<SessionMachine>>,
        on_spectrum: impl Fn(Vec<f32>, f32) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let stop = self.stop.clone();
        let player = self.player.clone();
        let mic = self.mic.clone();
        let client = self.client.clone();
        let played = self.played_frames.clone();
        let interruptions = self.interruptions.clone();
        let first_audio = self.first_audio_ms.clone();
        let on_spectrum = Arc::new(on_spectrum);

        let client_for_tools = client.clone();
        let dispatcher = self.dispatcher.clone();
        let store = self.store.clone();
        let mut ev_rx = client.events();
        // 「服务端 VAD」模式：服务端自己判定回合结束并触发推理，
        // 客户端不能再发 commit / create_response（否则重复触发）。
        let local_vad = self.local_vad;
        // 首包看门狗（两个任务共享）
        let turn_wait = Arc::new(std::sync::Mutex::new(TurnWait::default()));
        let turn_wait_push = turn_wait.clone();
        let turn_wait_ev = turn_wait.clone();
        // ★ 模型当前这条回复的 (item_id, 回复开始时的已播采样数)。
        //   打断时用它算「用户实际听到多少毫秒」，再发 conversation.item.truncate。
        //   官方控制台（Step-Realtime-Console）的 safeStartAudioInput 就是这么做的。
        type AssistRef = Arc<std::sync::Mutex<Option<(String, u64)>>>;
        let assist: AssistRef = Arc::new(std::sync::Mutex::new(None));
        let assist_push = assist.clone();
        let assist_ev = assist.clone();

        // 采集 → 推流 + 频谱
        let mic2 = mic.clone();
        let client2 = client.clone();
        #[cfg(feature = "kws")]
        let client_for_push = client.clone();
        let stop2 = stop.clone();
        let m_machine = machine.clone();
        let on_spec = on_spectrum.clone();
        // 本地 VAD 只在 --features kws 下存在，这些 clone 也只有那时候用
        #[cfg(feature = "kws")]
        let player_in_push = player.clone();
        #[cfg(feature = "kws")]
        let interruptions_in_push = interruptions.clone();
        // 打断去重用：本地 VAD 和云端 SpeechStarted 共用同一个「上次计数时间」
        let last_interrupt_ms = Arc::new(AtomicU64::new(0));
        let last_interrupt_in_push = last_interrupt_ms.clone();
        #[cfg(feature = "kws")]
        let machine_in_push = machine.clone();
        // 打断时要通知云端停止生成（修缺陷 #5）
        #[cfg(feature = "kws")]
        let client_in_push = client_for_push.clone();
        // ★ 本地 VAD 判定「用户开口」的共享标志。
        //
        // 为什么需要：纠正机制（说了话没干活 → 逼它真派活）原来只认云端
        // `input_audio_buffer.speech_started` 事件。但我们是本地 commit 的
        // （快了 1.95 秒），云端那条事件不一定来 —— 真机实测就出现过
        // 「用户明明说了话，user_spoke 却是 false，于是模型只回一句『好，查着呢』
        // 也没人纠正」。本机 VAD 也看到的开口，得算数。
        let user_spoke_local = Arc::new(AtomicBool::new(false));
        let user_spoke_in_push = user_spoke_local.clone();
        let user_spoke_in_ev = user_spoke_local.clone();

        let push_task = tokio::spawn(async move {
            let mut rx = mic2.subscribe();
            let mut spectrum = Spectrum::with_default_window();
            let mut idle_frames = 0u64;
            // 诊断计数（2026-10-25 加）：用来区分「麦克风没声音」和「VAD 没判出来」。
            let mut frames_seen = 0u64;
            let mut peak_rms = 0.0f32;
            // ★ 上行自动增益（2026-10-25）。
            //
            // 真机实测：耳机麦的语音峰值只有 ~0.09（-21dBFS），阶跃的**服务端 VAD
            // 是能量门控**的，这么小的声音它直接不判 → 服务端 VAD 模式完全不回复。
            // 探针（core/examples/vad_probe.rs）用同一段录音验证：
            //   原样 → 无 speech_started；×2 → 触发；×4 → 触发且 ASR 更准。
            // 这里只放大**上行**（送识别的那一路）；本地 VAD 仍用原始帧。
            let agc_enabled = std::env::var("MORPHOFLY_NO_AGC")
                .map(|v| {
                    !matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "on" | "yes"
                    )
                })
                .unwrap_or(true);
            let mut uplink_gain = morphofly_core::audio::AutoGain::new();
            tracing::debug!(agc_enabled, "上行自动增益状态");
            tracing::debug!("推流任务启动（等待采集帧）");
            // ★ 本地 VAD：三个用途
            //   1. 打断（用户说话 → 立刻停播）
            //   2. 判定「开口 / 说完了」→ 立刻 commit（不等服务端 VAD 的 1.95s）
            //   3. **「这一帧有没有人声」的唯一判据**（旧版用固定 RMS 阈值，见下）
            #[cfg(feature = "kws")]
            let vad =
                morphofly_core::vad::LocalVad::new(&morphofly_core::vad::VadConfig::default()).ok();
            #[cfg(feature = "kws")]
            let mut vad_cooldown = 0u32;
            // 「说完」判定（本地 VAD 驱动）。
            //
            // 状态机：
            //   Idle     --有声--> Speaking
            //   Speaking --连续 K 帧静音--> 发 commit + create_response --> Cooldown
            //   Cooldown --连续 M 帧静音--> Idle（等下一轮开口）
            //
            // ⚠️ 2026-10-24 踩坑：最初写成「只要 rms>0.02 就重置 committed 标志」，
            // 结果环境底噪（风扇/键盘）让标志反复重置，一次发言触发 4 次 commit。
            // 现在用「必须连续有声才算重新开口」——底噪过不了连续 N 帧这条线。
            //
            // ⚠️ 2026-10-25 第二个坑（本文件这轮改的就是它）：把 `rms > 0.02`
            // 当成「有没有人声」的判据，在真实麦克风上**根本达不到** ——
            // 内置麦正常说话的帧 RMS 最高 0.018（实测 5 分钟，底噪中位 0.00275）→
            // 永远判不到开口 → 永远不 commit → 云端收不到话 → 没有回复。
            // 所以「有没有人声」交给 VAD；`VOICE_RMS` 只在没有 VAD 的构建里兜底。
            #[derive(PartialEq)]
            enum Talk {
                Idle,
                Speaking,
                Cooldown,
            }
            let mut talk = Talk::Idle;
            let mut voiced_run = 0u32; // 连续有声帧数
            let mut silent_run = 0u32; // 连续静音帧数
                                       // 兜底阈值：只在 `--no-default-features`（没有 silero VAD）时使用。
            const VOICE_RMS: f32 = 0.02;
            const NEED_VOICE: u32 = 3; // 连续 3 帧（60ms）才算真开口
                                       // ⚠️ 200ms（10 帧）太短 —— 用户反馈 + 真机日志都证明：一句话中间
                                       // 稍微换气就会被切成两段，第二段拿着「已提交」的缓冲去 create_response，
                                       // 服务端回 `ongoing response already exists`（这条 WARN 在日志里刷过）。
                                       //
                                       // 有 VAD 时不用等这么久：VAD 自己已经有 0.5s 静音挂起（min_silence_duration），
                                       // 「说完」的防抖已经由它做了，这里只要几帧确认就 commit。
            #[cfg(feature = "kws")]
            const NEED_SILENCE: u32 = 3; // 60ms（真正的静音判定在 VAD 里）
            #[cfg(not(feature = "kws"))]
            const NEED_SILENCE: u32 = 20; // 400ms（纯 RMS 兜底，必须等够）
            loop {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Ok(mut frame)) => {
                        let rms = morphofly_core::spectrum::rms_i16(&frame);
                        let bins = spectrum.analyze_i16(&frame);
                        on_spec(bins, morphofly_core::spectrum::rms_to_visual(rms));
                        // ★「这一帧算不算人声」：本地 silero VAD 判定。
                        //
                        // 为什么不再用固定 RMS 阈值（2026-10-25 真机实测）：
                        // 用户正常说话经过内置麦的帧 RMS 最高只有 0.018，而旧阈值
                        // 写死 `VOICE_RMS = 0.02` → 永远判不到「开口」→ 永远不 commit →
                        // 云端一个字都收不到，自然也没有回复；日志上却一切正常
                        // （只有「本地 VAD 判定人声」，没有「本地判定开口」）。
                        // 实测依据：5 分钟录音里最响的 1 秒 RMS=0.0182，底噪中位 0.00275。
                        //
                        // ⚠️ 2026-10-25 第二次踩坑：`LocalVad::push()` 返回的是
                        // **上升沿**（只在「刚开始说话」那一帧为 true），不是「这一帧有人声」。
                        // 拿它当「连续 3 帧有声」的判据永远不成立 —— 所以还要取
                        // `is_speaking()`（当前是否在说话，带 VAD 自己的静音挂起）。
                        // 两个都要：
                        //   - `vad_edge`：只有上升沿 → 打断（一次开口只触发一次）
                        //   - `vad_voice`：当前在说话 → 「说完」状态机
                        #[cfg(feature = "kws")]
                        let (vad_edge, vad_voice) = match vad.as_ref() {
                            Some(v) => {
                                let f32s: Vec<f32> =
                                    frame.iter().map(|&s| s as f32 / 32768.0).collect();
                                let edge = v.push(&f32s);
                                (edge, v.is_speaking())
                            }
                            None => {
                                let on = rms > VOICE_RMS;
                                (on, on)
                            }
                        };
                        #[cfg(not(feature = "kws"))]
                        let (vad_edge, vad_voice) = {
                            let on = rms > VOICE_RMS;
                            (on, on)
                        };
                        // ★ 本地 VAD 先停，再推流（顺序不能反）
                        #[cfg(feature = "kws")]
                        {
                            // 冷却：刚打断完 800ms 内不重复触发
                            if vad_cooldown > 0 {
                                vad_cooldown = vad_cooldown.saturating_sub(1);
                            } else if vad_edge {
                                let queued = player_in_push.queued_frames();
                                // 「回复还在进行」也要算打断 —— 只看播放队列会漏：
                                // 音频边播边收，队列可能刚好空，但云端这一轮还没结束。
                                // 2026-10-25 用户反馈：回复进行中又来新一句时，
                                // 旧实现是硬发 create_response，被服务端拒
                                // （`ongoing response already exists`）。
                                let responding = {
                                    let m = machine_in_push.lock().await;
                                    matches!(m.state(), OrbState::S3 | OrbState::S4)
                                };
                                // 单独打一行「VAD 判定人声」：验收脚本要用它量
                                // 「检测到人声 → 停播」的真实反应时间
                                // （虚拟音源注入到被采集之间还有几百毫秒，
                                //   那是环境延迟，不该算进产品反应里）。
                                tracing::debug!(queued, responding, "本地 VAD 判定人声");
                                if queued > 0 || responding {
                                    if queued > 0 {
                                        player_in_push.stop_now();
                                    }
                                    // ★ 修缺陷 #5：还要告诉云端别生成了。
                                    // 之前只停本地播放，云端继续跑 —— 浪费 token，
                                    // 而且缓存的音频会让它「刚停又接着说」。
                                    if let Err(e) = client_in_push.cancel_response() {
                                        tracing::debug!(error = %e, "取消云端生成失败（可忽略）");
                                    }
                                    let counted = count_interruption(
                                        &interruptions_in_push,
                                        &last_interrupt_in_push,
                                    );
                                    vad_cooldown = 40; // 40 帧 * 20ms = 800ms
                                    if counted {
                                        tracing::info!(
                                            queued,
                                            responding,
                                            "⚡ 打断：停播 + 取消当前回复（记一次打断）"
                                        );
                                        #[cfg(feature = "kws")]
                                        truncate_played(
                                            &client_in_push,
                                            &assist_push,
                                            &player_in_push,
                                        );
                                    } else {
                                        tracing::info!(
                                            queued,
                                            responding,
                                            "打断：同一次开口的重复信号（不重复计数）"
                                        );
                                    }
                                    // ⚠️ 这里**不能** block_on。
                                    //
                                    // 2026-10-25 真机首次验证打断时就炸了：
                                    // 这段代码跑在 async 任务（push_task）里，
                                    // `block_on` 会 panic：「Cannot start a runtime
                                    // from within a runtime」→ 整个推流任务死掉
                                    // → 麦克风不再推流，得等下一次建会话才恢复。
                                    // 改成 spawn 出去通知状态机。
                                    let m_for_vad = machine_in_push.clone();
                                    tauri::async_runtime::spawn(async move {
                                        let mut m = m_for_vad.lock().await;
                                        let _ = m.handle(Stimulus::UserSpeechStarted);
                                    });
                                }
                            }
                        }
                        // ★ 本地判定「说完了」→ 立刻 commit（这是 802ms 的关键）
                        if vad_voice {
                            voiced_run += 1;
                            silent_run = 0;
                            match talk {
                                Talk::Idle if voiced_run >= NEED_VOICE => {
                                    talk = Talk::Speaking;
                                    user_spoke_in_push.store(true, Ordering::Relaxed);
                                    tracing::debug!("本地判定开口");
                                }
                                Talk::Cooldown if voiced_run >= NEED_VOICE => {
                                    // 上一轮处理完了，用户又说了一句
                                    talk = Talk::Speaking;
                                    user_spoke_in_push.store(true, Ordering::Relaxed);
                                    tracing::debug!("本地判定新一句开口");
                                }
                                _ => {}
                            }
                        } else {
                            voiced_run = 0;
                            silent_run += 1;
                            match talk {
                                Talk::Speaking if silent_run >= NEED_SILENCE => {
                                    talk = Talk::Cooldown;
                                    // 兜底：如果这一轮回复还挂着（VAD 没来得及取消 /
                                    // 打断路径没走到），先取消再开新轮。
                                    // 硬发 create_response 会被服务端拒：
                                    // `ongoing response already exists`（真机日志抓到过）。
                                    // 用 `m_machine`（不是 kws 门控的 machine_in_push）：
                                    // Windows 的 `--no-default-features` 构建没有 kws，
                                    // 引用门控变量会编不过（CI 抓到过）。
                                    let still_responding = {
                                        let m = m_machine.lock().await;
                                        matches!(m.state(), OrbState::S3 | OrbState::S4)
                                    };
                                    if local_vad {
                                        // ⚠️ 服务端 VAD 模式下**不能**在这里取消 ——
                                        // 回合归服务端管，本地 VAD 只是观察者；
                                        // 在这里 cancel 会把刚触发的那一轮回复打死
                                        // （真机表现：它刚开口就被自己掐掉）。
                                        if still_responding {
                                            let _ = client2.cancel_response();
                                            tracing::info!(
                                                "⚡ 上一轮回复还没结束 → 先取消，再开新一轮（不重复计数，打断已在开口时记过）"
                                            );
                                        }
                                        // 本地 VAD 模式：关掉了服务端 VAD，回合结束得我们自己报。
                                        let _ = client2.commit();
                                        let _ = client2.create_response();
                                        // 看门狗起点：从这里开始算「等它开口」
                                        if let Ok(mut w) = turn_wait_push.lock() {
                                            *w = TurnWait {
                                                since: Some(std::time::Instant::now()),
                                                audio_seen: false,
                                                retries: 0,
                                            };
                                        }
                                        tracing::info!(
                                            cancelled_previous = still_responding,
                                            "本地判定说完 → commit + response.create"
                                        );
                                    } else {
                                        // 服务端 VAD 模式：append 音频本身就会触发推理
                                        // （阶跃文档：ServerVAD 模式下 append 会触发大模型推理），
                                        // 这里发 commit/response.create 会造成两轮回复。
                                        if let Ok(mut w) = turn_wait_push.lock() {
                                            *w = TurnWait {
                                                since: Some(std::time::Instant::now()),
                                                audio_seen: false,
                                                retries: 0,
                                            };
                                        }
                                        tracing::info!(
                                            cancelled_previous = still_responding,
                                            "本地判定说完（服务端 VAD 模式：让服务端自己触发）"
                                        );
                                    }
                                }
                                Talk::Cooldown if silent_run >= NEED_SILENCE * 3 => {
                                    // 静音够久（600ms），回到待命
                                    talk = Talk::Idle;
                                }
                                _ => {}
                            }
                        }
                        // 上行增益：服务端 VAD 认音量，太小就完全没反应（见上面注释）
                        let uplink_peak = if agc_enabled {
                            uplink_gain.process_i16(&mut frame)
                        } else {
                            0.0
                        };
                        if let Err(e) = client2.send_audio(&frame) {
                            tracing::warn!(error = %e, "推流失败，推流任务退出（此后麦克风不再上传）");
                            break;
                        }
                        // 诊断心跳：每 5 秒（250 帧 × 20ms）留一行，
                        // 能一眼看出「麦克风到底有没有帧 / 声音多响」。
                        frames_seen += 1;
                        if rms > peak_rms {
                            peak_rms = rms;
                        }
                        if frames_seen % 250 == 0 {
                            // `player_in_push` 只有 kws 构建才有（--no-default-features 没有播放队列句柄）
                            #[cfg(feature = "kws")]
                            let queued = player_in_push.queued_frames();
                            #[cfg(not(feature = "kws"))]
                            let queued = 0u64;
                            tracing::debug!(
                                frames = frames_seen,
                                peak_rms = format!("{peak_rms:.4}"),
                                uplink_peak = format!("{uplink_peak:.3}"),
                                agc = format!("{:.1}", uplink_gain.gain()),
                                queued,
                                "推流心跳"
                            );
                            peak_rms = 0.0;
                        }
                        // 静音 30s 自动退出（验收 F9）
                        if rms < 0.005 {
                            idle_frames += 1;
                            if idle_frames > 30 * 50 {
                                tracing::info!(
                                    frames = frames_seen,
                                    "30 秒没人说话 → 回待机（IdleTimeout）"
                                );
                                let mut m = m_machine.lock().await;
                                let _ = m.handle(Stimulus::IdleTimeout);
                                break;
                            }
                        } else {
                            idle_frames = 0;
                        }
                    }
                    _ => continue,
                }
            }
            tracing::debug!(frames = frames_seen, "推流任务退出");
        });

        // 事件 → 状态机 + 播放 + 打断
        let stop3 = stop.clone();
        let m2 = machine.clone();
        let player2 = player.clone();
        // 打断时要通知云端停止生成（修缺陷 #5）
        let c_ev = client.clone();
        let ev_task = tokio::spawn(async move {
            let mut speech_started: Option<std::time::Instant> = None;
            // ★ 兜底纠正：模型说「好，查着呢」却没真调工具 = 假装干活。
            //
            // 统计单位必须是**一次用户说话**，不能是「一次模型响应」。
            // 一个用户请求天然跨多个 response：
            //   用户说话 → response1(调 delegate_task) → 工具结果回灌 → response2(说「好，干完说」)
            // 如果每个 response 结束都清零，response2 就会被误判成「这轮没干活」，
            // 于是注入纠正 → 模型再调一次 delegate_task → 死循环。
            //
            // 2026-10-24 wings-gpd 真机实测踩过：一句「看看 README 第一行」
            // 派了 4 个任务、起了 4 个 DSH 会话、报了 4 遍。
            //
            // 另外：`TranscriptDelta` 是**模型自己说的话**
            // （协议里 `response.audio_transcript.delta`），
            // 以前当成「用户说了什么」用，判据从一开始就是错的 —— 别再用它判用户意图。
            let mut tool_calls_this_turn = 0usize;
            // 工具调用已经派出去、结果还没回灌的数量
            let mut pending_tool_results = 0usize;
            // 这次用户开口之后有没有处理过（`SpeechStarted` 置位）
            let mut user_spoke = false;
            // 本轮模型说的话（判断「像不像在装忙」）
            let mut assistant_reply = String::new();
            // ★ 用户原话（云端 ASR 转写，2026-10-25 才开始接）。
            //
            // 它的用途是**救场**：模型有时候嘴上说「好，查着呢」却不调工具，
            // 光靠再问它一次不一定管用。有了原话，我们就可以直接拿这句话
            // 去调 delegate_task —— 用户说了什么就派什么，不依赖模型自觉。
            let mut user_said = String::new();
            // 只认「整句」转写（completed）。实测 delta 是**累积快照**、
            // 而且会把环境噪声和上一句一起吐出来，拿它拼字会串味。
            let mut user_said_is_final = false;
            // 一次用户说话最多纠正一次，防止无限打断
            let mut correction_sent = false;
            // 诊断：这一轮回复的音频块数 / 采样数（排查「卡顿、被截断」用）
            let mut resp_chunks = 0u32;
            let mut resp_frames = 0u64;
            loop {
                if stop3.load(Ordering::Relaxed) {
                    break;
                }
                let ev = match tokio::time::timeout(Duration::from_millis(120), ev_rx.recv()).await
                {
                    Ok(Ok(e)) => e,
                    Ok(Err(_)) => break,
                    Err(_) => {
                        // ★ 首包看门狗（2026-10-25）：
                        //
                        // 用户最气的一个现象是「说完了它不回」。查下来这条路径
                        // **完全没有超时兜底**：commit + response.create 发出去后，
                        // 服务端如果因为 `ongoing response`、空缓冲区、网络抖动
                        // 没回音频，就永久静默 —— 日志里也什么都没有。
                        //
                        // 开源框架（LiveKit / pipecat）的通用做法：1.5–2 秒没首包
                        // 就 cancel + 重发一次，仍没有就报错让上层重连。
                        let retry = {
                            let mut w = match turn_wait_ev.lock() {
                                Ok(g) => g,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                            match w.since {
                                Some(t)
                                    if !w.audio_seen
                                        && w.retries < 1
                                        && t.elapsed() >= Duration::from_millis(2000) =>
                                {
                                    w.retries += 1;
                                    w.since = Some(std::time::Instant::now());
                                    true
                                }
                                _ => false,
                            }
                        };
                        if retry {
                            tracing::warn!(
                                local_vad,
                                "2 秒没等到回复音频 → 取消后重新触发一次（看门狗）"
                            );
                            if local_vad {
                                let _ = c_ev.cancel_response();
                            }
                            if let Err(e) = c_ev.create_response() {
                                tracing::warn!(error = %e, "看门狗重发 response.create 失败");
                            }
                        }
                        continue;
                    }
                };
                let mut m = m2.lock().await;

                // ★ 本地 VAD 判定「用户开口」时，云端不一定发 input_audio_buffer.speech_started
                // （我们是本地提前 commit 的）。原来只有云端那条事件才置 user_spoke，
                // 结果「用户真说了话、模型只回一句『好，查着呢』」时纠正机制完全不触发。
                //
                // 放在 match 之前合并（不是放在 ResponseDone 里）：这样这一轮模型说的话
                // 会累积到干净的缓冲区，ResponseDone 时才有依据判断「像不像在装忙」。
                if user_spoke_in_ev.swap(false, Ordering::Relaxed) && !user_spoke {
                    tracing::debug!("云端没发 SpeechStarted，按本地 VAD 判定算用户开口");
                    user_spoke = true;
                    tool_calls_this_turn = 0;
                    assistant_reply.clear();
                    correction_sent = false;
                }

                match ev {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        if first_audio.load(Ordering::Relaxed) == u64::MAX {
                            if let Some(t) = speech_started {
                                first_audio
                                    .store(t.elapsed().as_millis() as u64, Ordering::Relaxed);
                            }
                        }
                        // 诊断：服务端是「按实时推」还是「一口气突发推」——
                        // 2026-10-25 真机实测是后者（2.42s 音频 650ms 到齐），
                        // 当时的 500ms 播放环撑不住 → 卡顿截断。这里留痕便于复核。
                        resp_chunks += 1;
                        tracing::debug!(
                            chunk = resp_chunks,
                            pcm_samples = pcm.len(),
                            sample_rate,
                            "收到 TTS 音频块"
                        );
                        let n = player2.append_pcm16(&pcm, sample_rate);
                        resp_frames += n as u64;
                        played.fetch_add(n as u64, Ordering::Relaxed);
                        // 记「说完 → 首音频」的真实延迟（用户体验的核心指标）
                        {
                            let mut w = match turn_wait_ev.lock() {
                                Ok(g) => g,
                                Err(poisoned) => poisoned.into_inner(),
                            };
                            if !w.audio_seen {
                                w.audio_seen = true;
                                if let Some(t) = w.since {
                                    tracing::info!(
                                        wait_ms = t.elapsed().as_millis() as u64,
                                        local_vad,
                                        "首次音频到达（说完 → 它开口）"
                                    );
                                }
                            }
                        }
                        let _ = m.handle(Stimulus::ModelSpeaking);
                    }
                    RealtimeEvent::AssistantItem { item_id } => {
                        // 记下这条回复的 item_id（打断时 truncate 用）
                        let base = player2.frames_pulled();
                        if let Ok(mut g) = assist_ev.lock() {
                            *g = Some((item_id, base));
                        }
                    }
                    RealtimeEvent::SpeechStarted => {
                        // ★ barge-in：本地先停，不等云端确认
                        let queued = player2.queued_frames();
                        // 「回复还在进行」也算一次打断（可能是思考中还没出声）。
                        let responding = matches!(m.state(), OrbState::S3 | OrbState::S4);
                        player2.stop_now();
                        // ★ 修缺陷 #5：云端已经知道用户开口了，但当前这轮生成
                        // 可能还在继续。显式取消，别浪费 token。
                        if let Err(e) = c_ev.cancel_response() {
                            tracing::debug!(error = %e, "取消云端生成失败（可忽略）");
                        }
                        if queued > 0 || responding {
                            let counted = count_interruption(&interruptions, &last_interrupt_ms);
                            if counted {
                                tracing::info!(
                                    queued,
                                    responding,
                                    "⚡ 云端 SpeechStarted 打断（barge-in，记一次打断）"
                                );
                                truncate_played(&c_ev, &assist_ev, &player2);
                            } else {
                                tracing::debug!(
                                    queued,
                                    responding,
                                    "云端 SpeechStarted：同一次开口的重复信号（不重复计数）"
                                );
                            }
                        }
                        speech_started = Some(std::time::Instant::now());
                        // ★ 新的一次用户说话 → 重置「这一轮」的统计。
                        // 只有这里能重置，`ResponseDone` 不能（一个请求跨多个 response）。
                        user_spoke = true;
                        user_spoke_local.store(false, Ordering::Relaxed);
                        user_said.clear();
                        user_said_is_final = false;
                        tool_calls_this_turn = 0;
                        pending_tool_results = 0;
                        assistant_reply.clear();
                        correction_sent = false;
                        let _ = m.handle(Stimulus::UserSpeechStarted);
                    }
                    RealtimeEvent::SpeechStopped => {
                        speech_started = Some(std::time::Instant::now());
                    }
                    RealtimeEvent::UserTranscript { text, is_final } => {
                        if is_final {
                            // 整句：直接采用（云端会修正前文），并用它做判断依据。
                            // ★ 先本地纠正专有名词：ASR 会听错（morphofly→mosofly、
                            //   README→vimi），不纠正的话本地代派会把错字原样交给 DSH。
                            //   StepFun 不支持转写热词 prompt（探针实测），只能本地补。
                            let fixed = morphofly_core::asr_fix::fix_transcript(&text);
                            if fixed != text {
                                tracing::info!(
                                    before = %text,
                                    after = %fixed,
                                    "用户转写本地纠正（ASR 听错专有名词）"
                                );
                            }
                            user_said = fixed;
                            user_said_is_final = true;
                            tracing::debug!(said = %user_said, "用户转写（整句）");
                        } else {
                            // 增量：只降级到 trace。实测它是**累积快照**，
                            // 还会把环境噪声一起吐出来（"嗯嗯嗯。无效语音…"），
                            // 拼起来会污染判断，所以不参与逻辑。
                            tracing::trace!(delta = %text, "用户转写（增量）");
                        }
                    }
                    RealtimeEvent::TranscriptDelta { text } | RealtimeEvent::TextDelta { text } => {
                        // 这是**模型说的话**（`response.audio_transcript.delta` / `response.text.delta`），
                        // 不是用户说的。攒起来用于判断它像不像在装忙。
                        if assistant_reply.len() < 200 {
                            assistant_reply.push_str(&text);
                        }
                    }
                    RealtimeEvent::ResponseDone { .. } => {
                        // 这一轮结束了，看门狗归零（下一轮重新计时）
                        if let Ok(mut w) = turn_wait_ev.lock() {
                            *w = TurnWait::default();
                        }
                        // 还有工具结果没回灌 → 这个 response 只是「调工具那一步」，
                        // 不是最终答复，不计入「没干活」的判断。
                        if pending_tool_results > 0 {
                            pending_tool_results -= 1;
                            assistant_reply.clear();
                            continue;
                        }
                        // ★ 兜底：用户说过话、这条链路一个工具都没调、回复又像在装忙，
                        // 说明模型只是嘴上说说 → 注入一条系统提醒逼它真派活。
                        //
                        // 判据三个都要满足，缺一不可：
                        // 1. `user_spoke`：这次确实是用户开口触发的（不是主动汇报/系统注入）
                        // 2. `tool_calls_this_turn == 0`：本次用户说话以来一次工具都没调
                        // 3. `looks_like_action`：回复听起来在办事（闲聊不该被纠正）
                        //
                        // `correction_sent` 保证一次用户说话最多纠正一次，不会无限打断。
                        if tool_calls_this_turn == 0
                            && user_spoke
                            && !correction_sent
                            && morphofly_core::dispatch::looks_like_action(&assistant_reply)
                        {
                            tracing::warn!(
                                reply = %assistant_reply,
                                "这一轮没有调用任何工具 —— 模型只说了话没干活"
                            );

                            correction_sent = true;

                            // ★★ 首选：本地直接代派（不依赖模型自觉）。
                            //
                            // 用户说了什么我们已经有原话（云端转写），
                            // 模型嘴上说「好，查着呢」却不调工具 = 假装干活，
                            // 这时候与其再求它一次（实测时好时坏、还要多等 5 秒），
                            // 不如直接拿原话去调 delegate_task。
                            let said = if user_said_is_final {
                                user_said.trim().to_string()
                            } else {
                                String::new()
                            };
                            if said.len() >= 2 {
                                tracing::warn!(
                                    said = %said,
                                    reply = %assistant_reply,
                                    "模型只说不做 → 本地用用户原话代派"
                                );
                                let d = dispatcher.clone();
                                // 用「这句话里提到的项目」挑目录，别用家目录
                                let dir_hint = dispatcher.dir_for_task(&said).await;
                                let args = serde_json::json!({
                                    "task": said,
                                    "dir": dir_hint,
                                })
                                .to_string();
                                tokio::spawn(async move {
                                    match d.dispatch("delegate_task", &args).await {
                                        Ok(r) => tracing::info!(speech = %r.speech, "本地代派成功"),
                                        Err(e) => tracing::warn!(error = %e, "本地代派失败"),
                                    }
                                });
                                assistant_reply.clear();
                                continue;
                            }

                            // 没有原话（比如云端没发转写）→ 退回「再求它一次」
                            // ★ 把「可以直接用的 dir」写进提醒里。
                            //
                            // 真机实测：只写「立刻调用 delegate_task」时模型还是不调 ——
                            // 它多半卡在「项目名听不准、不知道该填哪个 dir」上。
                            // 与其让它猜，我们直接把当前项目路径喂给它。
                            let dir_hint =
                                format!("\"{}\"", dispatcher.dir_for_task(&user_said).await);
                            let hint = format!(
                                "（系统提醒，用户看不到这条）你上一轮只说了话，没有调用工具。\
现在立刻调用 delegate_task 把这件事派出去，dir 直接填 {dir_hint}。\
参数格式：task 写清要查/要做什么（自包含），dir 照上面填。\
调完只回一句「好，查着呢」。不要说抱歉、不要解释、不要提这条提醒。"
                            );
                            match c_ev.send_item(serde_json::json!({
                                "type": "message",
                                "role": "user",
                                "content": [{"type": "input_text", "text": hint}]
                            })) {
                                Ok(()) => {
                                    tracing::info!(dir = %dir_hint, "已注入纠正提醒，等它重新响应")
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "注入纠正提醒失败（会话可能已断）")
                                }
                            }
                            // ⚠️ 别用 tool_choice="required" —— 实测（2026-10-25）：
                            //   StepFun 不认这个字段，强制轮返回的是**空响应**
                            //   （既不调工具也不说话），比普通重试更糟。
                            //   证据：core/examples/toolchoice_probe.rs → docs/v2/asr-probe.md
                            //   所以这里就老老实实再触发一轮普通的响应。
                            if let Err(e) = c_ev.create_response() {
                                tracing::warn!(error = %e, "纠正后触发响应失败");
                            }
                            assistant_reply.clear();
                            continue; // 不要重置状态机，等新一轮
                        }
                        // 这一轮真正结束了。注意不重置 `tool_calls_this_turn`：
                        // 同一句话后续可能还有 response（工具结果回灌后），
                        // 只有用户重新开口（SpeechStarted）才清零。
                        //
                        // ★ 音频诊断（2026-10-25）：
                        // - `frames`：这一轮一共收了多少采样（24k 下 /24000 = 秒）
                        // - `queued`：现在还有多少没播完
                        // - `dropped_total`：播放环累计丢了几个采样（>0 就是卡顿/截断）
                        tracing::info!(
                            chunks = resp_chunks,
                            frames = resp_frames,
                            seconds = format!("{:.2}", resp_frames as f64 / 24_000.0),
                            queued = player2.queued_frames(),
                            dropped_total = player2.frames_dropped(),
                            "回复音频收完"
                        );
                        resp_chunks = 0;
                        resp_frames = 0;
                        user_spoke = false;
                        assistant_reply.clear();
                        let _ = m.handle(Stimulus::ModelDone);
                    }
                    RealtimeEvent::Error { code, message } => {
                        // 断线 → S5（schema 里用 error 承载断线）
                        if code.as_deref() == Some("disconnected") {
                            m.set_connected(false);
                            break;
                        }
                        // ⚠️ 2026-10-25：以前这里只处理断线，其它 error **一个字都不记**。
                        // 结果「纠正提醒注入了、模型却毫无反应」这种现象完全查不出来 ——
                        // 分不清是没发出去、发了被拒、还是在慢慢生成。
                        tracing::warn!(code = ?code, message = %message, "实时语音服务端返回 error");
                    }
                    RealtimeEvent::ToolCall {
                        call_id,
                        name,
                        arguments,
                    } => {
                        tool_calls_this_turn += 1;
                        pending_tool_results += 1;
                        tracing::info!(%name, %arguments, "🔧 工具调用");
                        let _ = m.handle(Stimulus::Thinking);
                        // 分发到真实动作（派活给 DSH / 查进度 / 取消 / 追加）
                        let d = dispatcher.clone();
                        let c = client_for_tools.clone();
                        let n = name.clone();
                        let a = arguments.clone();
                        let cid = call_id.clone();
                        tokio::spawn(async move {
                            let speech = match d.dispatch(&n, &a).await {
                                Ok(r) => {
                                    tracing::info!(speech = %r.speech, "工具结果");
                                    r.speech
                                }
                                Err(e) => {
                                    // {e:#} = 完整因果链。只打顶层消息查不出「建会话失败」的真因
                                    tracing::warn!(error = %format!("{e:#}"), "工具执行失败");
                                    format!("这个没办成：{e:#}")
                                }
                            };
                            let _ = c.send_function_call_output(&cid, &speech);
                            let _ = c.create_response();
                        });
                    }
                    _ => {}
                }
            }
            // ★ 事件循环为什么退出必须留痕（2026-10-25）：
            // 之前「断线」只体现在状态机悄悄跳 S5 上，日志里看不出来，
            // 于是「会话刚连上就结束」查不到原因。
            tracing::info!(
                stop_flag = stop3.load(Ordering::Relaxed),
                "事件循环退出（收到停止或事件通道关闭）"
            );
        });

        // 主动汇报：轮询任务表，任务完成就主动开口（验收 F7）
        //
        // ⚠️ 2026-10-24 真机踩坑：原来用内存里的 HashSet 记「哪些念过了」，
        // App 一重启就全丢 —— 实测重启后把 4 个历史任务的结果又从头念了 4 遍。
        // 现在改成任务表里的 `reported` 字段，跟着 JSON 一起落盘。
        let store_rep = store.clone();
        let client_rep = client.clone();
        let stop_rep = stop.clone();
        let report_task = tokio::spawn(async move {
            loop {
                if stop_rep.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                let pending = {
                    let s = store_rep.lock().await;
                    s.unreported_finished()
                        .iter()
                        .map(|t| {
                            (
                                t.id.clone(),
                                t.status.spoken().to_string(),
                                t.summary.clone(),
                            )
                        })
                        .collect::<Vec<_>>()
                };
                for (id, spoken, summary) in pending {
                    // 先记账再开口：宁可漏报一次，也不能重复报
                    {
                        let mut s = store_rep.lock().await;
                        if let Err(e) = s.mark_reported(&id) {
                            tracing::warn!(task = %id, error = %e, "标记已汇报失败，跳过");
                            continue;
                        }
                    }
                    let line = format!(
                        "{spoken}：{}",
                        morphofly_core::dispatch::clean_for_speech(&summary)
                    );
                    tracing::info!(task = %id, report = %line, "🔔 主动汇报");
                    // 注入会话 + 触发开口
                    let _ = client_rep.send_item(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text":
                            format!("（系统通知，不是用户说的话）后台任务有新结果，请用一句话转告我：{line}")}]
                    }));
                    let _ = client_rep.create_response();
                }
            }
        });

        // 主循环：等状态离开通话，或用户点击退出
        loop {
            tokio::time::sleep(Duration::from_millis(200)).await;
            let (st, transitions) = {
                let m = machine.lock().await;
                (m.state(), m.transitions)
            };
            let stop_flag = stop.load(Ordering::Relaxed);
            if !st.is_active_call() || stop_flag {
                // ★ 退出原因必须留痕。
                //
                // 2026-10-25 真机：会话刚连上 1.6 秒就「会话结束」，
                // 日志里没有断线、没有报错、也没有状态迁移记录 ——
                // 完全看不出是谁结束的。这里把两个判据都打出来。
                tracing::info!(
                    state = st.as_str(),
                    stop_flag,
                    transitions,
                    "会话主循环退出（离开通话态或收到停止）"
                );
                break;
            }
        }

        stop.store(true, Ordering::Relaxed);
        let _ = tokio::time::timeout(Duration::from_secs(2), push_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), ev_task).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), report_task).await;
        self.player.stop_now();
        Ok(())
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.player.stop_now();
        let _ = self.mic.stop();
        self.client.close();
    }
}

/// 会话统计（给 UI / 验收用）
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionStats {
    pub played_frames: u64,
    pub interruptions: u64,
    pub first_audio_ms: Option<u64>,
}

impl VoiceSession {
    pub fn stats(&self) -> SessionStats {
        let fa = self.first_audio_ms.load(Ordering::Relaxed);
        SessionStats {
            played_frames: self.played_frames.load(Ordering::Relaxed),
            interruptions: self.interruptions.load(Ordering::Relaxed),
            first_audio_ms: if fa == u64::MAX { None } else { Some(fa) },
        }
    }
}

/// 状态机 → 是否该建/拆会话（纯逻辑，便于测试）
pub fn should_open_session(state: OrbState) -> bool {
    // 唤醒后连云端（S1 → S2 由 Connected 刺激完成）
    state == OrbState::S1
}

#[allow(dead_code)] // 有单测覆盖，生产路径暂时只在循环里内联判断

/// 纠正提醒里要塞的「dir 填什么」。
///
/// 优先用当前会话上下文（set_context 设过的），否则用默认目录。
/// 拿不到就退回一句「按已知项目表填」，别编路径。
async fn tool_dir_hint(dispatcher: &Arc<Dispatcher>) -> String {
    let d = dispatcher.current_dir().await;
    if d.is_empty() {
        "已知项目表里的路径（不知道就不填）".to_string()
    } else {
        format!("\"{d}\"")
    }
}

pub fn should_close_session(state: OrbState) -> bool {
    !state.is_active_call()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_opens_only_at_wake() {
        assert!(should_open_session(OrbState::S1));
        for s in [
            OrbState::S0,
            OrbState::S2,
            OrbState::S3,
            OrbState::S4,
            OrbState::S5,
        ] {
            assert!(!should_open_session(s), "{s:?} 不该新建会话");
        }
    }

    #[test]
    fn session_closes_when_leaving_call() {
        assert!(should_close_session(OrbState::S0));
        assert!(should_close_session(OrbState::S5));
        for s in [OrbState::S1, OrbState::S2, OrbState::S3, OrbState::S4] {
            assert!(!should_close_session(s), "{s:?} 还在通话中");
        }
    }

    #[test]
    fn stats_serializes() {
        let s = SessionStats {
            played_frames: 100,
            interruptions: 1,
            first_audio_ms: Some(429),
        };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"first_audio_ms\":429"));
    }
}
