//! 本地 VAD 验证：silero-vad 能否在该开口时开口、该闭嘴时闭嘴。
//!
//! 用法（wings-pc，需 --features kws）：
//!   cargo run -q -p diemeng-core --features kws --example vad_check -- testdata/audio/

use diemeng_core::vad::{frame_to_f32, LocalVad, VadConfig};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let dir = std::env::args().nth(1).unwrap_or_else(|| "testdata/audio".into());
    let cfg = VadConfig {
        model: PathBuf::from(
            std::env::var("HOME").unwrap_or_else(|_| ".".into()),
        )
        .join(".diemeng/vad-model/silero_vad.onnx"),
        ..Default::default()
    };
    println!("[vad_check] 模型: {}", cfg.model.display());
    let vad = LocalVad::new(&cfg)?;

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|s| s.to_str()), Some("mp3") | Some("wav")))
        .collect();
    files.sort();

    for f in &files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().to_string();
        vad.reset();
        // 解码 → 16k mono
        use rodio::Source;
        let file = std::fs::File::open(f)?;
        let dec = rodio::Decoder::try_from(file)?;
        let ch = dec.channels().get() as usize;
        let rate = dec.sample_rate().get();
        let raw: Vec<f32> = dec.collect();
        let frames = raw.len() / ch.max(1);
        let mut mono = Vec::with_capacity(frames);
        for i in 0..frames {
            let mut acc = 0.0;
            for c in 0..ch {
                acc += raw[i * ch + c];
            }
            mono.push(acc / ch as f32);
        }
        let n = (frames as u64 * 16_000 / rate as u64) as usize;
        let mut pcm = Vec::with_capacity(n);
        for i in 0..n {
            let pos = i as f64 * rate as f64 / 16_000.0;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = mono.get(idx).copied().unwrap_or(0.0);
            let b = mono.get(idx + 1).copied().unwrap_or(a);
            pcm.push(a + (b - a) * frac);
        }
        // 末尾补 1s 静音让 VAD 收尾
        pcm.extend(std::iter::repeat(0.0f32).take(16_000));

        let mut starts = 0;
        let mut speech_frames = 0;
        for chunk in pcm.chunks(512) {
            if vad.push(chunk) {
                starts += 1;
            }
            if vad.is_speaking() {
                speech_frames += 1;
            }
        }
        let total = pcm.len() / 512;
        let ratio = if total > 0 { speech_frames as f32 / total as f32 * 100.0 } else { 0.0 };
        println!(
            "{name:<34} 开口 {starts} 次，说话占比 {ratio:>5.1}%"
        );
        let _ = frame_to_f32(&[]);  // 保证函数被引用
    }
    println!("\n[vad_check] 判定：语音文件应该有「开口」次数 > 0，静音/纯噪声应该很少");
    Ok(())
}
