//! 阶段 4 · 唤醒词 → 状态机 → 云端 的完整接缝验证（不依赖声学环境）。
//!
//! 为什么需要它：外放测试受物理环境影响（回声消除、音量、距离），
//! 不适合做自动化证据。这里把「KWS 命中 → 状态机 S0→S1 → 连云端 → S2」
//! 这段接缝单独拿出来，用真实音频文件驱动，结果可复现。
//!
//! 用法（wings-pc，需要 --features kws）：
//!   cargo run -q -p diemeng-core --features kws --example wake_integration -- \
//!     --file testdata/audio/01_wake_short.mp3

use diemeng_core::kws::{detect_in_file, wake_stimulus_for, KwsConfig};
use diemeng_core::session::{OrbState, SessionMachine, Stimulus};
use std::path::Path;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut file = "testdata/audio/01_wake_short.mp3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--file" => file = args.next().unwrap_or(file),
            "-h" | "--help" => {
                println!("用法: wake_integration [--file <音频>]");
                return Ok(());
            }
            _ => {}
        }
    }

    let cfg = KwsConfig::default();
    println!("[wake] 模型: {}", cfg.model_dir.display());
    println!("[wake] 关键词: {}", cfg.keywords_file.display());

    let mut machine = SessionMachine::new(Duration::from_secs(30));
    println!("[wake] 初始状态: {}", machine.state().as_str());
    anyhow::ensure!(machine.state() == OrbState::S0, "必须从待机态开始");

    // 1) KWS 离线识别
    let hits = detect_in_file(&cfg, Path::new(&file))?;
    println!(
        "[wake] KWS 识别 {}：{}",
        file,
        if hits.is_empty() {
            "未命中".to_string()
        } else {
            hits.iter()
                .map(|(t, k)| format!("{t:.1}s {k}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    anyhow::ensure!(!hits.is_empty(), "KWS 没识别出唤醒词：{file}");

    // 2) 命中 → 状态机 S0 → S1
    let stim = wake_stimulus_for(machine.state()).expect("S0 应该能接收唤醒");
    machine.handle(stim).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("[wake] 唤醒刺激 → 状态 {}（应 S1）", machine.state().as_str());
    anyhow::ensure!(machine.state() == OrbState::S1, "应进入 S1");

    // 3) 重复唤醒词不应再次触发（已在 S1）
    anyhow::ensure!(
        wake_stimulus_for(machine.state()).is_none(),
        "S1 不该再接受唤醒词"
    );
    println!("[wake] 重复唤醒词被忽略 ✓");

    // 4) 连上云端 → S2
    machine.handle(Stimulus::Connected).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("[wake] 连上云端 → 状态 {}（应 S2）", machine.state().as_str());
    anyhow::ensure!(machine.state() == OrbState::S2, "应进入 S2");

    // 5) 静音超时 → 回 S0
    machine.tick(std::time::Instant::now() + Duration::from_secs(31));
    println!("[wake] 静音超时 → 状态 {}（应 S0）", machine.state().as_str());
    anyhow::ensure!(machine.state() == OrbState::S0, "应回到 S0");

    println!("[wake] 迁移次数: {}", machine.transitions);
    println!("[wake] ✅ 唤醒 → 聆听 → 待机 完整接缝通");
    Ok(())
}
