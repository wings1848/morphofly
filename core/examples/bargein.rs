//! 验收 P2/F6：打断（barge-in）实测。
//!
//! 场景：模型正在说话 → 用户插入说「小蝶，停一下」→ 本地必须立即停播。
//!
//! 关键设计（docs/开发方案.md §4.1）：**本地先停，服务端后确认**。
//!
//! 用法：
//!   cargo run -q -p morphofly-core --example bargein -- \
//!     --prompt testdata/audio/06_cmd_progress.mp3 \
//!     --interrupt testdata/audio/08_cmd_stop.mp3
//!
//! 步骤：
//!  1. 推 prompt 让模型开始说话
//!  2. 等第一批 AudioDelta 进队列（模型真的在说）
//!  3. 立刻推 interrupt 音频
//!  4. 量「SpeechStarted 到达 → 队列清零」耗时

use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::audio::playback::AudioPlayer;
use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::Config;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

    let mut prompt_file = "testdata/audio/06_cmd_progress.mp3".to_string();
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
    client.send_tools(&morphofly_core::tools::tool_defs())?;

    let played = Arc::new(AtomicU64::new(0));
    let stop_us = Arc::new(AtomicU64::new(u64::MAX));
    let queued_before = Arc::new(AtomicU64::new(0));
    let model_speaking = Arc::new(AtomicBool::new(false));
    let in_interrupt_phase = Arc::new(AtomicBool::new(false));
    let interrupted = Arc::new(AtomicBool::new(false));

    let player2 = player.clone();
    let played2 = played.clone();
    let su = stop_us.clone();
    let qb = queued_before.clone();
    let ms_ = model_speaking.clone();
    let ip = in_interrupt_phase.clone();
    let it = interrupted.clone();
    let ev_task = tokio::spawn(async move {
        loop {
            let ev = match tokio::time::timeout(Duration::from_secs(40), ev_rx.recv()).await {
                Err(_) | Ok(Err(_)) => break,
                Ok(Ok(e)) => e,
            };
            match ev {
                RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                    let n = player2.append_pcm16(&pcm, sample_rate);
                    played2.fetch_add(n as u64, Ordering::Relaxed);
                    ms_.store(true, Ordering::Relaxed);
                }
                RealtimeEvent::SpeechStarted => {
                    // 只有在我们「已经让它说话」且「处于打断阶段」时才算真打断
                    if ip.load(Ordering::Relaxed) && ms_.load(Ordering::Relaxed) {
                        let t = Instant::now();
                        let queued = player2.queued_frames();
                        if queued > 0 {
                            qb.store(queued, Ordering::Relaxed);
                            player2.stop_now();
                            let us = t.elapsed().as_micros() as u64;
                            let _ = su.compare_exchange(
                                u64::MAX,
                                us,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            );
                            it.store(true, Ordering::Relaxed);
                            println!(
                                "[bargein] ★ 打断生效：清空 {queued} 帧，耗时 {:.2}ms",
                                us as f64 / 1000.0
                            );
                        }
                    }
                }
                RealtimeEvent::ResponseDone { .. } if it.load(Ordering::Relaxed) => {
                    break;
                }
                _ => {}
            }
        }
    });

    // --- 阶段 1：让它说话 ---
    println!("[bargein] 阶段 1：推 {prompt_file} 让模型开口");
    {
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

    // 等模型真的开始说话（有 AudioDelta 进队列）
    let wait_start = Instant::now();
    while !model_speaking.load(Ordering::Relaxed) && wait_start.elapsed() < Duration::from_secs(20)
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::ensure!(
        model_speaking.load(Ordering::Relaxed),
        "模型一直没开口，没法测打断"
    );
    // 让它多说一会儿，队列里攒点东西
    tokio::time::sleep(Duration::from_millis(1200)).await;
    println!(
        "[bargein] 模型正在说话，队列 {} 帧 —— 现在推打断音频",
        player.queued_frames()
    );

    // --- 阶段 2：插入打断 ---
    in_interrupt_phase.store(true, Ordering::Relaxed);
    {
        let mic = AudioCapture::file_source(&interrupt_file)?;
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
    // 等打断事件（最多 15s）
    let t_wait = Instant::now();
    while !interrupted.load(Ordering::Relaxed) && t_wait.elapsed() < Duration::from_secs(15) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    client.close();
    player.stop_now();
    let _ = tokio::time::timeout(Duration::from_secs(2), ev_task).await;

    println!("[bargein] 播放帧数: {}", played.load(Ordering::Relaxed));
    let us = stop_us.load(Ordering::Relaxed);
    if !interrupted.load(Ordering::Relaxed) {
        println!("[bargein] ⚠️ 没打断成功（云端没有报 speech_started）");
        println!("[bargein]    可能原因：服务端 VAD 要 1.6s 才判定，而模型那时已经说完了");
        return Ok(());
    }
    println!(
        "[bargein] 队列原有 {} 帧，清空耗时 {:.2}ms",
        queued_before.load(Ordering::Relaxed),
        us as f64 / 1000.0
    );
    println!("[bargein] ✅ 打断生效");
    Ok(())
}
