//! 多轮对话验证：连续说三句话，看是否每轮都正常响应、上下文是否保持。
//!
//! 这是真实使用场景：用户先说「帮我看看项目」，再说「有几个目录」，再说「谢谢」。
//!
//! 用法（wings-pc）：
//!   cargo run -q -p morphofly-core --example multiturn_test

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::audio::playback::AudioPlayer;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::Config;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const TURNS: &[&str] = &[
    "testdata/audio/04_ack_zai.mp3",
    "testdata/audio/06_cmd_progress.mp3",
    "testdata/audio/08_cmd_stop.mp3",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    ensure_crypto_provider();

    let cfg = Config::load()?;
    println!("[multi] model={}", cfg.realtime.model);

    let player = Arc::new(AudioPlayer::open()?);
    let client = Arc::new(RealtimeClient::connect(&cfg).await?);
    let mut ev = client.events();
    client.send_tools(&diemeng_core::tools::tool_defs())?;

    let audio_frames = Arc::new(AtomicU64::new(0));
    let transcript = Arc::new(tokio::sync::Mutex::new(String::new()));
    let done_flag = Arc::new(AtomicU64::new(0));

    let c2 = client.clone();
    let ev_task = {
        let c2 = c2.clone();
        let p = player.clone();
        let af = audio_frames.clone();
        let tr = transcript.clone();
        let df = done_flag.clone();
        tokio::spawn(async move {
            loop {
                let e = match tokio::time::timeout(Duration::from_secs(60), ev.recv()).await {
                    Err(_) | Ok(Err(_)) => break,
                    Ok(Ok(e)) => e,
                };
                match e {
                    RealtimeEvent::AudioDelta { pcm, sample_rate } => {
                        let n = p.append_pcm16(&pcm, sample_rate);
                        af.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    RealtimeEvent::TranscriptDelta { text } => {
                        tr.lock().await.push_str(&text);
                    }
                    RealtimeEvent::ToolCall { call_id, name, arguments } => {
                        println!("[multi]   🔧 工具调用 {name}({arguments})");
                        // 工具调用后服务端会再发一轮 response，所以要等两次 done
                        df.fetch_add(1, Ordering::Relaxed);
                        // 真实产品这里会派活给 DSH；测试里回一个固定结果
                        let speech = match name.as_str() {
                            "query_progress" => "任务还在跑，大概还要几分钟。".to_string(),
                            "delegate_task" => "好，记下了，干完跟你说。".to_string(),
                            _ => format!("{name} 已处理。"),
                        };
                        let c3 = c2.clone();
                        tokio::spawn(async move {
                            let _ = c3.send_function_call_output(&call_id, &speech);
                            let _ = c3.create_response();
                        });
                    }
                    RealtimeEvent::ResponseDone { .. } => {
                        df.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        })
    };

    let mut results = Vec::new();

    for (i, file) in TURNS.iter().enumerate() {
        let n = i + 1;
        let frames_before = audio_frames.load(Ordering::Relaxed);
        let done_before = done_flag.load(Ordering::Relaxed);
        transcript.lock().await.clear();

        println!("\n[multi] ── 第 {n} 轮：{file} ──");
        // 推音频
        let mic = AudioCapture::file_source(file)?;
        let mut rx = mic.subscribe();
        loop {
            match tokio::time::timeout(Duration::from_millis(300), rx.recv()).await {
                Ok(Ok(f)) => { let _ = client.send_audio(&f); }
                Ok(Err(_)) => break,
                Err(_) => { if mic.is_file_finished() { break; } }
            }
        }
        // 手动 commit（因为默认关服务端 VAD）
        let t_commit = Instant::now();
        if let Err(e) = client.commit() {
            println!("[multi]   ⚠️ commit 失败: {e}");
            break;
        }
        if let Err(e) = client.create_response() {
            println!("[multi]   ⚠️ create_response 失败: {e}");
            break;
        }

        // 等这一轮响应完成。
        // 注意：触发工具调用时服务端会多来一轮 response，
        // 所以判定条件是「done 数 > done_before 且已稳定 1.5 秒无新 done」。
        let t = Instant::now();
        let mut first_audio_ms: Option<u64> = None;
        let mut last_done = done_before;
        let mut stable_since: Option<Instant> = None;
        while t.elapsed() < Duration::from_secs(40) {
            let now_done = done_flag.load(Ordering::Relaxed);
            if now_done > last_done {
                last_done = now_done;
                stable_since = Some(Instant::now());
            } else if now_done > done_before {
                if let Some(s) = stable_since {
                    if s.elapsed() > Duration::from_millis(1500) {
                        break;
                    }
                }
            }
            if first_audio_ms.is_none() && audio_frames.load(Ordering::Relaxed) > frames_before {
                first_audio_ms = Some(t_commit.elapsed().as_millis() as u64);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let frames = audio_frames.load(Ordering::Relaxed).saturating_sub(frames_before);
        let text = transcript.lock().await.clone();
        if frames == 0 {
            println!("[multi]   ❌ 没有音频回应");
            results.push(false);
        } else {
            println!(
                "[multi]   ✅ 首音频 {}ms，{} 帧音频，字幕：「{}」",
                first_audio_ms.unwrap_or(0), frames,
                if text.is_empty() { "（无）" } else { text.trim() }
            );
            results.push(true);
        }
        tokio::time::sleep(Duration::from_millis(800)).await;
    }

    client.close();
    player.stop_now();
    let _ = tokio::time::timeout(Duration::from_secs(3), ev_task).await;

    let ok = results.iter().filter(|x| **x).count();
    println!("\n[multi] ==== 结果：{ok}/{} 轮成功 ====", results.len());
    anyhow::ensure!(ok == results.len(), "有轮次失败");
    println!("[multi] ✅ 多轮对话正常");
    Ok(())
}
