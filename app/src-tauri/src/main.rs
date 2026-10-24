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

mod voice;

use std::sync::Arc;
use std::time::{Duration, Instant};

use morphofly_core::session::{OrbState, SessionMachine, Stimulus};
use morphofly_core::spectrum::{rms_i16, rms_to_visual, Spectrum};
use morphofly_core::Config;
use tauri::menu::{Menu, MenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tokio::sync::Mutex;

/// 全局共享状态
struct AppState {
    machine: Arc<Mutex<SessionMachine>>,
    caption: Arc<Mutex<String>>,
    /// 工具分发器 —— 提到全局是为了让托盘菜单等非语音入口也能调
    /// （语音切换 agent 暂不可用，见 docs/v2/voice-switch-design.md）
    dispatcher: Arc<morphofly_core::dispatch::Dispatcher>,
    #[allow(dead_code)] // 预留给「改配置不重启」等命令
    cfg: Arc<morphofly_core::Config>,
}

/// 前端在「拖动结束」时调用，让球贴到最近的边。
///
/// 为什么不监听窗口 move 事件做防抖：move 事件在 Wayland 下行为不一致，
/// 且拖动过程中频繁触发会抖。由前端在 mouseup 时主动调一次更可控。
#[tauri::command]
async fn snap_orb(window: WebviewWindow) -> Result<(), String> {
    snap_to_edge(&window, SnapMode::NearestHorizontal);
    Ok(())
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
/// 只在 `MORPHOFLY_DEMO_STATES=1` 时启用，产品运行时绝不会打开。
/// 用法：`MORPHOFLY_DEMO_STATES=1 ./target/debug/morphofly-app`，然后每 3s 截一张图。
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
            // 最后走一次异常态再恢复。
            // ⚠️ 2026-10-24 审计发现：原来直接 set_connected(false)，
            // 但「从未连过」的情况下会保持 S0（这是有意的设计），
            // 于是巡检器永远到不了 S5，六态截图缺 S5。
            // 修法：先标记「连过」，再断线。
            tokio::time::sleep(Duration::from_secs(3)).await;
            {
                let mut m = machine.lock().await;
                m.set_connected(true); // 让 ever_connected = true
                m.set_connected(false); // 断开 → S5
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
                    *slot =
                        ((amp * wobble * (2.0 * std::f32::consts::PI * freq * tt + phase).sin()
                            + amp * 0.35 * (2.0 * std::f32::consts::PI * (freq * 2.7) * tt).sin())
                            * 32767.0) as i16;
                }
                phase += 0.05;
                let bins = spectrum.analyze_i16(&pcm);
                (bins, rms_to_visual(rms_i16(&pcm)))
            } else {
                (vec![0.0f32; morphofly_core::SPECTRUM_BINS], 0.0)
            };

            if app
                .emit(
                    "spectrum",
                    serde_json::json!({ "bins": bins, "rms": energy }),
                )
                .is_err()
            {
                break;
            }

            let st = { machine.lock().await.state() };
            if st.as_str() != last_state {
                tracing::info!(state = st.as_str(), prev = %last_state, "推状态给前端");
                last_state = st.as_str().to_string();
                let (glow, radius, speed) = st.visual();
                let payload = serde_json::json!({
                    "state": st.as_str(),
                    "caption": st.label(),
                    "visual": { "glow": glow, "radius": radius, "speed": speed }
                });
                if let Err(e) = app.emit("state", payload) {
                    tracing::warn!(error = %e, "推状态失败");
                }
            }
        }
    });
}

