//! 接缝 0.5：频谱推给 WebView。
//!
//! 无音频设备也能验：用合成信号算出 64 bin，按 30fps 打印 JSON 行。
//! 验收命令：cargo run -p morphofly-core --example spectrum_json -- --seconds 2

use morphofly_core::spectrum::{rms_i16, rms_to_visual, Spectrum};
use std::io::Write;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let mut seconds = 2.0f32;
    let mut freq = 440.0f32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().unwrap_or("2".into()).parse()?,
            "--freq" => freq = args.next().unwrap_or("440".into()).parse()?,
            "-h" | "--help" => {
                println!("用法: spectrum_json [--seconds N] [--freq HZ]");
                return Ok(());
            }
            _ => {}
        }
    }

    let mut spec = Spectrum::with_default_window();
    let frame = Duration::from_millis(33);
    let start = Instant::now();
    let mut n = 0u64;
    let mut lines = std::io::stdout().lock();

    while start.elapsed().as_secs_f32() < seconds {
        let t = start.elapsed().as_secs_f32();
        // 变幅值 + 加二次谐波，让频谱不是一条直线
        let amp = 0.15 + 0.55 * (1.0 + (t * 2.0 * std::f32::consts::PI * 1.5).sin()) / 2.0;
        let pcm: Vec<i16> = (0..512)
            .map(|i| {
                let tt = i as f32 / 16_000.0;
                let v = amp * (2.0 * std::f32::consts::PI * freq * tt).sin()
                    + amp * 0.4 * (2.0 * std::f32::consts::PI * freq * 2.5 * tt).sin()
                    + amp * 0.2 * (2.0 * std::f32::consts::PI * freq * 4.0 * tt).sin();
                (v * 32767.0).clamp(-32768.0, 32767.0) as i16
            })
            .collect();

        let bins = spec.analyze_i16(&pcm);
        let rms = rms_i16(&pcm);
        let visual = rms_to_visual(rms);
        writeln!(
            lines,
            "{}",
            serde_json::json!({
                "bins": bins,
                "rms": visual,
                "rms_raw": rms,
                "t_ms": start.elapsed().as_millis() as u64,
            })
        )?;
        n += 1;
        std::thread::sleep(frame);
    }
    eprintln!(
        "[spectrum_json] 输出 {n} 帧，{:.0} fps",
        n as f32 / seconds.max(0.001)
    );
    Ok(())
}
