//! 接缝 0.4：Rust 侧 WS 连实时语音，推一段音频，收到文字/音频回复。
//!
//! 验收命令（wings-pc，有网）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p diemeng-core --example ws_roundtrip -- testdata/audio/04_ack_zai.mp3
//!
//! 干的事：
//!  1. 解码 mp3 → 重采样到 16k mono i16（用 rodio 解码，避免额外依赖）
//!  2. 连 WS，下发 session.update（4 个工具 + server_vad）
//!  3. 按 20ms 节奏 append 音频
//!  4. commit + response.create
//!  5. 收事件：记首音频延迟、字幕、response.done
//!
//! 输出 JSON 到 /tmp/diemeng-ws-roundtrip.json（不进仓库），供断言脚本用。

use diemeng_core::realtime::{
    ensure_crypto_provider, encode_pcm16_base64, RealtimeClient, RealtimeEvent,
};
use diemeng_core::tools::tool_defs;
use diemeng_core::Config;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "testdata/audio/04_ack_zai.mp3".into());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(&path))
}

async fn decode_to_uplink(path: &str) -> anyhow::Result<Vec<i16>> {
    // rodio 解码 → f32 交错 → 下混 mono → 线性重采样到 16k → i16
    use rodio::Source;
    let file = std::fs::File::open(path)?;
    let decoder = rodio::Decoder::try_from(file)?;
    let channels = decoder.channels().get() as usize;
    let rate = decoder.sample_rate().get();
    // rodio 0.23 的 Decoder::Item 就是 f32 的 Sample
    let samples: Vec<f32> = decoder.collect();
    anyhow::ensure!(channels > 0 && rate > 0, "解码结果异常");

    let frames = samples.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut acc = 0.0f32;
        for c in 0..channels {
            acc += samples[i * channels + c];
        }
        mono.push(acc / channels as f32);
    }

    let target = 16_000u32;
    let out_len = (frames as u64 * target as u64 / rate as u64) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * rate as f64 / target as f64;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono.get(idx).copied().unwrap_or(0.0);
        let b = mono.get(idx + 1).copied().unwrap_or(a);
        let v = a + (b - a) * frac;
        out.push((v.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
    }
    println!(
        "[ws_roundtrip] 解码 {}：{}Hz {}ch {} 采样 → 16k mono {} 采样（{:.2}s）",
         path,
         rate,
         channels,
         frames,
         out.len(),
         out.len() as f32 / 16_000.0
    );
    Ok(out)
}