/// 语音会话守护：一进通话态就建会话，离开就拆。
///
/// 这条循环是「S1 → 连云端 → S2」的实现：
/// - 看到 S1 → 建 VoiceSession → 成功后 handle(Connected) → S2
/// - 看到离开通话态 → 关会话、回 S0
fn spawn_session_guard(
    app: AppHandle,
    machine: Arc<Mutex<SessionMachine>>,
    cfg: Arc<morphofly_core::Config>,
) {
    tauri::async_runtime::spawn(async move {
        let mut current: Option<voice::VoiceSession> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let st = { machine.lock().await.state() };

            // 需要建会话？（S1 且还没有）
            if voice::should_open_session(st) && current.is_none() {
                tracing::info!("建立语音会话…");
                match voice::VoiceSession::open(&cfg).await {
                    Ok(sess) => {
                        {
                            let mut m = machine.lock().await;
                            let _ = m.handle(Stimulus::Connected);
                            tracing::info!(state = m.state().as_str(), "会话已建立");
                        }
                        // 频谱 → 前端
                        let app2 = app.clone();
                        let machine2 = machine.clone();
                        let sess_for_run = sess;
                        let machine3 = machine2.clone();
                        // run 会阻塞到会话结束，所以 spawn 出去
                        let app3 = app2.clone();
                        let handle = tauri::async_runtime::spawn(async move {
                            let r = sess_for_run
                                .run(machine3.clone(), move |bins, rms| {
                                    let _ = app3.emit(
                                        "spectrum",
                                        serde_json::json!({ "bins": bins, "rms": rms }),
                                    );
                                })
                                .await;
                            if let Err(e) = r {
                                tracing::warn!(error = %e, "会话异常结束");
                            }
                            let stats = sess_for_run.stats();
                            tracing::info!(?stats, "会话结束");
                            sess_for_run.shutdown();
                        });
                        let _ = handle;
                        // 注意：会话对象已经 move 进 run 任务，current 不再持有
                        // 下一轮如果看到 S0 就什么也不做（会话自己会退出）
                        current = None;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "建立语音会话失败");
                        let mut m = machine.lock().await;
                        let _ = m.handle(Stimulus::ConnectionFailed);
                    }
                }
            }
        }
    });
}

/// 启动唤醒词常驻监听（仅在 `--features kws` 时编译）。
///
/// 接法：KWS 命中 → 状态机 S0 --WakeWord--> S1 → 连云端 → S2。
/// 这里只做「命中即唤醒」；真正连云端在阶段 1 的 duplex 链路里。
#[cfg(feature = "kws")]
fn spawn_wake_listener(machine: Arc<Mutex<SessionMachine>>) {
    use morphofly_core::kws::{wake_stimulus_for, KwsConfig, WakeEvent, WakeListener};

    let cfg = KwsConfig::default();
    if let Err(e) = cfg.validate() {
        tracing::warn!(
            error = %e,
            "唤醒词没启用（模型/关键词文件缺失）。见 docs/唤醒词.md 的一次性准备步骤"
        );
        return;
    }
    let m = machine.clone();
    match WakeListener::start(cfg, move |ev: WakeEvent| {
        let WakeEvent::Detected { keyword } = ev;
        let m = m.clone();
        // 状态机是 tokio Mutex，回调在 KWS 线程里，用 block_on 拿一下
        tauri::async_runtime::block_on(async move {
            let mut guard = m.lock().await;
            if let Some(stim) = wake_stimulus_for(guard.state()) {
                match guard.handle(stim) {
                    Ok(_) => tracing::info!(%keyword, state = guard.state().as_str(), "🔊 唤醒"),
                    Err(e) => tracing::warn!(error = %e, "唤醒迁移失败"),
                }
            }
        });
    }) {
        Ok(l) => {
            tracing::info!("唤醒词常驻监听已启动（喊「小蝶」试试）");
            // 泄漏到进程结束：App 生命周期内一直监听
            std::mem::forget(l);
        }
        Err(e) => tracing::error!(error = %e, "唤醒词启动失败"),
    }
}

#[cfg(not(feature = "kws"))]
fn spawn_wake_listener(_machine: Arc<Mutex<SessionMachine>>) {
    tracing::info!("唤醒词未编译进来（用 --features kws 打开）");
}

/// 演示模式（MORPHOFLY_DEMO_STATES）下不启动 KWS，避免抢麦克风
fn demo_states_placeholder() -> bool {
    std::env::var("MORPHOFLY_DEMO_STATES")
        .map(|v| v != "0")
        .unwrap_or(false)
}

