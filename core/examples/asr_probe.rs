//! 探针：StepFun 实时语音到底支不支持「把用户说的话转成文字」？
//!
//! ## 为什么需要这个探针
//!
//! 现在的痛点：用户说「帮我看看蝶梦项目的 README」，模型有时只回一句
//! 「好，查着呢」却**不调 `delegate_task`** —— 我们没有任何办法在本地帮它派活，
//! 因为协议层把用户转写事件（`conversation.item.input_audio_transcription.*`）丢掉了，
//! 宿主**拿不到用户原话**（见 `providers/stepfun.rs` 顶部的注释）。
//!
//! 如果服务端支持开会话级转写，我们就能拿到原话 → 本地确定性派活，
//! 不再依赖模型自觉。但这属于「供应商行为断言」，必须实测，不能猜。
//!
//! ## 干什么
//!
//! 1. 连 WS，下发 `session.update`（带 `input_audio_transcription`）
//! 2. 把一个 wav（16k mono）按 20ms 帧推上去
//! 3. 补 1.5s 静音让它触发一轮
//! 4. **打印服务端发来的每一个原始事件类型**，重点看：
//!    - `session.updated` 里有没有回显 transcription 配置
//!    - 有没有 `conversation.item.input_audio_transcription.completed/delta`
//!    - 有没有 `error`
//!
//! ## 跑法
//!
//! ```bash
//! source tools/dev-env.sh
//! set -a; . ~/.diemeng/credentials; set +a     # 或者已有 ~/.diemeng/config.toml
//! cargo run -q -p morphofly-core --example asr_probe -- /tmp/task1.wav
//! ```
//!
//! 输出同时写到 `/tmp/morphofly-asr-probe.json`（不进仓库）。

