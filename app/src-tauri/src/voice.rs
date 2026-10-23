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

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::audio::playback::AudioPlayer;
use diemeng_core::realtime::{RealtimeClient, RealtimeEvent};
use diemeng_core::session::{OrbState, SessionMachine, Stimulus};
use diemeng_core::spectrum::Spectrum;
use diemeng_core::tools::tool_defs;
use diemeng_core::Config;
use tokio::sync::Mutex;

/// 语音会话（一次唤醒 = 一个会话）
pub struct VoiceSession {
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
        Ok(Self {
            client,
            player,
            mic,
            stop: Arc::new(AtomicBool::new(false)),
            played_frames: Arc::new(AtomicU64::new(0)),
            interruptions: Arc::new(AtomicU64::new(0)),
            first_audio_ms: Arc::new(AtomicU64::new(u64::MAX)),
        })
    }

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

        let mut ev_rx = client.events();

        // 采集 → 推流 + 频谱
        let mic2 = mic.clone();
        let client2 = client.clone();
        let stop2 = stop.clone();
        let m_machine = machine.clone();
        let on_spec = on_spectrum.clone();
        let push_task = tokio::spawn(async move {
            let mut rx = mic2.subscribe();
            let mut spectrum = Spectrum::with_default_window();
            let mut idle_frames = 0u64;
            loop {
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                    Ok(Ok(frame)) => {
                        let rms = diemeng_core::spectrum::rms_i16(&frame);
                        let bins = spectrum.analyze_i16(&frame);
                        on_spec(bins, diemeng_core::spectrum::rms_to_visual(rms));
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
        let ev_task = tokio::spawn(async move {
            let mut speech_started: Option<std::time::Instant> = None;
            loop {
                if stop3.load(Ordering::Relaxed) {
                    break;
                }
                let ev = match tokio::time::timeout(Duration::from_millis(120), ev_rx.recv()).await {
                    Ok(Ok(e)) => e,
                    Ok(Err(_)) => break,
                    Err(_) => continue,
                };
                let mut m = m2.lock().await;
                match ev {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        if first_audio.load(Ordering::Relaxed) == u64::MAX {
                            if let Some(t) = speech_started {
                                first_audio.store(
                                    t.elapsed().as_millis() as u64,
                                    Ordering::Relaxed,
                                );
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
                    RealtimeEvent::ToolCall { name, arguments, .. } => {
                        tracing::info!(%name, %arguments, "工具调用");
                        let _ = m.handle(Stimulus::Thinking);
                    }
                    _ => {}
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

pub fn should_close_session(state: OrbState) -> bool {
    !state.is_active_call()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_opens_only_at_wake() {
        assert!(should_open_session(OrbState::S1));
        for s in [OrbState::S0, OrbState::S2, OrbState::S3, OrbState::S4, OrbState::S5] {
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
        let s = SessionStats { played_frames: 100, interruptions: 1, first_audio_ms: Some(429) };
        let j = serde_json::to_string(&s).unwrap();
        assert!(j.contains("\"first_audio_ms\":429"));
    }
}
