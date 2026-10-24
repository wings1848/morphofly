//! 诊断探针：把一次「说话 → 回复」的事件时间线打印出来，用于校准延迟指标口径。
//!
//! 用法（wings-pc）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p morphofly-core --example timeline -- testdata/audio/04_ack_zai.mp3

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::spectrum::rms_i16;
use diemeng_core::Config;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    ensure_crypto_provider();

    let path = std::env::args().nth(1).unwrap_or_else(|| "testdata/audio/04_ack_zai.mp3".into());
    let cfg = Config::load()?;
    let mic = AudioCapture::file_source(&path)?;
    let mut mic_rx = mic.subscribe();
    let client = RealtimeClient::connect(&cfg).await?;
    let mut ev_rx = client.events();
    client.send_tools(&diemeng_core::tools::tool_defs())?;

    let t0 = Instant::now();
    let mut last_voiced: Option<Instant> = None;
    let mut last_frame_at: Option<Instant> = None;

    // 推流在主任务里做（client 不实现 Clone，事件与推流用 select! 交替）
    let mut pusher_done = false;

    // 主循环：推流同时收事件（同一 runtime）
    let mut first_audio: Option<Instant> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        // 用 select! 同时等上行帧和下行事件，避免任一侧挨饿
        let ev = tokio::select! {
            biased;
            r = ev_rx.recv() => match r {
                Ok(e) => e,
                Err(_) => break,
            },
            r = mic_rx.recv() => match r {
                Ok(frame) => {
                    let _ = client.send_audio(&frame);
                    continue;
                }
                Err(_) => { pusher_done = true; continue; }
            },
            _ = tokio::time::sleep_until(deadline) => break,
        };
        let _ = pusher_done;
        let now = Instant::now();
        let ms = now.duration_since(t0).as_millis();
        match ev {
            RealtimeEvent::SpeechStarted => println!("{ms:>6}ms  speech_started"),
            RealtimeEvent::SpeechStopped => println!("{ms:>6}ms  speech_stopped"),
            RealtimeEvent::AudioDelta { pcm, .. } => {
                if first_audio.is_none() {
                    first_audio = Some(now);
                    println!("{ms:>6}ms  ★ 第一个 response.audio.delta（{} 采样）", pcm.len());
                }
            }
            RealtimeEvent::TranscriptDelta { text } => {
                print!("{ms:>6}ms  transcript: {text}\n");
            }
            RealtimeEvent::ResponseDone { status } => {
                println!("{ms:>6}ms  response.done {status}");
                break;
            }
            RealtimeEvent::Error { message, .. } => println!("{ms:>6}ms  error: {message}"),
            _ => {}
        }
    }

    // 单独再统计上行帧的「最后一个有声帧」时刻
    let mut mic2 = AudioCapture::file_source(&path)?;
    let mut rx2 = mic2.subscribe();
    let t1 = Instant::now();
    while let Ok(Ok(frame)) = tokio::time::timeout(Duration::from_millis(800), rx2.recv()).await {
        if rms_i16(&frame) > 0.01 {
            last_voiced = Some(Instant::now());
        }
        last_frame_at = Some(Instant::now());
        if t1.elapsed() > Duration::from_secs(6) {
            break;
        }
    }
    println!(
        "上行统计：最后有声帧 = 文件开始后 {:?}ms；最后帧 = {:?}ms",
        last_voiced.map(|t| t.duration_since(t1).as_millis()),
        last_frame_at.map(|t| t.duration_since(t1).as_millis())
    );
    if let (Some(fa), Some(lv)) = (first_audio, last_voiced) {
        println!("★ 最后有声帧 → 首个音频 = {}ms", fa.duration_since(lv).as_millis());
    }
    client.close();
    Ok(())
}
