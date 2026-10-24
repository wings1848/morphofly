//! StepFun / OpenAI 风格实时语音 WebSocket 客户端。
//!
//! 结构：
//!
//! ```text
//!  调用方 ──send_audio/send_tools/...──▶ cmd 通道 ──▶ writer 任务 ──▶ Transport
//!  调用方 ◀──broadcast<RealtimeEvent>──── reader 任务 ◀────────────Transport
//! ```
//!
//! - 传输层抽象成 [`Transport`] trait，单测用假 Transport 覆盖「收到 response.audio.delta
//!   → 发出 AudioDelta(pcm 非空)」，不需要真网络、不需要音频设备。
//! - 断线一律返回明确的 [`RealtimeError`]，并广播一条 `Error { code: "disconnected" }`
//!   的统一事件（schema v1 没有独立的断线事件，用 error 承载，调用方据此进 S5 异常态）。
//! - 密钥只从 `Config::api_key()` 或显式参数进来，**从不写日志、不落盘**。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::config::{Config, RealtimeConfig};
use crate::realtime::protocol::{
    client_command_json, encode_pcm16_base64, parse_server_event_with_rate, ClientCommand,
    RealtimeEvent, SessionConfig,
};

/// 事件广播缓冲深度。
///
/// 20ms 一帧音频增量，一秒 50 个事件；1024 够 20 秒的积压。慢消费者会收到 `Lagged`，
/// 由调用方决定是丢帧（音频）还是重连（控制事件）。
pub const EVENT_BUFFER: usize = 1024;

/// 明确的错误类型（不 panic）。
#[derive(Debug, thiserror::Error)]
pub enum RealtimeError {
    #[error("缺少 API 密钥：{0}")]
    MissingKey(String),
    #[error("端点非法：{0}")]
    BadEndpoint(String),
    #[error("WebSocket 连接失败：{0}")]
    Connect(String),
    #[error("传输层错误：{0}")]
    Transport(String),
    #[error("连接已断开，无法发送")]
    Disconnected,
}

pub type Result<T> = std::result::Result<T, RealtimeError>;

/// 传输层抽象。真实实现见 [`WsTransport`]，测试实现见 client.rs 的 `tests::FakeTransport`。
///
/// 用 `BoxFuture` 而非 `async fn`，保证 trait 可 `dyn` 化且 future 一定是 `Send`
/// （reader / writer 两个任务要 `tokio::spawn`）。
pub trait Transport: Send + Sync + 'static {
    /// 发一条文本帧
    fn send_text<'a>(&'a self, text: String) -> BoxFuture<'a, Result<()>>;
    /// 收下一条文本帧；`None` = 连接正常关闭
    fn next_event<'a>(&'a self) -> BoxFuture<'a, Option<Result<String>>>;
    /// 主动关闭
    fn close<'a>(&'a self) -> BoxFuture<'a, ()>;
}

/// 真实 WebSocket 传输。
///
/// 读写各用一把锁分到两个任务里：读锁在等帧时一直持有，所以发送**绝不能**跟读取共用一把锁，
/// 否则 `response.create` 会卡到下一帧音频才发出去。
pub struct WsTransport {
    sink: tokio::sync::Mutex<
        futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    >,
    stream: tokio::sync::Mutex<
        futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    >,
}

impl WsTransport {
    pub fn new(ws: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, stream) = ws.split();
        Self {
            sink: tokio::sync::Mutex::new(sink),
            stream: tokio::sync::Mutex::new(stream),
        }
    }
}

impl Transport for WsTransport {
    fn send_text<'a>(&'a self, text: String) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.sink
                .lock()
                .await
                .send(Message::text(text))
                .await
                .map_err(|e| RealtimeError::Transport(e.to_string()))
        })
    }

    fn next_event<'a>(&'a self) -> BoxFuture<'a, Option<Result<String>>> {
        Box::pin(async move {
            let mut stream = self.stream.lock().await;
            loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => return Some(Ok(text.to_string())),
                    // StepFun 只发文本帧；万一发二进制也尽量解，解不了就跳过而不是断链
                    Some(Ok(Message::Binary(bin))) => {
                        if let Ok(text) = std::str::from_utf8(&bin) {
                            return Some(Ok(text.to_string()));
                        }
                        tracing::warn!(len = bin.len(), "收到非 UTF-8 二进制帧，忽略");
                        continue;
                    }
                    // Ping/Pong 由 tungstenite 自动回，不打扰上层
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                    Some(Ok(Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(frame))) => {
                        if let Some(f) = frame {
                            tracing::info!(code = ?f.code, reason = %f.reason, "服务端关闭连接");
                        }
                        return None;
                    }
                    Some(Err(e)) => return Some(Err(RealtimeError::Transport(e.to_string()))),
                    None => return None,
                }
            }
        })
    }

    fn close<'a>(&'a self) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let mut sink = self.sink.lock().await;
            let _ = sink.send(Message::Close(None)).await;
            let _ = sink.close().await;
        })
    }
}

