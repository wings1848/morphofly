// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 蝶梦桌面壳。
//!
//! 职责边界（铁律）：这里只做三件事
//!  1. 开一个透明窗口，把 WebView 装进去
//!  2. 把 Rust core 的频谱/状态推给前端
//!  3. 把前端的点击转成 core 的状态迁移
//!
//! **不在这里碰音频**（core 里碰），**不在这里拿 Key**（core 里拿）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use diemeng_core::session::{OrbState, SessionMachine, Stimulus};
use diemeng_core::spectrum::{rms_i16, rms_to_visual, Spectrum};
use diemeng_core::Config;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

/// 全局共享状态
struct AppState {
    machine: Arc<Mutex<SessionMachine>>,
    caption: Arc<Mutex<String>>,
}

#[tauri::command]
async fn orb_click(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let mut m = state.machine.lock().await;
    let from = m.state();
    m.handle(Stimulus::Click).map_err(|e| e.to_string())?;
    let to = m.state();
    Ok(serde_json::json!({ "from": from.as_str(), "state": to.as_str(), "caption": to.label() }))
}

#[tauri::command]
async fn orb_state(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let m = state.machine.lock().await;
    let caption = state.caption.lock().await.clone();
    Ok(serde_json::json!({
        "state": m.state().as_str(),
        "caption": if caption.is_empty() { m.state().label().to_string() } else { caption },
        "connected": m.is_connected(),
    }))
}

#[tauri::command]
async fn set_caption(state: State<'_, AppState>, text: String) -> Result<(), String> {
    *state.caption.lock().await = text;
    Ok(())
}

/// 驱动状态机的外部刺激（供 realtime/audio 层调用）
#[tauri::command]
async fn poke(state: State<'_, AppState>, stimulus: String) -> Result<String, String> {
    let stim = match stimulus.as_str() {
        "connected" => Stimulus::Connected,
        "connection_failed" => Stimulus::ConnectionFailed,
        "model_speaking" => Stimulus::ModelSpeaking,
        "model_done" => Stimulus::ModelDone,
        "thinking" => Stimulus::Thinking,
        "user_speech_started" => Stimulus::UserSpeechStarted,
        "user_speech_ended" => Stimulus::UserSpeechEnded,
        "recover" => Stimulus::Recover,
        other => return Err(format!("未知刺激: {other}")),
    };
    let mut m = state.machine.lock().await;
    m.handle(stim).map_err(|e| e.to_string())?;
    Ok(m.state().as_str().to_string())
}

