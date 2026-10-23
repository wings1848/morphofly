//! 接缝 2.1（阶段 2 核心）：说一句话 → 派活给 DSH → 拿回人话总结。
//!
//! 这条链路把三段接起来：实时语音（听懂）→ 工具分发（转交）→ DSH（干活）→ 语音（汇报）。
//!
//! 用法（wings-pc）：
//!   set -a; . ~/.diemeng/credentials; set +a
//!   cargo run -q -p diemeng-core --example voice_task -- --file testdata/audio/07_cmd_fix_bug.mp3
//!   cargo run -q -p diemeng-core --example voice_task -- --text "列一下 /tmp 里有什么"

use diemeng_core::audio::capture::AudioCapture;
use diemeng_core::dispatch::Dispatcher;
use diemeng_core::dsh::DshBridge;
use diemeng_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use diemeng_core::tasks::TaskStore;
use diemeng_core::tools::tool_defs;
use diemeng_core::Config;
use std::sync::Arc;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    ensure_crypto_provider();

    let mut file: Option<String> = None;
    let mut text: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--file" => file = args.next(),
            "--text" => text = args.next(),
            "-h" | "--help" => {
                println!("用法: voice_task [--file <音频>] | [--text <文本>]");
                return Ok(());
            }
            _ => {}
        }
    }

    let cfg = Config::load()?;
    diemeng_core::config::ensure_data_dir(&cfg)?;
    let store = Arc::new(tokio::sync::Mutex::new(TaskStore::open(
        cfg.task_store_path(),
    )?));
    let bridge = Arc::new(DshBridge::from_config(&cfg));
    let dispatcher = Arc::new(Dispatcher::new(
        store.clone(),
        bridge.clone(),
        cfg.dsh.default_cwd.clone(),
    ));

    // --- 完整链路 ---
    if let Some(path) = file {
        println!("[voice_task] 用音频文件走完整链路: {path}");
        let mic = AudioCapture::file_source(&path)?;
        let mut mic_rx = mic.subscribe();
        let client = RealtimeClient::connect(&cfg).await?;
        let mut ev_rx = client.events();
        client.send_tools(&tool_defs())?;
        println!("[voice_task] WS 已连，4 工具已下发");

        let t0 = std::time::Instant::now();
        let mut tool_calls: Vec<(String, String, String)> = Vec::new();
        loop {
            let ev = tokio::select! {
                r = ev_rx.recv() => match r { Ok(e)=>e, Err(_)=>break },
                r = mic_rx.recv() => match r {
                    Ok(frame) => { let _ = client.send_audio(&frame); continue; }
                    Err(_) => { tokio::time::sleep(Duration::from_millis(1500)).await; continue; }
                },
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    println!("[voice_task] 30s 超时");
                    break;
                }
            };
            let ms = t0.elapsed().as_millis();
            match ev {
                RealtimeEvent::SpeechStarted => println!("{ms:>6}ms  用户开口"),
                RealtimeEvent::SpeechStopped => println!("{ms:>6}ms  用户说完"),
                RealtimeEvent::TranscriptDelta { text } => println!("{ms:>6}ms  它说: {text}"),
                RealtimeEvent::ToolCall { call_id, name, arguments } => {
                    println!("{ms:>6}ms  🔧 工具调用 {name}({arguments})");
                    tool_calls.push((call_id, name, arguments));
                }
                RealtimeEvent::ResponseDone { status } => {
                    println!("{ms:>6}ms  本轮结束 {status}");
                    // 如果这一轮里有工具调用，回灌结果再让它说一句
                    if !tool_calls.is_empty() {
                        for (call_id, name, args) in tool_calls.drain(..) {
                            println!("[voice_task] 执行 {name}…");
                            match dispatcher.dispatch(&name, &args).await {
                                Ok(r) => {
                                    println!("[voice_task] → 会说：{}", r.speech);
                                    client.send_function_call_output(&call_id, &r.speech)?;
                                }
                                Err(e) => {
                                    let msg = format!("这个没办成：{e}");
                                    println!("[voice_task] ✗ {msg}");
                                    client.send_function_call_output(&call_id, &msg)?;
                                }
                            }
                        }
                        client.create_response()?;
                        continue;
                    }
                    break;
                }
                RealtimeEvent::Error { message, code } => {
                    println!("{ms:>6}ms  ⚠️ {message} ({code:?})");
                }
                RealtimeEvent::AudioDelta { pcm, .. } => {
                    // 只报大小，不打印 PCM（否则刷屏几千行）
                    let _ = pcm;
                }
                RealtimeEvent::AudioDone => println!("{ms:>6}ms  audio.done"),
                RealtimeEvent::SessionCreated { model, .. } => println!("{ms:>6}ms  session={model}"),
                RealtimeEvent::SessionUpdated => println!("{ms:>6}ms  会话配置已更新"),
                other => {
                    println!("{ms:>6}ms  · {}", other.kind());
                }
            }
        }
        println!("[voice_task] 事件循环结束，tool_calls={}", tool_calls.len());
        client.close();
    }

    // --- 纯文本模式（不走语音，直接测分发）---
    fs_mode_text(&dispatcher, store.clone(), text).await?;
    Ok(())
}

async fn fs_mode_text(
    dispatcher: &Arc<Dispatcher>,
    store: Arc<tokio::sync::Mutex<TaskStore>>,
    text: Option<String>,
) -> anyhow::Result<()> {
    let task = match text {
        Some(t) => t,
        None => return Ok(()),
    };
    println!("[voice_task] 直接派活: {task}");
    let args = serde_json::json!({"task": task, "dir": std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())});
    let r = dispatcher.dispatch("delegate_task", &args.to_string()).await?;
    println!("[voice_task] → 语音会说：{}", r.speech);
    let task_id = r.data["task_id"].as_str().unwrap_or("").to_string();

    // 等它真的干完（最多 5 分钟）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
    loop {
        {
            let s = store.lock().await;
            if let Some(t) = s.get(&task_id) {
                if t.status.is_terminal() {
                    println!(
                        "[voice_task] 任务结束 status={:?}\n[voice_task] 汇报文本：{}",
                        t.status, t.summary
                    );
                    // 验收 D5：汇报文案必须能朗读
                    let bad = ["```", "`", "http://", "https://", "#"];
                    for b in bad {
                        anyhow::ensure!(!t.summary.contains(b), "汇报里不能有 {b}: {}", t.summary);
                    }
                    anyhow::ensure!(!t.summary.trim().is_empty(), "汇报不能为空");
                    println!("[voice_task] ✅ 派活 → 执行 → 可朗读汇报 全链路通");
                    return Ok(());
                }
            }
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("等任务超时");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
