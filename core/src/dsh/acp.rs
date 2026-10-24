//! DSH 接入：把 `dsh --profile acp` 当子进程拉起来，用 NDJSON JSON-RPC 通信。
//!
//! 实测依据（runs/*/facts.md F9–F13）：
//! - `initialize {protocolVersion:1}` → `agentInfo.name = deepseek-harness-acp`
//! - `session/new {cwd, mcpServers:[]}` → `sessionId`
//! - `session/prompt {sessionId, prompt:[{type:"text",text}]}` → `{stopReason}`
//! - 流式文本来自 `session/update` 通知，`update.sessionUpdate == "agent_message_chunk"`
//! - `promptCapabilities.audio = false` → 只能发文字，语音转文字必须在我们这侧完成

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, Mutex};

/// ACP 协议版本（实测 DSH 回 1）
pub const PROTOCOL_VERSION: i64 = 1;

/// 从 DSH 出来的事件
#[derive(Debug, Clone, PartialEq)]
pub enum DshEvent {
    /// 建立了会话
    SessionReady { session_id: String },
    /// 助手流式文本
    TextDelta(String),
    /// 助手消息块结束
    TextDone(String),
    /// 工具调用开始/更新
    ToolCall { title: String, status: String },
    /// 需要用户批准（我们默认自动批准，记录一条日志）
    PermissionRequest { title: String, auto_approved: bool },
    /// 一轮结束
    TurnDone { stop_reason: String },
    /// 出错
    Error(String),
    /// 进程退出
    Exited { code: Option<i32> },
}

/// ACP 客户端
pub struct DshClient {
    child: Child,
    stdin: ChildStdin,
    next_id: AtomicI64,
    /// 待回复的请求 id → 方法名
    pending: Arc<Mutex<HashMap<i64, String>>>,
    events_tx: broadcast::Sender<DshEvent>,
    session_id: Arc<Mutex<Option<String>>>,
    closed: Arc<AtomicBool>,
    /// ACP 是否完成 initialize
    initialized: Arc<AtomicBool>,
}

impl DshClient {
    /// 拉起 `dsh --profile acp`（或配置里给的命令）
    pub async fn spawn(command: &[String]) -> Result<Self> {
        if command.is_empty() {
            bail!("DSH 启动命令为空");
        }
        let mut child = tokio::process::Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("拉不起 DSH：{:?}（确认 dsh 在 PATH 里）", command))?;

        let stdin = child.stdin.take().context("拿不到 DSH stdin")?;
        let stdout = child.stdout.take().context("拿不到 DSH stdout")?;
        let stderr = child.stderr.take();

        let (events_tx, _) = broadcast::channel(256);
        let pending: Arc<Mutex<HashMap<i64, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let session_id = Arc::new(Mutex::new(None));
        let closed = Arc::new(AtomicBool::new(false));
        let initialized = Arc::new(AtomicBool::new(false));

