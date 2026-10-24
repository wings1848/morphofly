//! 阶段 4 · Rust 侧唤醒词离线验证（不需要麦克风）。
//!
//! 这是 `tools/kws/kws_check.py` 的 Rust 等价物，验证 sherpa-onnx Rust 绑定能识别「小蝶」。
//!
//! 用法（wings-pc，需 --features kws）：
//!   cargo run -q -p morphofly-core --features kws --example kws_check -- \
//!     --model /tmp/kws/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20 \
//!     --keywords /tmp/kws/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20/keywords.txt \
//!     testdata/audio/
//!
//! 判定：
//! - 正样本（文件名 01_/02_/03_/06_/07_/08_/09_/16_/17_）应唤醒
//! - 负样本（04_/05_/10_/11_~/15_）不应唤醒
//! - 末尾打印召回/误唤醒统计

use morphofly_core::kws::{detect_in_file, KwsConfig};
use std::path::{Path, PathBuf};

/// 正样本（应该唤醒）：含「小蝶」的唤醒句
const POSITIVE: &[&str] = &[
    "01_", "02_", "03_", "06_", "07_", "08_", "09_", "16_", "17_",
];

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let mut model_dir: Option<PathBuf> = None;
    let mut keywords: Option<PathBuf> = None;
    let mut targets: Vec<PathBuf> = Vec::new();
    let mut blanks: i32 = 4;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--model" => model_dir = args.next().map(PathBuf::from),
            "--keywords" => keywords = args.next().map(PathBuf::from),
            "--blanks" => blanks = args.next().unwrap_or("4".into()).parse()?,
            "-h" | "--help" => {
                println!("用法: kws_check --model <模型目录> --keywords <关键词文件> [--blanks 4] <音频文件或目录>...");
                return Ok(());
            }
            other => targets.push(PathBuf::from(other)),
        }
    }
    let model_dir =
        model_dir.unwrap_or_else(|| morphofly_core::config::default_data_dir().join("kws-model"));
    let keywords =
        keywords.unwrap_or_else(|| morphofly_core::config::default_data_dir().join("keywords.txt"));
    anyhow::ensure!(!targets.is_empty(), "至少给一个音频文件或目录");

    let cfg = KwsConfig {
        model_dir,
        keywords_file: keywords,
        num_trailing_blanks: blanks,
        ..Default::default()
    };
    println!("[kws_check] 模型: {}", cfg.model_dir.display());
    println!("[kws_check] 关键词: {}", cfg.keywords_file.display());
    println!(
        "[kws_check] num_trailing_blanks = {}",
        cfg.num_trailing_blanks
    );

    // 收集音频文件
    let mut files: Vec<PathBuf> = Vec::new();
    for t in &targets {
        if t.is_dir() {
            for e in std::fs::read_dir(t)?.flatten() {
                let p = e.path();
                if matches!(
                    p.extension().and_then(|s| s.to_str()),
                    Some("mp3") | Some("wav") | Some("m4a") | Some("flac") | Some("ogg")
                ) {
                    files.push(p);
                }
            }
        } else {
            files.push(t.clone());
        }
    }
    files.sort();
    anyhow::ensure!(!files.is_empty(), "没找到音频文件");

    let mut pos_hit = 0;
    let mut pos_total = 0;
    let mut neg_hit = 0;
    let mut neg_total = 0;

    for f in &files {
        let name = f
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let should_wake = POSITIVE.iter().any(|p| name.starts_with(p));
        let hits = match detect_in_file(&cfg, f) {
            Ok(h) => h,
            Err(e) => {
                println!("❌ {name:<32} 处理失败: {e}");
                continue;
            }
        };
        let woke = !hits.is_empty();
        let mark = if woke == should_wake { "✅" } else { "❌" };
        let detail = hits
            .iter()
            .map(|(t, k)| format!("{t:.1}s {k}"))
            .collect::<Vec<_>>()
            .join(", ");
        let expect = if should_wake {
            "应唤醒"
        } else {
            "不应唤醒"
        };
        println!(
            "{mark} {name:<32} {expect:<8} {}",
            if woke {
                format!("[{detail}]")
            } else {
                String::new()
            }
        );
        if should_wake {
            pos_total += 1;
            if woke {
                pos_hit += 1;
            }
        } else {
            neg_total += 1;
            if woke {
                neg_hit += 1;
            }
        }
    }

    println!("\n===== 统计 =====");
    println!(
        "召回：{pos_hit}/{pos_total} = {:.0}%",
        if pos_total > 0 {
            pos_hit as f32 / pos_total as f32 * 100.0
        } else {
            0.0
        }
    );
    println!(
        "误唤醒：{neg_hit}/{neg_total} = {:.0}%",
        if neg_total > 0 {
            neg_hit as f32 / neg_total as f32 * 100.0
        } else {
            0.0
        }
    );
    anyhow::ensure!(pos_total > 0 && neg_total > 0, "样本不足，无法判定");
    anyhow::ensure!(
        pos_hit == pos_total,
        "召回不是 100%（{pos_hit}/{pos_total}）"
    );
    anyhow::ensure!(
        neg_hit == 0,
        "有误唤醒（{neg_hit}/{neg_total}）—— 检查 num_trailing_blanks 是否 >= 4"
    );
    println!("✅ Rust 侧 KWS 与 Python 侧结果一致：全召回、零误唤醒");
    Ok(())
}
