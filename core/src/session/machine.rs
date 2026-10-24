//! 会话状态机（六态）。纯逻辑，无 IO，可完整单测。
//!
//! 硬约束（docs/开发方案.md §3）：
//! - S0 → S1 只由 KWS 或点击触发
//! - S1 → S2 必须先连上云端
//! - S4 时用户说话 → 立即回 S2（打断是核心体验）
//! - S2 静音 30s → 回 S0
//! - 任何态 → S5 只由连接错误触发

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// 小球状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrbState {
    /// S0 待机：暗、小、慢呼吸
    S0,
    /// S1 唤醒：亮脉冲 + 光环
    S1,
    /// S2 聆听：粒子绕流
    S2,
    /// S3 思考：粒子内旋
    S3,
    /// S4 说话：粒子外扩
    S4,
    /// S5 异常：转暗、散落
    S5,
}

impl OrbState {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrbState::S0 => "S0",
            OrbState::S1 => "S1",
            OrbState::S2 => "S2",
            OrbState::S3 => "S3",
            OrbState::S4 => "S4",
            OrbState::S5 => "S5",
        }
    }

    /// 给用户看的短标签（字幕/悬浮提示用）
    pub fn label(&self) -> &'static str {
        match self {
            OrbState::S0 => "待机",
            OrbState::S1 => "在",
            OrbState::S2 => "听你说",
            OrbState::S3 => "想一下",
            OrbState::S4 => "说话中",
            OrbState::S5 => "连不上",
        }
    }

    /// 前端渲染参数：(发光强度, 半径倍数, 粒子速度)
    pub fn visual(&self) -> (f32, f32, f32) {
        match self {
            OrbState::S0 => (0.18, 0.82, 0.25),
            OrbState::S1 => (0.85, 1.12, 1.20),
            OrbState::S2 => (0.62, 1.00, 0.75),
            OrbState::S3 => (0.70, 0.92, 1.60),
            OrbState::S4 => (0.92, 1.15, 1.05),
            OrbState::S5 => (0.10, 0.70, 0.10),
        }
    }

    pub fn is_active_call(&self) -> bool {
        matches!(
            self,
            OrbState::S1 | OrbState::S2 | OrbState::S3 | OrbState::S4
        )
    }
}

/// 外部刺激
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stimulus {
    /// 用户点击小球
    Click,
    /// 唤醒词命中
    WakeWord,
    /// WS 连上了
    Connected,
    /// WS 连不上/断开/API 失败
    ConnectionFailed,
    /// 语音模型开始说话
    ModelSpeaking,
    /// 语音模型说完
    ModelDone,
    /// 工具调用中（DSH 还没回）
    Thinking,
    /// 云端检测到用户开口（barge-in）
    UserSpeechStarted,
    /// 用户说完
    UserSpeechEnded,
    /// 长时间没声音
    IdleTimeout,
    /// 从异常态手动恢复
    Recover,
    /// 有后台任务结果要念，主动开一轮
    ///
    /// 和 Click/WakeWord 的区别只在语义和日志（都是 S0 → S1）：
    /// 这句是「系统主动开口」，不是用户叫醒的。
    ProactiveReport,
}

/// 拒绝迁移的理由
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// 这条规则不允许
    Illegal { from: OrbState, stim: Stimulus },
    /// 云端还没连上，不能进聆听态
    NotConnected,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::Illegal { from, stim } => {
                write!(f, "非法迁移：{:?} 遇到 {:?}", from, stim)
            }
            TransitionError::NotConnected => write!(f, "云端未连接，不能进入聆听态"),
        }
    }
}

/// 会话状态机
#[derive(Debug)]
pub struct SessionMachine {
    state: OrbState,
    connected: bool,
    idle_timeout: Duration,
    last_activity: Instant,
    /// 状态变更次数（调试用）
    pub transitions: u64,
    /// 是否曾经成功连上过云端（区分「初始未连」和「连过又断」）
    ever_connected: bool,
}

impl SessionMachine {
    pub fn new(idle_timeout: Duration) -> Self {
        Self {
            state: OrbState::S0,
            connected: false,
            idle_timeout,
            last_activity: Instant::now(),
            transitions: 0,
            ever_connected: false,
        }
    }

    pub fn state(&self) -> OrbState {
        self.state
    }

