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
                                        tracing::info!(queued, "⚡ 本地 VAD 打断（已通知云端停止生成）");
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
                                    tracing::debug!("本地判定开口");
                                }
                                Talk::Cooldown if voiced_run >= NEED_VOICE => {
                                    // 上一轮处理完了，用户又说了一句
                                    talk = Talk::Speaking;
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
                        let _ = m.handle(Stimulus::UserSpeechStarted);
                    }
                    RealtimeEvent::SpeechStopped => {
                        speech_started = Some(std::time::Instant::now());
                    }
                    RealtimeEvent::ResponseDone { .. } => {
                        let _ = m.handle(Stimulus::ModelDone);
                    }
                    RealtimeEvent::Error { code, .. } => {
                        // 断线 → S5（schema 里用 error 承载断线）
                        if code.as_deref() == Some("disconnected") {
                            m.set_connected(false);
                            break;
                        }
                    }
                    RealtimeEvent::ToolCall {
                        call_id,
                        name,
                        arguments,
                    } => {
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
        let store_rep = store.clone();
        let client_rep = client.clone();
        let stop_rep = stop.clone();
        let reported: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>> =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));
        let report_task = tokio::spawn(async move {
            loop {
                if stop_rep.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
                let pending = {
                    let s = store_rep.lock().await;
                    let already = reported.lock().await;
                    s.all()
                        .iter()
                        .filter(|t| t.status.is_terminal() && !already.contains(&t.id))
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
                    reported.lock().await.insert(id.clone());
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
