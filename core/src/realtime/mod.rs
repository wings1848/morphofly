//! 实时语音：统一事件模型 + provider 适配（StepFun / Qwen）。
//!
//! 分层：
//! - [`protocol`] 跨端统一事件契约（`protocol/schema.json` 的 Rust 镜像），纯逻辑可单测
//! - [`providers`] provider 适配（StepFun / OpenAI 风格 JSON → 统一事件）
//! - [`client`] WebSocket 客户端（Transport trait + 真实 ws 实现 + tokio 任务）
//!
//! 密钥纪律：本模块任何地方都不打印密钥明文，只从 `Config::api_key()` 取。

pub mod client;
pub mod protocol;
pub mod providers;

pub use client::{
    build_request_url, ensure_crypto_provider, RealtimeClient, RealtimeError, SessionInfo,
    Transport, WsTransport, EVENT_BUFFER,
};
pub use protocol::{
    client_command_json, decode_pcm16_base64, encode_pcm16_base64, parse_server_event,
    parse_server_event_with_rate, ClientCommand, RealtimeEvent, SessionConfig, TurnDetection,
    DEFAULT_OUTPUT_SAMPLE_RATE,
};
pub use providers::stepfun;
