//! 验证多 agent 切换真的能跑（不只是配置里有个名字）。
use morphofly_core::dsh::DshBridge;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .without_time()
        .init();
    let bridge = DshBridge::new(
        vec!["dsh".into(), "--profile".into(), "acp".into()],
        std::env::current_dir()?.display().to_string(),
        Duration::from_secs(120),
    );
    println!("可用 agent: {:?}", bridge.agent_names());

    for name in ["dsh", "codex"] {
        match bridge.command_for(name) {
            Some(cmd) => println!("  {name} -> {:?}", cmd),
            None => println!("  {name} -> 未配置"),
        }
    }

    // 实测 dsh 能不能跑
    println!("\n=== 实测 dsh ===");
    match bridge.run_task_as("t1", "dsh", "只回复：好", "").await {
        Ok(t) => println!(
            "✅ dsh 可用: {}",
            t.trim().chars().take(30).collect::<String>()
        ),
        Err(e) => println!("❌ dsh 失败: {e}"),
    }

    // 实测 codex（大概率不可用，如实报告）
    println!("\n=== 实测 codex ===");
    match bridge.run_task_as("t2", "codex", "只回复：好", "").await {
        Ok(t) => println!(
            "✅ codex 可用: {}",
            t.trim().chars().take(30).collect::<String>()
        ),
        Err(e) => println!("⚠️ codex 不可用: {e}"),
    }

    println!("\n=== 未知 agent ===");
    match bridge.run_task_as("t3", "不存在的", "x", "").await {
        Ok(_) => println!("❌ 竟然成功了，应该失败"),
        Err(e) => println!("✅ 正确拒绝: {e}"),
    }
    Ok(())
}