/// 小球「归位坐标」——贴边后左上角应该在的地方。
///
/// 为什么要有它：`set_size` 和 `set_position` 都是异步的，
/// 用「当前尺寸/当前位置」反算新坐标会和窗口管理器抢，
/// 实测收起后窗口停在 (2712,1516)，比应有的 (2908,1516) 差 196px。
/// 记住一个确定的归位点，展开/收起都从它算，就不会漂。
static ORB_HOME: std::sync::OnceLock<std::sync::Mutex<Option<(i32, i32)>>> =
    std::sync::OnceLock::new();

fn orb_home() -> &'static std::sync::Mutex<Option<(i32, i32)>> {
    ORB_HOME.get_or_init(|| std::sync::Mutex::new(None))
}

fn set_orb_home(x: i32, y: i32) {
    if let Ok(mut g) = orb_home().lock() {
        *g = Some((x, y));
    }
}

/// 贴边完成了吗？没完成之前不许动气泡尺寸。
///
/// 2026-10-24 真机踩坑：前端一启动就会因为「推状态 S0」显示一次字幕，
/// 于是 `orb_tip(true)` 抢在贴边之前跑 —— 那时归位点还没记，它拿窗口
/// 当前所在的屏幕正中当基准，把窗口挪到 1249；随后贴边又按「已经被撑到
/// 360 宽」的尺寸算，最终停在 2696 而不是 2908，右下角就差 212px。
static ORB_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn get_orb_home() -> Option<(i32, i32)> {
    orb_home().lock().ok().and_then(|g| *g)
}

/// 让小球贴到屏幕右下角（避开中心，不挡视线）。
///
/// 用户要求：「小圆球默认显示在桌面合适位置，可自动贴边」
///
/// 位置策略：右下角内缩 24px（不贴死，留出视觉呼吸）。
/// 为什么是右下角：任务栏通常在底部，右下角是最不干扰主工作区的位置。
fn place_orb_bottom_right(win: &WebviewWindow) {
    snap_to_edge(win, SnapMode::BottomRight);
    // 置顶：配置里已经写了 alwaysOnTop，这里再设一次做兜底 ——
    // 实测 XWayland 下配置项有时要等窗口映射后补一次才稳。
    if let Err(e) = win.set_always_on_top(true) {
        tracing::warn!(error = %e, "设置置顶失败");
    } else {
        tracing::info!("小球已置顶（always on top）");
    }
}