/// `session.created` 带给我们的会话信息（给日志/证据用，不进统一事件模型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub session_id: String,
    pub model: String,
    /// 服务端实际生效的音色（实测默认 `jingdiannvsheng`）
    pub voice: Option<String>,
}

struct Shared {
    closed: AtomicBool,
    session: watch::Sender<Option<SessionInfo>>,
    shutdown: watch::Sender<bool>,
    /// 已上报过的工具调用 id —— StepFun 同一次调用会在
    /// `response.output_item.done` 和 `response.function_call_arguments.done` 里各报一遍，
    /// 不去重会让调用方重复派活。
    seen_tool_calls: std::sync::Mutex<std::collections::HashSet<String>>,
}

impl Shared {
    /// 工具调用是否首次出现。同一 call_id 只放行一次。
    fn first_tool_call(&self, event: &RealtimeEvent) -> bool {
        match event {
            RealtimeEvent::ToolCall { call_id, name, .. } => {
                if call_id.is_empty() {
                    // 没有 call_id 时退化成「name + 参数」没法可靠去重，
                    // 但空 call_id 本来就无法回灌，直接放行让上层报错更清楚。
                    tracing::warn!("工具调用缺少 call_id，无法去重");
                    return true;
                }
                let mut seen = match self.seen_tool_calls.lock() {
                    Ok(g) => g,
                    Err(poisoned) => poisoned.into_inner(),
                };
                let key = format!("{call_id}\u{1f}{name}");
                seen.insert(key)
            }
            _ => true,
        }
    }
}

/// 实时语音客户端。
///
/// 不泛型化：Transport 一旦交给后台任务就跟客户端句柄无关了，
/// 这里只留通道 + 状态。想换传输方式用 [`RealtimeClient::with_transport`]。
pub struct RealtimeClient {
    cmd_tx: mpsc::UnboundedSender<ClientCommand>,
    evt_tx: broadcast::Sender<RealtimeEvent>,
    session_rx: watch::Receiver<Option<SessionInfo>>,
    shared: Arc<Shared>,
    /// 当前会话模板（instructions / voice / pcm16 / vad）。`send_tools` 基于它改 tools，
    /// 避免用 `RealtimeConfig::default()` 把调用方配的 instructions 冲掉。
    session_template: SessionConfig,
    tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl RealtimeClient {
    /// 按 `~/.diemeng/config.toml`（+ 环境变量）连上实时语音服务。
    ///
    /// 密钥由 `Config::api_key()` 提供（环境变量 → `~/.diemeng/credentials`），
    /// 这里不接收、不打印明文密钥。
    pub async fn connect(cfg: &Config) -> Result<Self> {
        let key = cfg
            .api_key()
            .map_err(|e| RealtimeError::MissingKey(e.to_string()))?;
        Self::connect_with(&cfg.realtime, &key).await
    }

    /// 显式给端点与密钥的连接（测试/多账号用）。`connect` 最终调它。
    pub async fn connect_with(rc: &RealtimeConfig, api_key: &str) -> Result<Self> {
        ensure_crypto_provider();

        if api_key.trim().is_empty() {
            return Err(RealtimeError::MissingKey("密钥为空".into()));
        }
        let url = build_request_url(&rc.endpoint, &rc.model)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|e| RealtimeError::BadEndpoint(format!("{url}: {e}")))?;

        let auth = format!("Bearer {}", api_key.trim());
        let value = HeaderValue::from_str(&auth)
            .map_err(|_| RealtimeError::BadEndpoint("密钥含非法字符，无法放进握手头".into()))?;
        request.headers_mut().insert("Authorization", value);

        let started = std::time::Instant::now();
        let (ws, response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| RealtimeError::Connect(e.to_string()))?;
        tracing::info!(
            status = %response.status(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            model = %rc.model,
            "实时语音 WS 已连接"
        );

        // 连上先下发基础会话配置（音色 / 指令 / pcm16 / server_vad）。
        // 工具声明由调用方显式 `send_tools(&tools)` —— 谁声明谁负责，connect 不替它决定。
        // 注意 voice 非法只会收到 error 事件、不会断链（实测 F15），所以这里不预校验音色。
        let template = SessionConfig::from_realtime_config(rc);
        let transport = Arc::new(WsTransport::new(ws));
        let client = Self::with_transport_and_session(transport, template, rc.output_sample_rate)?;
        client.send_session_update(client.session_template.clone())?;
        Ok(client)
    }
}

impl RealtimeClient {
    /// 用任意 Transport 建客户端（测试接缝，也供未来换传输方式用）。
    ///
    /// 返回前 reader/writer 两个任务已经起来了；`events()` 之后产生的事件才收得到，
    /// 但 `SessionCreated` 另有 [`Self::session_info`] / [`Self::wait_session_created`] 兜底，
    /// 不会因为订阅早早晚晚丢会话信息。
    pub fn with_transport<T>(transport: Arc<T>, sample_rate: u32) -> Result<Self>
    where
        T: Transport,
    {
        let rc = RealtimeConfig {
            output_sample_rate: sample_rate,
            ..RealtimeConfig::default()
        };
        Self::with_transport_and_session(
            transport,
            SessionConfig::from_realtime_config(&rc),
            sample_rate,
        )
    }

