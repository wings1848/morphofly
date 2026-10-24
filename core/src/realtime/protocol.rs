//! 统一事件模型 —— `protocol/schema.json` 的 Rust 镜像。
//!
//! 硬规矩：
//! - 这是跨端契约（UI / 移动端 / ESP32 共用）。改字段先改 `protocol/schema.json`，再改这里。
//! - 本模块只做「JSON ↔ 事件」的纯逻辑映射，不碰 IO，所以可以完整单测。
//! - 密钥/端点不在这里，配置见 `crate::config::RealtimeConfig`。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 下行音频默认采样率。
///
/// StepFun 的事件里**不带**采样率（只有 `output_audio_format: pcm16`），所以这里给默认值 24k，
/// 实际值以 `Config.realtime.output_sample_rate` 为准（`client.rs` 用它覆盖）。
pub const DEFAULT_OUTPUT_SAMPLE_RATE: u32 = 24_000;

/// 下行统一事件（与 `protocol/schema.json#/$defs/RealtimeEvent` 一一对应）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RealtimeEvent {
    /// 连接建立后的第一条服务端事件，带默认 session 配置
    SessionCreated { session_id: String, model: String },
    /// `session.update` 被接受
    SessionUpdated,
    /// 服务端 VAD 检测到用户开口（barge-in 依据）
    SpeechStarted,
    /// 服务端 VAD 检测到用户说完
    SpeechStopped,
    /// 模型音频增量。`pcm` 已从 base64 解出来，直接可喂声卡
    AudioDelta { pcm: Vec<i16>, sample_rate: u32 },
    /// 本轮音频流结束
    AudioDone,
    /// **用户**说的话（云端 ASR 转写）。
    ///
    /// 2026-10-25 实测澄清：StepFun 其实一直在发这个
    /// （`conversation.item.input_audio_transcription.delta/.completed`），
    /// 只是我们以前在解析层直接丢掉了 —— 于是「宿主拿不到用户原话」，
    /// 连带「模型不肯调工具时没法在本地帮它派活」这个死结解不开。
    /// 现在收下来：`is_final=false` 是增量，`true` 是整句。
    UserTranscript { text: String, is_final: bool },
    /// 模型说的话（字幕用）。StepFun 要读 `response.audio_transcript.delta`
    TranscriptDelta { text: String },
    /// 本轮模型话说完，`text` 是完整一句
    TranscriptDone { text: String },
    /// 纯文本增量（modalities 里没开 audio 时才会出现）
    TextDelta { text: String },
    /// 工具调用：`arguments` 是 JSON 字符串，由调用方解析
    ToolCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// 本轮响应结束，`status` 可能是 completed / cancelled / failed / incomplete
    ResponseDone { status: String },
    /// 服务端错误。StepFun 的行为：发 `error` 但**会话不断**（例如 voice 非法），所以不 panic
    Error {
        message: String,
        code: Option<String>,
    },
}

impl RealtimeEvent {
    /// 事件名，与 schema 的 `kind` 一致（日志/测试用）
    pub fn kind(&self) -> &'static str {
        match self {
            RealtimeEvent::SessionCreated { .. } => "SessionCreated",
            RealtimeEvent::SessionUpdated => "SessionUpdated",
            RealtimeEvent::SpeechStarted => "SpeechStarted",
            RealtimeEvent::SpeechStopped => "SpeechStopped",
            RealtimeEvent::AudioDelta { .. } => "AudioDelta",
            RealtimeEvent::AudioDone => "AudioDone",
            RealtimeEvent::UserTranscript { .. } => "UserTranscript",
            RealtimeEvent::TranscriptDelta { .. } => "TranscriptDelta",
            RealtimeEvent::TranscriptDone { .. } => "TranscriptDone",
            RealtimeEvent::TextDelta { .. } => "TextDelta",
            RealtimeEvent::ToolCall { .. } => "ToolCall",
            RealtimeEvent::ResponseDone { .. } => "ResponseDone",
            RealtimeEvent::Error { .. } => "Error",
        }
    }

    /// 带文字的事件（字幕可以直接用）
    pub fn text(&self) -> Option<&str> {
        match self {
            RealtimeEvent::TranscriptDelta { text }
            | RealtimeEvent::UserTranscript { text, .. }
            | RealtimeEvent::TranscriptDone { text }
            | RealtimeEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        }
    }

    /// 是否代表「这一轮说完了」（状态机 S4 → S2 的依据之一）
    pub fn is_end_of_turn(&self) -> bool {
        matches!(
            self,
            RealtimeEvent::ResponseDone { .. } | RealtimeEvent::AudioDone
        )
    }
}