/// 展开/收起气泡（写字幕用）。
///
/// 小球窗口只有 `ORB_SIZE` 那么小，放不下字幕。展开时把窗口向左撑到
/// `TIP_WIDTH` 宽、同时 x 减小同样的量 —— 屏幕上看球没动，左边多出一块
/// 半透明气泡。收起时原样缩回。
///
/// 为什么不用第二个窗口：多一个 WebView 大约多 30–50MB 内存，
/// 而内存本来就是本项目的老大难（PSS 371MB）。
#[tauri::command]
async fn orb_tip(window: WebviewWindow, show: bool) -> Result<(), String> {
    // 还没贴好边就先别动窗口：此时归位点不可信，动了会把启动位置搞歪。
    // 前端每个字幕都会再调一次，所以跳过这一两次不影响功能。
    if !ORB_READY.load(std::sync::atomic::Ordering::Relaxed) {
        tracing::debug!("气泡请求被忽略：小球还没贴好边");
        return Ok(());
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let small = (f64::from(ORB_SIZE) * scale).round() as u32;
    let wide = (f64::from(TIP_WIDTH) * scale).round() as u32;
    if small == 0 || wide <= small {
        return Err("窗口尺寸异常".into());
    }
    // 归位点：贴边时记下的；没记下就用当前位置兜底
    let home = match get_orb_home() {
        Some(h) => h,
        None => {
            let p = window.outer_position().map_err(|e| e.to_string())?;
            (p.x, p.y)
        }
    };
    let target_x = if show {
        home.0 - (wide - small) as i32
    } else {
        home.0
    };
    let target_w = if show { wide } else { small };

    if let Ok(size) = window.outer_size() {
        if size.width == target_w {
            return Ok(());
        }
    }

    // ★ 顺序不能反，而且两个方向还不一样 —— 2026-10-24 真机踩出来的：
    //
    // KWin 不允许把窗口摆到「右边缘超出屏幕」的位置：它会直接把窗口拉回
    // x = 屏宽 - 窗宽。所以「先 set_position 再 set_size」在收起时会翻车：
    // 窗口还是 360 宽，把 x 设成 2908 → 右边缘 3268 越界 → KWin 拉回 2712；
    // 之后再 set_size(148)，窗口就停在 2712 而不是 2908（误差 196px）。
    //
    // 展开：窗口小，先左移不会越界；再长宽。
    // 收起：先缩宽，再右移。
    // 最后都读回实际位置纠一次，防止异步还没生效。
    if show {
        let _ = window.set_position(tauri::PhysicalPosition::new(target_x, home.1));
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = window.set_size(tauri::PhysicalSize::new(target_w, small));
    } else {
        let _ = window.set_size(tauri::PhysicalSize::new(target_w, small));
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = window.set_position(tauri::PhysicalPosition::new(target_x, home.1));
    }
    tokio::time::sleep(Duration::from_millis(60)).await;

    // 读回校验 + 纠正一次
    if let Ok(pos) = window.outer_position() {
        if pos.x != target_x {
            tracing::warn!(want = target_x, got = pos.x, "气泡位置没生效，纠正一次");
            let _ = window.set_position(tauri::PhysicalPosition::new(target_x, home.1));
        }
    }
    if let Ok(size) = window.outer_size() {
        if size.width != target_w {
            tracing::warn!(
                want = target_w,
                got = size.width,
                "气泡宽度没生效，纠正一次"
            );
            let _ = window.set_size(tauri::PhysicalSize::new(target_w, small));
        }
    }
    tracing::info!(show, target_w, target_x, "气泡窗口已调整");
    Ok(())
}

/// 贴边方式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapMode {
    /// 贴到屏幕右下角（首次启动用）
    BottomRight,
    /// 按当前位置贴最近的水平边（用户拖动后松手用）
    NearestHorizontal,
}

/// 计算并设置贴边位置。
///
/// 用户要求：「小圆球默认显示在桌面合适位置，可自动贴边」
///
/// Wayland 限制：`set_position` 在纯 Wayland 下会被合成器忽略
/// （没有全局坐标概念）。实测 wings-pc 是 KDE Wayland，
/// 所以启动时要 `GDK_BACKEND=x11` 走 XWayland 才能贴边。
/// 这是平台限制，不是代码问题。
fn snap_to_edge(win: &WebviewWindow, mode: SnapMode) {
    // ★ 先强制回到「收起」尺寸：归位点是按 148×148 定义的。
    // 如果此刻窗口因为显示字幕被撑到 360 宽，直接用当前尺寸算会把
    // 归位点算成「360 宽的右下角」，收起后就偏了 212px（真机实测）。
    let scale_now = win.scale_factor().unwrap_or(1.0);
    let small = (f64::from(ORB_SIZE) * scale_now).round() as u32;
    // 尺寸直接按「收起态」算，不去读 outer_size()：set_size 是异步的，
    // 刚设完就读回来可能还是旧值（360），那归位点就全错。
    let size = if small > 0 {
        let _ = win.set_size(tauri::PhysicalSize::new(small, small));
        // 等它真的缩下来：否则下面设位置时窗口还宽 360，
        // KWin 会以「右边缘越界」为由把 x 拉回去（真机实测差 212px）。
        for _ in 0..12 {
            match win.outer_size() {
                Ok(s) if s.width <= small => break,
                _ => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        tauri::PhysicalSize::new(small, small)
    } else {
        match win.outer_size() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "拿不到窗口尺寸，跳过贴边");
                return;
            }
        }
    };
    let Ok(Some(monitor)) = win.current_monitor() else {
        tracing::warn!("拿不到显示器信息，跳过贴边");
        return;
    };
    let screen = monitor.size();
    let pos = monitor.position();
    let scale = monitor.scale_factor();
    let margin = (16.0 * scale) as i32;

    // 水平：靠左还是靠右，看窗口现在离哪边近
    let cur = win.outer_position().ok();
    let go_right = match (mode, cur) {
        (SnapMode::BottomRight, _) => true,
        (SnapMode::NearestHorizontal, Some(p)) => {
            let win_center = p.x + (size.width as i32) / 2;
            let screen_center = pos.x + (screen.width as i32) / 2;
            win_center > screen_center
        }
        (SnapMode::NearestHorizontal, None) => true,
    };

    let x = if go_right {
        pos.x + screen.width as i32 - size.width as i32 - margin
    } else {
        pos.x + margin
    };
    // 垂直：贴底（避开底部任务栏，多留 48px）
    let y = pos.y + screen.height as i32 - size.height as i32 - margin - (48.0 * scale) as i32;

    let target = tauri::PhysicalPosition::new(x.max(pos.x), y.max(pos.y));
    if let Err(e) = win.set_position(target) {
        tracing::warn!(error = %e, "贴边失败（Wayland 下正常，需 XWayland）");
    } else {
        // 读回校验一次：位置设置是异步的，可能还没生效
        if let Ok(p) = win.outer_position() {
            if p.x != target.x || p.y != target.y {
                std::thread::sleep(std::time::Duration::from_millis(60));
                let _ = win.set_position(target);
            }
        }
        set_orb_home(x, y);
        ORB_READY.store(true, std::sync::atomic::Ordering::Relaxed);
        tracing::info!(x, y, right = go_right, "小球已贴边");
    }
}