    /// 同 [`Self::with_transport`]，但指定会话模板（`connect` 用它带上 4 个工具）。
    pub fn with_transport_and_session<T>(
        transport: Arc<T>,
        session_template: SessionConfig,
        sample_rate: u32,
    ) -> Result<Self>
    where
        T: Transport,
    {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<ClientCommand>();
        let (evt_tx, _) = broadcast::channel::<RealtimeEvent>(EVENT_BUFFER);
        let (session_tx, session_rx) = watch::channel::<Option<SessionInfo>>(None);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let shared = Arc::new(Shared {
            closed: AtomicBool::new(false),
            session: session_tx,
            shutdown: shutdown_tx,
            seen_tool_calls: std::sync::Mutex::new(std::collections::HashSet::new()),
        });

        let tasks = vec![
            tokio::spawn(reader_loop(
                transport.clone(),
                evt_tx.clone(),
                shared.clone(),
                sample_rate,
                shutdown_rx.clone(),
            )),
            tokio::spawn(writer_loop(
                transport,
                cmd_rx,
                evt_tx.clone(),
                shared.clone(),
                shutdown_rx,
            )),
        ];

        Ok(Self {
            cmd_tx,
            evt_tx,
            session_rx,
            shared,
            session_template,
            tasks: std::sync::Mutex::new(tasks),
        })
    }

    /// 当前会话模板（instructions / voice / 格式 / VAD / tools）
    pub fn session_template(&self) -> &SessionConfig {
        &self.session_template
    }

    /// 订阅统一事件。可以在任意时刻多次调用（每个订阅者独立收）。
    pub fn events(&self) -> broadcast::Receiver<RealtimeEvent> {
        self.evt_tx.subscribe()
    }

    /// 连接是否还活着（reader/writer 任一出错都会置为 false）
    pub fn is_connected(&self) -> bool {
        !self.shared.closed.load(Ordering::SeqCst)
    }

    /// `session.created` 里的会话信息（还没收到时是 `None`）
    pub fn session_info(&self) -> Option<SessionInfo> {
        self.session_rx.borrow().clone()
    }

    /// 等 `session.created`（带超时）。超时返回 `None` 而不是挂死。
    pub async fn wait_session_created(&self, timeout: Duration) -> Option<SessionInfo> {
        let mut rx = self.session_rx.clone();
        let fut = async move {
            match rx.wait_for(|v| v.is_some()).await {
                Ok(v) => v.clone(),
                Err(_) => None, // 发送端已销毁（客户端已关）
            }
        };
        tokio::time::timeout(timeout, fut).await.ok().flatten()
    }

    /// 发一条客户端命令（统一入口，方便以后加命令）
    pub fn send_command(&self, cmd: ClientCommand) -> Result<()> {
        if !self.is_connected() {
            return Err(RealtimeError::Disconnected);
        }
        self.cmd_tx
            .send(cmd)
            .map_err(|_| RealtimeError::Disconnected)
    }

    /// 上行音频：`i16` PCM（16kHz / mono / 20ms 帧 = 320 采样）→ base64 → `input_audio_buffer.append`。
    ///
    /// 空切片直接返回 Ok 且不发——StepFun 对空 append 会回「音频内容不完整」错误，没必要制造噪声。
    pub fn send_audio(&self, pcm: &[i16]) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let audio = encode_pcm16_base64(pcm);
        self.send_command(ClientCommand::AppendAudio { audio })
    }

    /// 提交输入缓冲区（非 server_vad 模式用）
    pub fn commit(&self) -> Result<()> {
        self.send_command(ClientCommand::Commit)
    }

    /// 丢弃输入缓冲区里还没提交的音频（打断时清半句）
    pub fn clear_input(&self) -> Result<()> {
        self.send_command(ClientCommand::Clear)
    }

    /// 让模型开始一轮回复
    pub fn create_response(&self) -> Result<()> {
        self.send_command(ClientCommand::CreateResponse)
    }

    /// 取消当前回复（打断用户听到的这半句）
    pub fn cancel_response(&self) -> Result<()> {
        self.send_command(ClientCommand::CancelResponse)
    }