/// 上传给语音模型的事件（OpenAI Realtime / StepFun 同构）。
///
/// 序列化后的 `type` 字段就是线上协议里的字符串，单测逐一锁死。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientCommand {
    /// 创建/更新会话配置
    #[serde(rename = "session.update")]
    SessionUpdate { session: SessionConfig },
    /// 创建/更新会话配置（原始 JSON，保留 `"turn_detection": null` 这类「显式 null」语义）
    ///
    /// 注意：不能也用 `#[serde(rename = "session.update")]`，否则和上面那个变体撞名
    /// （serde 会报 unreachable pattern，且序列化行为未定义）。
    /// 这个变体在 `client_command_json` 里手工序列化。
    RawSessionUpdate { session: Value },
    /// 追加一段上行音频（base64 的 pcm16）
    #[serde(rename = "input_audio_buffer.append")]
    AppendAudio { audio: String },
    /// 提交输入缓冲区（server_vad 模式下不用手动发）
    #[serde(rename = "input_audio_buffer.commit")]
    Commit,
    /// 清空输入缓冲区（打断后丢弃半句）
    #[serde(rename = "input_audio_buffer.clear")]
    Clear,
    /// 要求模型生成一轮回复
    #[serde(rename = "response.create")]
    CreateResponse,
    /// 要求模型生成一轮回复，并带上参数（如 `{"tool_choice": "required"}`）。
    ///
    /// 为什么要「强制调工具」这一档：真机实测模型会说「好，查着呢」却**不调工具**，
    /// 光靠提示词和兜底提醒都不稳。`tool_choice: "required"` 是协议级的硬约束 ——
    /// 一旦支持，这一轮它必须调用工具，没有「只说话」这个选项。
    ///
    /// 和 `RawSessionUpdate` 一样走手工序列化（不能和 `CreateResponse` 共用 rename，
    /// 否则反序列化时两个变体撞名）。
    RawCreateResponse { response: Value },
    /// 取消正在进行的回复（打断）
    #[serde(rename = "response.cancel")]
    CancelResponse,
    /// 手动往上下文里塞一条 item（系统提示、工具结果回灌等）
    #[serde(rename = "conversation.item.create")]
    ConversationItemCreate { item: Value },
}

/// `session.update` 的载荷。
///
/// 注意 `voice` 是 `Option`：StepFun 明确「voice 之外都能随时更新」，
/// 所以更新工具/指令时**不要**重复带 voice，避免被拒。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionConfig {
    pub modalities: Vec<String>,
    pub instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    pub input_audio_format: String,
    pub output_audio_format: String,
    /// 服务端 VAD 配置。
    ///
    /// ⚠️ **不要加 `skip_serializing_if = "Option::is_none"`**：
    /// 关掉服务端 VAD 必须真的发 `"turn_detection": null`，
    /// 而「字段缺失」和「字段为 null」在 StepFun 上是两种行为
    /// （缺失 = 用默认 server_vad，null = 关掉）。
    /// 2026-10-24 踩过：加了 skip 后 commit 报 `commit when server vad`。
    pub turn_detection: Option<TurnDetection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