use anyhow::{Context, Result};
use futures_util::SinkExt;
use morphofly_core::realtime::{build_request_url, ensure_crypto_provider};
use morphofly_core::Config;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// 读 16k mono 16bit wav
fn read_wav_16k_mono(path: &str) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("打开 {path} 失败"))?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.sample_rate == 16_000 && spec.bits_per_sample == 16,
        "探针只吃 16k mono 16bit wav，实际 {}Hz {}ch {}bit",
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample
    );
    Ok(reader
        .samples::<i16>()
        .filter_map(std::result::Result::ok)
        .collect())
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/task1.wav".into());
    let pcm = read_wav_16k_mono(&path)?;
    anyhow::ensure!(!pcm.is_empty(), "音频是空的");
    println!(
        "[探针] 音频 {}：{} 采样（{:.2}s）",
        path,
        pcm.len(),
        pcm.len() as f32 / 16000.0
    );

    ensure_crypto_provider();
    let cfg = Config::load()?;
    let key = cfg.api_key()?;
    let url = build_request_url(&cfg.realtime.endpoint, &cfg.realtime.model)?;
    println!("[探针] 连 {} （key {}）", url, cfg.masked_key());

    let mut req = url.as_str().into_client_request()?;
    req.headers_mut()
        .insert("Authorization", format!("Bearer {}", key.trim()).parse()?);
    let (mut ws, _resp) = tokio_tungstenite::connect_async(req).await?;
    println!("[探针] WS 已连");

    // ── 1) 等 session.created ──
    let mut seen: Vec<String> = Vec::new();
    let mut transcription_events = 0usize;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), ws.next_msg()).await {
            Ok(Some(v)) => {
                let ty = v
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                if ty.starts_with("conversation.item.input_audio_transcription") {
                    transcription_events += 1;
                    println!("[探针] ★ 转写事件: {}", serde_json::to_string(&v)?);
                }
                seen.push(ty.clone());
                if ty == "session.created" {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    println!("[探针] 连上后收到: {seen:?}");

    // ── 2) 下发带 input_audio_transcription 的 session.update ──
    //
    // 先试 OpenAI Realtime 的写法：{ model: "whisper-1" }。
    // StepFun 自称与 OpenAI 同构，但转写这块**没验证过** —— 这正是本探针的目的。
    let update = json!({
        "type": "session.update",
        "session": {
            "input_audio_transcription": { "model": "whisper-1" }
        }
    });
    ws.send(Message::Text(update.to_string().into())).await?;
    println!("[探针] 已下发 session.update（input_audio_transcription=whisper-1）");

    // 看它认不认
    let mut accepted: Option<bool> = None;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), ws.next_msg()).await {
            Ok(Some(v)) => {
                let ty = v
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                seen.push(ty.clone());
                if ty == "session.updated" {
                    accepted = Some(true);
                    // 回显里有没有 transcription 字段？
                    let echoed = v
                        .pointer("/session/input_audio_transcription")
                        .cloned()
                        .unwrap_or(Value::Null);
                    println!("[探针] ✅ session.updated；回显 transcription = {echoed}");
                } else if ty == "error" {
                    accepted = Some(false);
                    println!("[探针] ❌ 服务端 error: {}", serde_json::to_string(&v)?);
                } else if ty.starts_with("conversation.item.input_audio_transcription") {
                    transcription_events += 1;
                    println!("[探针] ★ 转写事件: {}", serde_json::to_string(&v)?);
                }
                if ty == "session.updated" || ty == "error" {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    println!("[探针] 配置结果: {accepted:?}（None = 没回 session.updated 也没报错）");

    // ── 3) 推音频（20ms 一帧，实时节奏）──
    let frame = 320usize; // 16k * 20ms
    for chunk in pcm.chunks(frame) {
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, {
            let mut bytes = Vec::with_capacity(chunk.len() * 2);
            for s in chunk {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
            bytes
        });
        let msg = json!({ "type": "input_audio_buffer.append", "audio": b64 });
        ws.send(Message::Text(msg.to_string().into())).await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 静音 1.5s，触发服务端 VAD
    let silence = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        vec![0u8; frame * 2 * 75],
    );
    ws.send(Message::Text(
        json!({ "type": "input_audio_buffer.append", "audio": silence })
            .to_string()
            .into(),
    ))
    .await?;
    println!("[探针] 音频推完 + 1.5s 静音，等转写/回复…");

    // ── 4) 收 20 秒，数转写事件 ──
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), ws.next_msg()).await {
            Ok(Some(v)) => {
                let ty = v
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                if ty.starts_with("conversation.item.input_audio_transcription") {
                    transcription_events += 1;
                    let text = v
                        .get("transcript")
                        .or_else(|| v.get("delta"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    println!("[探针] ★ 转写: {ty} = {text:?}");
                } else if ty == "error" {
                    println!("[探针] error: {}", serde_json::to_string(&v)?);
                }
                seen.push(ty);
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }

    // ── 汇总 ──
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for k in &seen {
        *kinds.entry(k.clone()).or_default() += 1;
    }
    println!("\n[探针] 收到的事件类型统计：");
    for (k, n) in &kinds {
        println!("  {n:>3} × {k}");
    }
    let support = if transcription_events > 0 {
        "✅ 支持（收到转写事件）"
    } else if accepted == Some(true) {
        "⚠️ 配置被接受，但没收到转写事件（可能模型名不对 / 只对 server_vad 生效）"
    } else {
        "❌ 不支持（配置被拒或没回 session.updated）"
    };
    println!("\n[探针] 结论：{support}");

    let report = json!({
        "wav": path,
        "transcription_supported": transcription_events > 0,
        "config_accepted": accepted,
        "transcription_events": transcription_events,
        "event_kinds": kinds,
    });
    std::fs::write(
        "/tmp/morphofly-asr-probe.json",
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("[探针] 报告写到 /tmp/morphofly-asr-probe.json");
    Ok(())
}

/// 小工具：从 ws 读下一条文本消息并解析成 JSON（非文本直接跳过）
trait NextMsg {
    async fn next_msg(&mut self) -> Option<Value>;
}

impl NextMsg
    for tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
{
    async fn next_msg(&mut self) -> Option<Value> {
        use futures_util::StreamExt;
        loop {
            let m = self.next().await?;
            match m.ok()? {
                Message::Text(t) => {
                    if let Ok(v) = serde_json::from_str::<Value>(&t) {
                        return Some(v);
                    }
                }
                Message::Close(_) => return None,
                _ => continue,
            }
        }
    }
}
