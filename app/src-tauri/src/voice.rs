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
use morphofly_core::dsh::DshBridge;
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
}

impl VoiceSession {
    /// 建立会话：连云端 + 开麦克风 + 开播放器
    pub async fn open(cfg: &Config) -> anyhow::Result<Self> {
        let client = Arc::new(RealtimeClient::connect(cfg).await?);
        client.send_tools(&tool_defs())?;
        let player = Arc::new(AudioPlayer::open()?);
        let mic = Arc::new(AudioCapture::start_default()?);
        // 工具分发器：语音模型调 delegate_task → 这里转交给 DSH
        morphofly_core::config::ensure_data_dir(cfg)?;
        let store = Arc::new(Mutex::new(TaskStore::open(cfg.task_store_path())?));
        let bridge = Arc::new(DshBridge::from_config(cfg));
        let dispatcher = Arc::new(Dispatcher::new(
            store.clone(),
            bridge,
            cfg.dsh.default_cwd.clone(),
        ));
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
            // ★ 本地 VAD：两个用途
            //   1. 打断（用户说话 → 立刻停播）
            //   2. 判定「说完了」→ 立刻 commit（不等服务端 VAD 的 1.95s）
            #[cfg(feature = "kws")]
            let vad =
                morphofly_core::vad::LocalVad::new(&morphofly_core::vad::VadConfig::default()).ok();
            #[cfg(feature = "kws")]
            let mut vad_cooldown = 0u32;
            // 「说完」判定（本地 VAD 的简化版）。
            //
            // 状态机：
            //   Idle     --有声--> Speaking
            //   Speaking --连续 K 帧静音--> 发 commit + create_response --> Cooldown
            //   Cooldown --连续 M 帧静音--> Idle（等下一轮开口）
            //
            // ⚠️ 2026-10-24 踩坑：最初写成「只要 rms>0.02 就重置 committed 标志」，
            // 结果环境底噪（风扇/键盘）让标志反复重置，一次发言触发 4 次 commit。
            // 现在用「必须连续有声才算重新开口」——底噪过不了连续 N 帧这条线。
            #[derive(PartialEq)]
            enum Talk {
                Idle,
                Speaking,
                Cooldown,
            }
            let mut talk = Talk::Idle;
            let mut voiced_run = 0u32; // 连续有声帧数
            let mut silent_run = 0u32; // 连续静音帧数
            const VOICE_RMS: f32 = 0.02; // 有声阈值
            const NEED_VOICE: u32 = 3; // 连续 3 帧（60ms）才算真开口
            const NEED_SILENCE: u32 = 10; // 连续 10 帧（200ms）才算说完
            loop {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Ok(frame)) => {
                        let rms = morphofly_core::spectrum::rms_i16(&frame);
                        let bins = spectrum.analyze_i16(&frame);
                        on_spec(bins, morphofly_core::spectrum::rms_to_visual(rms));
                        // ★ 本地 VAD 先停，再推流（顺序不能反）
                        #[cfg(feature = "kws")]
                        {
                            if let Some(v) = vad.as_ref() {
                                let f32s: Vec<f32> =
                                    frame.iter().map(|&s| s as f32 / 32768.0).collect();
                                // 冷却：刚打断完 800ms 内不重复触发
                                if vad_cooldown > 0 {
                                    vad_cooldown = vad_cooldown.saturating_sub(1);
                                } else if v.push(&f32s) {
                                    let queued = player_in_push.queued_frames();
                                    if queued > 0 {
                                        player_in_push.stop_now();
                                        // ★ 修缺陷 #5：还要告诉云端别生成了。
                                        // 之前只停本地播放，云端继续跑 —— 浪费 token，
                                        // 而且缓存的音频会让它「刚停又接着说」。
                                        if let Err(e) = client_in_push.cancel_response() {
                                            tracing::debug!(error = %e, "取消云端生成失败（可忽略）");
                                        }
                                        interruptions_in_push.fetch_add(1, Ordering::Relaxed);
                                        vad_cooldown = 40; // 40 帧 * 20ms = 800ms
                                        tracing::info!(
                                            queued,
                                            "⚡ 本地 VAD 打断（已通知云端停止生成）"
                                        );
                                        tauri::async_runtime::block_on(async {
                                            let mut m = machine_in_push.lock().await;
                                            let _ = m.handle(Stimulus::UserSpeechStarted);
                                        });
                                    }
                                }
                            }
                        }
                        // ★ 本地判定「说完了」→ 立刻 commit（这是 802ms 的关键）
                        if rms > VOICE_RMS {
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
                                    let _ = client2.commit();
                                    let _ = client2.create_response();
                                    tracing::info!("本地判定说完 → commit");
                                }
                                Talk::Cooldown if silent_run >= NEED_SILENCE * 3 => {
                                    // 静音够久（600ms），回到待命
                                    talk = Talk::Idle;
                                }
                                _ => {}
                            }
                        }
                        if let Err(_) = client2.send_audio(&frame) {
                            break;
                        }
                        // 静音 30s 自动退出（验收 F9）
                        if rms < 0.005 {
                            idle_frames += 1;
                            if idle_frames > 30 * 50 {
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
            // 一次用户说话最多纠正一次，防止无限打断
            let mut correction_sent = false;
            loop {
                if stop3.load(Ordering::Relaxed) {
                    break;
                }
                let ev = match tokio::time::timeout(Duration::from_millis(120), ev_rx.recv()).await
                {
                    Ok(Ok(e)) => e,
                    Ok(Err(_)) => break,
                    Err(_) => continue,
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
                        let n = player2.append_pcm16(&pcm, sample_rate);
                        played.fetch_add(n as u64, Ordering::Relaxed);
                        let _ = m.handle(Stimulus::ModelSpeaking);
                    }
                    RealtimeEvent::SpeechStarted => {
                        // ★ barge-in：本地先停，不等云端确认
                        let queued = player2.queued_frames();
                        player2.stop_now();
                        // ★ 修缺陷 #5：云端已经知道用户开口了，但当前这轮生成
                        // 可能还在继续。显式取消，别浪费 token。
                        if let Err(e) = c_ev.cancel_response() {
                            tracing::debug!(error = %e, "取消云端生成失败（可忽略）");
                        }
                        if queued > 0 {
                            interruptions.fetch_add(1, Ordering::Relaxed);
                        }
                        speech_started = Some(std::time::Instant::now());
                        // ★ 新的一次用户说话 → 重置「这一轮」的统计。
                        // 只有这里能重置，`ResponseDone` 不能（一个请求跨多个 response）。
                        user_spoke = true;
                        user_spoke_local.store(false, Ordering::Relaxed);
                        user_said.clear();
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
                        // 增量就往后追加；整句就整句替换（云端可能修正前文）
                        if is_final {
                            user_said = text;
                        } else {
                            user_said.push_str(&text);
                        }
                        tracing::debug!(final_ = is_final, said = %user_said, "用户转写");
                    }
                    RealtimeEvent::TranscriptDelta { text } | RealtimeEvent::TextDelta { text } => {
                        // 这是**模型说的话**（`response.audio_transcript.delta` / `response.text.delta`），
                        // 不是用户说的。攒起来用于判断它像不像在装忙。
                        if assistant_reply.len() < 200 {
                            assistant_reply.push_str(&text);
                        }
                    }
                    RealtimeEvent::ResponseDone { .. } => {
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
                            let said = user_said.trim().to_string();
                            if said.len() >= 2 {
                                tracing::warn!(
                                    said = %said,
                                    reply = %assistant_reply,
                                    "模型只说不做 → 本地用用户原话代派"
                                );
                                let d = dispatcher.clone();
                                let dir_hint = tool_dir_hint(&dispatcher).await;
                                let dir_hint = dir_hint.trim_matches('"').to_string();
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
                            let dir_hint = tool_dir_hint(&dispatcher).await;
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
                                    tracing::warn!(error = %e, "工具执行失败");
                                    format!("这个没办成：{e}")
                                }
                            };
                            let _ = c.send_function_call_output(&cid, &speech);
                            let _ = c.create_response();
                        });
                    }
                    _ => {}
                }
            }
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
            let st = { machine.lock().await.state() };
            if !st.is_active_call() || stop.load(Ordering::Relaxed) {
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
