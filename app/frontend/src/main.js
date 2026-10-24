// 前端入口：画球、收频谱、把点击告诉 Rust；没有 Tauri 时进演示模式。
// 永远不会调用 getUserMedia（铁律：WebView 永不碰麦克风）。
//
// URL 参数（无头截图/演示）：
//   ?state=S2  强制状态（忽略 Rust 推来的 state）
//   ?demo=1    强制假频谱数据
//   ?debug=1   显示状态调试文字（正常使用不显示，避免 148px 窗口里出现字）
//   ?hover=1   强制悬停态
import { OrbRenderer } from './orb/renderer.js';
import { StateSmoother, HoverEase, STATES } from './orb/state.js';
import { parseOrbParams } from './orb/params.js';

const params = parseOrbParams(window.location.search);
const forcedState = params.state;

const canvas = document.getElementById('orb');
const captionEl = document.getElementById('caption');
const hintEl = document.getElementById('hint');

const renderer = new OrbRenderer(canvas);
// 强制状态时直接落在目标态（截图要稳定画面，不要从 S0 过渡过去）
const smoother = new StateSmoother(forcedState || 'S0');
const hover = new HoverEase();
if (params.hover) {
  hover.value = 1;
  hover.target = 1;
}

let bins = new Array(64).fill(0);
let energy = 0;
let targetEnergy = 0;
let last = performance.now();
let captionTimer = null;

// ---- 与 Rust 的桥 ----
// Tauri 2 注入 window.__TAURI__（withGlobalTauri）或用 @tauri-apps/api。
// 这里不引入 npm 依赖：优先用注入的全局对象，没有就退化为「演示模式」。
const tauri = window.__TAURI__ || null;
const demoStream = params.demo || !tauri;

// ── 字幕气泡：窗口展开/收起 ──
// 148×148 放不下字幕。有字幕时调 Rust 的 orb_tip(show:true) 把窗口向左撑到
// 360×148（右边缘钉住，球在屏幕上不动），左边 212px 就是气泡区；4.2s 后收回。
// 演示模式没有 Rust 桥：不调 orb_tip，只在 ?caption= 造截图时切 CSS 布局。
const canExpand = !!tauri || params.caption != null;
let tipOpen = false;

function setTip(open) {
  if (!canExpand || tipOpen === open) return;
  tipOpen = open;
  document.body.classList.toggle('tip-open', open);
  if (tauri) ipc('orb_tip', { show: open }).catch(() => {});
}

function ipc(cmd, args) {
  if (tauri?.core?.invoke) return tauri.core.invoke(cmd, args);
  if (tauri?.invoke) return tauri.invoke(cmd, args);
  return Promise.resolve(null);
}

function listen(event, handler) {
  if (tauri?.event?.listen) return tauri.event.listen(event, (e) => handler(e.payload));
  if (tauri?.eventListen) return tauri.eventListen(event, handler);
  return Promise.resolve(() => {});
}

function setStateBadge(state) {
  // 给自动化/调试用的 DOM 钩子；正常界面里不显示文字
  document.documentElement.dataset.state = state;
  if (params.debug) {
    hintEl.textContent = `${state} · ${STATES[state]?.label || ''}`;
    hintEl.classList.add('show');
  }
}

setStateBadge(smoother.name);

if (!tauri && params.debug) {
  hintEl.textContent = '演示模式';
  hintEl.classList.add('show');
}

async function boot() {
  await listen('spectrum', (payload) => {
    // ?demo=1 时强制用假数据，真实事件不覆盖
    if (params.demo) return;
    if (Array.isArray(payload?.bins)) bins = payload.bins;
    if (typeof payload?.rms === 'string' || typeof payload?.rms === 'number') {
      targetEnergy = Number(payload.rms) || 0;
    }
  });

  await listen('state', (payload) => {
    if (payload?.state) {
      if (!forcedState) smoother.set(payload.state);
      setStateBadge(smoother.name);
    }
    if (payload?.caption) showCaption(payload.caption);
  });

  await listen('caption', (payload) => {
    if (payload?.text) showCaption(payload.text);
  });

  // 首帧问一次当前状态，避免和 Rust 不同步
  try {
    const s = await ipc('orb_state', {});
    if (s?.state && !forcedState) {
      smoother.set(s.state);
      setStateBadge(s.state);
    }
    if (s?.caption) showCaption(s.caption);
  } catch { /* 演示模式下没有后端 */ }
}

function showCaption(text) {
  captionEl.textContent = text;
  captionEl.classList.add('show');
  setTip(true);
  clearTimeout(captionTimer);
  captionTimer = setTimeout(() => {
    captionEl.classList.remove('show');
    setTip(false);
  }, 4200);
}

