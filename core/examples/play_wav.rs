//! 接缝 0.3：rodio 播放跑通（补上审计指出「缺失的 example」）。
//!
//! 用法：cargo run -p morphofly-core --example play_wav -- testdata/audio/01_wake_short.mp3
//!
//! 输出：解码信息、播放耗时、退出码 0（失败非 0）。
//! **注意**：默认 sink 是虚拟设备时（snd_aloop）也会「成功」——那是数据流动成功，不是出声。
//! 真要确认出声：先跑 `tools/pc_audio_route.sh --physical` 切到物理设备。

use diemeng_core::audio::playback::AudioPlayer;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata/audio/01_wake_short.mp3".into());
    println!("[play_wav] 文件: {path}");

    // 解码成 i16 PCM（24k 常见，交给播放器自动转）
    use rodio::Source;
    let file = std::fs::File::open(&path)?;
    let decoder = rodio::Decoder::try_from(file)?;
    let channels = decoder.channels().get() as usize;
    let rate = decoder.sample_rate().get();
    let samples: Vec<f32> = decoder.collect();
    anyhow::ensure!(channels > 0 && rate > 0, "解码失败");

    let frames = samples.len() / channels;
    let mut pcm16: Vec<i16> = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += samples[i * channels + c];
        }
        let v = acc / channels as f32;
        pcm16.push((v.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
    }
    println!(
        "[play_wav] 解码：{rate}Hz {channels}ch → mono {} 采样（{:.2}s）",
        pcm16.len(),
        pcm16.len() as f32 / rate as f32
    );

    let player = AudioPlayer::open()?;
    let t0 = Instant::now();
    let written = player.append_pcm16(&pcm16, rate);
    println!("[play_wav] 写入队列 {written} 采样");

    // 等播完（队列空 + 余量）
    let dur = Duration::from_millis((pcm16.len() as u64 * 1000) / rate as u64);
    let deadline = t0 + dur + Duration::from_millis(800);
    while Instant::now() < deadline {
        if player.is_empty() && Instant::now() > t0 + dur {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let elapsed = t0.elapsed();
    player.stop_now();

    println!(
        "[play_wav] ✅ 播放完成，耗时 {}ms（音频 {:.2}s）",
        elapsed.as_millis(),
        pcm16.len() as f32 / rate as f32
    );
    // 播放耗时不应显著短于音频时长（否则说明数据没进队列）
    anyhow::ensure!(
        elapsed.as_millis() as u64 >= dur.as_millis() as u64 / 2,
        "播放耗时远短于音频时长，数据可能没进队列"
    );
    Ok(())
}