    /// 下发 4 个工具声明。
    ///
    /// 只发 `tools`（+ 现有 instructions），**不带 voice**：StepFun 的 voice 不可在
    /// `session.update` 里改，带上旧值属于自找 error 事件。
    pub fn send_tools(&self, tools: &[Value]) -> Result<()> {
        // 不变量 I1：语音模型手上只有 4 个工具，发之前先在本地拦一道
        if let Err(msg) = crate::tools::assert_tool_invariant(tools) {
            tracing::error!(error = %msg, "工具声明不满足不变量 I1，拒绝下发");
            return Err(RealtimeError::BadEndpoint(msg));
        }
        // StepFun 实测要 OpenAI 嵌套格式（扁平格式会回 error），在 provider 边界归一化一次。
        let normalized = crate::realtime::providers::stepfun::normalize_tools(tools);
        // 基于当前会话模板，只改 tools；voice 去掉（StepFun 的 voice 不可在 update 里改）。
        let session = self
            .session_template
            .clone()
            .without_voice()
            .with_tools(normalized);
        self.send_session_update(session)
    }

    /// 下发一份会话配置
    pub fn send_session_update(&self, session: SessionConfig) -> Result<()> {
        self.send_command(ClientCommand::SessionUpdate { session })
    }

    /// 往上下文里塞一条任意 item（`conversation.item.create`）
    /// 发原始 session.update（绕过 `SessionConfig` 的 `skip_serializing_if`）。
    ///
    /// 为什么需要它：StepFun 文档说「把 `turn_detection` 设为 **null** 可禁用 VAD」，
    /// 而 `SessionConfig.turn_detection` 是 `Option` + `skip_serializing_if=is_none`，
    /// 序列化时会**整个字段省略** —— 服务端看到「没这个字段」和「字段为 null」是两种行为。
    /// 禁用 VAD 必须真的发 `"turn_detection": null`。
    pub fn send_session_update_raw(&self, session: Value) -> Result<()> {
        self.send_command(ClientCommand::RawSessionUpdate { session })
    }

    pub fn send_item(&self, item: Value) -> Result<()> {
        self.send_command(ClientCommand::ConversationItemCreate { item })
    }

    /// 把工具执行结果回灌给模型（OpenAI Realtime 同构的 `function_call_output`）。
    ///
    /// 回灌后模型会继续说话（调研记录 §3.1 实测过这条行为）。
    pub fn send_function_call_output(&self, call_id: &str, output: &str) -> Result<()> {
        self.send_item(serde_json::json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": output,
        }))
    }

    /// 主动关掉（先发 Close 帧，再置断开标志）
    pub fn close(&self) {
        let _ = self.shared.shutdown.send(true);
        self.shared.closed.store(true, Ordering::SeqCst);
    }
}

impl Drop for RealtimeClient {
    fn drop(&mut self) {
        let _ = self.shared.shutdown.send(true);
        self.shared.closed.store(true, Ordering::SeqCst);
        if let Ok(tasks) = self.tasks.lock() {
            for t in tasks.iter() {
                t.abort();
            }
        }
    }
}

/// 读：Transport → 统一事件 → broadcast
async fn reader_loop<T: Transport>(
    transport: Arc<T>,
    evt_tx: broadcast::Sender<RealtimeEvent>,
    shared: Arc<Shared>,
    sample_rate: u32,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            msg = transport.next_event() => {
                match msg {
                    Some(Ok(raw)) => {
                        // session.created 额外记一份会话信息（含实际生效音色）
                        if let Some(info) = parse_session_info(&raw) {
                            let _ = shared.session.send(Some(info));
                        }
                        if let Some(event) = parse_server_event_with_rate(&raw, sample_rate) {
                            // 工具调用去重：StepFun 会把同一次调用报两遍（见 Shared::seen_tool_calls）
                            if shared.first_tool_call(&event) {
                                // 没有订阅者时 send 返回 Err，属正常，忽略
                                let _ = evt_tx.send(event);
                            } else {
                                tracing::debug!(event = event.kind(), "重复的工具调用事件，已去重");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "读取 WebSocket 失败，判定断线");
                        broadcast_disconnect(&evt_tx);
                        break;
                    }
                    None => {
                        tracing::info!("实时语音连接已关闭");
                        broadcast_disconnect(&evt_tx);
                        break;
                    }
                }
            }
        }
    }
    shared.closed.store(true, Ordering::SeqCst);
    transport.close().await;
}