// ---- 交互 ----
// 点击小球：告诉 Rust「点了一下」；演示模式下本地轮转状态方便看视觉。
let suppressClickUntil = 0;
canvas.addEventListener('click', async () => {
  if (Date.now() < suppressClickUntil) return;
  hintEl.classList.remove('show');
  await ipc('orb_click', {});
  if (!tauri && !forcedState) {
    const order = ['S0', 'S1', 'S2', 'S3', 'S4', 'S5'];
    const next = order[(order.indexOf(smoother.name) + 1) % order.length];
    smoother.set(next);
    setStateBadge(next);
    showCaption(STATES[next]?.label || '');
  }
});

window.addEventListener('keydown', (e) => {
  const map = { 1: 'S0', 2: 'S1', 3: 'S2', 4: 'S3', 5: 'S4', 6: 'S5' };
  if (map[e.key] && !forcedState) {
    smoother.set(map[e.key]);
    setStateBadge(map[e.key]);
  }
});

// ---- 悬停：放大 1.06、环变亮 ----
// 命中用「到窗口中心的距离」，比整窗悬停更像悬浮球。
function updateHover(e) {
  if (params.hover) return;
  if (e.pointerType === 'touch') return;
  // 用画布的实际位置算距离：窗口展开成 360×148 后球贴在右边，窗口中心不是球心
  const rect = canvas.getBoundingClientRect();
  const dx = e.clientX - (rect.left + rect.width / 2);
  const dy = e.clientY - (rect.top + rect.height / 2);
  const hit = renderer.geometry?.hitRadius || (renderer.size || 148) * 0.32;
  hover.set(dx * dx + dy * dy <= hit * hit);
}
document.addEventListener('pointermove', updateHover);
document.addEventListener('pointerleave', () => hover.set(false));
window.addEventListener('blur', () => hover.set(false));

// ---- 拖动：移动 >5px 才算拖动；松手贴边（接口 snap_orb 保留）----
let downAt = null;
let dragging = false;

document.addEventListener('pointerdown', (e) => {
  downAt = { x: e.clientX, y: e.clientY };
  dragging = false;
});

document.addEventListener('pointermove', (e) => {
  updateHover(e);
  if (!downAt || dragging) return;
  const dx = Math.abs(e.clientX - downAt.x);
  const dy = Math.abs(e.clientY - downAt.y);
  if (dx > 5 || dy > 5) {
    // 移动超过 5px 才算拖动（否则是点击）
    dragging = true;
    // 交给 Tauri 拖动窗口（capabilities 已允许 core:window:allow-start-dragging）
    try {
      tauri?.window?.getCurrentWindow?.().startDragging?.();
    } catch { /* 没有桥/权限时忽略，拖动退化为无效 */ }
  }
});

function endPointerDrag() {
  if (dragging) {
    dragging = false;
    // 拖动后浏览器可能还会补一个 click，吞掉它，避免误触发「开始说话」
    suppressClickUntil = Date.now() + 300;
    ipc('snap_orb').catch(() => {});
  }
  downAt = null;
}
document.addEventListener('pointerup', endPointerDrag);
document.addEventListener('pointercancel', endPointerDrag);
window.addEventListener('blur', endPointerDrag);

// ---- 渲染循环 ----
function fillDemoAudio(now) {
  const t = now / 1000;
  const st = smoother.name;
  const active = st === 'S2' || st === 'S4';
  if (active) {
    // 假频谱：确定性正弦，截图/录屏稳定，不用随机数
    targetEnergy = 0.32 + 0.22 * Math.sin(t * 2.3) + 0.12 * Math.sin(t * 5.7);
    for (let i = 0; i < 64; i++) {
      const slow = 0.5 + 0.5 * Math.sin(t * 1.9 + i * 0.27);
      const fast = 0.5 + 0.5 * Math.sin(t * 6.1 + i * 0.11);
      bins[i] = Math.max(0, slow * 0.55 + fast * 0.30) * (st === 'S4' ? 1.1 : 0.8);
    }
  } else {
    targetEnergy = 0.04;
    for (let i = 0; i < 64; i++) bins[i] = 0.02 + 0.01 * Math.sin(t * 0.8 + i);
  }
}

function frame(now) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;

  if (demoStream) fillDemoAudio(now);

  // 能量平滑（攻击快、释放慢，像真的听感）
  const k = targetEnergy > energy ? 0.35 : 0.12;
  energy += (targetEnergy - energy) * k;

  const vis = smoother.update(dt);
  const hv = hover.update(dt);
  renderer.render(dt, vis, energy, bins, smoother.weights, hv);
  requestAnimationFrame(frame);
}

boot().finally(() => {
  // ?caption=… 直接造一条字幕（截图核对气泡用）
  if (params.caption) showCaption(params.caption);
  requestAnimationFrame(frame);
});
