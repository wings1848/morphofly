//! 用 snd_aloop 回环设备验证 AEC 真机效果。
//!
//! 原理：把音频播放到 aloop 的输出，同时从 aloop 的输入录音，
//! 录到的就是「经过物理回环的回声」。然后对比过 AEC / 不过 AEC 的能量。
//!
//! 这比合成信号更真实：走的是真正的 ALSA/PipeWire 路径，
//! 有真实的采样率转换和缓冲。
//!
//! 用法：
//!   cargo run --release -p morphofly-core --example aec_aloop_test

use morphofly_core::audio::aec::{AecConfig, AecProcessor};
use std::process::Command;

const RATE: usize = 16000;
// ★ 必须用 output 的 .monitor 源，不能用 aloop 的 input。
//
// 2026-10-24 实测：`pw-cat --record --target=alsa_input.platform-snd_aloop.0.analog-stereo`
// 录到的是纯静音（峰值 0）—— aloop 的「input」和「output」是两条独立流，
// 默认没有把它们连起来。而 `*.monitor` 源能录到「送到这个 sink 的内容」，
// 峰值 6808，通路正常。
const ALOOP_OUT: &str = "alsa_output.platform-snd_aloop.0.analog-stereo";
const ALOOP_MONITOR: &str = "alsa_output.platform-snd_aloop.0.analog-stereo.monitor";

fn energy(s: &[f32]) -> f32 {
    s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32
}

fn main() -> anyhow::Result<()> {
    let tmp = std::env::temp_dir();
    let speech = tmp.join("aec_aloop_speech.pcm");
    let recorded = tmp.join("aec_aloop_rec.pcm");

    // 1) 准备音频（用已有的 TTS 合成结果，或生成一段语音样信号）
    if !speech.exists() {
        anyhow::bail!(
            "缺少测试音频 {}。先用 e2e 的 TTS 生成一个 pcm 放过去。",
            speech.display()
        );
    }
    let raw = std::fs::read(&speech)?;
    let src: Vec<f32> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
        .collect();
    println!("源音频: {:.2}s", src.len() as f32 / RATE as f32);

    // 2) 重新编码成 wav 以便 pw-cat 播放
    let wav = tmp.join("aec_aloop_speech.wav");
    write_wav(&wav, &src)?;

    // 3) 后台播放 + 同时录音
    let rec = Command::new("pw-cat")
        .args([
            "--record",
            &format!("--target={ALOOP_MONITOR}"),
            "--format=s16",
            &format!("--rate={RATE}"),
            "--channels=1",
            recorded.to_str().unwrap(),
        ])
        .spawn()?;

    std::thread::sleep(std::time::Duration::from_millis(300));

    let play = Command::new("pw-cat")
        .args([
            "--playback",
            &format!("--target={ALOOP_OUT}"),
            wav.to_str().unwrap(),
        ])
        .status()?;
    println!("播放结束: {play}");

    std::thread::sleep(std::time::Duration::from_millis(500));
    let _ = Command::new("pkill").args(["-f", "pw-cat --record"]).status();
    drop(rec);

    // 4) 读录音
    let rec_raw = std::fs::read(&recorded)?;
    let mut cap: Vec<f32> = rec_raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c) as f32 / 32768.0)
        .collect();
    println!("录音: {:.2}s", cap.len() as f32 / RATE as f32);

    if cap.len() < RATE {
        anyhow::bail!("录音太短（{} 样本），aloop 可能没接通", cap.len());
    }

    let before = energy(&cap);

    // 5) 过 AEC（reference 就是源音频，对它做了对齐处理）
    let aec = AecProcessor::new(AecConfig::default());
    // 把源音频也凑成 160 的整数倍
    let mut reference = src.clone();
    let need = (160 - reference.len() % 160) % 160;
    reference.extend(std::iter::repeat_n(0.0f32, need));
    cap.extend(std::iter::repeat_n(0.0f32, (160 - cap.len() % 160) % 160));

    let n = reference.len().min(cap.len());
    for (r, c) in reference[..n]
        .as_chunks::<160>()
        .0
        .iter()
        .zip(cap[..n].as_chunks_mut::<160>().0)
    {
        aec.process_render(r);
        aec.process_capture(c);
    }

    let after = energy(&cap);
    let db = 10.0 * (before / after.max(1e-30)).log10();

    println!("\n=== 结果 ===");
    println!("AEC 前能量: {before:.6e}");
    println!("AEC 后能量: {after:.6e}");
    println!("抑制: {db:.1} dB");
    println!("stats: {:?}", aec.stats());
    println!(
        "{}",
        if db >= 10.0 {
            "✅ 真机有效"
        } else {
            "❌ 真机无效，需要排查"
        }
    );
    Ok(())
}

fn write_wav(path: &std::path::Path, samples: &[f32]) -> anyhow::Result<()> {
    let mut out = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(RATE as u32).to_le_bytes());
    out.extend_from_slice(&((RATE * 2) as u32).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, out)?;
    Ok(())
}