async fn run(path: &str) -> anyhow::Result<()> {
    ensure_crypto_provider();
    let cfg = Config::load()?;
    println!(
        "[ws_roundtrip] provider={} model={} key={} endpoint={}",
        cfg.realtime.provider, cfg.realtime.model, cfg.masked_key(), cfg.realtime.endpoint
    );

    let pcm = decode_to_uplink(path).await?;
    anyhow::ensure!(!pcm.is_empty(), "解码后没有音频");

    let t_connect = Instant::now();
    let client = RealtimeClient::connect(&cfg).await?;
    let connect_ms = t_connect.elapsed().as_millis() as u64;
    let mut rx = client.events();

    // 等 session.created（或第一个事件）
    let mut session_model = String::new();
    let mut session_id = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) => break,                       // 没等到 session.created，后面靠 response 判
            Ok(Err(_)) => break,                   // 通道关闭
            Ok(Ok(e)) => e,
        };
        match ev {
            RealtimeEvent::SessionCreated { session_id: sid, model } => {
                session_id = sid;
                session_model = model;
                break;
            }
            RealtimeEvent::Error { message, code } => {
                anyhow::bail!("连上但服务端报错: {message} ({code:?})");
            }
            _ => continue,
        }
    }
    println!("[ws_roundtrip] 连接 {connect_ms}ms，session={session_id} model={session_model}");

    // 轮次由服务端 VAD 驱动（StepFun 只支持 server_vad，且开着 VAD 时不能手动 commit）。
    // `TurnDetection::default()` 的阈值已按实测调到 500，否则正常音量音频不触发。
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 下发 4 个工具（不变量 I1）
    let tools = tool_defs();
    assert!(
        diemeng_core::tools::assert_tool_invariant(&tools).is_ok(),
        "工具不变量被破坏"
    );
    client.send_tools(&tools)?;
    println!("[ws_roundtrip] 已下发 {} 个工具", tools.len());

    // 按 20ms 帧推流
    let t_send = Instant::now();
    let frame = diemeng_core::FRAME_SAMPLES;
    let mut sent = 0usize;
    for chunk in pcm.chunks(frame) {
        client.send_audio(chunk)?;
        sent += chunk.len();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let send_ms = t_send.elapsed().as_millis() as u64;
    println!("[ws_roundtrip] 推流 {sent} 采样，耗时 {send_ms}ms");

    // 服务端 VAD：推完后补一段静音（1.5s）让它判定「说完了」并自动开一轮
    let silence = vec![0i16; diemeng_core::FRAME_SAMPLES];
    for _ in 0..75 {
        client.send_audio(&silence)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let t_commit = Instant::now();
    println!("[ws_roundtrip] 已推完 + 1.5s 静音，等服务端 VAD 触发回复…");

    // 收事件
    let mut first_audio_ms: Option<u128> = None;
    let mut audio_bytes = 0usize;
    let mut transcript = String::new();
    let mut stop = String::new();
    let mut errors: Vec<String> = Vec::new();
    let mut audio_frames = 0u64;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    loop {
        let got = tokio::time::timeout_at(deadline, rx.recv()).await;
        let ev = match got {
            Err(_) => {
                stop = "<超时>".into();
                break;
            }
            Ok(Err(_)) => {
                stop = "<通道关闭>".into();
                break;
            }
            Ok(Ok(e)) => e,
        };
        match ev {
            RealtimeEvent::AudioDelta { pcm, .. } => {
                if first_audio_ms.is_none() {
                    first_audio_ms = Some(t_commit.elapsed().as_millis());
                }
                audio_bytes += pcm.len() * 2;
                audio_frames += pcm.len() as u64 / 2;
            }
            RealtimeEvent::TranscriptDelta { text } => {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                transcript.push_str(&text);
            }
            RealtimeEvent::TranscriptDone { text } => {
                if !text.trim().is_empty() {
                    transcript = text;
                }
            }
            RealtimeEvent::TextDelta { text } => {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
                transcript.push_str(&text);
            }
            RealtimeEvent::ToolCall { name, arguments, .. } => {
                println!("\n[ws_roundtrip] 工具调用 {name}({arguments})");
            }
            RealtimeEvent::ResponseDone { status } => {
                stop = status;
                break;
            }
            RealtimeEvent::Error { message, code } => {
                errors.push(format!("{message} ({code:?})"));
            }
            _ => {}
        }
    }
    println!();

    let out = serde_json::json!({
        "endpoint": cfg.realtime.endpoint,
        "model": cfg.realtime.model,
        "connect_ms": connect_ms,
        "send_ms": send_ms,
        "sent_samples": sent,
        "session_id": session_id,
        "first_audio_ms": first_audio_ms,
        "audio_bytes": audio_bytes,
        "audio_frames": audio_frames,
        "transcript": transcript,
        "stop": stop,
        "errors": errors,
    });
    let p = std::env::var("ROUNDTRIP_OUT")
        .unwrap_or_else(|_| "/tmp/diemeng-ws-roundtrip.json".into());
    std::fs::write(&p, serde_json::to_string_pretty(&out)?)?;

    println!(
        "[ws_roundtrip] stop={stop} first_audio={:?}ms audio={}字节 transcript={}字 errors={:?}",
        first_audio_ms,
        audio_bytes,
        transcript.chars().count(),
        errors
    );
    client.close();

    anyhow::ensure!(audio_bytes > 0 || !transcript.trim().is_empty(),
        "既没收到音频也没收到文字（stop={stop}）");
    println!("[ws_roundtrip] ✅ WS 往返成功，结果已写 {p}");
    Ok(())
}
