//! 接缝 1.1（阶段 1 核心）：完整双工链路 —— 麦克风 → 云端 → 扬声器，中间不碰 WebView。
//!
//! 验收命令（wings-pc）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p morphofly-core --example duplex -- --seconds 15
//!
//! 三件事同时跑：
//!  1. 采集任务：cpal 20ms 帧 → WS append（带能量统计）
//!  2. 事件任务：收 AudioDelta → 播放队列；SpeechStarted → **立即停播**（barge-in）
//!  3. 主线程：打印延迟/能量/状态，到时退出
//!
//! 这就是开发方案 4.1「本地先停，服务端后确认」的真实实现。

use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::audio::playback::AudioPlayer;
use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::spectrum::{rms_i16, rms_to_visual};
use morphofly_core::tools::tool_defs;
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

    let mut seconds = 15u64;
    let mut input_file: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--seconds" => seconds = args.next().unwrap_or("15".into()).parse()?,
            // 用文件代替麦克风（自动化验证用；真机验收仍用真麦克风）
            "--input-file" => input_file = args.next(),
            "-h" | "--help" => {
                println!("用法: duplex [--seconds N] [--input-file <wav|mp3>]");
                return Ok(());
            }
            _ => {}
        }
    }

    ensure_crypto_provider();
    let cfg = Config::load()?;
    println!(
        "[duplex] provider={} model={} key={}",
        cfg.realtime.provider,
        cfg.realtime.model,
        cfg.masked_key()
    );

    // --- 音频硬件 ---
    let mic: Arc<AudioCapture>;
    let mic_fmt: String;
    if let Some(path) = input_file.clone() {
        // 文件模式：不打开麦克风，用文件内容按 20ms 节奏当上行音频
        mic = Arc::new(AudioCapture::file_source(&path)?);
        mic_fmt = format!("file:{path}");
        println!("[duplex] 上行来源: 文件 {path}（代替麦克风，自动化验证用）");
    } else {
        mic = Arc::new(AudioCapture::start_default()?);
        mic_fmt = mic.format().map(|f| format!("{f:?}")).unwrap_or_default();
        println!("[duplex] 麦克风: {mic_fmt}");
    }
    let player = Arc::new(AudioPlayer::open()?);
    println!("[duplex] 播放器: 24kHz nominal，抗抖动 200–500ms");

    // --- WS ---
    let t_conn = Instant::now();
    let client = Arc::new(RealtimeClient::connect(&cfg).await?);
    println!("[duplex] WS 连上 {}ms", t_conn.elapsed().as_millis());
    client.send_tools(&tool_defs())?;
    let mut ev_rx = client.events();

    // --- 统计 ---
    let mic_frames = Arc::new(AtomicU64::new(0));
    // P1 的真实起点：用户最后一次「有声」上行帧发出的时刻（之后就是静音尾巴）
    let last_voiced_at: Arc<std::sync::Mutex<Option<Instant>>> =
        Arc::new(std::sync::Mutex::new(None));
    let mic_energy_peak = Arc::new(AtomicU64::new(0)); // rms * 1e6
    let sent_audio_frames = Arc::new(AtomicU64::new(0));
    let play_frames = Arc::new(AtomicU64::new(0));
    let interruptions = Arc::new(AtomicU64::new(0));
    let e2e_shared = Arc::new(AtomicU64::new(u64::MAX));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let first_audio_ms = Arc::new(AtomicU64::new(u64::MAX));
    let speech_started_at = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let _last_speech_stop: Arc<std::sync::Mutex<Option<Instant>>> =
        Arc::new(std::sync::Mutex::new(None));

    // --- 任务1：采集 → WS ---
    let mic_rx = mic.subscribe();
    let c_push = client.clone();
    let (f1, e1, s1) = (
        mic_frames.clone(),
        mic_energy_peak.clone(),
        stop_flag.clone(),
    );
    let lv1 = last_voiced_at.clone();
    let push_task = tokio::spawn(async move {
        let mut rx = mic_rx;
        while !s1.load(Ordering::Relaxed) {
            match tokio::time::timeout(Duration::from_millis(100), rx.recv()).await {
                Ok(Ok(frame)) => {
                    f1.fetch_add(1, Ordering::Relaxed);
                    let rms = rms_i16(&frame);
                    let v = (rms * 1e6) as u64;
                    e1.fetch_max(v, Ordering::Relaxed);
                    if rms > 0.01 {
                        *lv1.lock().unwrap() = Some(Instant::now());
                    }
                    if let Err(e) = c_push.send_audio(&frame) {
                        eprintln!("[duplex] 推流失败: {e}");
                        break;
                    }
                }
                _ => continue,
            }
        }
    });

    // --- 任务2：收事件 → 播放 / 打断 ---
    let c_play = player.clone();
    let (f2, s2, st2, i2, si2, dl2) = (
        play_frames.clone(),
        stop_flag.clone(),
        sent_audio_frames.clone(),
        interruptions.clone(),
        speech_started_at.clone(),
        first_audio_ms.clone(),
    );
    let lv2 = last_voiced_at.clone();
    let e2e_ms = e2e_shared.clone();
    let ev_task = tokio::spawn(async move {
        loop {
            if s2.load(Ordering::Relaxed) {
                break;
            }
            let ev = match tokio::time::timeout(Duration::from_millis(120), ev_rx.recv()).await {
                Ok(Ok(e)) => e,
                Ok(Err(_)) => break,
                Err(_) => continue,
            };
            match ev {
                RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                    if dl2.load(Ordering::Relaxed) == u64::MAX {
                        if let Some(t) = *si2.lock().unwrap() {
                            dl2.store(t.elapsed().as_millis() as u64, Ordering::Relaxed);
                        }
                    }
                    // 端到端（P1 真实口径）：最后一段有声上行帧 → 第一个下行音频帧
                    if e2e_ms.load(Ordering::Relaxed) == u64::MAX {
                        if let Some(t) = *lv2.lock().unwrap() {
                            e2e_ms.store(t.elapsed().as_millis() as u64, Ordering::Relaxed);
                        }
                    }
                    st2.fetch_add(1, Ordering::Relaxed);
                    let n = c_play.append_pcm16(&pcm, sample_rate);
                    f2.fetch_add(n as u64, Ordering::Relaxed);
                }
                RealtimeEvent::SpeechStarted => {
                    // ★ barge-in：本地先停，不等云端确认
                    let queued = c_play.queued_frames();
                    c_play.stop_now();
                    if queued > 0 {
                        i2.fetch_add(1, Ordering::Relaxed);
                        println!("[duplex] ⚡ 打断：清空 {queued} 帧");
                    }
                    // 用户开口：记「开始说」，但 P1 的起点是「说完」，见 SpeechStopped
                    *si2.lock().unwrap() = Some(Instant::now());
                }
                RealtimeEvent::SpeechStopped => {
                    // ★ 验收 P1 的起点：用户「说完」的那一刻。
                    // 从这一刻到模型第一个音节，才是人真正感知的「等待」。
                    *si2.lock().unwrap() = Some(Instant::now());
                }
                RealtimeEvent::TranscriptDelta { text } => {
                    print!("{text}");
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
                RealtimeEvent::TranscriptDone { .. } => println!(),
                RealtimeEvent::Error { message, code } => {
                    eprintln!("\n[duplex] 服务端错误: {message} ({code:?})");
                }
                RealtimeEvent::ResponseDone { status } => {
                    println!("[duplex] 本轮结束 status={status}");
                }
                _ => {}
            }
        }
    });

    // --- 主循环：打印状态 ---
    let start = Instant::now();
    let mut last_print = Instant::now();
    let _transcript_done_at: Option<Instant> = None;
    while start.elapsed().as_secs() < seconds {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if last_print.elapsed() >= Duration::from_millis(700) {
            last_print = Instant::now();
            let energy = mic_energy_peak.swap(0, Ordering::Relaxed) as f32 / 1e6;
            println!(
                "[duplex] t={:>4.1}s 麦克风 {:>4}帧 峰值 {:.3}{} | 播放 {:>5}帧 队{:>4} 打断{}",
                start.elapsed().as_secs_f32(),
                mic_frames.load(Ordering::Relaxed),
                rms_to_visual(energy),
                "█".repeat((rms_to_visual(energy) * 12.0) as usize),
                play_frames.load(Ordering::Relaxed),
                player.queued_frames(),
                interruptions.load(Ordering::Relaxed),
            );
        }
    }

    stop_flag.store(true, Ordering::Relaxed);
    let _ = tokio::time::timeout(Duration::from_secs(3), push_task).await;
    let _ = tokio::time::timeout(Duration::from_secs(3), ev_task).await;
    let _ = mic.stop();
    player.stop_now();
    client.close();

    let summary = serde_json::json!({
        "seconds": seconds,
        "mic_frames": mic_frames.load(Ordering::Relaxed),
        "uplink_audio_chunks": sent_audio_frames.load(Ordering::Relaxed),
        "played_frames": play_frames.load(Ordering::Relaxed),
        "interruptions": interruptions.load(Ordering::Relaxed),
        "first_audio_ms_from_speech_stopped": first_audio_ms.load(Ordering::Relaxed),
        "e2e_ms_from_last_voiced_frame": e2e_shared.load(Ordering::Relaxed),
        "mic_format": mic_fmt,
    });
    let p = std::env::var("DUPLEX_OUT").unwrap_or_else(|_| "/tmp/diemeng-duplex.json".into());
    std::fs::write(&p, serde_json::to_string_pretty(&summary)?)?;
    println!("[duplex] 汇总: {summary}");
    println!("[duplex] 已写 {p}");

    anyhow::ensure!(mic_frames.load(Ordering::Relaxed) > 0, "麦克风没收到帧");
    anyhow::ensure!(
        play_frames.load(Ordering::Relaxed) > 0,
        "从没播到云端音频（对着麦克风说句话再跑）"
    );
    println!("[duplex] ✅ 双工链路闭环");
    Ok(())
}
