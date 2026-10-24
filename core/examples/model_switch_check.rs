//! 实测模型切换是否真的生效（不只是改字段）。
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

    println!("=== 1) 没有活跃进程时切模型 ===");
    match bridge
        .set_model(r#"["deepseek-official","deepseek-v4-pro"]"#)
        .await
    {
        Ok(()) => println!("  ❌ 竟然成功了，应该失败"),
        Err(e) => println!("  ✅ 正确拒绝: {e}"),
    }

    println!("\n=== 2) 先跑个任务（产生活跃进程）===");
    match bridge.run_task("只回复：好", "").await {
        Ok(t) => println!("  ✅ 任务完成: {}", t.trim()),
        Err(e) => println!("  ❌ 失败: {e}"),
    }

    println!("\n=== 3) 现在切模型 ===");
    for m in [
        r#"["deepseek-official","deepseek-v4-pro"]"#,
        "deepseek-v4-pro",
        r#"["deepseek-official","不存在的模型"]"#,
    ] {
        match bridge.set_model(m).await {
            Ok(()) => println!("  ✅ {m} → 成功"),
            Err(e) => println!("  ⚠️ {m} → {e}"),
        }
    }
    Ok(())
}
