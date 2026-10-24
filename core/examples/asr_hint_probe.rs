//! 探针：StepFun 的转写能不能带「热词提示」（prompt），减少专有名词听错。
//!
//! ## 为什么需要
//!
//! 真机全链路实测（2026-10-25）：用户说「morphofly 项目的 README」，
//! 云端转写成「mosofly 项目的 limit」——项目名和文件名全错。
//! 本地已经加了模糊匹配 + 任务文本纠正（`core/src/projects.rs`），
//! 但那只救得了「有限集合」的项目名，救不了 README/DSH 这类内容词。
//!
//! OpenAI Realtime 的转写配置里有 `prompt` 字段（给 ASR 的上下文提示）。
//! StepFun 自称与 OpenAI 同构，但**没人验证过它支不支持 prompt**。
//! 供应商行为必须实测，不能猜。
//!
//! ## 跑法
//!
//! ```bash
//! source tools/dev-env.sh
//! set -a; . ~/.diemeng/credentials; set +a
//! # A：不带提示（基线）
//! cargo run -q -p morphofly-core --example asr_hint_probe -- /tmp/task1.wav
//! # B：带提示
//! cargo run -q -p morphofly-core --example asr_hint_probe -- /tmp/task1.wav \
//!   --prompt "蝶梦 morphofly DSH README 阶跃星辰 语音智能体"
//! ```
//!
//! 判定：
//! - 收到 `session.updated` 且回显里带 prompt → 支持
//! - 收到 `error` → 不支持（记下来，别再试）
//! - 没回显但转写文本变准了 → 支持但没有回显
//! - 回显里 prompt 被吞掉、文本没变 → 不支持（静默忽略）

use anyhow::{Context, Result};
use futures_util::SinkExt;
use morphofly_core::realtime::{build_request_url, ensure_crypto_provider};
use morphofly_core::Config;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

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

fn b64_pcm16(pcm: &[i16]) -> String {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for s in pcm {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    BASE64_STANDARD.encode(bytes)
}

/// 收消息直到 predicate 或超时，返回收到的事件类型列表
async fn drain(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    secs: u64,
    mut on_event: impl FnMut(&Value),
) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), ws.next_msg()).await {
            Ok(Some(v)) => on_event(&v),
            Ok(None) => return,
            Err(_) => continue,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut wav = "/tmp/task1.wav".to_string();
    let mut prompt: Option<String> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--prompt" => prompt = args.next(),
            other => wav = other.to_string(),
        }
    }
    let pcm = read_wav_16k_mono(&wav)?;
    println!(
        "[探针] 音频 {wav}：{} 采样（{:.2}s）；prompt={:?}",
        pcm.len(),
        pcm.len() as f32 / 16000.0,
        prompt
    );

    ensure_crypto_provider();
    let cfg = Config::load()?;
    let key = cfg.api_key()?;
    let url = build_request_url(&cfg.realtime.endpoint, &cfg.realtime.model)?;
    let mut req = url.as_str().into_client_request()?;
    req.headers_mut()
        .insert("Authorization", format!("Bearer {}", key.trim()).parse()?);
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
    println!("[探针] WS 已连（key {}）", cfg.masked_key());

    // 等 session.created
    drain(&mut ws, 10, |v| {
        if v.get("type").and_then(Value::as_str) == Some("session.created") {
            println!("[探针] 收到 session.created");
        }
    })
    .await;

    // 下发转写配置（带/不带 prompt）
    let mut transcription = json!({ "model": "whisper-1" });
    if let Some(p) = &prompt {
        transcription["prompt"] = json!(p);
    }
    let update = json!({
        "type": "session.update",
        "session": { "input_audio_transcription": transcription }
    });
    ws.send(Message::Text(update.to_string().into())).await?;
    println!("[探针] 已下发 session.update: {transcription}");

    let mut accepted: Option<bool> = None;
    drain(&mut ws, 8, |v| {
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("?");
        match ty {
            "session.updated" => {
                accepted = Some(true);
                let echoed = v
                    .pointer("/session/input_audio_transcription")
                    .cloned()
                    .unwrap_or(Value::Null);
                println!("[探针] ✅ session.updated；回显 input_audio_transcription = {echoed}");
            }
            "error" => {
                accepted = Some(false);
                println!(
                    "[探针] ❌ 服务端 error: {}",
                    serde_json::to_string(v).unwrap_or_default()
                );
            }
            _ => {}
        }
    })
    .await;
    println!("[探针] 配置结果 accepted={accepted:?}");

    // 推音频（20ms 帧）+ 1.5s 静音
    let frame = 320usize;
    for chunk in pcm.chunks(frame) {
        ws.send(Message::Text(
            json!({ "type": "input_audio_buffer.append", "audio": b64_pcm16(chunk) })
                .to_string()
                .into(),
        ))
        .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ws.send(Message::Text(
        json!({ "type": "input_audio_buffer.append",
                "audio": b64_pcm16(&vec![0i16; frame * 75]) })
        .to_string()
        .into(),
    ))
    .await?;
    // 本地 VAD 模式：手动 commit + 触发一轮
    ws.send(Message::Text(
        json!({ "type": "input_audio_buffer.commit" })
            .to_string()
            .into(),
    ))
    .await?;
    ws.send(Message::Text(
        json!({ "type": "response.create" }).to_string().into(),
    ))
    .await?;
    println!("[探针] 音频推完 + commit + create_response，等转写…");

    let mut texts: Vec<String> = Vec::new();
    drain(&mut ws, 25, |v| {
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("?");
        if ty.ends_with("input_audio_transcription.completed") {
            if let Some(t) = v.get("transcript").and_then(Value::as_str) {
                println!("[探针] ★ 转写完成: {t:?}");
                texts.push(t.to_string());
            }
        } else if ty.ends_with("input_audio_transcription.delta") {
            if let Some(d) = v.get("delta").and_then(Value::as_str) {
                print!("{d}");
            }
        } else if ty == "error" {
            println!(
                "[探针] error: {}",
                serde_json::to_string(v).unwrap_or_default()
            );
        }
    })
    .await;

    println!("\n[探针] 结论：");
    println!("  accepted = {accepted:?}");
    println!("  转写条数 = {}", texts.len());
    for t in &texts {
        println!("  → {t}");
    }
    if accepted == Some(false) {
        println!("  ❌ 服务端拒绝了带 prompt 的转写配置 —— 这个方向别再试");
    } else if texts.is_empty() {
        println!("  ⚠️ 没拿到转写，无法判断");
    } else if prompt.is_some() {
        println!("  看上面转写文本：专有名词变准了 = 支持 prompt；没变 = 被静默忽略");
    }
    Ok(())
}

/// 收一条 JSON 事件（丢掉二进制/坏 JSON），和 asr_probe 用同一套写法
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