impl SessionConfig {
    /// 从配置生成默认 session 配置（voice / instructions / pcm16 固定）
    ///
    /// 当 `rc.use_local_vad = true` 时 **turn_detection 为 null**，
    /// 即关掉服务端 VAD，由客户端手动 commit。
    /// 实测：这样端到端从 1950ms 降到 **790ms**（见 `evidence/p1-model-sweep.md`）。
    pub fn from_realtime_config(rc: &crate::config::RealtimeConfig) -> Self {
        Self {
            modalities: vec!["text".into(), "audio".into()],
            instructions: rc.instructions.clone(),
            voice: Some(rc.voice.clone()),
            input_audio_format: "pcm16".into(),
            output_audio_format: "pcm16".into(),
            turn_detection: if rc.use_local_vad {
                None
            } else {
                Some(TurnDetection::default())
            },
            tools: None,
        }
    }

    /// 带上工具声明（4 个，见 `crate::tools`）
    pub fn with_tools(mut self, tools: Vec<Value>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// 去掉 voice —— 更新工具/指令时用，避免踩「voice 不可变更」
    pub fn without_voice(mut self) -> Self {
        self.voice = None;
        self
    }

    /// 关掉服务端 VAD（改成手动 commit）
    pub fn without_turn_detection(mut self) -> Self {
        self.turn_detection = None;
        self
    }

    pub fn with_turn_detection(mut self, td: TurnDetection) -> Self {
        self.turn_detection = Some(td);
        self
    }
}

/// 服务端 VAD 参数（StepFun `turn_detection`，默认关闭）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnDetection {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix_padding_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silence_duration_ms: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_awakeness_threshold: Option<u32>,
}

impl Default for TurnDetection {
    fn default() -> Self {
        // ⚠️ 2026-10-24 二次实测（参数扫描）——**这些参数对 StepFun 实际不生效**：
        //    silence_duration_ms = 100 / 300 / 500 / 1000 → speech_stopped 恒在语音结束后 1604~1613ms
        //    energy_awakeness_threshold = 100 / 500 / 1000 → 同样无差别
        //    结论：**这个特定模型**的 VAD 判定固定约 1.6 秒，参数调不动。
        //
        // ⚠️ 2026-10-24 后续发现（这是本轮最重要的发现）：上面那个「1.6 秒硬下限」是
        //    `stepaudio-3-realtime-preview` **单个模型**的限制，不是 StepFun 平台的限制！
        //    换 `step-1o-audio` 后 VAD 判定变成 **~490ms**，端到端 520–530ms，
        //    正好落在验收理想值（<500ms 边缘）。
        //    全模型对比见 `core/src/config.rs` 的 RealtimeConfig 默认值注释。
        //
        // 2026-10-24 首次实测（wings-pc，testdata/audio/04_ack_zai.mp3 走 WS）：
        //  - energy_awakeness_threshold **默认 2500 太高**：用正常音量 TTS 音频推流，
        //    VAD 完全不触发（收不到 speech_started），等 45s 也没有任何 response。
        //  - 调到 500 后立刻出现完整链路：
        //    speech_started → speech_stopped → committed → response.created
        //    → audio_transcript.delta → response.audio.delta(12 段) → response.done(completed)
        //  - silence_duration_ms 100ms 太急会切碎一句话；400–500ms 合适。
        //  - turn_detection.type 只接受 "server_vad"，传 "none" 会被拒：
        //    error "only server_vad is supported"。
        //  - server_vad 开着时客户端**不能**手动 commit：
        //    error "commit when server vad"。轮次完全由服务端 VAD 驱动。
        Self {
            kind: "server_vad".into(),
            prefix_padding_ms: Some(300), // 300：曾用 500，降到 300 减少用户等待
            silence_duration_ms: Some(300), // 300：原 450 让「说完」判定太慢（占 P1 延迟大头）
            energy_awakeness_threshold: Some(500),
        }
    }
}

/// 服务端 JSON → 统一事件。
///
/// 未知事件、坏 JSON、解析不了的事件一律返回 `None`（**绝不 panic**），
/// 这样前端加新事件、服务端升级都不会把我们搞崩。
pub fn parse_server_event(raw: &str) -> Option<RealtimeEvent> {
    parse_server_event_with_rate(raw, DEFAULT_OUTPUT_SAMPLE_RATE)
}

