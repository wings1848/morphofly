//! AEC 真机活体验证：外放一段语音，麦克风同时录音，对比过 AEC 前后的能量。
//!
//! **这是唯一能真实验证 AEC 的方法** —— 需要真实声学路径
//! （扬声器 → 空气 → 麦克风）。数字域回环（snd_aloop / monitor 源）不行，
//! 因为没有真实卷积关系，AEC3 的自适应滤波器收敛不了。
//!
//! 用法（在有麦克风+扬声器的机器上）：
//!   cargo run --release -p morphofly-core --example aec_live_test
//!
//! 前提：需要有 /tmp/aec_speech.pcm（16k mono s16le 裸流）。

use morphofly_core::audio::aec::{AecConfig, AecProcessor};
use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::audio::playback::AudioPlayer;
use std::time::Duration;

const RATE: usize = 16_000;

fn energy(s: &[f32]) -> f32 {
    s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("warn")
        .without_time()
        .init();

    let speech_path = std::env::var("AEC_SPEECH")
        .unwrap_or_else(|_| "/tmp/aec_speech.pcm".to_string());
    let raw = std::fs::read(&speech_path)
        .map_err(|e| anyhow::anyhow!("读不到 {speech_path}: {e}（需要 16k mono s16le）"))?;
    let speech: Vec<i16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect();
    let dur = speech.len() as f32 / RATE as f32;
    println!("测试语音: {:.2}s, {} 采样", dur, speech.len());

    // 1) 打开真实播放器和采集器
    let player = AudioPlayer::open()?;
    let mic = AudioCapture::start_default()?;
    let mut mic_rx = mic.subscribe();
    println!("播放/采集设备已打开");

    // 2) 先录 1 秒底噪
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut noise: Vec<f32> = Vec::new();
    while let Ok(frame) = mic_rx.try_recv() {
        noise.extend(frame.iter().map(|s| *s as f32 / 32768.0));
    }
    println!("底噪 RMS: {:.5}", energy(&noise).sqrt());

    // 3) 播放 + 同时录音
    println!("开始播放（请勿说话，模拟「只有回声」的场景）...");
    player.append_pcm16(&speech, RATE as u32);
    let expect_frames = speech.len() / 320;
    let mut capture: Vec<f32> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis((dur * 1500.0) as u64);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(100), mic_rx.recv()).await {
            Ok(Ok(frame)) => {
                capture.extend(frame.iter().map(|s| *s as f32 / 32768.0));
                if capture.len() / 320 > expect_frames + 20 {
                    break;
                }
            }
            _ => {}
        }
    }
    let cap_rms = energy(&capture).sqrt();
    println!(
        "录音 {:.2}s, RMS = {:.5}",
        capture.len() as f32 / RATE as f32,
        cap_rms
    );

    if cap_rms < 0.001 {
        anyhow::bail!(
            "录到的是静音（RMS {cap_rms:.6}）—— 麦克风没收到声音。\n\
             检查：音量是否静音、设备是否被占用、是否需要 --device 显式指定"
        );
    }

    // 4) 过 AEC
    let aec = AecProcessor::new(AecConfig::default());
    let before = energy(&capture);

    // reference 用播放的原始音频（真实产品里就是这个）
    let mut reference: Vec<f32> = speech.iter().map(|s| *s as f32 / 32768.0).collect();
    let pad = (160 - reference.len() % 160) % 160;
    reference.extend(std::iter::repeat_n(0.0f32, pad));
    let pad2 = (160 - capture.len() % 160) % 160;
    capture.extend(std::iter::repeat_n(0.0f32, pad2));

    let n = reference.len().min(capture.len());
    for (r, c) in reference[..n]
        .as_chunks::<160>()
        .0
        .iter()
        .zip(capture[..n].as_chunks_mut::<160>().0)
    {
        aec.process_render(r);
        aec.process_capture(c);
    }

    let after = energy(&capture);
    let db = 10.0 * (before / after.max(1e-30)).log10();

    println!("\n=== AEC 结果 ===");
    println!("过之前能量: {before:.6e}");
    println!("过之后能量: {after:.6e}");
    println!("抑制: {db:.1} dB");
    println!("stats: {:?}", aec.stats());
    println!();
    if db >= 10.0 {
        println!("✅ 真机有效（≥10dB）");
    } else {
        println!("❌ 抑制不足 —— 可能原因：");
        println!("   1. 录音里的主要成分不是相干回声（比如环境噪声）");
        println!("   2. 播放和录音的时钟不同步严重");
        println!("   3. 设备本身已经做了硬件回声消除，AEC 没有可消的");
    }
    Ok(())
}