/// 建系统托盘图标（用户要求「支持跨平台任务栏图标」）。
///
/// 这是 V1 最严重的体验问题：窗口无边框 + 始终置顶 + 任务栏不显示，
/// 用户启动后既关不掉也找不到，只能 pkill。
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示小球", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏小球", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出蝶梦", true, None::<&str>)?;

    // ── 切换 agent（语音切换暂不可用时的兜底入口）──
    //
    // 为什么需要它：真机实测「切换到 codex」这类语音命令，
    // 语音模型从不调 set_context 工具（它以为说一句话就够了），
    // 我们在宿主层也拿不到用户原话（协议没开 ASR 转写）。
    //
    // 所以先用菜单兜底 —— 至少能用，只是需要点一下。
    // 等语音方案验证可行再替换。详见 docs/v2/voice-switch-design.md
    let agent_dsh = MenuItem::with_id(app, "agent-dsh", "用 dsh 干活", true, None::<&str>)?;
    let agent_codex = MenuItem::with_id(app, "agent-codex", "用 codex 干活", true, None::<&str>)?;
    let switch_menu = Submenu::with_items(app, "切换 agent", true, &[&agent_dsh, &agent_codex])?;

    let menu = Menu::with_items(app, &[&show, &hide, &switch_menu, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .tooltip("蝶梦 · 语音智能体")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("orb") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "hide" => {
                if let Some(w) = app.get_webview_window("orb") {
                    let _ = w.hide();
                }
            }
            "agent-dsh" => {
                tracing::info!("托盘菜单：切到 dsh");
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let st = app.state::<AppState>();
                    match st
                        .dispatcher
                        .dispatch("set_context", r#"{"agent":"dsh"}"#)
                        .await
                    {
                        Ok(r) => tracing::info!("切换结果: {}", r.speech),
                        Err(e) => tracing::warn!(error = %e, "切换失败"),
                    }
                });
            }
            "agent-codex" => {
                tracing::info!("托盘菜单：切到 codex");
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let st = app.state::<AppState>();
                    match st
                        .dispatcher
                        .dispatch("set_context", r#"{"agent":"codex"}"#)
                        .await
                    {
                        Ok(r) => tracing::info!("切换结果: {}", r.speech),
                        Err(e) => tracing::warn!(error = %e, "切换失败"),
                    }
                });
            }
            "quit" => {
                tracing::info!("托盘菜单：退出");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击 = 显示/隐藏切换（符合常见托盘交互习惯）
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("orb") {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.hide();
                    } else {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            }
        });

    // 托盘图标：用打包进去的 32x32 png
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    } else {
        tracing::warn!("没有默认窗口图标，托盘可能显示为空白");
    }

    builder.build(app)?;
    tracing::info!("系统托盘已就绪（左键显示/隐藏，右键菜单）");
    Ok(())
}

