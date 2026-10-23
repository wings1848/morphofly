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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// DSH 桥
pub struct DshBridge {
    command: Vec<String>,
    default_cwd: String,
    /// 单个任务最长跑多久（超时后取消）
    task_timeout: Duration,
    /// 全局关停开关（App 退出时置 true）
    shutdown: Arc<AtomicBool>,
}

impl DshBridge {
    pub fn new(command: Vec<String>, default_cwd: String, task_timeout: Duration) -> Self {
        Self {
            command,
            default_cwd,
            task_timeout,
            shutdown: Arc::new(AtomicBool::new(false)),
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
    pub async fn run_task(&self, task: &str, cwd: &str) -> Result<String> {
        let cwd = if cwd.trim().is_empty() { &self.default_cwd } else { cwd };
        let mut client = DshClient::spawn(&self.command)
            .await
            .context("拉不起 DSH")?;
        let mut rx = client.subscribe();

        let sid = client.connect(cwd).await.context("建 DSH 会话失败")?;
        tracing::info!(session = %sid, cwd = %cwd, "DSH 会话已建");
        // 消费掉 SessionReady
        let _ = rx.recv().await;

        client.prompt(task).await.context("派活失败")?;

        let mut text = String::new();
        let deadline = tokio::time::Instant::now() + self.task_timeout;
        let stop_reason;

        loop {
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
                DshEvent::PermissionRequest { title, auto_approved } => {
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

        client.shutdown().await;

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
