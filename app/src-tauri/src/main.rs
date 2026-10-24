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
    // 拖动循环可能还在跑（松手事件比它晚到），先让它停，否则贴完边又被搬回去
    DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    snap_to_edge(&window, SnapMode::NearestHorizontal);
    Ok(())
}

// ────────────────────────────── 拖动 ──────────────────────────────
//
// 真机实测（gpd，KWin/XWayland）：前端直接调 `startDragging()` 窗口一动不动，
// 用户反馈「悬浮球拖不动」。所以拖动改成自己搬窗口：
//   orb_drag_begin  开始拖动（Linux 起一个后台循环，每 16ms 读全局指针搬窗口）
//   orb_drag_end    结束拖动（前端松手/失焦时调）
//   orb_metrics     窗口物理尺寸，前端用来算 CSS→物理像素比
// 证据与复现步骤见 docs/v2/orb-drag.md。

/// 手动拖动是否进行中（true 时后台循环跟着指针搬窗口）
static DRAG_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 本次拖动的校准参数：(抓取偏移 x, 抓取偏移 y, 指针单位→物理像素比例)
/// `orb_drag_end` 要用它把最后一帧补上（合成的快速拖动可能一帧都不给机会采样）
static DRAG_CAL: std::sync::OnceLock<std::sync::Mutex<Option<(f64, f64, f64)>>> =
    std::sync::OnceLock::new();

fn drag_cal() -> &'static std::sync::Mutex<Option<(f64, f64, f64)>> {
    DRAG_CAL.get_or_init(|| std::sync::Mutex::new(None))
}

/// 把「指针读数」换成窗口应该去的位置（物理像素）
fn drag_target(
    win: &WebviewWindow,
    cursor: tauri::PhysicalPosition<f64>,
    k: f64,
    ox: f64,
    oy: f64,
) -> tauri::PhysicalPosition<i32> {
    clamp_to_monitor(
        win,
        (cursor.x * k - ox).round() as i32,
        (cursor.y * k - oy).round() as i32,
    )
}

/// 把窗口位置夹到当前显示器范围内，别把球拖丢（拖出屏幕就再也点不到了）
fn clamp_to_monitor(win: &WebviewWindow, x: i32, y: i32) -> tauri::PhysicalPosition<i32> {
    let mut tx = x;
    let mut ty = y;
    if let Ok(Some(mon)) = win.current_monitor() {
        let mpos = mon.position();
        let msize = mon.size();
        let size = win
            .outer_size()
            .unwrap_or(tauri::PhysicalSize::new(ORB_SIZE, ORB_SIZE));
        let max_x = (mpos.x + msize.width as i32 - size.width as i32).max(mpos.x);
        let max_y = (mpos.y + msize.height as i32 - size.height as i32).max(mpos.y);
        tx = tx.clamp(mpos.x, max_x);
        ty = ty.clamp(mpos.y, max_y);
    }
    tauri::PhysicalPosition::new(tx, ty)
}

/// 窗口物理尺寸 + 缩放（前端拿它算 CSS→物理像素比）
#[tauri::command]
fn orb_metrics(window: WebviewWindow) -> serde_json::Value {
    let inner = window.inner_size();
    let outer = window.outer_size();
    let cursor = window.cursor_position().ok();
    serde_json::json!({
        "inner_width": inner.as_ref().map(|s| s.width).unwrap_or(0),
        "inner_height": inner.as_ref().map(|s| s.height).unwrap_or(0),
        "outer_width": outer.as_ref().map(|s| s.width).unwrap_or(0),
        "outer_height": outer.as_ref().map(|s| s.height).unwrap_or(0),
        "scale": window.scale_factor().unwrap_or(1.0),
        "cursor_x": cursor.map(|c| c.x),
        "cursor_y": cursor.map(|c| c.y),
    })
}

/// 前端日志写进 journal。
///
/// 为什么需要：窗口透明无边框、没开 devtools，前端出错在真机上完全看不见。
/// 拖动这类「点了没反应」的问题，没有这行日志只能瞎猜。
#[tauri::command]
fn orb_log(msg: String) {
    tracing::info!(target: "morphofly_app::js", js = %msg, "前端日志");
}