/// 小球窗口的基准尺寸（一个边长，正方形）。见 `docs/v2/float-window.md`。
///
/// 为什么要小：窗口的**透明区域照样吃鼠标事件**。
/// 之前是 360×360 而球只占 122px，等于在桌面上盖了一张 360×360 的隐形膜，
/// 挡住下面的窗口点不到。360 悬浮窗那种紧凑小球才是对的。
const ORB_SIZE: u32 = 148;
/// 显示气泡（字幕）时窗口向右扩出去的宽度。
///
/// 展开时窗口向左长（x 减小），球本身在屏幕上**位置不变** ——
/// 所以前端必须把画布钉在窗口右下角，这里只负责把窗口撑开。
const TIP_WIDTH: u32 = 360;

/// 在 Wayland 会话里强制 GTK 走 XWayland。
///
/// 为什么必须：Wayland 协议不给客户端全局坐标，所以
/// `set_position`（贴右下角）和 `alwaysOnTop`（置顶）在纯 Wayland 下**无效** —
/// 实测窗口停在屏幕正中、`_NET_WM_STATE` 查不到。
/// 走 XWayland 后 `_NET_WM_STATE_ABOVE` 真的出现，定位也生效。
///
/// 安全条件：只有同时存在 `WAYLAND_DISPLAY` 和 `DISPLAY`（说明有 XWayland）才切，
/// 纯 Wayland 无 XWayland 的机器保持原样（至少能启动）。
/// 想手动覆盖就设 `GDK_BACKEND` 或 `MORPHOFLY_NO_X11=1`。
fn force_x11_backend() {
    if std::env::var_os("GDK_BACKEND").is_some() || std::env::var_os("MORPHOFLY_NO_X11").is_some() {
        return;
    }
    let has_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let has_x11 = std::env::var_os("DISPLAY").is_some();
    if has_wayland && has_x11 {
        std::env::set_var("GDK_BACKEND", "x11");
        tracing::info!("Wayland 会话：强制 GDK_BACKEND=x11（否则无法定位/置顶）");
    }
}

