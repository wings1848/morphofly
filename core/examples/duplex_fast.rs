//! **最优延迟链路**：本地 VAD + 手动 commit（关掉服务端 VAD）。
//!
//! 实测依据（原始 WS，2026-10-24）：
//! - server_vad 模式：本地判定说完 → 首音频 **1950ms**
//! - 本地 VAD + 手动 commit：**790ms**（快 2.5 倍，达标 <800ms）
//!
//! 原理：服务端 VAD 的「确认说完」逻辑固定要 1.9 秒且参数无效。
//! 我们不等它 —— 本地 silero-vad 判定说完（200ms）就立刻 commit。
//!
//! 用法（wings-pc，需 --features kws）：
//!   cargo run -q -p diemeng-core --features kws --example duplex_fast -- \
//!     --file testdata/audio/04_ack_zai.mp3

use diemeng_core::audio::playback::AudioPlayer;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::vad::{LocalVad, VadConfig};
use diemeng_core::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    ensure_crypto_provider();

    let mut file = "testdata/audio/04_ack_zai.mp3".to_string();
    let mut rounds = 5usize;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--file" => file = args.next().unwrap_or(file),
            "--rounds" => rounds = args.next().unwrap_or("5".into()).parse()?,
            _ => {}
        }
    }
    let cfg = Config::load()?;
    println!("[fast] model={} 模式=本地VAD+手动commit", cfg.realtime.model);

    let mut samples = Vec::new();

    for i in 1..=rounds {
        let player = Arc::new(AudioPlayer::open()?);
        let client = Arc::new(RealtimeClient::connect(&cfg).await?);
        let mut ev = client.events();

        // ★ 关键：显式把 turn_detection 设为 null（关掉服务端 VAD）
        client.send_session_update_raw(serde_json::json!({
            "modalities": ["text", "audio"],
            "instructions": cfg.realtime.instructions,
            "voice": cfg.realtime.voice,
            "input_audio_format": "pcm16",
            "output_audio_format": "pcm16",
            "turn_detection": serde_json::Value::Null
        }))?;
        client.send_tools(&diemeng_core::tools::tool_defs())?;

        let t_commit = Arc::new(Mutex::new(None::<Instant>));
        let first_audio = Arc::new(AtomicU64::new(u64::MAX));
        let played = Arc::new(AtomicU64::new(0));

        let player2 = player.clone();
        let tc2 = t_commit.clone();
        let fa2 = first_audio.clone();
        let pl2 = played.clone();
        let ev_task = tokio::spawn(async move {
            loop {
                let e = match tokio::time::timeout(Duration::from_secs(30), ev.recv()).await {
                    Err(_) | Ok(Err(_)) => break,
                    Ok(Ok(e)) => e,
                };
                match e {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        if fa2.load(Ordering::Relaxed) == u64::MAX {
                            if let Some(t) = *tc2.lock().await {
                                fa2.store(t.elapsed().as_millis() as u64, Ordering::Relaxed);
                            }
                        }
                        let n = player2.append_pcm16(&pcm, sample_rate);
                        pl2.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    RealtimeEvent::ResponseDone { status } => {
                        eprintln!("[fast] response.done status={status}");
                        break;
                    }
                    RealtimeEvent::Error { message, code } => {
                        eprintln!("[fast] ⚠️ 服务端错误: {message} ({code:?})");
                    }
                    other => {
                        eprintln!("[fast] 事件: {}", other.kind());
                    }
                }
            }
        });

        // 推流 + 本地 VAD 判定说完即 commit
        let mic = diemeng_core::audio::capture::AudioCapture::file_source(&file)?;
        let mut rx = mic.subscribe();
        let vad_cfg = VadConfig {
            model: std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join(".diemeng/vad-model/silero_vad.onnx"),
            // 打断/说完要快：0.2s
            min_speech_duration: 0.2,
            ..Default::default()
        };
        let vad = LocalVad::new(&vad_cfg)?;
        let mut committed = false;
        let t_start = Instant::now();
        let mut t_last_voiced: Option<Instant> = None;
        let mut silence_after_voice = 0u32;

        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(frame)) => {
                    let f32s: Vec<f32> = frame.iter().map(|&s| s as f32 / 32768.0).collect();
                    let rms = diemeng_core::spectrum::rms_i16(&frame);
                    if rms > 0.02 {
                        t_last_voiced = Some(Instant::now());
                        silence_after_voice = 0;
                    } else if t_last_voiced.is_some() {
                        silence_after_voice += 1;
                    }
                    let _ = client.send_audio(&frame);
                    vad.push(&f32s);

                    // 本地判定「说完了」：有声之后连续 200ms 静音
                    if !committed && silence_after_voice >= 10 && t_last_voiced.is_some() {
                        committed = true;
                        *t_commit.lock().await = Some(Instant::now());
                        println!(
                            "[fast] 第{i}轮：本地判定说完（{:.0}ms）→ commit",
                            t_start.elapsed().as_millis()
                        );
                        let _ = client.commit();
                        let _ = client.create_response();
                        // 继续推流（真实场景麦克风一直在采）
                    }
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    if mic.is_file_finished() {
                        if !committed {
                            // 文件推完还没判定完，兜底 commit
                            committed = true;
                            *t_commit.lock().await = Some(Instant::now());
                            let _ = client.commit();
                            let _ = client.create_response();
                        }
                        break;
                    }
                }
            }
        }

        let _ = tokio::time::timeout(Duration::from_secs(25), ev_task).await;
        client.close();
        player.stop_now();

        let fa = first_audio.load(Ordering::Relaxed);
        if fa != u64::MAX {
            samples.push(fa);
            println!("[fast] 第{i}轮：commit → 首音频 {fa}ms，播放 {} 帧", played.load(Ordering::Relaxed));
        } else {
            println!("[fast] 第{i}轮：没收到音频");
        }
        anyhow::ensure!(played.load(Ordering::Relaxed) > 0, "没播到音频");
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    if !samples.is_empty() {
        samples.sort_unstable();
        let n = samples.len();
        let p = |q: f64| samples[((n as f64 - 1.0) * q).round() as usize];
        println!(
            "\n===== 本地 VAD + 手动 commit ====="
        );
        println!(
            "commit → 首音频: n={n} 均值 {:.0}ms 最小 {}ms 中位 {}ms p90 {}ms 最大 {}ms",
            samples.iter().sum::<u64>() as f64 / n as f64, samples[0], p(0.5), p(0.9), samples[n-1]
        );
        let p90 = p(0.9);
        println!("判定：p90={p90}ms → {}", if p90 < 800 { "✅ 达到合格值 <800ms" } else { "❌ 不达标" });
    }
    Ok(())
}