    /// 强制回到待机（会话结束时用）。
    ///
    /// 为什么需要：会话结束（云端超时断开、服务端主动关）时，
    /// 状态机可能停在 S2/S3/S4 —— 原来的 `IdleTimeout` 只在**推音频的循环里**发，
    /// 会话一断那个循环就退出了，于是状态永远回不到 S0。
    /// 后果很实在：会话守卫只在 S0 才检查「有没有结果要念」，
    /// 于是「干完主动汇报」整个机制静默失效（真机实测：DSH 干完了，
    /// 任务表里 reported 一直是 false，日志里连一句相关记录都没有）。
    pub fn force_idle(&mut self) {
        self.connected = false;
        self.goto(OrbState::S0);
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// 更新连接状态。
    ///
    /// 断线时：通话中（S1–S4）→ S5；**待机（S0）也要进 S5**。
    /// 2026-10-24 审计指出：原实现只在 `is_active_call()` 时跳 S5，
    /// 导致「待机时连不上云端」完全没反馈，用户以为软件坏了却没任何提示。
    /// 待机态的唯一例外：从未尝试过连接时不算异常（`!ever_connected`）。
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        if connected {
            self.ever_connected = true;
            return;
        }
        // 断线：除了「还没连过」的初始态，一律进异常
        if (self.ever_connected || self.state.is_active_call()) && self.state != OrbState::S5 {
            self.state = OrbState::S5;
            self.transitions += 1;
        }
    }

    pub fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    fn goto(&mut self, next: OrbState) {
        if self.state != next {
            self.state = next;
            self.transitions += 1;
        }
        self.last_activity = Instant::now();
    }

    /// 处理一个刺激，返回是否发生了迁移。
    pub fn handle(&mut self, stim: Stimulus) -> Result<bool, TransitionError> {
        use OrbState::*;
        let from = self.state;
        let next = match (from, stim) {
            // 待机 → 唤醒：只由点击/唤醒词触发
            (S0, Stimulus::Click) | (S0, Stimulus::WakeWord) | (S0, Stimulus::ProactiveReport) => {
                S1
            }
            // 唤醒态：连接成功才能进聆听
            (S1, Stimulus::Connected) => {
                self.connected = true;
                S2
            }
            (S1, Stimulus::ConnectionFailed) => S5,
            (S1, Stimulus::Click) => S0, // 再点一下取消
            (S1, Stimulus::WakeWord) => S1,

            // 聆听 + 思考 + 说话：用户开口立刻回聆听（打断）
            (S2, Stimulus::UserSpeechStarted)
            | (S3, Stimulus::UserSpeechStarted)
            | (S4, Stimulus::UserSpeechStarted) => S2,

            (S2, Stimulus::Thinking) => S3,
            (S2, Stimulus::ModelSpeaking) => S4,
            (S3, Stimulus::ModelSpeaking) => S4,
            (S4, Stimulus::ModelDone) => S2,
            (S3, Stimulus::ModelDone) => S2,

            (S2, Stimulus::IdleTimeout) => S0,
            (S2, Stimulus::Click) | (S3, Stimulus::Click) | (S4, Stimulus::Click) => S0,

            (_, Stimulus::ConnectionFailed) => S5,

            (S5, Stimulus::Click) | (S5, Stimulus::Recover) => {
                if self.connected {
                    S2
                } else {
                    S0
                }
            }

            _ => return Err(TransitionError::Illegal { from, stim }),
        };
        let changed = next != from;
        self.goto(next);
        Ok(changed)
    }

    /// 时间驱动的迁移：静音超时回待机。
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.state == OrbState::S2 && now.duration_since(self.last_activity) >= self.idle_timeout
        {
            self.goto(OrbState::S0);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> SessionMachine {
        SessionMachine::new(Duration::from_secs(30))
    }

    #[test]
    fn starts_idle() {
        assert_eq!(m().state(), OrbState::S0);
    }

    /// 任务干完了但会话关着 → 系统主动开一轮（S0 → S1）。
    /// 会话结束时必须能强制回到待机（否则主动汇报的守卫永远看不到 S0）。
    #[test]
    fn force_idle_returns_from_any_state() {
        let mut m = m();
        m.handle(Stimulus::WakeWord).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        assert_eq!(m.state(), OrbState::S2);
        m.force_idle();
        assert_eq!(m.state(), OrbState::S0);
        assert!(!m.is_connected());
    }

    #[test]
    fn proactive_report_wakes_from_idle() {
        let mut m = m();
        assert_eq!(m.state(), OrbState::S0);
        assert!(m.handle(Stimulus::ProactiveReport).unwrap());
        assert_eq!(m.state(), OrbState::S1);
    }

    #[test]
    fn idle_to_wake() {
        let mut m = m();
        assert!(m.handle(Stimulus::WakeWord).unwrap());
        assert_eq!(m.state(), OrbState::S1);
    }

    #[test]
    fn wake_needs_connection_before_listening() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        // 没连上就 Nothing（Thoughting 不应进）
        assert!(m.handle(Stimulus::Thinking).is_err());
        assert_eq!(m.state(), OrbState::S1);
        m.handle(Stimulus::Connected).unwrap();
        assert_eq!(m.state(), OrbState::S2);
        assert!(m.is_connected());
    }

