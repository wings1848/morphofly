//! 阶段 4 骨架：唤醒词 + 已进入通话 的两种状态循环（概念验证）。
//!
//! ⚠️ 这是**接口骨架**，不是完整实现：
//! Rust 侧还没引入 sherpa-onnx 绑定，所以这里用一个「关键词出现即唤醒」的
//! 简易能量门限做占位，真实 KWS 走 sherpa-onnx（Python 侧已实测 9/9 召回 0/8 误唤醒）。
//!
//! 这个 example 的价值是：把「KWS 命中 → 状态机 S0→S1 → 连云端 → S2」这条接缝
//! 先固定下来，等 sherpa-onnx 接进来时只换中间的检测器。
//!
//! 用法（wings-pc）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p morphofly-core --example wake_word -- --seconds 20

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::session::{OrbState, SessionMachine, Stimulus};
use diemeng_core::spectrum::rms_to_visual;
use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let mut seconds = 20u64;
    let mut file: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().unwrap_or("20".into()).parse()?,
            "--file" => file = args.next(),
            "-h" | "--help" => {
                println!("用法: wake_word [--seconds N] [--file <音频>]");
                return Ok(());
            }
            _ => {}
        }
    }

    let mic = match file {
        Some(ref p) => {
            println!("[wake_word] 上行来源：文件 {p}（代替麦克风）");
            AudioCapture::file_source(p)?
        }
        None => AudioCapture::start_default()?,
    };
    let mut frames = mic.subscribe();
    let mut machine = SessionMachine::new(Duration::from_secs(30));

    println!("[wake_word] 待机监听中（喊「小蝶」测试；{seconds}s 后退出）");
    println!("[wake_word] ⚠️ 当前用能量门限代替真实 KWS —— 只验证状态机接缝，不验证识别率");

    let start = std::time::Instant::now();
    let mut last_state = machine.state();
    // 能量门限：连续 3 帧超过阈值才算「有人说话」（防抖）
    let mut hot_frames = 0;
    let mut announced = false;

    while start.elapsed().as_secs() < seconds {
        match frames.blocking_recv() {
            Ok(frame) => {
                let rms = diemeng_core::spectrum::rms_i16(&frame);
                let visual = rms_to_visual(rms);

                // 占位「唤醒检测」：大声说话 = 唤醒
                if machine.state() == OrbState::S0 && visual > 0.55 {
                    hot_frames += 1;
                    if hot_frames >= 3 {
                        hot_frames = 0;
                        // S0 --WakeWord--> S1
                        let _ = machine.handle(Stimulus::WakeWord);
                        println!(
                            "[wake_word] 🔊 检测到唤醒（视觉强度 {visual:.2}）→ {}",
                            machine.state().as_str()
                        );
                        // 真实实现这里会连云端，成功后再 handle(Connected)
                        let _ = machine.handle(Stimulus::Connected);
                        println!("[wake_word] → {} 聆听中…", machine.state().as_str());
                        announced = true;
                    }
                } else if machine.state() == OrbState::S2 && visual < 0.05 {
                    hot_frames += 1;
                    if hot_frames >= 150 {
                        // 3 秒静音
                        hot_frames = 0;
                        let _ = machine.handle(Stimulus::IdleTimeout);
                        println!("[wake_word] 💤 静音超时 → {}", machine.state().as_str());
                    }
                } else {
                    hot_frames = 0;
                }

                if machine.state() != last_state {
                    last_state = machine.state();
                }
            }
            Err(_) => break,
        }
    }

    let _ = mic.stop();
    println!(
        "[wake_word] 结束：累计迁移 {} 次，最终状态 {}",
        machine.transitions,
        machine.state().as_str()
    );
    let _ = announced;
    anyhow::ensure!(machine.transitions > 0 || seconds < 5, "一次状态迁移都没发生（没喊？）");
    println!("[wake_word] ✅ 唤醒接缝（KWS → 状态机 → 云端）结构就位");
    Ok(())
}
