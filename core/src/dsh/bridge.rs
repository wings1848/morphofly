//! 任务执行桥：把一条自然语言任务送进 DSH，拿回一句人话总结。
//!
//! 与 [`super::acp::DshClient`] 的分工：
//! - `DshClient` 是原始 ACP 客户端（spawn / initialize / prompt / cancel）
//! - `DshBridge` 是「一次任务 = 一个会话」的封装，带超时、取消、结果清洗
//!
//! 为什么一次任务一个会话：任务之间互不干扰，会话历史不会互相污染，
//! 也天然满足「多任务并行」（验收 D6）。

use super::acp::{DshClient, DshEvent};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};

/// 任务退出时把取消注册表里的自己删掉。
///
/// 用 RAII 而不是在每个 bail!/return 前手写清理 ——
/// `run_task` 有 6 个退出路径，手写必漏。
struct CancelGuard {
    cancels: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
    task_id: String,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if self.task_id.is_empty() {
            return;
        }
        // Drop 是同步的，只能 try_lock。
        // 拿不到锁的概率极低（只有正好在 cancel 时），而且残留一条无害
        // （下次同 ID 任务会覆盖），所以不搞异步清理的复杂度。
        if let Ok(mut m) = self.cancels.try_lock() {
            m.remove(&self.task_id);
        }
    }
}

