//! 关键实验：**禁用服务端 VAD，改客户端手动 commit** —— 这是压延迟的正确姿势。
//!
//! StepFun 官方文档（guides/developer/realtime.md）明确写：
//! > 可以通过 session.update 把 turn_detection 设置为 **null** 来禁用 VAD。
//! > 禁用后客户端必须手动发 input_audio_buffer.commit + response.create。
//!
//! 我们之前传 `{"type":"none"}` 被拒（only server_vad is supported），
//! 但传**字面量 null** 是另一回事 —— 那是「不设 turn_detection」，服务端就不会跑 VAD。
//!
//! 这是把 P1 延迟从 3.2s 压到 500ms 以内的关键。
//!
//! 用法：cargo run -q -p diemeng-core --example manual_vad -- --rounds 5 --file testdata/audio/04_ack_zai.mp3

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::spectrum::rms_i16;
use diemeng_core::Config;
use serde_json::json;
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
    let mut e2e = Vec::new();

    for i in 1..=rounds {
        let mic = AudioCapture::file_source(&file)?;
        let mut mic_rx = mic.subscribe();
        let client = Arc::new(RealtimeClient::connect(&cfg).await?);
        let mut ev_rx = client.events();

        // ★ 核心：把 turn_detection 显式设为 JSON null（不是省略！）
        // SessionConfig 里 skip_serializing_if=is_none 会把 None 整个省略掉，
        // 所以这里直接发原始 JSON。
        client.send_session_update_raw(json!({
            "modalities": ["text", "audio"],
            "instructions": cfg.realtime.instructions,
            "voice": cfg.realtime.voice,
            "input_audio_format": "pcm16",
            "output_audio_format": "pcm16",
            "turn_detection": serde_json::Value::Null
        }))?;

        // 等 session.updated，确认 turn_detection:null 生效后再推流
        let t_wait = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(t_wait, ev_rx.recv()).await {
                Ok(Ok(RealtimeEvent::SessionUpdated)) => { println!("[manual] session.updated 已收到（VAD 应为 null）"); break; }
                Ok(Ok(RealtimeEvent::Error { message, code })) => {
                    println!("[manual] ⚠️ session.update 被拒: {message} ({code:?})");
                    break;
                }
                Ok(Ok(_)) => continue,
                _ => { println!("[manual] ⚠️ 没等到 session.updated"); break; }
            }
        }

        let last_voiced: Arc<AtomicU64> = Arc::new(AtomicU64::new(u64::MAX));
        let t0 = Instant::now();
        let c = client.clone();
        let lv = last_voiced.clone();
        let mic_handle = mic;
        let push = tokio::spawn(async move {
            loop {
                // 文件源自己持着发送端，通道不会关闭；靠 is_file_finished() 判断
                match tokio::time::timeout(Duration::from_millis(300), mic_rx.recv()).await {
                    Ok(Ok(frame)) => {
                        if rms_i16(&frame) > 0.02 {
                            lv.store(t0.elapsed().as_millis() as u64, Ordering::Relaxed);
                        }
                        let _ = c.send_audio(&frame);
                    }
                    Ok(Err(_)) => break,
                    Err(_) => {
                        // 超时：文件推完了就退出
                        if mic_handle.is_file_finished() {
                            break;
                        }
                    }
                }
            }
        });

        // 推完后立刻 commit + response.create（客户端决定何时算说完）
        let t_commit = Arc::new(tokio::sync::Mutex::new(None::<Instant>));
        let tc = t_commit.clone();
        let c2 = client.clone();
        let committer = tokio::spawn(async move {
            match tokio::time::timeout(Duration::from_secs(15), push).await {
                Ok(_) => println!("[manual] 推流任务正常结束"),
                Err(_) => println!("[manual] ⚠️ 推流任务超时（15s）"),
            }
            match c2.commit() {
                Ok(_) => println!("[manual] commit 已发"),
                Err(e) => println!("[manual] ❌ commit 失败: {e}"),
            }
            match c2.create_response() {
                Ok(_) => println!("[manual] response.create 已发"),
                Err(e) => println!("[manual] ❌ response.create 失败: {e}"),
            }
            *tc.lock().await = Some(Instant::now());
        });

        let mut t_audio: Option<Instant> = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        loop {
            let ev = match tokio::time::timeout_at(deadline, ev_rx.recv()).await {
                Err(_) | Ok(Err(_)) => break,
                Ok(Ok(e)) => e,
            };
            match ev {
                RealtimeEvent::AudioDelta { .. } => {
                    if t_audio.is_none() {
                        t_audio = Some(Instant::now());
                    }
                }
                RealtimeEvent::ResponseDone { .. } => break,
                RealtimeEvent::Error { message, .. } => eprintln!("[manual] err: {message}"),
                _ => {}
            }
        }
        let _ = committer.await;
        client.close();

        let lv_ms = last_voiced.load(Ordering::Relaxed);
        match (lv_ms, t_audio) {
            (l, Some(au)) if l != u64::MAX => {
                let lv_at = t0 + Duration::from_millis(l);
                let ms = au.duration_since(lv_at).as_millis() as u64;
                e2e.push(ms);
                println!("[manual] 第 {i}/{rounds} 轮：最后有声帧 → 首个音频 {ms}ms");
            }
            _ => println!("[manual] 第 {i}/{rounds} 轮：没测到（VAD 没触发？）"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    if e2e.is_empty() {
        println!("\n[manual] ❌ 一轮都没成 —— turn_detection:null 可能被拒");
        return Ok(());
    }
    e2e.sort_unstable();
    let n = e2e.len();
    let p = |q: f64| e2e[((n as f64 - 1.0) * q).round() as usize];
    println!(
        "\n===== 手动 VAD 模式 P1（{file}）=====\n最后有声帧 → 首个音频: n={n} 均值 {:.0}ms 中位 {}ms p90 {}ms 最大 {}ms",
        e2e.iter().sum::<u64>() as f64 / n as f64, p(0.5), p(0.9), e2e[n - 1]
    );
    let p90 = p(0.9);
    println!("判定：p90={p90}ms → {}", if p90 < 500 { "达到理想值 ✅✅" } else if p90 < 800 { "达到合格值 ✅" } else { "仍不达标 ❌" });
    Ok(())
}