/// 同 [`parse_server_event`]，但指定下行采样率（StepFun 事件里不带采样率）
pub fn parse_server_event_with_rate(raw: &str, sample_rate: u32) -> Option<RealtimeEvent> {
    crate::realtime::providers::stepfun::parse_event(raw, sample_rate)
}

/// 客户端命令 → 线上 JSON
pub fn client_command_json(cmd: &ClientCommand) -> Value {
    // RawSessionUpdate 需要保留 `"turn_detection": null` 这种显式 null 语义，
    // 不能走 derive（SessionConfig 的 skip_serializing_if 会整个省略字段）。
    if let ClientCommand::RawSessionUpdate { session } = cmd {
        return json!({ "type": "session.update", "session": session });
    }
    if let ClientCommand::RawCreateResponse { response } = cmd {
        return json!({ "type": "response.create", "response": response });
    }
    serde_json::to_value(cmd).unwrap_or_else(|e| {
        // 理论上不可能失败（全是简单结构），真失败了也不能 panic 掉整条语音链路
        tracing::error!(error = %e, "客户端命令序列化失败");
        json!({ "type": "noop" })
    })
}

/// i16 PCM → base64（小端，和 pcm16 约定一致）
pub fn encode_pcm16_base64(pcm: &[i16]) -> String {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    BASE64_STANDARD.encode(bytes)
}