/// 前端首帧画完后调用：告诉 Rust「可以把小球窗口显形了」。
///
/// 为什么需要它：Linux 下窗口先设成全透明再 show（见 setup 里的启动顺序），
/// WebKit 画完第一帧之前后端无从得知；由前端在渲染循环跑过一帧后发信号最可靠。
#[tauri::command]
fn orb_ready() {
    FRONTEND_READY.store(true, std::sync::atomic::Ordering::Relaxed);
    tracing::info!("前端首帧已画（通知 Rust 显示窗口）");
}

/// 拖动开始：Linux 用手动伺服（后台每 16ms 读一次全局指针，把窗口摆在「指针 − 抓取偏移」）。
///
/// 为什么不用系统原生拖动（`start_dragging()`）：2026-10-25 真机实测，
/// gpd 的 KWin/XWayland 上请求发出去了窗口一动不动（采样 8 秒位置零变化），
/// 而且合成器抓一次指针会把 pointerup 吞掉。见 docs/v2/orb-drag.md。
///
/// 为什么改成「Rust 后台循环读指针」而不是「前端 pointermove 算位移」：
/// 球只有 148×148，指针一动就出画面，前端收不到后续 pointermove，拖动立刻断；
/// 全局指针位置不受窗口大小限制，拖多快都跟得上。
#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
async fn orb_drag_begin(
    window: WebviewWindow,
    client_x: Option<f64>,
    client_y: Option<f64>,
    ratio: Option<f64>,
) -> Result<serde_json::Value, String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;

    // Windows/macOS 上原生拖动是好的，继续用
    if !cfg!(target_os = "linux") {
        match window.start_dragging() {
            Ok(()) => {
                tracing::info!(x = pos.x, y = pos.y, "拖动开始：用系统原生拖动");
                return Ok(serde_json::json!({ "native": true, "x": pos.x, "y": pos.y }));
            }
            Err(e) => tracing::warn!(error = %e, "原生拖动失败，改手动"),
        }
    }

    // 抓取偏移 = 按下时指针在窗口内的物理像素位置。
    // 直接由前端给的 CSS 坐标 × 像素比算出（精确），不依赖全局指针坐标系。
    let ratio = ratio.filter(|r| *r > 0.1).unwrap_or(1.0);
    let half = f64::from(ORB_SIZE) / 2.0;
    let offset_phys = match (client_x, client_y) {
        (Some(cx), Some(cy)) if cx.is_finite() && cy.is_finite() => (cx * ratio, cy * ratio),
        _ => (half, half),
    };

    // 指针坐标系：`cursor_position()` 在 tao 里已经由 GDK 的 logical 换成
    // physical（X11 上实测返回的就是物理像素，读数 2982 == 屏幕物理坐标），
    // 所以这里固定 k=1，不再「反推比例」。
    //
    // 为什么不反推：拖动开始时指针可能已经离开按下点（第一个 pointermove 才触发
    // 拖动），反推出来的只是噪声。2026-10-25 真机踩过：噪声正好落在 [0.5,2] 里
    // 被当成 0.76，结果拖到底只走到一半。宁可固定 1，也不要随机偏差。
    let cursor = window.cursor_position().ok();
    let k = 1.0;
    tracing::info!(
        win_x = pos.x,
        win_y = pos.y,
        cursor_x = ?cursor.map(|c| c.x),
        cursor_y = ?cursor.map(|c| c.y),
        offset_x = offset_phys.0,
        offset_y = offset_phys.1,
        k,
        "拖动开始：后台跟指针搬窗口"
    );

    if let Ok(mut g) = drag_cal().lock() {
        *g = Some((offset_phys.0, offset_phys.1, k));
    }
    DRAG_ACTIVE.store(true, std::sync::atomic::Ordering::Relaxed);
    let win = window.clone();
    tauri::async_runtime::spawn(async move {
        let mut last: Option<(i32, i32)> = None;
        let mut idle = 0u32;
        while DRAG_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
            match win.cursor_position() {
                Ok(c) => {
                    let target = drag_target(&win, c, k, offset_phys.0, offset_phys.1);
                    if last != Some((target.x, target.y)) {
                        last = Some((target.x, target.y));
                        idle = 0;
                        if DRAG_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
                            let _ = win.set_position(target);
                        }
                    } else {
                        // 指针 640ms 没动 = 用户松手了（pointerup 没收到时的兜底）
                        idle += 1;
                        if idle > 40 {
                            break;
                        }
                    }
                }
                Err(_) => {
                    idle += 1;
                    if idle > 40 {
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Ok(p) = win.outer_position() {
            tracing::info!(x = p.x, y = p.y, "拖动结束（后台循环退出）");
        }
    });

    Ok(serde_json::json!({ "native": false, "x": pos.x, "y": pos.y }))
}

/// 拖动结束：前端 pointerup / pointercancel / 失焦时调用。
///
/// 除了停掉后台循环，这里还会**按当前指针再摆一次窗口**：合成/脚本产生的快速
/// 拖动可能比 16ms 采样还快，松手那一帧没被采到就会停在半路（真机实测过一次，
/// 拖到底只走到一半）。补这一帧之后，自动验收的位置判定才是确定的。
#[tauri::command]
async fn orb_drag_end(window: WebviewWindow) -> serde_json::Value {
    DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
    let cal = drag_cal().lock().ok().and_then(|g| *g);
    if let (Some((ox, oy, k)), Ok(c)) = (cal, window.cursor_position()) {
        let target = drag_target(&window, c, k, ox, oy);
        let _ = window.set_position(target);
        tracing::info!(x = target.x, y = target.y, "拖动结束：按最后一帧指针补位");
    }
    serde_json::json!({ "ok": true })
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
        // ★ 没显式设过字幕就返回空串，别拿状态名顶替。
        //   2026-10-25 真机踩坑：这里原来回退成 `m.state().label()`（"待机"），
        //   前端启动时把它当字幕弹出来 → 窗口被向左撑开、黑色气泡在球旁边停 4.2 秒，
        //   就是用户报的「KDE 启动弹黑框、字和球叠一会儿」。
        "caption": caption,
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
        // 50ms（20fps）足够顺，之前 33ms 在 N6000 上纯属浪费
        let mut ticker = tokio::time::interval(Duration::from_millis(50));
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

            // 空闲态不需要频谱（S0 的呼吸动画不吃 bins），直接跳过，省 IPC 和 CPU
            let st_now = { machine.lock().await.state() };
            if !demo_audio && matches!(st_now, OrbState::S0 | OrbState::S1) {
                continue;
            }

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
    store: Arc<Mutex<morphofly_core::tasks::store::TaskStore>>,
    dispatcher: Arc<morphofly_core::dispatch::Dispatcher>,
) {
    tauri::async_runtime::spawn(async move {
        let mut current: Option<voice::VoiceSession> = None;
        // 主动汇报开窗的冷却：失败时别每 250ms 试一次
        let mut last_proactive: Option<std::time::Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let st = { machine.lock().await.state() };

            // ★ 任务干完了但会话已经关了 → 主动开一轮把它念出来。
            //
            // 真机踩坑（2026-10-25）：DSH 干一个任务要 1–3 分钟，
            // 而语音会话 30 秒空闲就回 S0、云端 90 秒没操作直接断。
            // 于是「主动汇报」这个核心承诺根本兑现不了 ——
            // 日志里看到的是一次成功的派活 + 一条 `too long without operation`。
            //
            // 做法：S0 且有未念过的已完成任务 → 模拟一次唤醒，让守卫去建会话，
            // 新建的会话里 report 循环会立刻发现它并开口。
            if st == morphofly_core::session::OrbState::S0 {
                let pending = {
                    let s = store.lock().await;
                    s.unreported_finished().len()
                };
                let cooled = last_proactive
                    .map(|t| t.elapsed() >= Duration::from_secs(15))
                    .unwrap_or(true);
                if pending > 0 && cooled {
                    tracing::info!(pending, "有任务结果还没念，主动开一轮汇报");
                    last_proactive = Some(std::time::Instant::now());
                    let mut m = machine.lock().await;
                    let _ = m.handle(Stimulus::ProactiveReport);
                }
            }

            // 需要建会话？（S1 且还没有）
            if voice::should_open_session(st) && current.is_none() {
                tracing::info!("建立语音会话…");
                match voice::VoiceSession::open(&cfg, store.clone(), dispatcher.clone()).await {
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
                            // ★ 会话结束了就把状态机拉回待机。
                            // 否则会停在 S2/S3/S4，而「主动汇报」的守卫只在 S0 才检查 ——
                            // 结果就是 DSH 干完了、任务表里 reported 一直 false、
                            // 日志里连痕迹都没有（真机踩过，查了很久）。
                            {
                                let mut m = machine3.lock().await;
                                m.force_idle();
                            }
                            tracing::debug!("会话结束，状态机已复位到待机");
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

/// 前端首帧画完了吗？窗口启动时先全透明，等这个标记置位才恢复可见。
///
/// 为什么由前端发信号：窗口映射后 WebKit 才提交第一帧，后端无法直接知道画没画完。
/// 前端在启动渲染循环跑过一帧之后再调 `orb_ready`。
static FRONTEND_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

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
    // 垂直：
    //  - 启动归位（BottomRight）贴底，避开底部任务栏，多留 48px
    //  - 用户拖动松手（NearestHorizontal）只吸水平边，垂直保持用户放的位置。
    //    以前这里无脑贴底，用户把球拖到屏幕上方松手，球又弹回底部，
    //    看起来就像「拖不动」——2026-10-25 真机反馈。
    let bottom_y =
        pos.y + screen.height as i32 - size.height as i32 - margin - (48.0 * scale) as i32;
    let max_y = (pos.y + screen.height as i32 - size.height as i32).max(pos.y);
    let y = match mode {
        SnapMode::BottomRight => bottom_y,
        SnapMode::NearestHorizontal => match cur {
            Some(p) => p.y.clamp(pos.y, max_y),
            None => bottom_y,
        },
    };
    tracing::info!(
        from = ?cur.map(|p| (p.x, p.y)),
        to_x = x,
        to_y = y,
        mode = ?mode,
        "贴边计算"
    );

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

/// 进程启动时刻：日志里算「首帧就绪」耗时用（验收 T3 直接 grep elapsed_ms）。
static PROCESS_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// 启动时等前端首帧的上限。超时也会显示窗口，避免前端坏了导致永远不可见。
const STARTUP_READY_TIMEOUT_MS: u64 = 3000;

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
#[cfg(target_os = "linux")]
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

/// Windows / macOS 不需要这层：它们的窗口管理器本来就允许客户端定位和置顶。
#[cfg(not(target_os = "linux"))]
fn force_x11_backend() {}

/// 设置整个窗口的不透明度（0.0–1.0）。Linux 走 GTK，其它平台没有对应 API、返回 false。
///
/// 为什么需要：`transparent: true` 只保证「页面背景透明」，页面还没画出来时
/// 窗口在 KDE 下仍会先显示成空的黑框。启动时先设 0，等前端首帧画完再设 1，
/// 用户就完全看不到那个中间态。
///
/// 注意：`gtk_window()` 只能在主线程调（Tauri 的 Linux 限制），调用点都在主线程。
#[cfg(target_os = "linux")]
fn linux_set_window_opacity(win: &WebviewWindow, opacity: f64) -> bool {
    use gtk::prelude::WidgetExt;
    match win.gtk_window() {
        Ok(gwin) => {
            gwin.set_opacity(opacity);
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, opacity, "设置 GTK 窗口不透明度失败");
            false
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)] // 只有 Linux 启动流程会调它，其它平台留这个空实现是为了 cfg 代码统一
fn linux_set_window_opacity(_win: &WebviewWindow, _opacity: f64) -> bool {
    false
}

fn main() {
    // 启动时刻（验收会 grep 「启动首帧就绪 ... elapsed_ms」判断有没有变慢）
    PROCESS_START.get_or_init(Instant::now);

    // ★ 必须最先执行：GTK 在第一次连显示服务器时就定后端，晚了没用。
    force_x11_backend();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,diemeng=debug".into()),
        )
        // journal / 管道里不要 ANSI 颜色码。
        //
        // 真机踩坑（2026-10-25）：默认 `.init()` 会写颜色码，
        // 于是 `state="S4"` 在 journal 里变成 `\x1b[3mstate\x1b[0m\x1b[2m=\x1b[0m"S4"`，
        // 脚本 `grep 'state="S4"'` 永远匹配不到（打断了自动化验收，查了一阵）。
        // 只有真的在终端里跑时才上色。
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stdout()))
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

    // ★ 默认关。2026-10-25 真机实测：开着演示频谱时空闲 CPU 77%
    // （tokio worker 28% 在每秒 30 次「合成 512 点 PCM + FFT + IPC 发给前端」），
    // 而 WebKit 那边的渲染进程反而 0% —— 白烧 CPU，风扇一直转。
    // 只有做视觉演示/截图时才需要它：MORPHOFLY_DEMO_AUDIO=1。
    let demo_audio = std::env::var("MORPHOFLY_DEMO_AUDIO")
        .map(|v| v != "0")
        .unwrap_or(false);
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
    // 上次进程留下的「正在干」任务永远不会有结果（结果只能由还在跑的那个进程
    // 写回来），启动时自愈成失败 + 未汇报，用户下次会话就能听到「那个活中断了」。
    let mut task_store =
        morphofly_core::tasks::TaskStore::open(cfg.task_store_path()).expect("打开任务表");
    let recovered = task_store.recover_interrupted();
    if !recovered.is_empty() {
        tracing::warn!(
            tasks = ?recovered,
            "上次没跑完的任务已标记为中断（下次会话会主动汇报）"
        );
    }
    let store = std::sync::Arc::new(Mutex::new(task_store));
    let bridge = std::sync::Arc::new(morphofly_core::dsh::DshBridge::from_config(&cfg));
    let store_for_guard = store.clone();
    let dispatcher = std::sync::Arc::new(morphofly_core::dispatch::Dispatcher::new(
        store,
        bridge,
        cfg.dsh.default_cwd.clone(),
    ));
    // 守卫和 AppState 用同一个 dispatcher / 同一个 TaskStore：
    // 2026-10-25 真机踩坑 —— 会话里各自 new 一个 store 实例，
    // 结果「主动汇报」守卫看不到会话派出去的任务，DSH 干完了也不汇报。
    let dispatcher_for_guard = dispatcher.clone();

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
            snap_orb,
            orb_log,
            orb_ready,
            orb_metrics,
            orb_drag_begin,
            orb_drag_end
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
            spawn_session_guard(
                handle,
                machine.clone(),
                cfg.clone(),
                store_for_guard.clone(),
                dispatcher_for_guard.clone(),
            );
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
                //
                // ★ 2026-10-25 加第三件事：show() 之前先把窗口设成全透明（Linux/GTK），
                //   等前端首帧画完再恢复不透明度。这样用户看到的第一帧就是画好的球，
                //   不会先看到一个空的黑框/黑底窗口（KDE 下的启动闪黑框就是这么来的）。
                #[cfg(target_os = "linux")]
                if linux_set_window_opacity(&win, 0.0) {
                    tracing::info!("启动：小球窗口先全透明（等前端首帧画完再显示）");
                } else {
                    tracing::warn!("启动：设置窗口透明失败，启动瞬间可能闪一下");
                }

                let w2 = win.clone();
                let app_handle = app.handle().clone();
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

                    // ── 等前端首帧画完再显形 ──
                    // 前端在渲染循环跑过一帧后调 orb_ready。这里最多等 3 秒：
                    // 超时也要显示（前端挂了不能让用户看不到球），并在日志里留痕（验收 T3 会查）。
                    let wait_start = Instant::now();
                    let mut frontend_ready = false;
                    while wait_start.elapsed() < Duration::from_millis(STARTUP_READY_TIMEOUT_MS) {
                        if FRONTEND_READY.load(std::sync::atomic::Ordering::Relaxed) {
                            frontend_ready = true;
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    if !frontend_ready {
                        tracing::warn!(
                            timeout_ms = STARTUP_READY_TIMEOUT_MS,
                            "启动：等前端首帧超时，先显示窗口（可能闪一下）"
                        );
                    }
                    // GTK 调用只能在主线程，挪回去再恢复不透明度
                    let w3 = w2.clone();
                    if let Err(e) = app_handle.run_on_main_thread(move || {
                        #[cfg(target_os = "linux")]
                        {
                            if !linux_set_window_opacity(&w3, 1.0) {
                                tracing::warn!("启动：恢复窗口不透明度失败");
                            }
                        }
                        #[cfg(not(target_os = "linux"))]
                        let _ = &w3; // 非 Linux 不设不透明度，避免 unused 警告
                        let elapsed_ms = PROCESS_START
                            .get()
                            .map(|t| t.elapsed().as_millis())
                            .unwrap_or(0);
                        tracing::info!(
                            elapsed_ms = elapsed_ms,
                            frontend_ready,
                            "启动首帧就绪：小球可见"
                        );
                    }) {
                        tracing::warn!(error = %e, "切回主线程显示小球失败");
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}
