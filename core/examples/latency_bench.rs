//! 验收 P1 的正式测法：连续 N 次往返，统计首音频延迟分布。
//!
//! 用法（wings-pc）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p morphofly-core --example latency_bench -- --rounds 10 --file testdata/audio/01_wake_short.mp3
//!
//! 每轮：新连接 → 推流 → 等 speech_started/stopped → 记首音频 → 断开
//! 为什么每轮新连接：避免上下文累积影响延迟测量（也顺便验证连接稳定性）

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
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

    let mut rounds = 10usize;
    let mut file = "testdata/audio/01_wake_short.mp3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rounds" => rounds = args.next().unwrap_or("10".into()).parse()?,
            "--file" => file = args.next().unwrap_or(file),
            "-h" | "--help" => {
                println!("用法: latency_bench [--rounds N] [--file <音频>]");
                return Ok(());
            }
            _ => {}
        }
    }

    let cfg = Config::load()?;
    let mut connect_ms = Vec::new();
    let mut first_audio_ms = Vec::new();
    let mut failures = 0usize;

    for i in 1..=rounds {
        let mic = AudioCapture::file_source(&file)?;
        let mut mic_rx = mic.subscribe();

        let t_conn = Instant::now();
        let client = match RealtimeClient::connect(&cfg).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[bench] 第 {i} 轮连接失败: {e}");
                failures += 1;
                continue;
            }
        };
        connect_ms.push(t_conn.elapsed().as_millis() as u64);
        let mut ev_rx = client.events();

        let mut t_speech_stopped: Option<Instant> = None;
        let mut t_first_audio: Option<Instant> = None;
        let mut done = false;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let ev = tokio::select! {
                biased;
                r = ev_rx.recv() => match r { Ok(e)=>Some(e), Err(_)=>None },
                r = mic_rx.recv() => {
                    if let Ok(frame) = r { let _ = client.send_audio(&frame); }
                    continue;
                }
                _ = tokio::time::sleep_until(deadline) => { break; }
            };
            let Some(ev) = ev else { break };
            match ev {
                RealtimeEvent::SpeechStopped => t_speech_stopped = Some(Instant::now()),
                RealtimeEvent::AudioDelta { .. } => {
                    if t_first_audio.is_none() {
                        t_first_audio = Some(Instant::now());
                    }
                }
                RealtimeEvent::ResponseDone { .. } => {
                    done = true;
                    break;
                }
                RealtimeEvent::Error { message, .. } => {
                    eprintln!("[bench] 第 {i} 轮服务端错误: {message}");
                }
                _ => {}
            }
        }

        if let (Some(a), Some(b)) = (t_speech_stopped, t_first_audio) {
            let ms = b.duration_since(a).as_millis() as u64;
            first_audio_ms.push(ms);
            println!("[bench] 第 {i:>2}/{rounds} 轮：首音频 {ms:>4}ms{}",
                if done { "" } else { " (未收到 response.done)" });
        } else if done {
            println!("[bench] 第 {i:>2}/{rounds} 轮：收到 response.done 但没测到延迟");
            failures += 1;
        } else {
            println!("[bench] 第 {i:>2}/{rounds} 轮：超时");
            failures += 1;
        }
        client.close();
        tokio::time::sleep(Duration::from_millis(400)).await;
    }

    fn stats(v: &mut Vec<u64>, name: &str) {
        if v.is_empty() {
            println!("{name}: 无数据");
            return;
        }
        v.sort_unstable();
        let n = v.len();
        let sum: u64 = v.iter().sum();
        let mean = sum as f64 / n as f64;
        let p = |q: f64| v[((n as f64 - 1.0) * q).round() as usize];
        println!(
            "{name}: n={n} 均值 {mean:.0}ms 最小 {}ms 中位 {}ms p90 {}ms p95 {}ms 最大 {}ms",
            v[0], p(0.5), p(0.9), p(0.95), v[n - 1]
        );
    }

    println!("\n===== 验收 P1 采样结果（{rounds} 轮，音频 {file}）=====");
    stats(&mut connect_ms, "连接耗时");
    stats(&mut first_audio_ms, "首音频延迟（speech_stopped → 第一个 delta）");
    println!("失败轮数: {failures}/{rounds}");

    // 验收标准：p90 < 800ms（合格） / < 500ms（理想）
    if !first_audio_ms.is_empty() {
        first_audio_ms.sort_unstable();
        let p90 = first_audio_ms[((first_audio_ms.len() as f64 - 1.0) * 0.9).round() as usize];
        let ideal = p90 < 500;
        let ok = p90 < 800;
        println!(
            "判定：p90={p90}ms → {}",
            if ideal { "达到理想值 <500ms ✅✅" }
            else if ok { "达到合格值 <800ms ✅" }
            else { "不达标 ❌" }
        );
        anyhow::ensure!(ok, "P1 不达标：p90={p90}ms");
    }
    Ok(())
}
