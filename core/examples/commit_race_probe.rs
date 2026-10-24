//! 探针三：`commit()` 之后立刻 `create_response()` 会不会抢跑？
//!
//! ## 假设
//!
//! App 里「用户说完 → 本地 VAD 判 200ms 静音 → commit + create_response」是
//! **一口气发出去的**。如果服务端处理 `response.create` 时还没处理完
//! `input_audio_buffer.commit`，这一轮就会基于**不完整/空的输入**生成 ——
//! 模型只会按提示词敷衍一句「好，查着呢」，**不会调 delegate_task**。
//!
//! 这能解释真机那个时好时坏的现象（同样是那句任务，有时派活有时不派）。
//!
//! ## 干什么
//!
//! 同一段音频放两遍，只改一个变量：
//! - A 轮：commit 后**立刻** create_response（复刻 App）
//! - B 轮：commit 后**等 300ms** 再 create_response
//!
//! 比较两轮的工具调用次数。
//!
//! ## 跑法
//!
//! ```bash
//! source tools/dev-env.sh
//! cargo run -q -p morphofly-core --example commit_race_probe -- /tmp/task1.wav
//! ```

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
        "只吃 16k mono 16bit wav"
    );
    Ok(reader.samples::<i16>().filter_map(std::result::Result::ok).collect())
}

async fn play(client: &RealtimeClient, pcm: &[i16]) -> Result<()> {
    for chunk in pcm.chunks(320) {
        client.send_audio(chunk)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // 尾部静音，模拟本地 VAD 判「说完了」之前的 200ms
    let silence = vec![0i16; 320];
    for _ in 0..10 {
        client.send_audio(&silence)?;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

async fn collect(
    rx: &mut tokio::sync::broadcast::Receiver<RealtimeEvent>,
    secs: u64,
) -> (usize, String) {
    let mut calls = 0usize;
    let mut said = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Err(_) => break,
            Ok(Err(_)) => break,
            Ok(Ok(ev)) => match ev {
                RealtimeEvent::ToolCall { name, .. } => {
                    calls += 1;
                    println!("    🔧 {name}");
                }
                RealtimeEvent::TranscriptDelta { text } | RealtimeEvent::TextDelta { text } => {
                    said.push_str(&text);
                }
                RealtimeEvent::Error { message, .. } => println!("    ❌ {message}"),
                _ => {}
            },
        }
    }
    (calls, said)
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
    let client = RealtimeClient::connect(&cfg).await?;
    let mut rx = client.events();
    let tools = tool_defs();
    morphofly_core::tools::assert_tool_invariant(&tools)
        .map_err(|e| anyhow::anyhow!("工具不变量: {e:?}"))?;
    client.send_tools(&tools)?;
    tokio::time::sleep(Duration::from_millis(400)).await;

    // ── A 轮：commit 后立刻 create_response（复刻 App 现状）──
    println!("\n[A 轮] 推音频 → commit → **立刻** create_response");
    play(&client, &pcm).await?;
    client.commit()?;
    client.create_response()?; // 不等
    let (a_calls, a_said) = collect(&mut rx, 20).await;
    println!("[A 轮] 工具 {a_calls} 次，说「{}」", a_said.trim());

    tokio::time::sleep(Duration::from_secs(1)).await;

    // ── B 轮：commit 后等 300ms 再 create_response ──
    println!("\n[B 轮] 推音频 → commit → **等 300ms** → create_response");
    play(&client, &pcm).await?;
    client.commit()?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    client.create_response()?;
    let (b_calls, b_said) = collect(&mut rx, 20).await;
    println!("[B 轮] 工具 {b_calls} 次，说「{}」", b_said.trim());

    println!("\n[探针] 结论：");
    println!("  A（立刻）: {a_calls} 次工具");
    println!("  B（等 300ms）: {b_calls} 次工具");
    if b_calls > a_calls {
        println!("  ✅ 竞态假设成立 —— commit 后要等一下再 create_response");
    } else if a_calls > 0 && b_calls > 0 {
        println!("  ⚠️ 两轮都调了工具，看不出来 —— 这个场景不复现 App 的问题");
    } else {
        println!("  ❌ 两轮都不调工具，竞态不是（唯一）原因");
    }

    let report = json!({
        "wav": path,
        "immediate": { "tool_calls": a_calls, "said": a_said },
        "delayed_300ms": { "tool_calls": b_calls, "said": b_said },
        "race_hypothesis": b_calls > a_calls,
    });
    std::fs::write(
        "/tmp/morphofly-commit-race.json",
        serde_json::to_string_pretty(&report)?,
    )?;
    println!("[探针] 报告写到 /tmp/morphofly-commit-race.json");
    Ok(())
}
