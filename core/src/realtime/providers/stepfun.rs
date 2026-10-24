//! StepFun / OpenAI Realtime 风格 JSON ↔ 统一事件。
//!
//! 事实依据（已实测，见 `runs/.../facts.md` F14–F17）：
//! - 端点 `wss://api.stepfun.com/v1/realtime?model=stepaudio-3-realtime-preview`
//! - 事件名与 OpenAI Realtime 同构
//! - 输入输出音频格式固定 `pcm16`，事件里**不带**采样率（这里由调用方传进来）
//! - 非法 voice 只回 `error` 事件，会话不断 → 解析层永远不 panic
//!
//! 这个模块是纯函数：`raw JSON str → Option<RealtimeEvent>`，没有状态、没有 IO，
//! 所以每种服务端事件都能被单测覆盖。

use serde_json::Value;

use crate::realtime::protocol::{decode_pcm16_base64, RealtimeEvent};

/// 把一帧服务端 JSON 映射成统一事件。
///
/// 返回 `None` 的情况（都是「合法但不需要转成统一事件」，不是错误）：
/// - 未知/暂不关心的事件（`response.created`、`response.content_part.*` 等）
/// - `response.thinking.*`（推理过程不念给用户听，也不当字幕）
/// - `conversation.item.*` 里非 function_call 的 item
/// - `response.function_call_arguments.delta`（参数是碎片，等 `.done` 一次性给全）
/// - 音频增量 base64 解不出来（宁可丢一帧也不往声卡塞噪声）
///
/// 注意：统一事件模型 v1（`protocol/schema.json`）里**没有用户侧转写事件**，
/// 所以 `conversation.item.input_audio_transcription.completed` 目前丢弃。
pub fn parse_event(raw: &str, sample_rate: u32) -> Option<RealtimeEvent> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let ty = v.get("type").and_then(Value::as_str)?;

    match ty {
        "session.created" => Some(RealtimeEvent::SessionCreated {
            session_id: str_at(&v, &["session", "id"]).unwrap_or_default(),
            model: str_at(&v, &["session", "model"]).unwrap_or_default(),
        }),
        "session.updated" => Some(RealtimeEvent::SessionUpdated),

        "input_audio_buffer.speech_started" => Some(RealtimeEvent::SpeechStarted),
        "input_audio_buffer.speech_stopped" => Some(RealtimeEvent::SpeechStopped),

        "response.audio.delta" => {
            let b64 = v.get("delta").and_then(Value::as_str)?;
            let pcm = decode_pcm16_base64(b64)?;
            if pcm.is_empty() {
                tracing::debug!("收到空音频增量，忽略");
                return None;
            }
            Some(RealtimeEvent::AudioDelta { pcm, sample_rate })
        }
        "response.audio.done" => Some(RealtimeEvent::AudioDone),

        "response.audio_transcript.delta" => {
            let text = v.get("delta").and_then(Value::as_str)?.to_string();
            Some(RealtimeEvent::TranscriptDelta { text })
        }
        "response.audio_transcript.done" => {
            let text = v
                .get("transcript")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(RealtimeEvent::TranscriptDone { text })
        }

        "response.text.delta" => {
            let text = v.get("delta").and_then(Value::as_str)?.to_string();
            Some(RealtimeEvent::TextDelta { text })
        }
        // 文本模式没有独立的「done」事件位（schema v1 只有 TextDelta）。
        // 语义等同 TranscriptDone：整句文本一次给全，字幕侧用它替换累积内容。
        "response.text.done" => {
            let text = v
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(RealtimeEvent::TranscriptDone { text })
        }

        "response.done" => Some(RealtimeEvent::ResponseDone {
            status: str_at(&v, &["response", "status"]).unwrap_or_else(|| "unknown".into()),
        }),
        "response.cancelled" => Some(RealtimeEvent::ResponseDone {
            status: "cancelled".into(),
        }),

        // 工具调用：完整参数只出现在 done / 带参数的 added 上
        "response.output_item.done" | "response.output_item.created" => tool_call(&v, false),
        "response.output_item.added" => tool_call(&v, true),
        "response.function_call_arguments.done" => tool_call_flat(&v),
        "response.function_call_arguments.delta" => None,

        "error" => {
            let message = str_at(&v, &["error", "message"])
                .or_else(|| str_at(&v, &["message"]))
                .unwrap_or_else(|| "服务端未知错误".into());
            let code = v
                .get("error")
                .and_then(|e| e.get("code"))
                .or_else(|| v.get("code"))
                .map(code_to_string)
                .filter(|c| !c.is_empty());
            Some(RealtimeEvent::Error { message, code })
        }

        // 明确丢弃：文档里存在但统一事件模型 v1 没有对应位置
        "conversation.item.input_audio_transcription.completed"
        | "conversation.item.created"
        | "conversation.item.deleted"
        | "input_audio_buffer.committed"
        | "input_audio_buffer.cleared"
        | "response.created"
        | "response.content_part.added"
        | "response.content_part.done"
        | "response.thinking.delta"
        | "response.thinking.done" => None,

        _ => {
            tracing::debug!(event_type = ty, "未处理的服务端事件，忽略");
            None
        }
    }
}

