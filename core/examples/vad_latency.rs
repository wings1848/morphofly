//! 量真正的端到端延迟：**最后一段有声音频 → 听见模型第一个音节**。
//!
//! 这是验收 P1 的正确口径（用户感知的是「我说完到它开口」）。
//!
//! 用法：cargo run -q -p morphofly-core --example vad_latency -- --rounds 6 --file testdata/audio/04_ack_zai.mp3
//!
//! 实现要点：推流用独立任务，主循环只收事件 —— 避免 select! 里文件流结束后
//! 忙循环把事件饿死（2026-10-24 踩过这个坑）。

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::spectrum::rms_i16;
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

    let mut rounds = 5usize;
    let mut file = "testdata/audio/04_ack_zai.mp3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rounds" => rounds = args.next().unwrap_or("5".into()).parse()?,
            "--file" => file = args.next().unwrap_or(file),
            _ => {}
        }
    }
    let cfg = Config::load()?;
    let mut vad_stop = Vec::new();
    let mut e2e = Vec::new();

    for i in 1..=rounds {
        // 先把这个文件解码成帧列表（file_source 内部也是这么做的）
        let mic = AudioCapture::file_source(&file)?;
        let mut mic_rx = mic.subscribe();
        let client = Arc::new(RealtimeClient::connect(&cfg).await?);
        let mut ev_rx = client.events();

        // 最后一段有声帧的时刻（由推流任务写）
        let last_voiced_ms: Arc<AtomicU64> = Arc::new(AtomicU64::new(u64::MAX));
        let t0 = Instant::now();
        let c_push = client.clone();
        let lv = last_voiced_ms.clone();
        let push = tokio::spawn(async move {
            loop {
                match mic_rx.recv().await {
                    Ok(frame) => {
                        if rms_i16(&frame) > 0.02 {
                            lv.store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
                        }
                        let _ = c_push.send_audio(&frame);
                    }
                    Err(_) => break, // 文件推完
                }
            }
        });

        let mut t_stopped: Option<Instant> = None;
        let mut t_audio: Option<Instant> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let ev = match tokio::time::timeout_at(deadline, ev_rx.recv()).await {
                Err(_) | Ok(Err(_)) => break,
                Ok(Ok(e)) => e,
            };
            match ev {
                RealtimeEvent::SpeechStopped => {
                    if t_stopped.is_none() {
                        t_stopped = Some(Instant::now());
                    }
                }
                RealtimeEvent::AudioDelta { .. } => {
                    if t_audio.is_none() {
                        t_audio = Some(Instant::now());
                    }
                }
                RealtimeEvent::ResponseDone { .. } => break,
                RealtimeEvent::Error { message, .. } => eprintln!("[vad] err: {message}"),
                _ => {}
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), push).await;
        client.close();

        let lv_ms = last_voiced_ms.load(Ordering::Relaxed);
        if lv_ms == u64::MAX {
            println!("[vad] 第 {i}/{rounds} 轮：没检测到有声帧（音频太安静？）");
            continue;
        }
        let lv_at = t0 + Duration::from_millis(lv_ms);
        match (t_stopped, t_audio) {
            (Some(st), Some(au)) => {
                let a = st.duration_since(lv_at).as_millis() as u64;
                let b = au.duration_since(lv_at).as_millis() as u64;
                vad_stop.push(a);
                e2e.push(b);
                println!("[vad] 第 {i}/{rounds} 轮：最后有声帧 → speech_stopped {a}ms；→ 首个音频 {b}ms");
            }
            (Some(st), None) => {
                let a = st.duration_since(lv_at).as_millis() as u64;
                vad_stop.push(a);
                println!("[vad] 第 {i}/{rounds} 轮：VAD 判完 {a}ms，但没收到音频");
            }
            _ => println!("[vad] 第 {i}/{rounds} 轮：VAD 没触发"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    fn stat(v: &mut Vec<u64>, n: &str) {
        if v.is_empty() {
            println!("{n}: 无数据");
            return;
        }
        v.sort_unstable();
        let len = v.len();
        let p = |q: f64| v[((len as f64 - 1.0) * q).round() as usize];
        println!(
            "{n}: n={len} 均值 {:.0}ms 最小 {}ms 中位 {}ms p90 {}ms 最大 {}ms",
            v.iter().sum::<u64>() as f64 / len as f64,
            v[0],
            p(0.5),
            p(0.9),
            v[len - 1]
        );
    }
    println!("\n===== P1 端到端延迟（{file}）=====");
    stat(&mut vad_stop, "最后有声帧 → speech_stopped（VAD 判定）");
    stat(&mut e2e, "最后有声帧 → 首个音频（端到端，不含播放缓冲）");
    Ok(())
}
