//! 端到端测试驱动：喂一段音频，输出结构化结果给测试脚本判定。
//!
//! **为什么单独一个 example 而不是复用 voice_task**：
//! voice_task 是给人看的（打印人类可读日志），
//! 这个是给脚本看的（输出 JSON，字段稳定，不含颜色和装饰）。
//!
//! 用法：
//!   cargo run -q -p morphofly-core --example e2e_drive -- --file /tmp/a.pcm
//!
//! 输出（stdout 最后一行是 JSON）：
//!   {"wake":true,"tool_calls":["delegate_task"],"reply":"好，干完说",
//!    "asked_confirm":false,"elapsed_ms":1234,"error":null}

use morphofly_core::audio::capture::AudioCapture;
use morphofly_core::dispatch::Dispatcher;
use morphofly_core::dsh::DshBridge;
use morphofly_core::realtime::{ensure_crypto_provider, RealtimeClient, RealtimeEvent};
use morphofly_core::tasks::TaskStore;
use morphofly_core::Config;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_env_filter())
        .with_writer(std::io::stderr) // 日志走 stderr，stdout 只留 JSON
        .init();
    ensure_crypto_provider();

    let mut file = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--file" {
            file = args.next().unwrap_or_default();
        }
    }
    if file.is_empty() {
        anyhow::bail!("用法: e2e_drive --file <pcm|mp3>");
    }

    let cfg = Config::load()?;
    morphofly_core::config::ensure_data_dir(&cfg)?;

    // ★ 先过本地唤醒词 —— 这是真实产品的第一道门。
    //
    // 不加这一步，测试就变成「直接连云端」，
    // 谐音（小蝶儿/蝴蝶）会被云端 ASR 骗过去，误报成「误唤醒」。
    // 但真实产品里本地 KWS 不命中就**根本不会连云端**。
    #[cfg(feature = "kws")]
    {
        let kws_cfg = morphofly_core::kws::KwsConfig::default();
        match morphofly_core::kws::detect_in_file(&kws_cfg, std::path::Path::new(&file)) {
            Ok(hits) if hits.is_empty() => {
                // 本地没听到唤醒词 → 真实产品不会唤醒
                println!(
                    "{}",
                    serde_json::json!({
                        "wake": false,
                        "tool_calls": [],
                        "reply": "",
                        "asked_confirm": false,
                        "elapsed_ms": 0,
                        "speaking_done": false,
                        "error": null,
                        "kws_blocked": true,
                    })
                );
                return Ok(());
            }
            Ok(hits) => {
                eprintln!("[e2e] 本地唤醒命中: {hits:?}");
            }
            Err(e) => {
                eprintln!("[e2e] KWS 检测失败（继续）: {e}");
            }
        }
    }

    let store = Arc::new(Mutex::new(TaskStore::open(&cfg.task_store_path())?));
    let bridge = Arc::new(DshBridge::from_config(&cfg));
    let dispatcher = Arc::new(Dispatcher::new(
        store.clone(),
        bridge,
        cfg.dsh.default_cwd.clone(),
    ));

    let client = Arc::new(RealtimeClient::connect(&cfg).await?);
    let mut ev_rx = client.events();
    client.send_tools(&morphofly_core::tools::tool_defs())?;

    let mic = AudioCapture::file_source(&file)?;
    let mut mic_rx = mic.subscribe();

    let reply_shared = Arc::new(Mutex::new(String::new()));
    let t0 = Instant::now();
    let mut wake = false;
    let mut tool_calls: Vec<String> = Vec::new();
    let mut reply = String::new();
    let mut error: Option<String> = None;
    let mut speaking_done = false;
    // 工具调用后服务端会再来一轮 response，
    // 不能在第一轮 ResponseDone 就退出 —— 那样看不到工具的执行结果。
    let mut pending_tools = 0usize;

    // 推流任务
    //
    // ⚠️ 现在默认 use_local_vad=true（关掉服务端 VAD 来把延迟从 1994ms 压到 802ms），
    // 所以推完必须**手动 commit + create_response**，
    // 否则服务端永远不知道「用户说完了」，一个事件都不会来。
    let c = client.clone();
    let push = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_millis(300), mic_rx.recv()).await {
                Ok(Ok(frame)) => {
                    let _ = c.send_audio(&frame);
                }
                Ok(Err(_)) => break,
                Err(_) => {
                    if mic.is_file_finished() {
                        break;
                    }
                }
            }
        }
        // 推完立刻 commit
        if let Err(e) = c.commit() {
            eprintln!("commit 失败: {e}");
        }
        if let Err(e) = c.create_response() {
            eprintln!("create_response 失败: {e}");
        }
    });

    // 事件循环
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    loop {
        let ev = match tokio::time::timeout_at(deadline, ev_rx.recv()).await {
            Err(_) => break,
            Ok(Err(_)) => break,
            Ok(Ok(e)) => e,
        };
        match ev {
            RealtimeEvent::SpeechStarted => {
                wake = true;
            }
            // 关掉服务端 VAD 后不会有 SpeechStarted，
            // 用「收到任何回复内容」当作「唤醒成功」的等价信号
            RealtimeEvent::AudioDelta { .. } => {
                wake = true;
            }
            RealtimeEvent::TranscriptDelta { text } => {
                reply.push_str(&text);
            }
            RealtimeEvent::ToolCall {
                call_id,
                name,
                arguments,
            } => {
                eprintln!("[e2e] 工具调用 {name} 参数: {arguments}");
                tool_calls.push(name.clone());
                pending_tools += 1;
                // 真的分发（和产品行为一致）
                let d = dispatcher.clone();
                let c2 = client.clone();
                let reply_slot = reply_shared.clone();
                let n = name.clone();
                let a = arguments.clone();
                let cid = call_id.clone();
                tokio::spawn(async move {
                    let speech = match d.dispatch(&n, &a).await {
                        Ok(r) => r.speech,
                        Err(e) => format!("没办成：{e}"),
                    };
                    // 把工具执行结果也记进 reply，这样判定能看到拦截效果
                    reply_slot.lock().await.push_str(&speech);
                    let _ = c2.send_function_call_output(&cid, &speech);
                    let _ = c2.create_response();
                });
            }
            RealtimeEvent::ResponseDone { .. } => {
                if pending_tools > 0 {
                    // 还有工具没回灌结果，等下一轮
                    pending_tools -= 1;
                    continue;
                }
                speaking_done = true;
                break;
            }
            RealtimeEvent::Error { message, .. } => {
                error = Some(message);
            }
            _ => {}
        }
    }

    let _ = push.await;
    client.close();

    // 判断是否要求确认（看回复里有没有「要我…吗」的模式）
    let asked_confirm = reply.contains("要我") || reply.trim_end().ends_with('吗');

    // 诊断：模型说了话但没调工具 = 假装干活
    if tool_calls.is_empty() && !reply.trim().is_empty() {
        eprintln!("[e2e] ⚠️ 模型只说了话没调工具 —— 这是「假装干活」");
    }
    let tool_reply = reply_shared.lock().await.clone();
    if reply.trim().is_empty() {
        reply = tool_reply;
    }

    let out = serde_json::json!({
        "wake": wake,
        "tool_calls": tool_calls,
        "reply": reply.trim(),
        "asked_confirm": asked_confirm,
        "elapsed_ms": t0.elapsed().as_millis() as u64,
        "speaking_done": speaking_done,
        "error": error,
    });
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}

fn tracing_env_filter() -> String {
    std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into())
}
