//! Qwen（阿里云百炼）Realtime 适配。
//!
//! **状态：未实测**。已按官方文档写好端点/鉴权/参数映射，但没有 DashScope key，
//! 所以所有断言都是「文档口径」，不能当已验证结论。
//!
//! 为什么值得试（`evidence/p1-vad-parameter-sweep.md` 的结论）：
//! StepFun 的 VAD 判定「说完」固定 1.6 秒、手动 commit 要 5 秒，P1 端到端 2–7s。
//! Qwen 的官方口径首音频 1.37s，且**支持客户端 VAD + 手动 commit**，
//! 理论上可以把「说完 → 开口」压到 1 秒以内。
//!
//! 与 StepFun 的协议差异（文档口径，待实测确认）：
//!
//! | 项 | StepFun | Qwen（百炼） |
//! |---|---|---|
//! | 端点 | `wss://api.stepfun.com/v1/realtime?model=...` | `wss://{WorkspaceId}.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?model=...` |
//! | 鉴权 | `Authorization: Bearer <key>` | 同 |
//! | 事件名 | OpenAI Realtime 同构 | 同构，但输出模态决定事件名 |
//! | 纯文本输出 | `response.text.delta` | 只开 text 时才收 `response.text.delta` |
//! | 音频+文本 | `response.audio_transcript.delta` | 同 |
//! | VAD | 只支持 server_vad，参数不生效 | **支持手动 commit**，也支持 server_vad |
//! | 音频格式 | pcm16 全链路 | 输入 pcm16 16k mono；输出 WAV 24k |
//! | 历史长度 | 未查 | 音频历史 600 秒（超出截断） |
//!
//! 复用策略：事件解析直接用 `stepfun::parse_event`（同构）。
//! 这个模块只负责「端点构造 + 参数差异 + 需要特别注意的坑」。

use crate::config::{Provider, RealtimeConfig};
use anyhow::{bail, Result};

/// 已知的 Qwen realtime 模型（写死会过期，优先用配置里的 `model`）
pub const KNOWN_MODELS: &[&str] = &["qwen3.8-omni-flash-realtime", "qwen3.5-omni-plus-realtime"];

/// 构造 Qwen 的 WS 端点。
///
/// ⚠️ **必须带 WorkspaceId**，不是通用的 dashscope.aliyuncs.com。
/// 这是 docs/技术选型.md 里记录的「很多人卡在这」的坑。
pub fn build_endpoint(workspace_id: &str, model: &str) -> Result<String> {
    if workspace_id.trim().is_empty() {
        bail!(
            "Qwen 端点需要 WorkspaceId。请在配置里设置 \
             realtime.endpoint = \"wss://<你的WorkspaceId>.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?model={model}\""
        );
    }
    Ok(format!(
        "wss://{workspace_id}.cn-beijing.maas.aliyuncs.com/api-ws/v1/realtime?model={model}"
    ))
}

/// 判断当前配置是否已经切到 Qwen
pub fn is_qwen(rc: &RealtimeConfig) -> bool {
    rc.provider == Provider::Qwen
}

/// 给 Qwen 的建议 session 配置（关掉 server_vad，改手动 commit 压低延迟）
///
/// 依据：StepFun 侧实测证明 server_vad 的「说完」判定是延迟大头，
/// 换成能手动 commit 的 provider 才有可能达标。
/// **但这只是假设，必须拿到 key 实测确认。**
pub fn low_latency_session_json(instructions: &str, voice: &str) -> serde_json::Value {
    serde_json::json!({
        "modalities": ["text", "audio"],
        "instructions": instructions,
        "voice": voice,
        "input_audio_format": "pcm16",
        "output_audio_format": "pcm16",
        // 手动模式：客户端用本地 VAD 判断说完 → commit → response.create
        "turn_detection": serde_json::Value::Null
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_requires_workspace_id() {
        let e = build_endpoint("", "qwen3.8-omni-flash-realtime");
        assert!(e.is_err(), "没 WorkspaceId 必须报错，不能静默拼出错误 URL");
        assert!(e.unwrap_err().to_string().contains("WorkspaceId"));
    }

    #[test]
    fn endpoint_contains_workspace_and_model() {
        let e = build_endpoint("ws-test-123", "qwen3.8-omni-flash-realtime").unwrap();
        assert!(e.starts_with("wss://ws-test-123.cn-beijing.maas.aliyuncs.com"));
        assert!(e.contains("/api-ws/v1/realtime?model=qwen3.8-omni-flash-realtime"));
    }

    #[test]
    fn low_latency_session_disables_vad() {
        let s = low_latency_session_json("你是小蝶", "Tina");
        assert!(s["turn_detection"].is_null(), "低延迟模式必须显式 null");
        assert_eq!(s["input_audio_format"], "pcm16");
    }

    #[test]
    fn known_models_are_non_empty() {
        assert!(!KNOWN_MODELS.is_empty());
        for m in KNOWN_MODELS {
            assert!(m.contains("realtime"), "模型名应带 realtime: {m}");
        }
    }
}