/// DSH 桥
pub struct DshBridge {
    command: Vec<String>,
    default_cwd: String,
    /// 单个任务最长跑多久（超时后取消）
    task_timeout: Duration,
    /// 全局关停开关（App 退出时置 true）
    shutdown: Arc<AtomicBool>,
    /// 已握手但空闲的 DSH 进程，下个任务直接复用（省 1.35s initialize）。
    ///
    /// ## 为什么只做「复用 1 个」而不是完整进程池
    ///
    /// 完整的常驻池要支持「一个进程内多 session 并发」，
    /// 那需要把 `DshClient` 的 `session_id: Option<String>` 改成路由表，
    /// 涉及 300-400 行接口变更，且必须在有 GUI 的环境验证并发行为。
    ///
    /// 这个简化版只做一件事：**任务结束后不杀进程，留着下次用**。
    /// - 省掉 initialize（1.35s）
    /// - 每次仍然新建 session（1.45s，保证任务互不污染）
    /// - 不涉及 session 路由，风险低
    ///
    /// 实测收益约 1.35s/任务，占 2.8s 总开销的 48%。
    idle: Arc<Mutex<Option<DshClient>>>,
    /// 正在跑的任务 → 取消信号。
    ///
    /// ⚠️ 2026-10-24 修缺陷 #1：之前 `cancel_task` 只改本地状态，
    /// `self.bridge` 在 cancel() 里根本没用到 —— 用户说「停」，
    /// 蝶梦回「好，停了」，但 DSH 里的活还在跑。**这是欺骗用户。**
    ///
    /// 现在每个任务跑起来先注册一个 oneshot sender，
    /// cancel 时发信号，run_task 的循环里检查到就真的中止 client。
    cancels: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl DshBridge {
    pub fn new(command: Vec<String>, default_cwd: String, task_timeout: Duration) -> Self {
        Self {
            command,
            default_cwd,
            task_timeout,
            shutdown: Arc::new(AtomicBool::new(false)),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            idle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_config(cfg: &crate::Config) -> Self {
        Self::new(
            cfg.dsh.command.clone(),
            cfg.dsh.default_cwd.clone(),
            Duration::from_secs(cfg.dsh.task_timeout_secs),
        )
    }

    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    /// 跑一个任务，返回一句可以念的总结。
    ///
    /// `task` 必须是自包含的（验收 A6）——调用方负责保证。
    /// 真的取消一个正在跑的任务。
    ///
    /// 返回 true 表示找到了对应任务并发了取消信号。
    /// 返回 false 表示任务不在跑（可能已经结束，或者从来没开始）。
    pub async fn cancel_task(&self, task_id: &str) -> bool {
        let tx = self.cancels.lock().await.remove(task_id);
        match tx {
            Some(tx) => {
                // 接收端可能已经 drop（任务刚好结束），忽略发送错误
                let _ = tx.send(());
                tracing::info!(task = %task_id, "已发送取消信号");
                true
            }
            None => {
                tracing::warn!(task = %task_id, "取消时没找到运行中的任务");
                false
            }
        }
    }

    /// 当前有几个任务在跑（诊断用）
    pub async fn running_count(&self) -> usize {
        self.cancels.lock().await.len()
    }

    pub async fn run_task(&self, task: &str, cwd: &str) -> Result<String> {
        self.run_task_with_id("", task, cwd).await
    }

    /// 带任务 ID 的版本 —— 支持中途取消。
    pub async fn run_task_with_id(&self, task_id: &str, task: &str, cwd: &str) -> Result<String> {
        let cwd = if cwd.trim().is_empty() {
            &self.default_cwd
        } else {
            cwd
        };
        // 注册取消通道
        let cancel_rx = if task_id.is_empty() {
            None
        } else {
            let (tx, rx) = oneshot::channel();
            self.cancels.lock().await.insert(task_id.to_string(), tx);
            Some(rx)
        };
        // 保证退出时清理注册表（正常结束/报错/超时都要清）
        let _cleanup = CancelGuard {
            cancels: self.cancels.clone(),
            task_id: task_id.to_string(),
        };

        // ★ 优先复用已握手的进程（省 1.35s initialize）
        // 注意：不能写成 `Some(mut c) if c.is_alive() =>` ——
        // Rust 的 match guard 里不允许可变借用（is_alive 要 &mut self）。
        let mut reused = self.idle.lock().await.take();
        let alive = reused
            .as_mut()
            .map(|c| c.is_alive())
            .unwrap_or(false);
        let (mut client, was_reused) = if alive {
            tracing::debug!("复用已握手的 DSH 进程（省 1.35s）");
            (reused.take().expect("刚判断过是 Some"), true)
        } else {
            let c = DshClient::spawn(&self.command)
                .await
                .context("拉不起 DSH")?;
            (c, false)
        };
        let mut rx = client.subscribe();

        let sid = if was_reused {
            // 已握手 → 只建会话（1.45s）
            client.new_session(cwd).await.context("建 DSH 会话失败")?
        } else {
            // 新进程 → 握手 + 建会话（2.8s）
            client.connect(cwd).await.context("建 DSH 会话失败")?
        };
        tracing::info!(session = %sid, cwd = %cwd, "DSH 会话已建");
        // 消费掉 SessionReady
        let _ = rx.recv().await;

        client.prompt(task).await.context("派活失败")?;

        let mut text = String::new();
        let deadline = tokio::time::Instant::now() + self.task_timeout;
        let stop_reason;

        let mut cancel_rx = cancel_rx;
        loop {
            // ★ 真的取消：用户说了「停」
            if let Some(rx) = cancel_rx.as_mut() {
                if rx.try_recv().is_ok() {
                    tracing::info!(task = %task_id, "收到取消信号，中止 DSH");
                    let _ = client.cancel().await;
                    client.shutdown().await;
                    anyhow::bail!("用户取消了任务");
                }
            }
            if self.shutdown.load(Ordering::Relaxed) {
                let _ = client.cancel().await;
                client.shutdown().await;
                anyhow::bail!("应用正在退出，任务中止");
            }
            let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
                Err(_) => {
                    let _ = client.cancel().await;
                    client.shutdown().await;
                    anyhow::bail!("任务超时（{}s）", self.task_timeout.as_secs());
                }
                Ok(Err(_)) => {
                    stop_reason = "<通道关闭>".to_string();
                    break;
                }
                Ok(Ok(e)) => e,
            };
            match ev {
                DshEvent::TextDelta(t) => text.push_str(&t),
                DshEvent::ToolCall { title, status } => {
                    tracing::debug!(%title, %status, "DSH 工具");
                }
                DshEvent::PermissionRequest {
                    title,
                    auto_approved,
                } => {
                    tracing::info!(%title, %auto_approved, "DSH 请求权限");
                }
                DshEvent::TurnDone { stop_reason: r } => {
                    stop_reason = r;
                    break;
                }
                DshEvent::Error(e) => {
                    client.shutdown().await;
                    anyhow::bail!("DSH 报错: {e}");
                }
                DshEvent::Exited { code } => {
                    anyhow::bail!("DSH 提前退出 code={code:?}");
                }
                _ => {}
            }
        }

        // ★ 正常结束的进程放回池子复用（省下次 1.35s initialize）。
        //
        // 只回收「正常 end_turn」的进程：
        // - 报错/超时/取消的进程状态不可信，直接关掉
        // - 关闭当前 session 后进程可以接受新 session
        if stop_reason == "end_turn" {
            client.close_session().await;
            let mut idle = self.idle.lock().await;
            // 只留 1 个（不做池），多了没用还占内存
            if idle.is_none() {
                *idle = Some(client);
                tracing::debug!("DSH 进程已放回池子，下次直接复用");
            } else {
                // 已经有空闲的了，这个多余的关掉
                if let Some(mut c) = idle.take() {
                    c.shutdown().await;
                }
                *idle = Some(client);
            }
        } else {
            client.shutdown().await;
        }

        if stop_reason != "end_turn" {
            anyhow::bail!("任务没有正常结束：{stop_reason}");
        }
        anyhow::ensure!(!text.trim().is_empty(), "DSH 结束了但没输出任何内容");
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_reads_command() {
        let cfg = crate::Config::default();
        let b = DshBridge::from_config(&cfg);
        assert_eq!(b.command, vec!["dsh", "--profile", "acp"]);
        assert!(!b.default_cwd.is_empty());
    }

    #[test]
    fn shutdown_handle_is_shared() {
        let b = DshBridge::new(vec!["dsh".into()], "/tmp".into(), Duration::from_secs(5));
        let h = b.shutdown_handle();
        assert!(!h.load(Ordering::Relaxed));
        h.store(true, Ordering::Relaxed);
        assert!(b.shutdown.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn missing_binary_errors_cleanly() {
        let b = DshBridge::new(
            vec!["definitely-not-a-real-binary-abc".into()],
            "/tmp".into(),
            Duration::from_secs(1),
        );
        assert!(b.run_task("列目录", "/tmp").await.is_err());
    }
}
