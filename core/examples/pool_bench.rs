//! 实测「常驻进程复用」到底省了多少时间。
//!
//! 跑 3 次派活，打印每次的会话建立耗时。
//! 第 1 次应是完整的 2.8s（spawn + initialize + session/new），
//! 第 2、3 次应该快约 1.35s（只做 session/new）。

use morphofly_core::dsh::DshBridge;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .without_time()
        .init();

    let cwd = std::env::current_dir()?.display().to_string();
    let bridge = DshBridge::new(
        vec!["dsh".into(), "--profile".into(), "acp".into()],
        cwd,
        Duration::from_secs(120),
    );

    let mut timings = Vec::new();
    for i in 1..=3 {
        let t = Instant::now();
        // 用一个极简任务，主要测「建会话 + 跑完」的总时间
        match bridge.run_task("只回复两个字：收到", "").await {
            Ok(text) => {
                let ms = t.elapsed().as_millis();
                timings.push(ms);
                println!(
                    "第 {i} 次: {ms}ms  → {}",
                    text.trim().chars().take(20).collect::<String>()
                );
            }
            Err(e) => {
                println!("第 {i} 次失败: {e}");
                timings.push(0);
            }
        }
    }

    println!("\n=== 结果 ===");
    for (i, ms) in timings.iter().enumerate() {
        println!("  第 {} 次: {ms}ms", i + 1);
    }
    if timings.len() == 3 && timings[0] > 0 && timings[1] > 0 {
        let saved = timings[0] as i64 - timings[1] as i64;
        println!("\n第 1 次 vs 第 2 次: 省 {saved}ms");
        println!(
            "{}",
            if saved > 800 {
                "✅ 复用生效（应省约 1350ms）"
            } else {
                "⚠️ 省得不多，可能没复用上"
            }
        );
    }
    Ok(())
}