    #[test]
    fn s4_user_speech_interrupts_back_to_s2() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        m.handle(Stimulus::ModelSpeaking).unwrap();
        assert_eq!(m.state(), OrbState::S4);
        m.handle(Stimulus::UserSpeechStarted).unwrap();
        assert_eq!(
            m.state(),
            OrbState::S2,
            "打断是核心体验：S4 听到人声立即回 S2"
        );
    }

    #[test]
    fn s3_user_speech_also_interrupts() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        m.handle(Stimulus::Thinking).unwrap();
        assert_eq!(m.state(), OrbState::S3);
        m.handle(Stimulus::UserSpeechStarted).unwrap();
        assert_eq!(m.state(), OrbState::S2);
    }

    #[test]
    fn idle_timeout_returns_to_standby() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        assert_eq!(m.state(), OrbState::S2);
        let past = Instant::now() + Duration::from_secs(31);
        assert!(m.tick(past));
        assert_eq!(m.state(), OrbState::S0);
    }

    #[test]
    fn tick_does_not_fire_early() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        assert!(!m.tick(Instant::now() + Duration::from_secs(5)));
        assert_eq!(m.state(), OrbState::S2);
    }

    #[test]
    fn connection_failure_from_any_state_goes_s5() {
        for pre in [
            OrbState::S0,
            OrbState::S1,
            OrbState::S2,
            OrbState::S3,
            OrbState::S4,
        ] {
            let mut m = m();
            match pre {
                OrbState::S0 => {}
                OrbState::S1 => {
                    m.handle(Stimulus::Click).unwrap();
                }
                OrbState::S2 => {
                    m.handle(Stimulus::Click).unwrap();
                    m.handle(Stimulus::Connected).unwrap();
                }
                OrbState::S3 => {
                    m.handle(Stimulus::Click).unwrap();
                    m.handle(Stimulus::Connected).unwrap();
                    m.handle(Stimulus::Thinking).unwrap();
                }
                OrbState::S4 => {
                    m.handle(Stimulus::Click).unwrap();
                    m.handle(Stimulus::Connected).unwrap();
                    m.handle(Stimulus::ModelSpeaking).unwrap();
                }
                _ => unreachable!(),
            }
            // 注意 S0 收到 ConnectionFailed 也会进 S5（连接错误统一异常态）
            m.handle(Stimulus::ConnectionFailed).unwrap();
            assert_eq!(m.state(), OrbState::S5, "pre={:?}", pre);
        }
    }

    /// 审计发现的缺口：待机时断线也必须进 S5（否则用户看不到任何故障提示）
    #[test]
    fn disconnect_while_idle_also_goes_s5_after_first_connect() {
        let mut m = m();
        // 连上过再断 → 必须 S5
        m.set_connected(true);
        assert!(m.ever_connected);
        m.set_connected(false);
        assert_eq!(m.state(), OrbState::S5, "待机时断线也要进异常态");

        let mut m2 = SessionMachine::new(Duration::from_secs(30));
        m2.set_connected(true);
        m2.handle(Stimulus::Click).unwrap(); // S1
        m2.handle(Stimulus::Connected).unwrap(); // S2
        m2.handle(Stimulus::Click).unwrap(); // 回 S0
        assert_eq!(m2.state(), OrbState::S0);
        m2.set_connected(false);
        assert_eq!(m2.state(), OrbState::S5, "从 S0 断线同样要进 S5");
    }

    /// 从未连过时，初始化调用 set_connected(false) 不应误判为故障
    #[test]
    fn initial_disconnect_without_ever_connecting_stays_idle() {
        let mut m = m();
        m.set_connected(false);
        assert_eq!(m.state(), OrbState::S0, "启动时还没连过，不该显示异常");
    }

    #[test]
    fn disconnect_while_active_forces_s5() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap();
        m.handle(Stimulus::Connected).unwrap();
        m.set_connected(false);
        assert_eq!(m.state(), OrbState::S5);
    }

    #[test]
    fn click_toggles_out_of_call() {
        let mut m = m();
        m.handle(Stimulus::Click).unwrap(); // S1
        m.handle(Stimulus::Connected).unwrap(); // S2
        m.handle(Stimulus::Click).unwrap(); // S0
        assert_eq!(m.state(), OrbState::S0);
    }

    #[test]
    fn illegal_transition_is_rejected_not_panicking() {
        let mut m = m();
        assert!(m.handle(Stimulus::ModelSpeaking).is_err());
        assert_eq!(m.state(), OrbState::S0);
    }

    #[test]
    fn visual_params_differ_between_states() {
        let states = [
            OrbState::S0,
            OrbState::S1,
            OrbState::S2,
            OrbState::S3,
            OrbState::S4,
            OrbState::S5,
        ];
        let mut seen = std::collections::HashSet::new();
        for s in states {
            assert!(
                seen.insert(format!("{:?}", s.visual())),
                "视觉参数必须可区分: {:?}",
                s
            );
        }
    }
}