fn main() {
    // ★ 必须最先执行：GTK 在第一次连显示服务器时就定后端，晚了没用。
    force_x11_backend();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,diemeng=debug".into()),
        )
        .init();

    let cfg = std::sync::Arc::new(Config::load().unwrap_or_default());
    tracing::info!(
        provider = %cfg.realtime.provider,
        model = %cfg.realtime.model,
        key = %cfg.masked_key(),
        "蝶梦启动"
    );
    let _ = morphofly_core::config::ensure_data_dir(&cfg);

    let machine = Arc::new(Mutex::new(SessionMachine::new(Duration::from_millis(
        cfg.idle_timeout_ms,
    ))));

    let demo_audio = std::env::var("MORPHOFLY_DEMO_AUDIO")
        .map(|v| v != "0")
        .unwrap_or(true);
    // 唤醒词常驻监听的启动挪到 .setup() 里 —— 必须在 MORPHOFLY_BUNDLE_DIR 设好之后，
    // 否则 AppImage 场景下会去 ~/.diemeng 找模型（找不到就静默降级）。
    let enable_kws = std::env::var("MORPHOFLY_KWS")
        .map(|v| v != "0")
        .unwrap_or(true);
    let skip_kws = demo_states_placeholder();

    // 自动点击（仅 QA：绕开 X11 点击不达 WebView 的问题，直接驱动状态机）
    if std::env::var("MORPHOFLY_AUTOCLICK")
        .map(|v| v != "0")
        .unwrap_or(false)
    {
        let m = machine.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let mut g = m.lock().await;
            match g.handle(Stimulus::Click) {
                Ok(_) => tracing::info!(state = g.state().as_str(), "自动点击 → 唤醒"),
                Err(e) => tracing::warn!(error = %e, "自动点击失败"),
            }
        });
    }

    // 视觉巡检（仅 QA 用，见 spawn_state_walker 文档）
    let demo_states = std::env::var("MORPHOFLY_DEMO_STATES")
        .map(|v| v != "0")
        .unwrap_or(false);
    if demo_states {
        spawn_state_walker(machine.clone());
    }

    // 全局 Dispatcher —— 让托盘菜单等非语音入口也能调工具。
    //
    // 注意：VoiceSession 内部还会建自己的 Dispatcher（历史结构）。
    // 两者共享同一个 TaskStore 和 DshBridge，所以状态是一致的。
    let store = std::sync::Arc::new(Mutex::new(
        morphofly_core::tasks::TaskStore::open(cfg.task_store_path()).expect("打开任务表"),
    ));
    let bridge = std::sync::Arc::new(morphofly_core::dsh::DshBridge::from_config(&cfg));
    let dispatcher = std::sync::Arc::new(morphofly_core::dispatch::Dispatcher::new(
        store,
        bridge,
        cfg.dsh.default_cwd.clone(),
    ));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            machine: machine.clone(),
            caption: Arc::new(Mutex::new(String::new())),
            dispatcher,
            cfg: cfg.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            orb_click,
            orb_state,
            orb_tip,
            set_caption,
            poke,
            snap_orb
        ])
        .setup(move |app| {
            // ★ 让 core 能找到随包资源（唤醒词模型 / VAD 模型 / 关键词表）。
            //
            // AppImage 是自包含的，用户拿到就该能用，不该先去 GitHub 下模型。
            // Tauri 的 resource_dir() 在 Linux 上把 bundle.resources 放到
            // /usr/lib/<product>/ （deb）或 AppDir/usr/lib/ （AppImage），
            // 我们打的目录名是 resources/，所以拼上去。
            if let Ok(rd) = app.path().resource_dir() {
                let bundle = rd.join("resources");
                let dir = if bundle.is_dir() { bundle } else { rd.clone() };
                std::env::set_var("MORPHOFLY_BUNDLE_DIR", &dir);
                tracing::info!(dir = %dir.display(), "随包资源目录");
            }

            // ★ 必须在 MORPHOFLY_BUNDLE_DIR 设好之后再启动唤醒词
            if enable_kws && !skip_kws {
                spawn_wake_listener(machine.clone());
            }

            let handle = app.handle().clone();
            spawn_pump(handle.clone(), machine.clone(), demo_audio);
            spawn_session_guard(handle, machine.clone(), cfg.clone());
            // ★ 系统托盘（用户要求「支持跨平台任务栏图标」）
            // V1 最严重的体验问题：无边框+置顶+任务栏不显示 = 关不掉也找不到
            if let Err(e) = setup_tray(app) {
                tracing::error!(error = %e, "创建系统托盘失败——用户将无法通过托盘退出");
            }

            if let Some(win) = app.get_webview_window("orb") {
                // ★ 启动顺序很讲究，2026-10-24 在 gpd 上踩了两次：
                //
                // 1. 窗口还没映射（visible:false）时 `outer_size()` 返回 0，
                //    贴边公式就用 0 当尺寸算，x/y 直接飞到屏幕外
                //    （实测 x=3056 y=1664，右边缘 3204 > 屏宽 3072）。
                // 2. 未映射时 `set_position` 被 KWin 忽略，窗口最后还是弹在屏幕正中。
                //
                // 所以：先 show() → 等尺寸变非 0 → 显式设一次尺寸 → 再贴边。
                let w2 = win.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    if let Err(e) = w2.show() {
                        tracing::warn!(error = %e, "显示小球失败");
                    }
                    // 等窗口真的映射出来（最多 1.5 秒）
                    for _ in 0..30 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        if let Ok(s) = w2.outer_size() {
                            if s.width > 0 && s.height > 0 {
                                break;
                            }
                        }
                    }
                    // 显式定尺寸：WebView 内容可能把窗口顶大（旧前端 360 布局就顶到 200），
                    // 不能指望配置文件里的 width/height 就是最终值。
                    let scale = w2.scale_factor().unwrap_or(1.0);
                    let px = (f64::from(ORB_SIZE) * scale).round() as u32;
                    if let Err(e) = w2.set_size(tauri::PhysicalSize::new(px, px)) {
                        tracing::warn!(error = %e, "设置小球尺寸失败");
                    }
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    place_orb_bottom_right(&w2);
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