/// 从 `item`（或顶层）取 `function_call`。
fn tool_call(v: &Value, need_arguments: bool) -> Option<RealtimeEvent> {
    let src = v.get("item").filter(|i| i.is_object()).unwrap_or(v);
    if src.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    build_tool_call(src, need_arguments)
}

/// `response.function_call_arguments.done` 把字段摊在顶层。
fn tool_call_flat(v: &Value) -> Option<RealtimeEvent> {
    build_tool_call(v, false).or_else(|| {
        // 有些实现会把 name 放 item 里
        v.get("item")
            .filter(|i| i.is_object())
            .and_then(|item| build_tool_call(item, false))
    })
}

fn build_tool_call(src: &Value, need_arguments: bool) -> Option<RealtimeEvent> {
    let name = src.get("name").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        // 没有函数名，不是可执行的调用（可能是流式碎片）
        return None;
    }
    let arguments = src
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if need_arguments && arguments.is_empty() {
        // added 阶段参数通常还是空的，等 done 再报，避免同一次调用报两遍
        return None;
    }
    let call_id = src
        .get("call_id")
        .or_else(|| src.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(RealtimeEvent::ToolCall {
        call_id,
        name: name.to_string(),
        arguments,
    })
}

/// 工具声明归一化成 StepFun 要求的格式。
///
/// 线上实测（2026-10-24，wings-pc 真机）：发扁平格式
/// `{"type":"function","name":"delegate_task","parameters":{...}}` 会收到
/// `error: Request param: session.tools[0].function.name is required`，
/// 说明服务端要的是 OpenAI **嵌套**格式：
/// `{"type":"function","function":{"name":...,"description":...,"parameters":{...}}}`。
///
/// 本函数幂等：已经是嵌套格式的原样返回；扁平格式才包一层。已经是 `type: retrieval` 的跳过。
pub fn normalize_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            let ty = t.get("type").and_then(Value::as_str).unwrap_or("function");
            if ty != "function" {
                return t.clone();
            }
            // 已经是 {"type":"function","function":{...}} → 不动
            if t.get("function").map(Value::is_object).unwrap_or(false) {
                return t.clone();
            }
            let mut function = serde_json::Map::new();
            for key in ["name", "description", "parameters"] {
                if let Some(v) = t.get(key) {
                    function.insert(key.to_string(), v.clone());
                }
            }
            serde_json::json!({
                "type": "function",
                "function": Value::Object(function),
            })
        })
        .collect()
}

fn str_at(v: &Value, path: &[&str]) -> Option<String> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key)?;
    }
    cur.as_str().map(|s| s.to_string())
}