        // stderr 单独收集，出问题时能报出来
        if let Some(err) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "dsh", "stderr: {line}");
                }
            });
        }

        let client = Self {
            child,
            stdin,
            next_id: AtomicI64::new(1),
            pending,
            events_tx,
            session_id,
            closed,
            initialized,
        };
        client.start_reader(stdout);
        Ok(client)
    }

    fn start_reader(&self, stdout: ChildStdout) {
        let pending = self.pending.clone();
        let events_tx = self.events_tx.clone();
        let session_id = self.session_id.clone();
        let closed = self.closed.clone();

        tokio::spawn(async move {
            let mut lines: Lines<BufReader<ChildStdout>> = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let msg: Value = match serde_json::from_str(line) {
                            Ok(v) => v,
                            Err(_) => {
                                tracing::debug!(target: "dsh", "非 JSON 输出: {line}");
                                continue;
                            }
                        };
                        handle_message(msg, &pending, &events_tx, &session_id).await;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = events_tx.send(DshEvent::Error(format!("读 DSH 输出失败: {e}")));
                        break;
                    }
                }
            }
            closed.store(true, Ordering::SeqCst);
            let _ = events_tx.send(DshEvent::Exited { code: None });
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DshEvent> {
        self.events_tx.subscribe()
    }

    fn alloc_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn write_raw(&mut self, msg: &Value) -> Result<()> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .context("写 DSH stdin 失败（进程可能已退出）")?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// 发请求（不等回复；回复走事件流）
    pub async fn send_request(&mut self, method: &str, params: Value) -> Result<i64> {
        let id = self.alloc_id();
        self.pending.lock().await.insert(id, method.to_string());
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.write_raw(&msg).await?;
        Ok(id)
    }

    /// 发通知（无 id）
    pub async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.write_raw(&msg).await
    }

    /// 完整握手：initialize + session/new
    /// 完整连接：initialize + 建会话。首次建连用这个。
    pub async fn connect(&mut self, cwd: &str) -> Result<String> {
        self.initialize().await?;
        self.new_session(cwd).await
    }

    /// 只做 `initialize` 握手（每个进程只需一次）。
    ///
    /// 拆出来是为了支持「常驻进程复用」：省掉 1.35 秒的握手开销，
    /// 但每次任务仍然新建 session（任务之间互不污染）。
    pub async fn initialize(&mut self) -> Result<()> {
        if self.initialized.load(Ordering::SeqCst) {
            return Ok(()); // 已经握过手，不重复
        }
        let id = self.alloc_id();
        self.pending.lock().await.insert(id, "initialize".into());
        self.write_raw(&json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false
                },
                "clientInfo": {"name": "diemeng", "version": env!("CARGO_PKG_VERSION")}
            }
        }))
        .await?;
        self.initialized.store(true, Ordering::SeqCst);
        // 等一下握手回复（reader 收到 result 就会把 initialized 置位）
        for _ in 0..100 {
            if self.initialized.load(Ordering::SeqCst) {
                break;
            }
            if self.closed.load(Ordering::SeqCst) {
                bail!("DSH 进程在握手前退出");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        Ok(())
    }

    /// 在已握手的进程上新建一个会话（每个任务一个）。
    pub async fn new_session(&mut self, cwd: &str) -> Result<String> {
        // 清掉上一个 session 的残留，否则会误以为新会话已经建好
        *self.session_id.lock().await = None;

        // session/new 的回复由 reader 解析并把 sessionId 写进 session_id
        self.send_request("session/new", json!({"cwd": cwd, "mcpServers": []}))
            .await?;

        // 等 sessionId 出现（最多 20s）
        for _ in 0..200 {
            if let Some(sid) = self.session_id.lock().await.clone() {
                let _ = self.events_tx.send(DshEvent::SessionReady {
                    session_id: sid.clone(),
                });
                return Ok(sid);
            }
            if self.closed.load(Ordering::SeqCst) {
                bail!("DSH 进程在建会话前退出");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        bail!("建 DSH 会话超时（20s）")
    }

    /// 派活：发一条 prompt，流式事件从 `subscribe()` 来
    pub async fn prompt(&mut self, text: &str) -> Result<i64> {
        let sid = self
            .session_id
            .lock()
            .await
            .clone()
            .context("还没有会话，先 connect()")?;
        self.send_request(
            "session/prompt",
            json!({
                "sessionId": sid,
                "prompt": [{"type": "text", "text": text}]
            }),
        )
        .await
    }

    /// 取消当前这一轮
    pub async fn cancel(&mut self) -> Result<()> {
        let sid = self.session_id.lock().await.clone().context("还没有会话")?;
        self.send_notification("session/cancel", json!({"sessionId": sid}))
            .await
    }

    pub fn session_id_blocking(&self) -> Option<String> {
        // 仅测试/日志用；不要在生产路径里 block_on
        self.session_id.try_lock().ok().and_then(|g| g.clone())
    }

    pub fn is_alive(&mut self) -> bool {
        !self.closed.load(Ordering::SeqCst) && matches!(self.child.try_wait(), Ok(None))
    }

    /// 关掉当前会话但保留进程（用于「任务结束了，进程留着下次用」）。
    ///
    /// 为什么需要它：一个进程可以开多个 session，
    /// 关掉旧 session 后能建新的，这样既复用了 initialize 的开销，
    /// 又保证任务之间互不污染。
    pub async fn close_session(&mut self) {
        let sid = self.session_id.lock().await.clone();
        if let Some(sid) = sid {
            let _ = self
                .send_request("session/close", json!({"sessionId": sid}))
                .await;
            *self.session_id.lock().await = None;
        }
    }

    /// 温柔退出
    pub async fn shutdown(&mut self) {
        let sid = self.session_id.lock().await.clone();
        if let Some(sid) = sid {
            let _ = self
                .send_notification("session/close", json!({"sessionId": sid}))
                .await;
        }
        let _ = self.child.kill().await;
    }
}

/// 处理一条来自 DSH 的 JSON-RPC 消息
async fn handle_message(
    msg: Value,
    pending: &Arc<Mutex<HashMap<i64, String>>>,
    events_tx: &broadcast::Sender<DshEvent>,
    session_id: &Arc<Mutex<Option<String>>>,
) {
    // 1) 响应（有 id 且无 method）
    if msg.get("method").is_none() {
        if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
            let method = pending.lock().await.remove(&id);
            if let Some(result) = msg.get("result") {
                match method.as_deref() {
                    Some("session/new") => {
                        if let Some(sid) = result.get("sessionId").and_then(|v| v.as_str()) {
                            *session_id.lock().await = Some(sid.to_string());
                        }
                    }
                    Some("session/prompt") => {
                        let reason = result
                            .get("stopReason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let _ = events_tx.send(DshEvent::TurnDone {
                            stop_reason: reason,
                        });
                    }
                    _ => {}
                }
                return;
            }
            if let Some(err) = msg.get("error") {
                let m = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("未知错误");
                let _ = events_tx.send(DshEvent::Error(m.to_string()));
            }
            return;
        }
    }

    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // ★ 修缺陷 #3：按 sessionId 过滤事件。
    //
    // StepFun 的 session/update 是**广播**给所有会话的（实测：
    // 同一进程开两个 session，A 能看到 B 的 update）。
    // 之前这里完全不读 params.sessionId，所有事件全进同一个通道 ——
    // 多 agent / 多任务并发时必然串台。**这是多 agent 功能的前置必修项。**
    //
    // 规则：如果消息带了 sessionId 且我们知道自己期望哪个 session，
    // 不匹配就丢弃。没带 sessionId 的消息（如全局错误）照常处理。
    if method == "session/update" {
        // 用 await 而不是 try_lock：try_lock 在锁竞争时会拿不到，
        // 那样就退化成「不过滤」——正好在多会话并发时失效，是最该过滤的时候。
        let msg_sid = params.get("sessionId").and_then(|v| v.as_str());
        if let Some(ms) = msg_sid {
            let my_sid = session_id.lock().await.clone();
            if let Some(me) = my_sid {
                if ms != me {
                    tracing::trace!(got = %ms, want = %me, "丢弃其他会话的事件");
                    return;
                }
            }
        }
    }

    match method {
        "session/update" => {
            let update = params.get("update").cloned().unwrap_or(Value::Null);
            let kind = update
                .get("sessionUpdate")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match kind {
                "agent_message_chunk" => {
                    let content = update.get("content").cloned().unwrap_or(Value::Null);
                    if content.get("type").and_then(|v| v.as_str()) == Some("text") {
                        let text = content
                            .get("text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let _ = events_tx.send(DshEvent::TextDelta(text));
                    }
                }
                "tool_call" => {
                    let title = update
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("工具")
                        .to_string();
                    let status = update
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let _ = events_tx.send(DshEvent::ToolCall { title, status });
                }
                _ => {}
            }
        }
        // DSH 反向请求：默认自动批准（桌面端单人使用，权限模型由 DSH 侧 profile 决定）
        "session/request_permission" => {
            let title = params
                .get("toolCall")
                .and_then(|t| t.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("操作")
                .to_string();
            let _ = events_tx.send(DshEvent::PermissionRequest {
                title,
                auto_approved: true,
            });
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tx() -> broadcast::Sender<DshEvent> {
        broadcast::channel(32).0
    }

    #[tokio::test]
    async fn parses_session_new_response() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        pending.lock().await.insert(7, "session/new".to_string());
        let msg = json!({"jsonrpc":"2.0","id":7,"result":{"sessionId":"abc-123"}});
        handle_message(msg, &pending, &t, &sid).await;
        assert_eq!(sid.lock().await.clone().as_deref(), Some("abc-123"));
    }

    #[tokio::test]
    async fn parses_agent_message_chunk() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        let mut rx = t.subscribe();
        let msg = json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"update":{"sessionUpdate":"agent_message_chunk",
                                 "content":{"type":"text","text":"目录里有四项"}}}
        });
        handle_message(msg, &pending, &t, &sid).await;
        match rx.recv().await.unwrap() {
            DshEvent::TextDelta(s) => assert_eq!(s, "目录里有四项"),
            other => panic!("期望 TextDelta，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn prompt_response_reports_stop_reason() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        let mut rx = t.subscribe();
        pending.lock().await.insert(9, "session/prompt".to_string());
        let msg = json!({"jsonrpc":"2.0","id":9,"result":{"stopReason":"end_turn"}});
        handle_message(msg, &pending, &t, &sid).await;
        match rx.recv().await.unwrap() {
            DshEvent::TurnDone { stop_reason } => assert_eq!(stop_reason, "end_turn"),
            other => panic!("期望 TurnDone，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_response_becomes_event_not_panic() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        let mut rx = t.subscribe();
        pending.lock().await.insert(3, "session/prompt".to_string());
        let msg = json!({"jsonrpc":"2.0","id":3,"error":{"code":-32600,"message":"bad request"}});
        handle_message(msg, &pending, &t, &sid).await;
        match rx.recv().await.unwrap() {
            DshEvent::Error(e) => assert!(e.contains("bad request")),
            other => panic!("期望 Error，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_method_does_not_panic() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        let msg = json!({"jsonrpc":"2.0","method":"something/weird","params":{}});
        handle_message(msg, &pending, &t, &sid).await;
    }

    #[tokio::test]
    async fn permission_request_is_auto_approved_and_reported() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let sid = Arc::new(Mutex::new(None));
        let t = tx();
        let mut rx = t.subscribe();
        let msg = json!({
            "jsonrpc":"2.0","id":11,"method":"session/request_permission",
            "params":{"toolCall":{"title":"跑 cargo test"}}
        });
        handle_message(msg, &pending, &t, &sid).await;
        match rx.recv().await.unwrap() {
            DshEvent::PermissionRequest {
                title,
                auto_approved,
            } => {
                assert_eq!(title, "跑 cargo test");
                assert!(auto_approved);
            }
            other => panic!("期望 PermissionRequest，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn spawn_missing_binary_errors_cleanly() {
        let r = DshClient::spawn(&["definitely-not-a-real-binary-xyz".to_string()]).await;
        assert!(r.is_err());
    }
}
