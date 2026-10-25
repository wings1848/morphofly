//! 探针：StepFun realtime 的**服务端 VAD 到底会不会自己触发一轮回复**？
//!
//! 2026-10-25 真机（wings-gpd）现象：切到 server_vad 模式后，服务端
//! 从不发 `input_audio_buffer.speech_started/stopped`，也从不自动回复；
//! 只有我们手动 `response.create` 才有声音。这跟官方控制台（默认 server_vad）
//! 的行为不一致，必须先弄清契约再改代码。
//!
//! 用法（wings-gpd，有网 + 有 key）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p morphofly-core --example vad_probe -- testdata/audio/07_cmd_fix_bug.mp3 [--no-create]
//!
//! 它干的事：
//!   1. 连 WS → 下发**只有 turn_detection(server_vad)** 的 session.update
//!   2. 按 20ms 节奏推音频（16k pcm16，和 App 上行完全一致）
//!   3. 推完补 1.5s 静音
//!   4. 打印**每一个**服务端事件（带相对毫秒）
//!   5. 默认**不发** commit / response.create —— 纯观察服务端会不会自己触发
//!      （加 `--create` 则补发 response.create，用来看「手动触发能不能出声音」）
//!
//! 判读：
//!   - 有 speech_started + speech_stopped + response.created + audio.delta
//!     → 服务端 VAD 是全自动的，客户端只需 append
//!   - 只有 speech_* 没有 response.created → 服务端只做 VAD，回复要客户端 create
//!   - 什么都没有 → 服务端 VAD 没生效（配置/参数问题），只能本地判定

use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::Config;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .unwrap_or_else(|| "testdata/audio/07_cmd_fix_bug.mp3".into());
    let mut with_tools = false;
    let mut do_create = false;
    for a in args {
        match a.as_str() {
            "--create" => do_create = true,
            "--tools" => with_tools = true,
            _ => {}
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(&path, do_create, with_tools))
}

/// 解码任意音频 → 16k mono i16（和 App 上行一致的格式）
fn decode_to_uplink(path: &str) -> anyhow::Result<Vec<i16>> {
    use rodio::Source;
    let file = std::fs::File::open(path)?;
    let decoder = rodio::Decoder::try_from(file)?;
    let channels = decoder.channels().get() as usize;
    let rate = decoder.sample_rate().get();
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
        out.push(
            (v.clamp(-1.0, 1.0) * 32768.0)
                .round()
                .clamp(-32768.0, 32767.0) as i16,
        );
    }
    Ok(out)
}

fn ev_name(ev: &RealtimeEvent) -> String {
    match ev {
        RealtimeEvent::AudioDelta { pcm, sample_rate } => {
            format!("AudioDelta(samples={}, rate={sample_rate})", pcm.len())
        }
        RealtimeEvent::UserTranscript { text, is_final } => {
            format!("UserTranscript(final={is_final}, {text:?})")
        }
        RealtimeEvent::TranscriptDelta { text } => format!("TranscriptDelta({text:?})"),
        RealtimeEvent::ResponseDone { status } => format!("ResponseDone(status={status})"),
        RealtimeEvent::AssistantItem { item_id } => format!("AssistantItem({item_id})"),
        RealtimeEvent::Error { message, code } => format!("Error({code:?}, {message})"),
        other => other.kind().to_string(),
    }
}

async fn run(path: &str, do_create: bool, with_tools: bool) -> anyhow::Result<()> {
    ensure_crypto_provider();
    let cfg = Config::load()?;
    println!(
        "[probe] model={} endpoint={} server_vad={} create={}",
        cfg.realtime.model, cfg.realtime.endpoint, !cfg.realtime.use_local_vad, do_create
    );
    let pcm = decode_to_uplink(path)?;
    anyhow::ensure!(!pcm.is_empty(), "解码后没有音频");
    println!(
        "[probe] 音频 {} 采样（{:.2}s）",
        pcm.len(),
        pcm.len() as f32 / 16_000.0
    );

    let client = RealtimeClient::connect(&cfg).await?;
    let mut rx = client.events();
    let t0 = Instant::now();

    // 先等 session.created（确认连上）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(RealtimeEvent::SessionCreated { model, .. })) => {
                println!(
                    "[probe] +{}ms session.created model={model}",
                    t0.elapsed().as_millis()
                );
                break;
            }
            Ok(Ok(ev)) => println!("[probe] +{}ms {}", t0.elapsed().as_millis(), ev_name(&ev)),
            Ok(Err(_)) | Err(_) => anyhow::bail!("等 session.created 超时"),
        }
    }
    println!(
        "[probe] turn_detection = {:?}",
        morphofly_core::realtime::protocol::SessionConfig::from_realtime_config(&cfg.realtime)
            .turn_detection
    );

    if with_tools {
        // App 在 connect 之后会再发一次 session.update（带工具）——
        // 用这个开关复现「发了工具之后服务端 VAD 还灵不灵」。
        let tools = morphofly_core::tools::tool_defs();
        client.send_tools(&tools)?;
        println!(
            "[probe] 已下发 {} 个工具（复现 App 的第二次 session.update）",
            tools.len()
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // 推音频：20ms 一帧，严格实时节奏
    let frame = morphofly_core::FRAME_SAMPLES;
    let mut sent = 0usize;
    for chunk in pcm.chunks(frame) {
        client.send_audio(chunk)?;
        sent += chunk.len();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    println!(
        "[probe] +{}ms 推完 {sent} 采样，补 1.5s 静音…",
        t0.elapsed().as_millis()
    );
    let silence = vec![0i16; frame];
    for _ in 0..75 {
        client.send_audio(&silence)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let t_done = Instant::now();
    println!(
        "[probe] +{}ms 静音推完，开始观察 12 秒",
        t0.elapsed().as_millis()
    );

    if do_create {
        tokio::time::sleep(Duration::from_millis(300)).await;
        client.create_response()?;
        println!(
            "[probe] +{}ms 已补发 response.create",
            t0.elapsed().as_millis()
        );
    }

    let mut counts = std::collections::BTreeMap::<String, u32>::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Ok(ev)) => {
                let name = ev.kind().to_string();
                *counts.entry(name.clone()).or_default() += 1;
                if name != "AudioDelta" {
                    println!(
                        "[probe] +{}ms {} （audio流结束 +{}ms）",
                        t0.elapsed().as_millis(),
                        ev_name(&ev),
                        t_done.elapsed().as_millis()
                    );
                }
            }
            Ok(Err(_)) => {
                println!("[probe] 事件通道关闭");
                break;
            }
            Err(_) => break,
        }
    }
    println!("[probe] ==== 统计 ====");
    for (k, v) in counts {
        println!("[probe]   {k}: {v}");
    }
    Ok(())
}
