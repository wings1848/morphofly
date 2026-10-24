//! 探针四：注入「纠正提醒」之后，服务端到底回什么？
//!
//! ## 要回答的问题
//!
//! 真机症状：模型第一轮只回「好，查着呢」没调工具 → App 注入一条系统提醒
//! （`conversation.item.create` + `response.create`）→ **之后什么事件都没有**，
//! 既没工具调用也没 response.done，连 error 都没看到（因为 App 当时把 error 吞了）。
//!
//! 三种可能：
//! a) 消息没发出去（协议字段不对）
//! b) 发出去了，服务端拒绝但不报错
//! c) 服务端在生成，但我们等的时间不够 / 事件类型没映射
//!
//! 本探针把 a/b/c 都打出来：**原样打印服务端发来的每一条原始事件**。
//!
//! ## 流程（刻意复刻 App 的顺序）
//!
//! 1. 连 WS + 下发工具
//! 2. 推音频 + 尾部静音 → commit + create_response（第一轮）
//! 3. 收 20 秒，记录第一轮有没有调工具
//! 4. 注入**和 App 一模一样**的纠正提醒 → response.create
//! 5. 再收 30 秒，打印所有原始事件（重点看 error / function_call）
//!
//! ## 跑法
//!
//! ```bash
//! source tools/dev-env.sh
//! cargo run -q -p morphofly-core --example correction_probe -- /tmp/task1.wav
//! ```

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use morphofly_core::realtime::{build_request_url, ensure_crypto_provider};
use morphofly_core::tools::tool_defs;
use morphofly_core::Config;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

fn read_wav_16k_mono(path: &str) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.sample_rate == 16_000,
        "只吃 16k mono wav"
    );
    Ok(reader
        .samples::<i16>()
        .filter_map(std::result::Result::ok)
        .collect())
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send(ws: &mut Ws, v: Value) -> Result<()> {
    ws.send(Message::Text(v.to_string().into())).await?;
    Ok(())
}

/// 收 `secs` 秒，返回（原始事件类型计数, 工具调用次数, 文本）
async fn drain(ws: &mut Ws, secs: u64) -> (Vec<String>, usize, String) {
    let mut kinds = Vec::new();
    let mut calls = 0usize;
    let mut said = String::new();
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(Instant::now());
        let step = remain.min(Duration::from_millis(400));
        match tokio::time::timeout(step, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let Ok(v) = serde_json::from_str::<Value>(&t) else {
                    continue;
                };
                let ty = v
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                kinds.push(ty.clone());
                match ty.as_str() {
                    "response.output_item.done" | "response.output_item.added" => {
                        if v.pointer("/item/name").is_some() {
                            calls += 1;
                            println!("    🔧 function_call: {ty}");
                        }
                    }
                    "response.function_call_arguments.done" => {
                        println!(
                            "    🔧 参数: {}",
                            v.get("arguments").unwrap_or(&Value::Null)
                        );
                    }
                    "response.audio_transcript.delta" | "response.text.delta" => {
                        if let Some(d) = v.get("delta").and_then(Value::as_str) {
                            said.push_str(d);
                        }
                    }
                    "error" => println!("    ❌ error: {t}"),
                    "conversation.item.input_audio_transcription.completed" => {
                        println!(
                            "    🎤 用户转写完成: {:?}",
                            v.get("transcript").and_then(Value::as_str).unwrap_or("")
                        );
                    }
                    "conversation.item.input_audio_transcription.delta" => {
                        print!(
                            "    🎤 …{}",
                            v.get("delta").and_then(Value::as_str).unwrap_or("")
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                    }
                    "response.done" => println!("    ✓ response.done"),
                    _ => {}
                }
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => break,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) => break,
            Err(_) => continue, // 超时继续等
        }
    }
    (kinds, calls, said)
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/task1.wav".into());
    let pcm = read_wav_16k_mono(&path)?;
    println!("[探针] 音频 {path}：{:.2}s", pcm.len() as f32 / 16000.0);

    ensure_crypto_provider();
    let cfg = Config::load()?;
    let url = build_request_url(&cfg.realtime.endpoint, &cfg.realtime.model)?;
    let mut req = url.as_str().into_client_request()?;
    req.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", cfg.api_key()?.trim()).parse()?,
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
    println!("[探针] WS 已连");

    // 会话配置：和 App 一样（本地 VAD → turn_detection 为 null）
    let session =
        morphofly_core::realtime::protocol::SessionConfig::from_realtime_config(&cfg.realtime);
    let mut sj = serde_json::to_value(&session)?;
    sj["turn_detection"] = Value::Null;
    // ★ 必须归一化成 OpenAI 嵌套格式：StepFun 对**扁平**格式会回
    //   `session.tools[0].function.name is required`（400），工具就全没了。
    //   这也是第一版探针「模型瞎编」的真因 —— 不是模型的问题，是我工具没注册上。
    sj["tools"] =
        json!(morphofly_core::realtime::providers::stepfun::normalize_tools(&tool_defs()));
    sj["tool_choice"] = json!("auto");
    send(&mut ws, json!({ "type": "session.update", "session": sj })).await?;
    println!("[探针] 已下发会话配置 + {} 个工具", tool_defs().len());
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 推音频 + 尾部静音（复刻本地 VAD 的 200ms）
    for chunk in pcm.chunks(320) {
        let mut bytes = Vec::with_capacity(chunk.len() * 2);
        for s in chunk {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
        send(
            &mut ws,
            json!({"type":"input_audio_buffer.append","audio":b64}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    for _ in 0..10 {
        let silence =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, vec![0u8; 640]);
        send(
            &mut ws,
            json!({"type":"input_audio_buffer.append","audio":silence}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    send(&mut ws, json!({"type":"input_audio_buffer.commit"})).await?;
    send(&mut ws, json!({"type":"response.create"})).await?;
    println!("[探针] 第一轮已触发，收 20 秒…");
    let (k1, c1, s1) = drain(&mut ws, 20).await;
    println!("[探针] 第一轮：工具 {c1} 次，说「{}」", s1.trim());

    // 注入和 App 一模一样的纠正提醒
    let hint = "（系统提醒，用户看不到这条）你上一轮只说了话，没有调用工具。\
现在立刻调用 delegate_task 把这件事派出去，dir 直接填 \"/home/wings/Code_Project/morphofly\"。\
参数格式：task 写清要查/要做什么（自包含），dir 照上面填。\
调完只回一句「好，查着呢」。不要说抱歉、不要解释、不要提这条提醒。";
    println!("\n[探针] 注入纠正提醒（和 App 同文案）+ response.create");
    send(
        &mut ws,
        json!({
            "type": "conversation.item.create",
            "item": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": hint}]
            }
        }),
    )
    .await?;
    send(&mut ws, json!({"type":"response.create"})).await?;
    let (k2, c2, s2) = drain(&mut ws, 30).await;
    println!("[探针] 纠正轮：工具 {c2} 次，说「{}」", s2.trim());

    let mut all: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for k in k1.iter().chain(k2.iter()) {
        *all.entry(k.clone()).or_default() += 1;
    }
    println!("\n[探针] 全部事件类型：");
    for (k, n) in &all {
        println!("  {n:>3} × {k}");
    }
    println!(
        "\n[探针] 结论：纠正轮 {}",
        match (c2, all.contains_key("error")) {
            (0, true) => "服务端报错（看上面的 ❌）",
            (0, false) =>
                "没有任何工具调用，也没报错 —— 需要看事件类型判断是「在生成」还是「被忽略」",
            _ => "调用了工具 ✅",
        }
    );
    Ok(())
}