/// 写：命令 → JSON → Transport
async fn writer_loop<T: Transport>(
    transport: Arc<T>,
    mut cmd_rx: mpsc::UnboundedReceiver<ClientCommand>,
    evt_tx: broadcast::Sender<RealtimeEvent>,
    shared: Arc<Shared>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => {
                        let json = client_command_json(&cmd);
                        let text = match serde_json::to_string(&json) {
                            Ok(t) => t,
                            Err(e) => {
                                // 理论不可达：命令结构都是纯数据
                                tracing::error!(error = %e, "命令序列化失败，丢弃");
                                continue;
                            }
                        };
                        if let Err(e) = transport.send_text(text).await {
                            tracing::warn!(error = %e, "发送失败，判定断线");
                            broadcast_disconnect(&evt_tx);
                            break;
                        }
                    }
                    None => break, // 客户端句柄已释放
                }
            }
        }
    }
    shared.closed.store(true, Ordering::SeqCst);
}

fn broadcast_disconnect(evt_tx: &broadcast::Sender<RealtimeEvent>) {
    let _ = evt_tx.send(RealtimeEvent::Error {
        message: "实时语音连接已断开".into(),
        code: Some("disconnected".into()),
    });
}

/// 从 `session.created` 原始 JSON 里取会话信息；音色单独取（统一事件里没这个字段）。
fn parse_session_info(raw: &str) -> Option<SessionInfo> {
    let v: Value = serde_json::from_str(raw).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session.created") {
        return None;
    }
    let session = v.get("session")?;
    Some(SessionInfo {
        session_id: session
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: session
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        voice: session
            .get("voice")
            .and_then(Value::as_str)
            .map(String::from),
    })
}

/// 端点补 model 查询参数。
///
/// 配置里的 `endpoint` 按约定是「完整 WS 端点（含 model）」，但环境变量覆盖时容易漏，
/// 这里兜一道：没有 `model=` 就按 `model` 字段补上。已经是完整 URL 的原样返回。
pub fn build_request_url(endpoint: &str, model: &str) -> Result<String> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() {
        return Err(RealtimeError::BadEndpoint("端点为空".into()));
    }
    if !(endpoint.starts_with("wss://") || endpoint.starts_with("ws://")) {
        return Err(RealtimeError::BadEndpoint(format!(
            "端点必须以 wss:// 或 ws:// 开头：{endpoint}"
        )));
    }
    if endpoint.contains("model=") {
        return Ok(endpoint.to_string());
    }
    let sep = if endpoint.contains('?') { '&' } else { '?' };
    Ok(format!("{endpoint}{sep}model={model}"))
}

