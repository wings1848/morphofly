//! 延迟测量（正确口径）：从**文件推完**到 **speech_stopped** / **首个音频**。
//!
//! 为什么不用「最后有声帧」当基准：文件末尾的静音 pad 会被算进延迟，
//! 而 pad 是我自己造的（为了触发 VAD），不该算进用户体验。
//!
//! 用法：cargo run -q -p diemeng-core --example latency2 -- --rounds 8 --file testdata/audio/04_ack_zai.mp3

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::Config;
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

    let mut rounds = 8usize;
    let mut file = "testdata/audio/04_ack_zai.mp3".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--rounds" => rounds = args.next().unwrap_or("8".into()).parse()?,
            "--file" => file = args.next().unwrap_or(file),
            _ => {}
        }
    }
    let cfg = Config::load()?;
    println!("[lat2] model={} file={file}", cfg.realtime.model);
    let mut vad = Vec::new();
    let mut e2e = Vec::new();

    for i in 1..=rounds {
        let mic = AudioCapture::file_source(&file)?;
        let mut rx = mic.subscribe();
        let client = Arc::new(RealtimeClient::connect(&cfg).await?);
        let mut ev = client.events();

        // 推流任务：推完立刻记录 t_done（这是基准点）
        let c = client.clone();
        let t0 = Instant::now();
        let push = tokio::spawn(async move {
            loop {
                match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                    Ok(Ok(f)) => { let _ = c.send_audio(&f); }
                    Ok(Err(_)) => break,
                    Err(_) => { if mic.is_file_finished() { break; } }
                }
            }
            t0.elapsed().as_millis() as u64
        });

        let mut t_stop: Option<Instant> = None;
        let mut t_audio: Option<Instant> = None;
        let dl = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let evv = match tokio::time::timeout_at(dl, ev.recv()).await {
                Err(_) | Ok(Err(_)) => break,
                Ok(Ok(e)) => e,
            };
            match evv {
                RealtimeEvent::SpeechStopped => { if t_stop.is_none() { t_stop = Some(Instant::now()); } }
                RealtimeEvent::AudioDelta { .. } => { if t_audio.is_none() { t_audio = Some(Instant::now()); } }
                RealtimeEvent::ResponseDone { .. } => break,
                _ => {}
            }
        }
        client.close();
        let pushed_ms = match tokio::time::timeout(Duration::from_secs(3), push).await {
            Ok(Ok(v)) => v,
            _ => t0.elapsed().as_millis() as u64,
        };
        let t_push_done = t0 + Duration::from_millis(pushed_ms);

        match (t_stop, t_audio) {
            (Some(s), Some(a)) => {
                let v = s.duration_since(t_push_done).as_millis() as i64;
                let e = a.duration_since(t_push_done).as_millis() as i64;
                vad.push(v); e2e.push(e);
                println!("[lat2] 第 {i}/{rounds} 轮：推流完 → VAD判完 {v:>6}ms，→ 首个音频 {e:>6}ms");
            }
            _ => println!("[lat2] 第 {i}/{rounds} 轮：没触发（推流 {pushed_ms}ms）"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    fn stat(v: &mut Vec<i64>, n: &str) {
        if v.is_empty() { println!("{n}: 无数据"); return; }
        v.sort_unstable();
        let len = v.len();
        let p = |q: f64| v[((len as f64 - 1.0) * q).round() as usize];
        println!("{n}: n={len} 均值 {}ms 最小 {}ms 中位 {}ms p90 {}ms 最大 {}ms",
            v.iter().sum::<i64>() as f64 / len as f64, v[0], p(0.5), p(0.9), v[len-1]);
    }
    println!("\n===== 从「推流结束」算起 =====");
    stat(&mut vad, "→ VAD 判定说完");
    stat(&mut e2e, "→ 首个音频（不含播放缓冲）");
    Ok(())
}