fn code_to_string(c: &Value) -> String {
    match c {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::protocol::encode_pcm16_base64;

    const RATE: u32 = 24_000;

    fn p(raw: &str) -> Option<RealtimeEvent> {
        parse_event(raw, RATE)
    }

    #[test]
    fn session_created() {
        let raw = r#"{"event_id":"event_def","type":"session.created","session":{
            "id":"sess_001","object":"realtime.session","model":"stepaudio-3-realtime-preview",
            "modalities":["text","audio"],"voice":"jingdiannvsheng",
            "input_audio_format":"pcm16","output_audio_format":"pcm16"}}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::SessionCreated {
                session_id: "sess_001".into(),
                model: "stepaudio-3-realtime-preview".into(),
            })
        );
    }

    #[test]
    fn session_created_tolerates_missing_session_fields() {
        // 真实抓包里 session 里字段可能缺（比如没有 id），不能 panic
        let ev = p(r#"{"type":"session.created","session":{}}"#).unwrap();
        assert_eq!(
            ev,
            RealtimeEvent::SessionCreated {
                session_id: String::new(),
                model: String::new(),
            }
        );
        assert_eq!(
            p(r#"{"type":"session.created"}"#),
            Some(RealtimeEvent::SessionCreated {
                session_id: String::new(),
                model: String::new(),
            })
        );
    }

    #[test]
    fn session_updated() {
        assert_eq!(
            p(r#"{"type":"session.updated"}"#),
            Some(RealtimeEvent::SessionUpdated)
        );
    }

    #[test]
    fn speech_started_and_stopped() {
        let raw = r#"{"event_id":"e1","type":"input_audio_buffer.speech_started","audio_start_ms":1000,"item_id":"msg_003"}"#;
        assert_eq!(p(raw), Some(RealtimeEvent::SpeechStarted));
        let raw = r#"{"type":"input_audio_buffer.speech_stopped","audio_end_ms":2000,"item_id":"msg_003"}"#;
        assert_eq!(p(raw), Some(RealtimeEvent::SpeechStopped));
    }

    #[test]
    fn audio_delta_decodes_pcm16() {
        let pcm: Vec<i16> = (0..320).map(|i| ((i * 97) as i16) - 1000).collect();
        let raw = format!(
            r#"{{"type":"response.audio.delta","item_id":"msg_008","delta":"{}"}}"#,
            encode_pcm16_base64(&pcm)
        );
        match p(raw.as_str()) {
            Some(RealtimeEvent::AudioDelta {
                pcm: got,
                sample_rate,
            }) => {
                assert_eq!(got, pcm, "解出的 PCM 必须逐样本一致");
                assert_eq!(sample_rate, RATE);
            }
            other => panic!("期望 AudioDelta，实际 {other:?}"),
        }
    }

    #[test]
    fn audio_delta_with_bad_base64_is_dropped_not_panicking() {
        assert_eq!(p(r#"{"type":"response.audio.delta","delta":"@@@@"}"#), None);
        assert_eq!(p(r#"{"type":"response.audio.delta"}"#), None);
        assert_eq!(p(r#"{"type":"response.audio.delta","delta":""}"#), None);
        // 奇数字节
        let raw = format!(
            r#"{{"type":"response.audio.delta","delta":"{}"}}"#,
            base64::prelude::Engine::encode(&base64::prelude::BASE64_STANDARD, &[1u8, 2, 3][..])
        );
        assert_eq!(p(raw.as_str()), None);
    }

    #[test]
    fn audio_done() {
        let raw = r#"{"type":"response.audio.done","response_id":"traceid","item_id":"msg_008"}"#;
        assert_eq!(p(raw), Some(RealtimeEvent::AudioDone));
    }

    #[test]
    fn transcript_delta_and_done() {
        let raw = r#"{"type":"response.audio_transcript.delta","item_id":"msg_002","output_index":0,"delta":"Hello, how can I a"}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::TranscriptDelta {
                text: "Hello, how can I a".into()
            })
        );
        let raw = r#"{"type":"response.audio_transcript.done","response_id":"resp_001","item_id":"msg_008","content_index":0,"transcript":"Hello, how can I assist you today?"}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::TranscriptDone {
                text: "Hello, how can I assist you today?".into()
            })
        );
    }

    #[test]
    fn text_delta_and_done() {
        let raw = r#"{"type":"response.text.delta","delta":"Sure, "}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::TextDelta {
                text: "Sure, ".into()
            })
        );
        // text.done 用 TranscriptDone 表示「整句文本给全」
        let raw = r#"{"type":"response.text.done","text":"Sure, I can help with that."}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::TranscriptDone {
                text: "Sure, I can help with that.".into()
            })
        );
    }

    #[test]
    fn response_done_with_status() {
        let raw = r#"{"type":"response.done","response":{"id":"resp_001","object":"realtime.response","status":"completed","output":[]}}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::ResponseDone {
                status: "completed".into()
            })
        );
        // 没有 response.status 时给 unknown，不 panic
        assert_eq!(
            p(r#"{"type":"response.done"}"#),
            Some(RealtimeEvent::ResponseDone {
                status: "unknown".into()
            })
        );
    }

    #[test]
    fn response_cancelled_maps_to_response_done() {
        assert_eq!(
            p(r#"{"type":"response.cancelled","response_id":"resp_1"}"#),
            Some(RealtimeEvent::ResponseDone {
                status: "cancelled".into()
            })
        );
    }

    #[test]
    fn error_event_keeps_session_alive() {
        // F15 实测原文
        let raw =
            r#"{"type":"error","error":{"message":"voice default is not valid","code":"400"}}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::Error {
                message: "voice default is not valid".into(),
                code: Some("400".into()),
            })
        );
        // code 是数字也要能收
        let raw = r#"{"type":"error","error":{"type":"invalid_request_error","code":400,"message":"音频内容不完整"}}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::Error {
                message: "音频内容不完整".into(),
                code: Some("400".into()),
            })
        );
        // 连 message 都没有也不能 panic
        assert_eq!(
            p(r#"{"type":"error"}"#),
            Some(RealtimeEvent::Error {
                message: "服务端未知错误".into(),
                code: None,
            })
        );
    }

    #[test]
    fn tool_call_from_output_item_done() {
        let raw = r#"{"type":"response.output_item.done","response_id":"resp_001","output_index":0,
            "item":{"id":"fc_1","object":"realtime.item","type":"function_call",
            "status":"completed","call_id":"call_abc","name":"delegate_task",
            "arguments":"{\"task\":\"列目录\"}"}}"#;
        match p(raw) {
            Some(RealtimeEvent::ToolCall {
                call_id,
                name,
                arguments,
            }) => {
                assert_eq!(call_id, "call_abc");
                assert_eq!(name, "delegate_task");
                let args: Value = serde_json::from_str(&arguments).unwrap();
                assert_eq!(args["task"], "列目录");
            }
            other => panic!("期望 ToolCall，实际 {other:?}"),
        }
    }

    #[test]
    fn tool_call_from_function_call_arguments_done() {
        // 这条事件文档里没写，是抓包才知道的（调研记录 §3.1）
        let raw = r#"{"type":"response.function_call_arguments.done","response_id":"resp_1",
            "item_id":"fc_1","call_id":"call_xyz","name":"query_progress","arguments":"{}"}"#;
        assert_eq!(
            p(raw),
            Some(RealtimeEvent::ToolCall {
                call_id: "call_xyz".into(),
                name: "query_progress".into(),
                arguments: "{}".into(),
            })
        );
    }

    #[test]
    fn tool_call_added_waits_for_arguments() {
        // added 阶段参数为空 → 不报，等 done
        let raw = r#"{"type":"response.output_item.added","output_index":0,
            "item":{"type":"function_call","call_id":"c1","name":"cancel_task","arguments":""}}"#;
        assert_eq!(p(raw), None);
        // 若 added 已经带全参数，就直接报
        let raw = r#"{"type":"response.output_item.added","output_index":0,
            "item":{"type":"function_call","call_id":"c1","name":"cancel_task","arguments":"{\"task_id\":\"t1\"}"}}"#;
        assert!(matches!(p(raw), Some(RealtimeEvent::ToolCall { .. })));
    }

    #[test]
    fn tool_call_argument_delta_is_not_emitted() {
        let raw = r#"{"type":"response.function_call_arguments.delta","call_id":"c1","delta":"{\"task\""}"#;
        assert_eq!(p(raw), None, "参数碎片不应satisfy成半截 ToolCall");
    }

    #[test]
    fn message_items_are_not_tool_calls() {
        let raw = r#"{"type":"response.output_item.done","item":{"id":"msg_1","type":"message","role":"assistant","content":[]}}"#;
        assert_eq!(p(raw), None);
        let raw = r#"{"type":"response.output_item.added","item":{"id":"msg_1","type":"message"}}"#;
        assert_eq!(p(raw), None);
    }

    #[test]
    fn input_transcription_is_ignored_for_now() {
        // 统一事件模型 v1 没有用户侧转写事件；明确丢弃而不是 panic
        let raw = r#"{"type":"conversation.item.input_audio_transcription.completed","item_id":"msg_003","content_index":0,"transcript":"你好"}"#;
        assert_eq!(p(raw), None);
    }

    #[test]
    fn known_but_irrelevant_events_are_ignored() {
        for raw in [
            r#"{"type":"response.created","response":{"id":"resp_1"}}"#,
            r#"{"type":"response.content_part.added","part":{"type":"text","text":""}}"#,
            r#"{"type":"response.content_part.done","part":{"type":"text","text":"x"}}"#,
            r#"{"type":"response.thinking.delta","delta":"嗯…"}"#,
            r#"{"type":"response.thinking.done","thinking":"嗯…"}"#,
            r#"{"type":"conversation.item.created","item":{"id":"msg_1"}}"#,
            r#"{"type":"conversation.item.deleted","item_id":"msg_1"}"#,
            r#"{"type":"input_audio_buffer.committed","item_id":"msg_2"}"#,
            r#"{"type":"input_audio_buffer.cleared"}"#,
        ] {
            assert_eq!(p(raw), None, "应忽略：{raw}");
        }
    }

    #[test]
    fn unknown_and_malformed_never_panic() {
        assert_eq!(p(r#"{"type":"totally.new.event"}"#), None);
        assert_eq!(p("not json"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("[]"), None);
        assert_eq!(p(r#"{"no_type":true}"#), None);
        assert_eq!(p(r#"{"type":123}"#), None);
        // 类型对但字段类型怪：不 panic
        assert_eq!(
            p(r#"{"type":"session.created","session":"oops"}"#),
            Some(RealtimeEvent::SessionCreated {
                session_id: String::new(),
                model: String::new(),
            })
        );
        assert_eq!(
            p(r#"{"type":"response.done","response":{"status":42}}"#),
            Some(RealtimeEvent::ResponseDone {
                status: "unknown".into()
            })
        );
        assert_eq!(
            p(r#"{"type":"error","error":{"message":["x"],"code":null}}"#),
            Some(RealtimeEvent::Error {
                message: "服务端未知错误".into(),
                code: None,
            })
        );
    }

    #[test]
    fn sample_rate_is_passed_through() {
        let raw = format!(
            r#"{{"type":"response.audio.delta","delta":"{}"}}"#,
            encode_pcm16_base64(&[1, 2, 3])
        );
        match parse_event(&raw, 16_000) {
            Some(RealtimeEvent::AudioDelta { sample_rate, .. }) => assert_eq!(sample_rate, 16_000),
            other => panic!("期望 AudioDelta，实际 {other:?}"),
        }
    }

    #[test]
    fn normalize_tools_wraps_flat_function_defs() {
        let flat = vec![serde_json::json!({
            "type": "function",
            "name": "delegate_task",
            "description": "派活",
            "parameters": {"type": "object", "properties": {"task": {"type": "string"}}}
        })];
        let got = normalize_tools(&flat);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["type"], "function");
        assert_eq!(got[0]["function"]["name"], "delegate_task");
        assert_eq!(got[0]["function"]["description"], "派活");
        assert_eq!(
            got[0]["function"]["parameters"]["properties"]["task"]["type"],
            "string"
        );
        // 扁平字段不该留在顶层（StepFun 只认 function.name）
        assert!(got[0].get("name").is_none());
    }

    #[test]
    fn normalize_tools_is_idempotent() {
        let flat = crate::tools::tool_defs();
        let once = normalize_tools(&flat);
        let twice = normalize_tools(&once);
        assert_eq!(once, twice, "重复归一化不能改变结构");
        assert_eq!(once.len(), crate::tools::TOOL_COUNT);
        for t in &once {
            assert!(t["function"]["name"].is_string());
            assert_eq!(t["type"], "function");
        }
        let mut names: Vec<&str> = once
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            {
                let mut expect = crate::tools::TOOL_NAMES.to_vec();
                expect.sort_unstable();
                expect
            },
            "不变量 I1：归一化后仍然只有这 4 个工具"
        );
    }

    #[test]
    fn normalize_tools_keeps_retrieval_untouched() {
        let retrieval = vec![serde_json::json!({
            "type": "retrieval",
            "function": {"description": "知识库", "options": {"vector_store_id": "1"}}
        })];
        assert_eq!(normalize_tools(&retrieval), retrieval);
    }
}
