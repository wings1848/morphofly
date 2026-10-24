//! 接缝 0.7：把任务派给 DSH 并拿回结果。
//!
//! 验收命令：
//!   cargo run -p morphofly-core --example delegate -- '列一下 /tmp 里有什么'
//!   DELEGATE_TIMEOUT=120 cargo run ... （默认 180s）

use morphofly_core::dsh::acp::{DshClient, DshEvent};
use morphofly_core::Config;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let task = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let task = if task.trim().is_empty() {
        "运行 `ls /home/wings/Code_Project/morphofly`，用一句话告诉我里面有哪些文件。不要修改任何文件。".to_string()
    } else {
        task
    };
    let timeout: u64 = std::env::var("DELEGATE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180);

    let cfg = Config::load().unwrap_or_default();
    let cwd = std::env::var("MORPHOFLY_CWD").unwrap_or_else(|_| cfg.dsh.default_cwd.clone());
    let t0 = std::time::Instant::now();

    println!("[delegate] 启动 DSH: {:?}", cfg.dsh.command);
    let mut client = DshClient::spawn(&cfg.dsh.command).await?;
    let mut rx = client.subscribe();

    let sid = client.connect(&cwd).await?;
    let t_connect = t0.elapsed().as_millis();
    println!("[delegate] 会话建立 sessionId={sid}，耗时 {t_connect}ms");

    let _ = rx.recv().await; // SessionReady
    let t1 = std::time::Instant::now();
    client.prompt(&task).await?;
    println!("[delegate] 已派活，等结果…");

    let mut text = String::new();
    let mut stop_reason = String::from("<超时>");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout);

    loop {
        let ev = tokio::time::timeout_at(deadline, rx.recv()).await;
        match ev {
            Err(_) => {
                println!("[delegate] 超时 {timeout}s");
                break;
            }
            Ok(Err(_)) => break,
            Ok(Ok(DshEvent::TextDelta(t))) => {
                print!("{t}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                text.push_str(&t);
            }
            Ok(Ok(DshEvent::ToolCall { title, status })) => {
                eprintln!("\n[delegate] 工具 {title} → {status}");
            }
            Ok(Ok(DshEvent::PermissionRequest {
                title,
                auto_approved,
            })) => {
                eprintln!("\n[delegate] 请求权限 {title}，自动批准={auto_approved}");
            }
            Ok(Ok(DshEvent::TurnDone { stop_reason: r })) => {
                stop_reason = r;
                break;
            }
            Ok(Ok(DshEvent::Error(e))) => {
                eprintln!("\n[delegate] 错误: {e}");
                stop_reason = format!("error: {e}");
                break;
            }
            Ok(Ok(DshEvent::Exited { code })) => {
                eprintln!("\n[delegate] DSH 退出 code={code:?}");
                break;
            }
            Ok(Ok(_)) => {}
        }
    }

    let total = t1.elapsed().as_millis();
    println!(
        "\n[delegate] stopReason={stop_reason} 本轮耗时 {total}ms 文本 {} 字",
        text.chars().count()
    );
    client.shutdown().await;

    if stop_reason == "end_turn" && !text.trim().is_empty() {
        // 写到 /tmp 供断言脚本读取（不进仓库）
        let out = serde_json::json!({
            "session_id": sid,
            "connect_ms": t_connect,
            "turn_ms": total,
            "stop_reason": stop_reason,
            "text": text,
            "cwd": cwd,
        });
        let p =
            std::env::var("DELEGATE_OUT").unwrap_or_else(|_| "/tmp/diemeng-delegate.json".into());
        std::fs::write(&p, serde_json::to_string_pretty(&out)?)?;
        println!("[delegate] ✅ 结果已写 {p}");
        Ok(())
    } else {
        anyhow::bail!("派活失败：stopReason={stop_reason} 文本长度={}", text.len())
    }
}
