//! 探针：StepFun 认不认 `response.create` 里的 `tool_choice: "required"`？
//!
//! ## 为什么需要
//!
//! 真机反复出现：用户说了任务，模型只回一句「好，查着呢」**不调工具**。
//! 文字提醒不够（模型继续说同一句），所以改用协议级硬约束：
//! 纠正时发 `response.create` + `{"tool_choice":"required"}`，这一轮强制调工具。
//!
//! 但这是**供应商行为断言**，必须实测：
//! - 认的话 → 纠正一定能派出活（产品承诺「说任务就真派活」才成立）
//! - 不认的话 → 服务端静默忽略（无害，但也别指望它）
//!
//! ## 干什么
//!
//! 1. 连 WS，下发工具声明（5 个，不变量）
//! 2. 推一段真实语音（20ms 帧），补静音触发第一轮
//! 3. 看第一轮有没有调工具（正常情况可能不调 —— 这正是我们要修的毛病）
//! 4. 再发一次 `response.create` + `tool_choice=required`
//! 5. 看这一轮**有没有** `function_call` 事件
//!
//! ## 跑法
//!
//! ```bash
//! source tools/dev-env.sh
//! cargo run -q -p morphofly-core --example toolchoice_probe -- /tmp/task1.wav
//! ```
//!
//! 报告写到 `/tmp/morphofly-toolchoice-probe.json`。

use anyhow::Result;
use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::tools::tool_defs;
use morphofly_core::Config;
use serde_json::json;
use std::time::Duration;

fn read_wav_16k_mono(path: &str) -> Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    anyhow::ensure!(
        spec.channels == 1 && spec.sample_rate == 16_000 && spec.bits_per_sample == 16,
        "只吃 16k mono 16bit wav，实际 {}Hz {}ch {}bit",
        spec.sample_rate,
        spec.channels,
        spec.bits_per_sample
    );
    Ok(reader
        .samples::<i16>()
        .filter_map(std::result::Result::ok)
        .collect())
}

/// 收一段时间的事件，返回值：调了几次工具 / 模型说了什么 / 有没有 error
async fn collect(
    rx: &mut tokio::sync::broadcast::Receiver<RealtimeEvent>,
    secs: u64,
) -> (usize, String, Vec<String>) {
    let mut tool_calls = 0usize;
    let mut said = String::new();
    let mut errors = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) => break,
            Ok(Err(_)) => break,
            Ok(Ok(ev)) => match ev {
                RealtimeEvent::ToolCall { name, .. } => {
                    tool_calls += 1;
                    println!("  🔧 工具调用: {name}");
                }
                RealtimeEvent::TranscriptDelta { text } | RealtimeEvent::TextDelta { text } => {
                    said.push_str(&text);
                }
                RealtimeEvent::ResponseDone { status } => {
                    println!("  ✓ 一轮结束（{status}）");
                }
                RealtimeEvent::Error { message, code } => {
                    println!("  ❌ error: {message}（{code:?}）");
                    errors.push(format!("{message} {code:?}"));
                }
                _ => {}
            },
        }
    }
    (tool_calls, said, errors)
}

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/task1.wav".into());
    let pcm = read_wav_16k_mono(&path)?;
    println!(
        "[探针] 音频 {path}：{} 采样（{:.2}s）",
        pcm.len(),
        pcm.len() as f32 / 16000.0
    );

    ensure_crypto_provider();
    let cfg = Config::load()?;
    let client = RealtimeClient::connect(&cfg).await?;
    let mut rx = client.events();

    let tools = tool_defs();
    morphofly_core::tools::assert_tool_invariant(&tools)
        .map_err(|e| anyhow::anyhow!("工具不变量被破坏: {e:?}"))?;
    client.send_tools(&tools)?;
    println!("[探针] 已下发 {} 个工具", tools.len());
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 推音频 + 静音，触发第一轮
    for chunk in pcm.chunks(320) {
        client.send_audio(chunk)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let silence = vec![0i16; 320];
    for _ in 0..75 {
        client.send_audio(&silence)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // ★ 我们的会话关掉了服务端 VAD（turn_detection=null，为了 790ms 低延迟），
    // 所以必须手动 commit + create_response —— 否则服务端根本不知道「说完了」，
    // 第一版探针忘了这步，结果第一轮什么都没发生。
    client.commit()?;
    tokio::time::sleep(Duration::from_millis(120)).await;
    client.create_response()?;
    println!("[探针] 已 commit + create_response，等第一轮（最多 20s）…");
    let (calls_auto, said_auto, errs_auto) = collect(&mut rx, 20).await;
    println!(
        "[探针] 第一轮（默认 tool_choice=auto）：工具 {} 次，模型说「{}」",
        calls_auto,
        said_auto.trim()
    );

    // 关键实验：强制这一轮必须调工具
    println!("\n[探针] 发 response.create + tool_choice=required");
    client.create_response_with(json!({ "tool_choice": "required" }))?;
    let (calls_required, said_required, errs_required) = collect(&mut rx, 25).await;
    println!(
        "[探针] 强制轮：工具 {} 次，模型说「{}」",
        calls_required,
        said_required.trim()
    );

    let supported = calls_required > 0;
    println!(
        "\n[探针] 结论：{}",
        if supported {
            "✅ 支持 tool_choice=required（强制轮真的调了工具）"
        } else if said_required.trim().is_empty() && calls_required == 0 {
            "❌ 不支持：强制轮返回空响应（既不调工具也不说话）—— 比普通重试更糟，别用"
        } else {
            "❌ 不支持 / 没生效（强制轮仍然没调工具）"
        }
    );

    let report = json!({
        "wav": path,
        "auto_turn": { "tool_calls": calls_auto, "said": said_auto, "errors": errs_auto },
        "required_turn": { "tool_calls": calls_required, "said": said_required, "errors": errs_required },
        "tool_choice_required_supported": supported,
    });
    std::fs::write(
        "/tmp/morphofly-toolchoice-probe.json",
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("[探针] 报告写到 /tmp/morphofly-toolchoice-probe.json");
    Ok(())
}