/// 后台循环：频谱 + 状态推给前端。
/// 视觉巡检模式：每 3 秒自动走一个状态，用于截图核对六态差异（验收 U2）。
///
/// 只在 `DIEMENG_DEMO_STATES=1` 时启用，产品运行时绝不会打开。
/// 用法：`DIEMENG_DEMO_STATES=1 ./target/debug/diemeng-app`，然后每 3s 截一张图。
fn spawn_state_walker(machine: Arc<Mutex<SessionMachine>>) {
    tauri::async_runtime::spawn(async move {
        // 合法迁移序列：S0 --click--> S1 --connected--> S2 --thinking--> S3 --speaking--> S4
        //               然后 disconnect --> S5，再 recover --> S0
        let steps: Vec<(&str, Stimulus)> = vec![
            ("S1 唤醒", Stimulus::Click),
            ("S2 聆听", Stimulus::Connected),
            ("S3 思考", Stimulus::Thinking),
            ("S4 说话", Stimulus::ModelSpeaking),
            ("S2 聆听(打断)", Stimulus::UserSpeechStarted),
            ("S0 回待机", Stimulus::Click),
        ];
        loop {
            for (label, stim) in &steps {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let mut m = machine.lock().await;
                match m.handle(*stim) {
                    Ok(_) => tracing::info!(state = m.state().as_str(), "视觉巡检 → {label}"),
                    Err(e) => tracing::warn!("视觉巡检迁移失败（{label}）: {e}"),
                }
            }
            // 最后走一次异常态再恢复
            tokio::time::sleep(Duration::from_secs(3)).await;
            {
                let mut m = machine.lock().await;
                m.set_connected(false);
                tracing::info!(state = m.state().as_str(), "视觉巡检 → S5 异常");
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            {
                let mut m = machine.lock().await;
                m.set_connected(true);
                let _ = m.handle(Stimulus::Recover);
                tracing::info!(state = m.state().as_str(), "视觉巡检 → 恢复");
            }
        }
    });
}

fn spawn_pump(app: AppHandle, machine: Arc<Mutex<SessionMachine>>, demo_audio: bool) {
    tauri::async_runtime::spawn(async move {
        let mut spectrum = Spectrum::with_default_window();
        let mut ticker = tokio::time::interval(Duration::from_millis(33));
        let started = Instant::now();
        let mut last_state = String::new();
        let mut phase = 0.0f32;

        loop {
            ticker.tick().await;
            let t = started.elapsed().as_secs_f32();

            let (bins, energy) = if demo_audio {
                let st = machine.lock().await.state();
                let (amp, freq) = match st {
                    OrbState::S0 => (0.02, 90.0),
                    OrbState::S1 => (0.25, 220.0),
                    OrbState::S2 => (0.45, 180.0),
                    OrbState::S3 => (0.30, 320.0),
                    OrbState::S4 => (0.70, 260.0),
                    OrbState::S5 => (0.01, 70.0),
                };
                let n = 512;
                let mut pcm = vec![0i16; n];
                for (i, slot) in pcm.iter_mut().enumerate() {
                    let tt = i as f32 / 16_000.0;
                    let wobble = 1.0 + 0.35 * (t * 3.7).sin();
                    *slot = ((amp * wobble
                        * (2.0 * std::f32::consts::PI * freq * tt + phase).sin()
                        + amp * 0.35 * (2.0 * std::f32::consts::PI * (freq * 2.7) * tt).sin())
                        * 32767.0) as i16;
                }
                phase += 0.05;
                let bins = spectrum.analyze_i16(&pcm);
                (bins, rms_to_visual(rms_i16(&pcm)))
            } else {
                (vec![0.0f32; diemeng_core::SPECTRUM_BINS], 0.0)
            };

            if app
                .emit("spectrum", serde_json::json!({ "bins": bins, "rms": energy }))
                .is_err()
            {
                break;
            }

            let st = { machine.lock().await.state() };
            if st.as_str() != last_state {
                last_state = st.as_str().to_string();
                let (glow, radius, speed) = st.visual();
                let payload = serde_json::json!({
                    "state": st.as_str(),
                    "caption": st.label(),
                    "visual": { "glow": glow, "radius": radius, "speed": speed }
                });
                let _ = app.emit("state", payload);
            }
        }
    });
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,diemeng=debug".into()),
        )
        .init();

    let cfg = Config::load().unwrap_or_default();
    tracing::info!(
        provider = %cfg.realtime.provider,
        model = %cfg.realtime.model,
        key = %cfg.masked_key(),
        "蝶梦启动"
    );
    let _ = diemeng_core::config::ensure_data_dir(&cfg);

    let machine = Arc::new(Mutex::new(SessionMachine::new(Duration::from_millis(
        cfg.idle_timeout_ms,
    ))));

    let demo_audio = std::env::var("DIEMENG_DEMO_AUDIO")
        .map(|v| v != "0")
        .unwrap_or(true);
    // 视觉巡检（仅 QA 用，见 spawn_state_walker 文档）
    let demo_states = std::env::var("DIEMENG_DEMO_STATES").map(|v| v != "0").unwrap_or(false);
    if demo_states {
        spawn_state_walker(machine.clone());
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            machine: machine.clone(),
            caption: Arc::new(Mutex::new(String::new())),
        })
        .invoke_handler(tauri::generate_handler![orb_click, orb_state, set_caption, poke])
        .setup(move |app| {
            let handle = app.handle().clone();
            spawn_pump(handle, machine.clone(), demo_audio);
            if let Some(win) = app.get_webview_window("orb") {
                let _ = win.show();
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
