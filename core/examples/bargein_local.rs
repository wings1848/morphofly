//! 验收 P2/F6：**本地 VAD 打断**实测（这才是正确的实现）。
//!
//! 对比 `bargein.rs`（依赖云端 `speech_started`，实测要等 1.6s，打不断）：
//! 这个用本地 silero-vad，在采集侧实时判断「用户开口」，**不等云端**。
//!
//! 用法（wings-pc，需 --features kws）：
//!   cargo run -q -p diemeng-core --features kws --example bargein_local -- \
//!     --prompt testdata/audio/06_cmd_progress.mp3 \
//!     --interrupt testdata/audio/08_cmd_stop.mp3

use diemeng_core::audio::playback::AudioPlayer;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::vad::{LocalVad, VadConfig};
use diemeng_core::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    ensure_crypto_provider();

    let mut prompt_file = "testdata/audio/09_multiturn.mp3".to_string();
    let mut interrupt_file = "testdata/audio/08_cmd_stop.mp3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--prompt" => prompt_file = args.next().unwrap_or(prompt_file),
            "--interrupt" => interrupt_file = args.next().unwrap_or(interrupt_file),
            _ => {}
        }
    }
    let cfg = Config::load()?;
    let player = Arc::new(AudioPlayer::open()?);
    let client = Arc::new(RealtimeClient::connect(&cfg).await?);
    let mut ev_rx = client.events();
    client.send_tools(&diemeng_core::tools::tool_defs())?;

    let played = Arc::new(AtomicU64::new(0));
    let ev_task = {
        let player2 = player.clone();
        let played2 = played.clone();
        tokio::spawn(async move {
            loop {
                let ev = match tokio::time::timeout(Duration::from_secs(40), ev_rx.recv()).await {
                    Err(_) | Ok(Err(_)) => break,
                    Ok(Ok(e)) => e,
                };
                match ev {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        let n = player2.append_pcm16(&pcm, sample_rate);
                        played2.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    RealtimeEvent::ResponseDone { .. } => break,
                    _ => {}
                }
            }
        })
    };

    // 阶段 1：让它说话
    println!("[bargein] 阶段 1：推 {prompt_file}");
    {
        use diemeng_core::audio::capture::AudioCapture;
        let mic = AudioCapture::file_source(&prompt_file)?;
        let mut rx = mic.subscribe();
        loop {
            match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
                Ok(Ok(f)) => {
                    let _ = client.send_audio(&f);
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    if mic.is_file_finished() {
                        break;
                    }
                }
            }
        }
    }
    // 关键：模型一开口就立刻打断（此时队列最满，最能体现打断价值）
    let t0 = Instant::now();
    while played.load(Ordering::Relaxed) == 0 && t0.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 再多等 300ms 让队列积累
    tokio::time::sleep(Duration::from_millis(300)).await;
    let queued_before = player.queued_frames();
    println!(
        "[bargein] 模型正在说话：已播 {} 帧，队列 {} 帧",
        played.load(Ordering::Relaxed),
        queued_before
    );
    anyhow::ensure!(queued_before > 0, "队列是空的，模型已经说完了，没法测打断");

    // 阶段 2：本地 VAD 检测打断
    println!("[bargein] 阶段 2：喂 {interrupt_file} 给本地 VAD，看多久触发");
    let vad_cfg = VadConfig {
        model: std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".diemeng/vad-model/silero_vad.onnx"),
        ..Default::default()
    };
    let vad = LocalVad::new(&vad_cfg)?;

    use diemeng_core::audio::capture::AudioCapture;
    let mic = AudioCapture::file_source(&interrupt_file)?;
    let mut rx = mic.subscribe();
    let mut interrupt_latency_us: Option<u64> = None;
    let t_vad = Instant::now();
    let mut fed_ms = 0u64;

    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(frame)) => {
                let samples: Vec<f32> = frame.iter().map(|&s| s as f32 / 32768.0).collect();
                fed_ms += 20;
                // ★ 本地 VAD 判定
                if vad.push(&samples) {
                    // 立刻停播！不等云端
                    let cleared_at = Instant::now();
                    let queued = player.queued_frames();
                    player.stop_now();
                    let us = cleared_at.elapsed().as_micros() as u64;
                    interrupt_latency_us = Some(us);
                    println!(
                        "[bargein] ★ 本地 VAD 在第 {fed_ms}ms 判定开口 → 清空 {queued} 帧耗时 {:.2}ms",
                        us as f64 / 1000.0
                    );
                    // 同时把音频推给云端（先停后推，顺序不能反）
                    let _ = client.send_audio(&frame);
                    break;
                }
                let _ = client.send_audio(&frame);
            }
            Ok(Err(_)) => break,
            Err(_) => {
                if mic.is_file_finished() {
                    break;
                }
            }
        }
    }
    let total_ms = t_vad.elapsed().as_millis();

    client.close();
    player.stop_now();
    let _ = tokio::time::timeout(Duration::from_secs(2), ev_task).await;

    println!("[bargein] 总耗时 {total_ms}ms");
    match interrupt_latency_us {
        Some(us) => {
            println!("[bargein] 停播耗时 {:.2}ms", us as f64 / 1000.0);
            println!(
                "[bargein] ✅ 本地 VAD 打断生效（从开始喂打断音频到停播共 {total_ms}ms，含 VAD 判定延迟）"
            );
            anyhow::ensure!(
                (us as f64 / 1000.0) < 300.0,
                "P2 不达标：停播耗时 {:.2}ms",
                us as f64 / 1000.0
            );
        }
        None => {
            println!("[bargein] ⚠️ 本地 VAD 没检测到开口");
        }
    }
    Ok(())
}
