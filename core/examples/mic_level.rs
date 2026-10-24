//! 接缝 0.2：cpal 原生采集跑通（**不经 WebView**）。
//!
//! 验收命令：cargo run -p morphofly-core --example mic_level -- --seconds 3
//! 有音频设备时打印实时 RMS 与帧数；无设备时明确报错退出（不 panic）。

use morphofly_core::audio::capture::AudioCapture;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut seconds = 3.0f32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().unwrap_or("3".into()).parse()?,
            "-h" | "--help" => {
                println!("用法: mic_level [--seconds N]");
                return Ok(());
            }
            _ => {}
        }
    }

    println!("[mic_level] 打开默认输入设备（16kHz/mono/i16/20ms）…");
    let cap = AudioCapture::start_default()?;
    println!("[mic_level] 已启动，采集 {seconds}s");

    let mut frames = 0u64;
    let mut samples = 0u64;
    let mut peak_rms = 0.0f32;
    let mut sum_rms = 0.0f64;
    let start = Instant::now();

    while start.elapsed().as_secs_f32() < seconds {
        match cap.recv_timeout(Duration::from_millis(120)) {
            Ok(frame) => {
                frames += 1;
                samples += frame.len() as u64;
                let rms = morphofly_core::spectrum::rms_i16(&frame);
                peak_rms = peak_rms.max(rms);
                sum_rms += rms as f64;
                let bars = (rms * 40.0).round() as usize;
                println!(
                    "[mic_level] frame={frames:4} len={:3} rms={rms:.4} {}",
                    frame.len(),
                    "#".repeat(bars.min(50))
                );
            }
            Err(_) => continue,
        }
    }
    let _ = cap.stop();

    let avg = if frames > 0 {
        (sum_rms / frames as f64) as f32
    } else {
        0.0
    };
    println!(
        "[mic_level] 完成：{frames} 帧 / {samples} 采样，平均 RMS={avg:.4}，峰值 RMS={peak_rms:.4}"
    );

    anyhow::ensure!(frames > 0, "一帧都没收到：麦克风没工作");
    anyhow::ensure!(
        samples / frames.max(1) == morphofly_core::FRAME_SAMPLES as u64,
        "每帧必须是 {} 采样（20ms@16k），实际平均 {}",
        morphofly_core::FRAME_SAMPLES,
        samples / frames.max(1)
    );
    if peak_rms < 0.0005 {
        eprintln!("[mic_level] ⚠️ 峰值 RMS 接近 0：设备通了但没声音（静音/被占用？）");
    }
    println!("[mic_level] ✅ 原生采集链路正常");
    Ok(())
}
