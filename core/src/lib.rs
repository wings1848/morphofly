//! 蝶梦语音智能体 · core
//!
//! 架构铁律（来自 docs/开发方案.md）：
//! 1. WebView 永不碰麦克风 —— 音频全部在本 crate 里用 cpal/rodio 原生处理
//! 2. 语音模型只有 4 个工具，物理上没有文件/命令能力
//! 3. 语音链路与任务链路是两条独立连接，互不阻塞

pub mod audio;
pub mod config;
pub mod correction;
pub mod dispatch;
pub mod dsh;
pub mod intent;
#[cfg(feature = "kws")]
pub mod kws;
pub mod projects;
pub mod realtime;
pub mod session;
pub mod spectrum;
pub mod tasks;
pub mod tools;
#[cfg(feature = "kws")]
pub mod vad;

pub use config::Config;
pub use session::{OrbState, SessionMachine};
pub use tasks::{Task, TaskStatus, TaskStore};

/// 上行音频格式定死：16kHz / mono / i16 / 20ms 帧 = 320 采样 = 640 字节
pub const UPLINK_SAMPLE_RATE: u32 = 16_000;
pub const UPLINK_CHANNELS: u16 = 1;
pub const FRAME_MS: u32 = 20;
pub const FRAME_SAMPLES: usize = (UPLINK_SAMPLE_RATE as usize * FRAME_MS as usize) / 1000; // 320
pub const FRAME_BYTES: usize = FRAME_SAMPLES * 2; // 640

/// 频谱推给 WebView 的 bin 数（64 float @ 30fps ≈ 7.7KB/s，远低于 IPC 带宽）
pub const SPECTRUM_BINS: usize = 64;