/// rustls 的加密后端显式安装（幂等）。
///
/// 背景：`tokio-tungstenite` 对 rustls 用 `default-features = false`，只开 `std`，
/// 不带 ring / aws-lc-rs。当前 workspace 额外声明了 `rustls` + `ring` feature，
/// rustls 能自动推断；但万一以后有人再开 `aws-lc-rs`，自动推断就会变成「歧义」并 panic。
/// 这里显式装一次，feature 怎么组合都能连。
pub fn ensure_crypto_provider() {
    // 已安装时返回 Err(已安装的 provider)，忽略即可
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime::protocol::{parse_server_event, RealtimeEvent};

    // ---------- 假 Transport（不碰网络/声卡） ----------

    struct FakeTransport {
        inbound: tokio::sync::Mutex<mpsc::UnboundedReceiver<String>>,
        sent: tokio::sync::Mutex<Vec<String>>,
        closed: AtomicBool,
    }

    impl FakeTransport {
        fn new() -> (Arc<Self>, mpsc::UnboundedSender<String>) {
            let (tx, rx) = mpsc::unbounded_channel();
            let fake = Arc::new(Self {
                inbound: tokio::sync::Mutex::new(rx),
                sent: tokio::sync::Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
            });
            (fake, tx)
        }

        async fn sent_json(&self) -> Vec<Value> {
            self.sent
                .lock()
                .await
                .iter()
                .map(|s| serde_json::from_str(s).expect("发出的必须是合法 JSON"))
                .collect()
        }
    }

    impl Transport for FakeTransport {
        fn send_text<'a>(&'a self, text: String) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                if self.closed.load(Ordering::SeqCst) {
                    return Err(RealtimeError::Disconnected);
                }
                self.sent.lock().await.push(text);
                Ok(())
            })
        }

        fn next_event<'a>(&'a self) -> BoxFuture<'a, Option<Result<String>>> {
            Box::pin(async move { self.inbound.lock().await.recv().await.map(Ok) })
        }

        fn close<'a>(&'a self) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                self.closed.store(true, Ordering::SeqCst);
            })
        }
    }

    /// 等 writer 任务把前 n 条发出去（异步，必须轮询）
    async fn wait_sent(fake: &FakeTransport, n: usize) -> Vec<Value> {
        for _ in 0..400 {
            let sent = fake.sent_json().await;
            if sent.len() >= n {
                return sent;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("等待发送 {n} 条超时");
    }

    async fn recv_event(rx: &mut broadcast::Receiver<RealtimeEvent>) -> RealtimeEvent {
        match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            Ok(Ok(ev)) => ev,
            other => panic!("等事件超时/失败: {other:?}"),
        }
    }

    fn fake_client() -> (
        Arc<FakeTransport>,
        mpsc::UnboundedSender<String>,
        RealtimeClient,
    ) {
        let (fake, inbound) = FakeTransport::new();
        let client = RealtimeClient::with_transport(fake.clone(), 24_000).unwrap();
        (fake, inbound, client)
    }

    // ---------- 核心验收：audio.delta → AudioDelta(pcm 非空) ----------

    #[tokio::test]
    async fn audio_delta_becomes_audio_delta_event() {
        let (_fake, inbound, client) = fake_client();
        let mut events = client.events();

        // 320 采样 = 一帧 20ms 上行等效长度，内容非全零
        let pcm: Vec<i16> = (0..320).map(|i| (i as i16) * 3 - 500).collect();
        let raw = serde_json::json!({
            "type": "response.audio.delta",
            "item_id": "msg_008",
            "delta": encode_pcm16_base64(&pcm),
        })
        .to_string();
        inbound.send(raw).unwrap();

        match recv_event(&mut events).await {
            RealtimeEvent::AudioDelta {
                pcm: got,
                sample_rate,
            } => {
                assert_eq!(got, pcm, "PCM 必须逐样本还原");
                assert!(!got.is_empty(), "AudioDelta 的 pcm 不能为空");
                assert_eq!(sample_rate, 24_000);
            }
            other => panic!("期望 AudioDelta，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_and_malformed_frames_do_not_kill_the_loop() {
        let (_fake, inbound, client) = fake_client();
        let mut events = client.events();

        inbound.send("这不是 JSON".into()).unwrap();
        inbound
            .send(r#"{"type":"brand.new.event"}"#.into())
            .unwrap();
        inbound
            .send(
                serde_json::json!({
                    "type": "response.audio.delta",
                    "delta": encode_pcm16_base64(&[7, -7]),
                })
                .to_string(),
            )
            .unwrap();

        match recv_event(&mut events).await {
            RealtimeEvent::AudioDelta { pcm, .. } => assert_eq!(pcm, vec![7, -7]),
            other => panic!("坏帧后仍应继续解析，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn session_created_is_captured_even_if_subscribed_late() {
        let (_fake, inbound, client) = fake_client();
        inbound
            .send(
                serde_json::json!({
                    "type": "session.created",
                    "session": {
                        "id": "sess_001",
                        "model": "stepaudio-3-realtime-preview",
                        "voice": "jingdiannvsheng",
                    }
                })
                .to_string(),
            )
            .unwrap();

        let info = client
            .wait_session_created(Duration::from_secs(2))
            .await
            .expect("应收到 session.created");
        assert_eq!(info.session_id, "sess_001");
        assert_eq!(info.model, "stepaudio-3-realtime-preview");
        assert_eq!(info.voice.as_deref(), Some("jingdiannvsheng"));
        assert_eq!(client.session_info(), Some(info));
    }

    // ---------- 发送侧 ----------

    #[tokio::test]
    async fn send_audio_emits_base64_append() {
        let (fake, _inbound, client) = fake_client();
        let pcm: Vec<i16> = vec![0, 100, -100, 32000];
        client.send_audio(&pcm).unwrap();

        let sent = wait_sent(&fake, 1).await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["type"], "input_audio_buffer.append");
        let audio = sent[0]["audio"].as_str().unwrap();
        assert_eq!(
            crate::realtime::protocol::decode_pcm16_base64(audio).unwrap(),
            pcm,
            "上行 base64 必须能还原成同一串 PCM"
        );
    }

    #[tokio::test]
    async fn empty_audio_is_not_sent() {
        let (fake, _inbound, client) = fake_client();
        client.send_audio(&[]).unwrap();
        // 给 writer 一点时间；如果发了就会出现在 sent 里
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            fake.sent_json().await.is_empty(),
            "空音频不应产生 append（会触发服务端 400）"
        );
    }

    #[tokio::test]
    async fn control_commands_have_openai_wire_names() {
        let (fake, _inbound, client) = fake_client();
        client.create_response().unwrap();
        client.cancel_response().unwrap();
        client.commit().unwrap();
        client.clear_input().unwrap();

        let sent = wait_sent(&fake, 4).await;
        let types: Vec<&str> = sent.iter().map(|v| v["type"].as_str().unwrap()).collect();
        assert_eq!(
            types,
            vec![
                "response.create",
                "response.cancel",
                "input_audio_buffer.commit",
                "input_audio_buffer.clear",
            ]
        );
    }

    #[tokio::test]
    async fn send_tools_sends_four_tools_without_voice() {
        let (fake, _inbound, client) = fake_client();
        let tools = crate::tools::tool_defs();
        client.send_tools(&tools).unwrap();

        let sent = wait_sent(&fake, 1).await;
        let msg = &sent[0];
        assert_eq!(msg["type"], "session.update");
        let got = msg["session"]["tools"].as_array().unwrap();
        assert_eq!(got.len(), 4, "不变量 I1：恰好 4 个工具");
        // StepFun 要 OpenAI 嵌套格式：名称必须在 function.name 下（真机实测）
        for t in got {
            assert_eq!(t["type"], "function");
            assert!(
                t["function"]["name"].is_string(),
                "工具名必须放在 function.name：{t}"
            );
        }
        let mut names: Vec<&str> = got
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "append_requirement",
                "cancel_task",
                "delegate_task",
                "query_progress"
            ]
        );
        assert!(
            msg["session"].get("voice").is_none(),
            "session.update 不能重发 voice（StepFun voice 不可变更）"
        );
        assert_eq!(msg["session"]["input_audio_format"], "pcm16");
        assert_eq!(msg["session"]["output_audio_format"], "pcm16");
    }

    #[tokio::test]
    async fn send_tools_rejects_wrong_count() {
        let (_fake, _inbound, client) = fake_client();
        let bad = vec![serde_json::json!({"type":"function","name":"bash","parameters":{}})];
        let err = client.send_tools(&bad).unwrap_err();
        assert!(matches!(err, RealtimeError::BadEndpoint(_)), "实际 {err:?}");
    }

    #[tokio::test]
    async fn function_call_output_is_wrapped_as_item() {
        let (fake, _inbound, client) = fake_client();
        client
            .send_function_call_output("call_abc", "{\"ok\":true}")
            .unwrap();

        let sent = wait_sent(&fake, 1).await;
        assert_eq!(sent[0]["type"], "conversation.item.create");
        assert_eq!(sent[0]["item"]["type"], "function_call_output");
        assert_eq!(sent[0]["item"]["call_id"], "call_abc");
        assert_eq!(sent[0]["item"]["output"], "{\"ok\":true}");
    }

    #[tokio::test]
    async fn send_tools_keeps_session_instructions() {
        let (fake, _inbound) = FakeTransport::new();
        // 用非默认 instructions 建会话，确认 send_tools 不会把它冲回默认值
        let mut template = SessionConfig::from_realtime_config(&RealtimeConfig::default());
        template.instructions = "自定义指令：测试用".into();
        let client =
            RealtimeClient::with_transport_and_session(fake.clone(), template, 24_000).unwrap();

        client.send_tools(&crate::tools::tool_defs()).unwrap();
        let sent = wait_sent(&fake, 1).await;
        assert_eq!(
            sent[0]["session"]["instructions"], "自定义指令：测试用",
            "send_tools 不能覆盖调用方配置的 instructions"
        );
        assert!(sent[0]["session"].get("voice").is_none());
    }

    #[test]
    fn connect_template_carries_config_values() {
        let rc = RealtimeConfig::default();
        let template = SessionConfig::from_realtime_config(&rc);
        assert_eq!(template.voice.as_deref(), Some("jingdiannvsheng"));
        assert_eq!(template.input_audio_format, "pcm16");
        assert!(template.instructions.contains("小蝶"));
        // connect 不再替调用方塞工具：谁声明谁负责
        assert!(template.tools.is_none());
    }

    // ---------- 工具调用去重（真机实测：同一次调用报两遍） ----------

    #[tokio::test]
    async fn duplicate_tool_call_is_reported_once() {
        let (_fake, inbound, client) = fake_client();
        let mut events = client.events();

        // StepFun 实际会发这两条（同一 call_id / name）
        let item_done = serde_json::json!({
            "type": "response.output_item.done",
            "item": {"type": "function_call", "id": "fc_1", "call_id": "call_0_2727",
                     "name": "query_progress", "arguments": "{}"}
        })
        .to_string();
        let args_done = serde_json::json!({
            "type": "response.function_call_arguments.done",
            "call_id": "call_0_2727", "name": "query_progress", "arguments": "{}"
        })
        .to_string();
        inbound.send(item_done).unwrap();
        inbound.send(args_done).unwrap();

        match recv_event(&mut events).await {
            RealtimeEvent::ToolCall { name, .. } => assert_eq!(name, "query_progress"),
            other => panic!("期望 ToolCall，实际 {other:?}"),
        }
        // 第二条必须被吞掉，不能变成第二次派活
        let dup = tokio::time::timeout(Duration::from_millis(200), events.recv()).await;
        assert!(
            dup.is_err(),
            "同一 call_id 的工具调用只能上报一次，实际又收到 {dup:?}"
        );
    }

    #[tokio::test]
    async fn different_tool_calls_both_pass() {
        let (_fake, inbound, client) = fake_client();
        let mut events = client.events();
        for (cid, name) in [("call_a", "query_progress"), ("call_b", "delegate_task")] {
            inbound
                .send(
                    serde_json::json!({
                        "type": "response.output_item.done",
                        "item": {"type": "function_call", "call_id": cid, "name": name,
                                 "arguments": "{\"task_id\":\"t1\"}"}
                    })
                    .to_string(),
                )
                .unwrap();
        }
        let mut got = Vec::new();
        for _ in 0..2 {
            match recv_event(&mut events).await {
                RealtimeEvent::ToolCall { name, .. } => got.push(name),
                other => panic!("期望 ToolCall，实际 {other:?}"),
            }
        }
        got.sort();
        assert_eq!(got, vec!["delegate_task", "query_progress"]);
    }

    // ---------- 断线 ----------

    #[tokio::test]
    async fn disconnect_broadcasts_error_and_blocks_sends() {
        let (fake, inbound, client) = fake_client();
        let mut events = client.events();
        assert!(client.is_connected());

        drop(inbound); // 对端关闭 → next_event 返回 None

        let ev = recv_event(&mut events).await;
        match ev {
            RealtimeEvent::Error { code, .. } => assert_eq!(code.as_deref(), Some("disconnected")),
            other => panic!("期望 Error(disconnected)，实际 {other:?}"),
        }

        // 等 reader 任务把 closed 置 false
        for _ in 0..200 {
            if !client.is_connected() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!client.is_connected());
        assert!(matches!(
            client.send_audio(&[1, 2, 3]),
            Err(RealtimeError::Disconnected)
        ));
        let _ = fake;
    }

    #[tokio::test]
    async fn close_flips_connected_and_rejects_sends() {
        let (_fake, inbound, client) = fake_client();
        assert!(client.is_connected());

        client.close(); // 主动关闭 → 通知两个任务退出

        for _ in 0..200 {
            if !client.is_connected() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!client.is_connected(), "close() 后必须立即报告断开");
        assert!(matches!(
            client.create_response(),
            Err(RealtimeError::Disconnected)
        ));
        assert!(matches!(
            client.send_tools(&crate::tools::tool_defs()),
            Err(RealtimeError::Disconnected)
        ));
        // 关闭不应 panic；重复 close 也安全
        client.close();
        drop(inbound);
    }

    // ---------- 端点构造 ----------

    #[test]
    fn build_request_url_keeps_existing_model() {
        let url = "wss://api.stepfun.com/v1/realtime?model=stepaudio-3-realtime-preview";
        assert_eq!(build_request_url(url, "other").unwrap(), url);
    }

    #[test]
    fn build_request_url_appends_model_when_missing() {
        assert_eq!(
            build_request_url(
                "wss://api.stepfun.com/v1/realtime",
                "stepaudio-3-realtime-preview"
            )
            .unwrap(),
            "wss://api.stepfun.com/v1/realtime?model=stepaudio-3-realtime-preview"
        );
        assert_eq!(
            build_request_url("wss://api.stepfun.com/v1/realtime?foo=1", "m").unwrap(),
            "wss://api.stepfun.com/v1/realtime?foo=1&model=m"
        );
    }

    #[test]
    fn build_request_url_rejects_empty_and_bad_scheme() {
        assert!(matches!(
            build_request_url("", "m"),
            Err(RealtimeError::BadEndpoint(_))
        ));
        assert!(matches!(
            build_request_url("https://api.stepfun.com/v1/realtime", "m"),
            Err(RealtimeError::BadEndpoint(_))
        ));
    }

    #[test]
    fn auth_header_is_bearer_form() {
        // 只验证格式化形状，用假 key，不涉及真密钥
        let auth = format!("Bearer {}", "test-key-not-real");
        let v = HeaderValue::from_str(&auth).unwrap();
        assert_eq!(v.to_str().unwrap(), "Bearer test-key-not-real");
        // 含控制字符的「密钥」必须被拒（而不是 panic）
        assert!(HeaderValue::from_str("Bearer ab\ncd").is_err());
        // 中文密钥本身是合法 header 值（obs-text），只会在服务端被拒，不能因此 panic
        assert!(HeaderValue::from_str("Bearer 密钥").is_ok());
    }

    // ---------- 纯逻辑兜底：事件解析在 client 里也能用 ----------

    #[test]
    fn parse_server_event_is_reachable_via_client_module() {
        assert!(matches!(
            parse_server_event(r#"{"type":"input_audio_buffer.speech_started"}"#),
            Some(RealtimeEvent::SpeechStarted)
        ));
    }

    #[test]
    fn session_info_parser_ignores_other_events() {
        assert!(parse_session_info(r#"{"type":"session.updated"}"#).is_none());
        assert!(parse_session_info("garbage").is_none());
        let info =
            parse_session_info(r#"{"type":"session.created","session":{"id":"s1","model":"m1"}}"#)
                .unwrap();
        assert_eq!(info.voice, None, "服务端没给 voice 时不能瞎猜");
    }
}