/// base64 → i16 PCM。字节数为奇数 / 非法 base64 都返回 `None`，不 panic。
pub fn decode_pcm16_base64(b64: &str) -> Option<Vec<i16>> {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let bytes = BASE64_STANDARD.decode(b64.as_bytes()).ok()?;
    if bytes.len() % 2 != 0 {
        tracing::warn!(len = bytes.len(), "pcm16 base64 解出奇数字节，丢弃");
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：RawSessionUpdate 必须真的发出 `"turn_detection": null`
    /// （这是压低延迟的唯一手段，服务端区分「字段缺失」和「字段为 null」）
    #[test]
    fn raw_session_update_keeps_explicit_null() {
        let cmd = ClientCommand::RawSessionUpdate {
            session: json!({"modalities": ["audio"], "turn_detection": Value::Null}),
        };
        let v = client_command_json(&cmd);
        assert_eq!(v["type"], "session.update");
        let td = &v["session"]["turn_detection"];
        assert!(td.is_null(), "必须真的是 null，不是被省略");
        let text = serde_json::to_string(&v).unwrap();
        assert!(
            text.contains("\"turn_detection\":null"),
            "序列化后要能看到 null: {text}"
        );
    }

    /// turn_detection=None 必须序列化成 `null`，不能省略。
    ///
    /// 2026-10-24 踩坑：加了 skip_serializing_if 后字段被整个省略，
    /// StepFun 把「字段缺失」当成「用默认 server_vad」，
    /// 于是手动 commit 报 `commit when server vad`，延迟优化失败。
    #[test]
    fn none_turn_detection_serializes_as_null() {
        let mut sc = SessionConfig::from_realtime_config(&crate::config::RealtimeConfig::default());
        sc.turn_detection = None;
        let v = client_command_json(&ClientCommand::SessionUpdate { session: sc });
        assert!(
            v["session"].get("turn_detection").is_some(),
            "字段必须存在（不能省略）"
        );
        assert!(v["session"]["turn_detection"].is_null(), "值必须是 null");
    }

    /// 契约测试：我们的 kind 字符串必须和 protocol/schema.json 里的一模一样。
    #[test]
    fn kinds_match_schema_contract() {
        let schema_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("protocol")
            .join("schema.json");
        let text = std::fs::read_to_string(&schema_path)
            .unwrap_or_else(|e| panic!("读不到 {}: {e}", schema_path.display()));
        let schema: Value = serde_json::from_str(&text).expect("schema.json 不是合法 JSON");

        let mut schema_kinds: Vec<String> = schema
            .pointer("/$defs/RealtimeEvent/oneOf")
            .and_then(Value::as_array)
            .expect("schema 里找不到 RealtimeEvent.oneOf")
            .iter()
            .filter_map(|v| v.pointer("/properties/kind/const"))
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        schema_kinds.sort();

        let mut our_kinds: Vec<String> = [
            RealtimeEvent::SessionCreated {
                session_id: "sess_1".into(),
                model: "m".into(),
            },
            RealtimeEvent::SessionUpdated,
            RealtimeEvent::SpeechStarted,
            RealtimeEvent::SpeechStopped,
            RealtimeEvent::AudioDelta {
                pcm: vec![0],
                sample_rate: 24_000,
            },
            RealtimeEvent::AudioDone,
            RealtimeEvent::TranscriptDelta { text: "a".into() },
            RealtimeEvent::TranscriptDone { text: "a".into() },
            RealtimeEvent::TextDelta { text: "a".into() },
            RealtimeEvent::ToolCall {
                call_id: "c".into(),
                name: "n".into(),
                arguments: "{}".into(),
            },
            RealtimeEvent::ResponseDone {
                status: "completed".into(),
            },
            RealtimeEvent::Error {
                message: "e".into(),
                code: None,
            },
        ]
        .iter()
        .map(|ev| {
            // 序列化形态也要符合 schema：kind 字段必须是字符串
            let v = serde_json::to_value(ev).unwrap();
            v.get("kind")
                .and_then(Value::as_str)
                .expect("kind 必须是字符串")
                .to_string()
        })
        .collect();
        our_kinds.sort();

        assert_eq!(
            our_kinds, schema_kinds,
            "RealtimeEvent 与 schema 契约不一致"
        );
    }

    #[test]
    fn audio_delta_serializes_with_pcm_and_rate() {
        let ev = RealtimeEvent::AudioDelta {
            pcm: vec![1, -2, 3],
            sample_rate: 24_000,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["kind"], "AudioDelta");
        assert_eq!(v["pcm"], json!([1, -2, 3]));
        assert_eq!(v["sample_rate"], 24_000);
    }

    /// 回归：纠正时要用 `response.create` + `tool_choice: "required"`。
    ///
    /// 为什么要测序列化：这个命令走手工序列化（不能和 `CreateResponse` 共用 rename），
    /// 写错的话服务端只会静默忽略一个不认识的字段，纠正就退化成普通一轮 —— 很难发现。
    #[test]
    fn raw_create_response_carries_tool_choice() {
        let cmd = ClientCommand::RawCreateResponse {
            response: json!({ "tool_choice": "required" }),
        };
        let v = client_command_json(&cmd);
        assert_eq!(v["type"], "response.create");
        assert_eq!(v["response"]["tool_choice"], "required");
    }

    #[test]
    fn plain_create_response_has_no_extra_fields() {
        // 普通那一轮不能带 response 字段，免得改坏正常路径
        let v = client_command_json(&ClientCommand::CreateResponse);
        assert_eq!(v["type"], "response.create");
        assert!(v.get("response").is_none());
    }

    #[test]
    fn command_types_are_exact_wire_strings() {
        let cases = [
            (
                ClientCommand::SessionUpdate {
                    session: SessionConfig::from_realtime_config(
                        &crate::config::RealtimeConfig::default(),
                    ),
                },
                "session.update",
            ),
            (
                ClientCommand::AppendAudio {
                    audio: "AAAA".into(),
                },
                "input_audio_buffer.append",
            ),
            (ClientCommand::Commit, "input_audio_buffer.commit"),
            (ClientCommand::Clear, "input_audio_buffer.clear"),
            (ClientCommand::CreateResponse, "response.create"),
            (ClientCommand::CancelResponse, "response.cancel"),
            (
                ClientCommand::ConversationItemCreate {
                    item: json!({"type": "message"}),
                },
                "conversation.item.create",
            ),
        ];
        for (cmd, expect_type) in cases {
            let v = client_command_json(&cmd);
            assert_eq!(v["type"], expect_type);
        }
    }

    #[test]
    fn session_update_has_pcm16_and_voice() {
        let rc = crate::config::RealtimeConfig::default();
        let sc = SessionConfig::from_realtime_config(&rc);
        let v = client_command_json(&ClientCommand::SessionUpdate { session: sc });
        assert_eq!(v["session"]["voice"], rc.voice);
        assert_eq!(v["session"]["input_audio_format"], "pcm16");
        assert_eq!(v["session"]["output_audio_format"], "pcm16");
        assert_eq!(v["session"]["modalities"], json!(["text", "audio"]));
        assert!(v["session"]["instructions"]
            .as_str()
            .unwrap()
            .contains("小蝶"));
        // 默认 use_local_vad=true → turn_detection 应为 null（关掉服务端 VAD）
        assert!(
            v["session"]["turn_detection"].is_null(),
            "默认配置用本地 VAD，turn_detection 应为 null，实际 {:?}",
            v["session"]["turn_detection"]
        );
    }

    /// 反过来：use_local_vad=false 时必须恢复 server_vad
    #[test]
    fn server_vad_mode_restores_turn_detection() {
        let rc = crate::config::RealtimeConfig {
            use_local_vad: false,
            ..Default::default()
        };
        let sc = SessionConfig::from_realtime_config(&rc);
        let v = client_command_json(&ClientCommand::SessionUpdate { session: sc });
        assert_eq!(v["session"]["turn_detection"]["type"], "server_vad");
    }

    #[test]
    fn session_update_without_voice_omits_field() {
        let rc = crate::config::RealtimeConfig::default();
        let sc = SessionConfig::from_realtime_config(&rc).without_voice();
        let v = client_command_json(&ClientCommand::SessionUpdate { session: sc });
        assert!(
            v["session"].get("voice").is_none(),
            "更新工具时不能重发 voice（StepFun voice 不可变更）"
        );
    }

    #[test]
    fn pcm16_base64_roundtrip() {
        let pcm: Vec<i16> = vec![0, 1, -1, i16::MAX, i16::MIN, 1234, -4321];
        let b64 = encode_pcm16_base64(&pcm);
        assert_eq!(decode_pcm16_base64(&b64).as_deref(), Some(&pcm[..]));
    }

    #[test]
    fn pcm16_bytes_are_little_endian() {
        // 0x0000, 0x0100 → 字节 00 00 01 00 → "AAABAA=="
        assert_eq!(encode_pcm16_base64(&[0, 1]), "AAABAA==");
        assert_eq!(encode_pcm16_base64(&[]), "");
    }

    #[test]
    fn bad_base64_or_odd_bytes_do_not_panic() {
        assert!(decode_pcm16_base64("不是 base64").is_none());
        // "AAAAA" → 3 字节（奇数）→ None
        assert!(decode_pcm16_base64("AAAAA").is_none());
        assert_eq!(decode_pcm16_base64("").as_deref(), Some(&[][..]));
    }

    #[test]
    fn unknown_event_returns_none_not_panic() {
        assert!(parse_server_event(r#"{"type":"brand.new.event","x":1}"#).is_none());
        assert!(parse_server_event("not json at all").is_none());
        assert!(parse_server_event("{}").is_none());
        assert!(parse_server_event("").is_none());
    }

    #[test]
    fn event_helpers() {
        let ev = RealtimeEvent::TranscriptDelta {
            text: "你好".into(),
        };
        assert_eq!(ev.kind(), "TranscriptDelta");
        assert_eq!(ev.text(), Some("你好"));
        assert!(!ev.is_end_of_turn());
        assert!(RealtimeEvent::AudioDone.is_end_of_turn());
        assert!(RealtimeEvent::ResponseDone {
            status: "completed".into()
        }
        .is_end_of_turn());
    }
}
