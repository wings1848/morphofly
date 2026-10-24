//! 验收 P2/F6：**本地 VAD 打断**（正确做法，不依赖云端 speech_started）。
//!
//! 用法（wings-pc，需 --features kws）：
//!   cargo run -q -p morphofly-core --features kws --example bargein_local
//!
//! 流程：
//!  1. 推一段语音 + commit → 让模型开口
//!  2. 等模型真的在说话（队列里有音频）
//!  3. 喂「停一下」音频给本地 VAD → 命中即 stop_now()
//!  4. 量「开始喂打断音频 → 停播」耗时

use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::audio::playback::AudioPlayer;
use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::vad::{LocalVad, VadConfig};
use morphofly_core::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PROMPT: &str = "testdata/audio/04_ack_zai.mp3";
const INTERRUPT: &str = "testdata/audio/08_cmd_stop.mp3";

async fn push_file(client: &RealtimeClient, path: &str) -> anyhow::Result<()> {
    let mic = AudioCapture::file_source(path)?;
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
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    ensure_crypto_provider();

    let cfg = Config::load()?;
    println!("[bargein] model={}", cfg.realtime.model);

    let player = Arc::new(AudioPlayer::open()?);
    let client = Arc::new(RealtimeClient::connect(&cfg).await?);
    let mut ev = client.events();
    client.send_tools(&morphofly_core::tools::tool_defs())?;

    let played = Arc::new(AtomicU64::new(0));
    let ev_task = {
        let player2 = player.clone();
        let played2 = played.clone();
        tokio::spawn(async move {
            loop {
                let e = match tokio::time::timeout(Duration::from_secs(40), ev.recv()).await {
                    Err(_) | Ok(Err(_)) => break,
                    Ok(Ok(e)) => e,
                };
                match e {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        let n = player2.append_pcm16(&pcm, sample_rate);
                        played2.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    RealtimeEvent::ResponseDone { .. } => break,
                    RealtimeEvent::Error { message, .. } => eprintln!("[bargein] err: {message}"),
                    _ => {}
                }
            }
        })
    };

    // 阶段 1：让模型说话
    println!("[bargein] 阶段 1：推 {PROMPT} 并 commit");
    push_file(&client, PROMPT).await?;
    if let Err(e) = client.commit() {
        eprintln!("[bargein] ⚠️ commit 失败: {e}");
    }
    if let Err(e) = client.create_response() {
        eprintln!("[bargein] ⚠️ create_response 失败: {e}");
    }
    println!("[bargein] 已 commit + response.create");

    // 等模型开口。98688 采样的长 prompt 需要更久，给 40s。
    let t = Instant::now();
    let mut last_report = 0u64;
    while played.load(Ordering::Relaxed) == 0 && t.elapsed() < Duration::from_secs(40) {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let s = t.elapsed().as_secs();
        if s != last_report {
            last_report = s;
            if s % 5 == 0 && s > 0 {
                println!("[bargein] 等待模型开口… {s}s");
            }
        }
    }
    anyhow::ensure!(played.load(Ordering::Relaxed) > 0, "模型 40s 内没开口");
    // 让它多说一会儿，队列攒起来
    tokio::time::sleep(Duration::from_millis(500)).await;
    let queued_before = player.queued_frames();
    println!(
        "[bargein] 模型正在说话：已播 {} 帧，队列 {queued_before} 帧",
        played.load(Ordering::Relaxed)
    );
    anyhow::ensure!(queued_before > 0, "队列是空的，没法测打断");

    // 阶段 2：本地 VAD 打断
    let vad = LocalVad::new(&VadConfig {
        model: std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            .join(".diemeng/vad-model/silero_vad.onnx"),
        ..Default::default()
    })?;
    println!("[bargein] 阶段 2：喂 {INTERRUPT} 给本地 VAD");
    let mic = AudioCapture::file_source(INTERRUPT)?;
    let mut rx = mic.subscribe();
    let t_vad = Instant::now();
    let mut fed_ms = 0u64;
    let mut latency_us: Option<u64> = None;

    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(frame)) => {
                fed_ms += 20;
                let f32s: Vec<f32> = frame.iter().map(|&s| s as f32 / 32768.0).collect();
                if vad.push(&f32s) {
                    let t_stop = Instant::now();
                    let q = player.queued_frames();
                    player.stop_now();
                    let us = t_stop.elapsed().as_micros() as u64;
                    latency_us = Some(us);
                    println!(
                        "[bargein] ★ 本地 VAD 在 {fed_ms}ms 判定开口 → 清空 {q} 帧，耗时 {:.2}ms",
                        us as f64 / 1000.0
                    );
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

    match latency_us {
        Some(us) => {
            println!("\n[bargein] ==== 结果 ====");
            println!(
                "[bargein] 停播操作耗时: {:.2}ms（清空 {queued_before} 帧）",
                us as f64 / 1000.0
            );
            println!("[bargein] 端到端打断延迟: {total_ms}ms（从开始说「停」到声音停止）");
            println!(
                "[bargein] 判定（P2 要求 <300ms）: {}",
                if total_ms < 300 {
                    "✅ 达标"
                } else {
                    "⚠️ 超出，但可调 min_speech_duration 优化"
                }
            );
        }
        None => println!("[bargein] ⚠️ 本地 VAD 没检测到开口"),
    }
    Ok(())
}
