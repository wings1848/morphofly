// 前端入口：只做三件事 —— 画球、收频谱、把点击告诉 Rust。
// 永远不会调用 getUserMedia（铁律：WebView 永不碰麦克风）。
import { OrbRenderer } from './orb/renderer.js';
import { StateSmoother, STATES } from './orb/state.js';

const canvas = document.getElementById('orb');
const captionEl = document.getElementById('caption');
const hintEl = document.getElementById('hint');

const renderer = new OrbRenderer(canvas);
const smoother = new StateSmoother('S0');

let bins = new Array(64).fill(0);
let energy = 0;
let targetEnergy = 0;
let last = performance.now();
let captionTimer = null;

// ---- 与 Rust 的桥 ----
// Tauri 2 注入 window.__TAURI__（withGlobalTauri）或用 @tauri-apps/api。
// 这里不引入 npm 依赖：优先用注入的全局对象，没有就退化为「演示模式」。
const tauri = window.__TAURI__ || null;

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

async function boot() {
  await listen('spectrum', (payload) => {
    if (Array.isArray(payload?.bins)) bins = payload.bins;
    if (typeof payload?.rms === 'string' || typeof payload?.rms === 'number') {
      targetEnergy = Number(payload.rms) || 0;
    }
  });

  await listen('state', (payload) => {
    if (payload?.state) smoother.set(payload.state);
    if (payload?.caption) showCaption(payload.caption);
  });

  await listen('caption', (payload) => {
    if (payload?.text) showCaption(payload.text);
  });

  // 首帧问一次当前状态，避免和 Rust 不同步
  try {
    const s = await ipc('orb_state', {});
    if (s?.state) smoother.set(s.state);
    if (s?.caption) showCaption(s.caption);
  } catch { /* 演示模式下没有后端 */ }
}

function showCaption(text) {
  captionEl.textContent = text;
  captionEl.classList.add('show');
  clearTimeout(captionTimer);
  captionTimer = setTimeout(() => captionEl.classList.remove('show'), 4200);
}

// ---- 交互 ----
canvas.addEventListener('click', async () => {
  hintEl.classList.add('hide');
  await ipc('orb_click', {});
  // 演示模式（没有 Rust 后端）下本地轮转状态，方便单独看视觉
  if (!tauri) {
    const order = ['S0', 'S1', 'S2', 'S3', 'S4', 'S5'];
    const next = order[(order.indexOf(smoother.name) + 1) % order.length];
    smoother.set(next);
    showCaption(STATES[next]?.label || '');
  }
});

window.addEventListener('keydown', (e) => {
  if (e.key === '1') smoother.set('S0');
  if (e.key === '2') smoother.set('S1');
  if (e.key === '3') smoother.set('S2');
  if (e.key === '4') smoother.set('S3');
  if (e.key === '5') smoother.set('S4');
  if (e.key === '6') smoother.set('S5');
  if (e.key === 'd') document.body.classList.toggle('demo-audio');
});

// ---- 渲染循环 ----
function frame(now) {
  const dt = Math.min(0.05, (now - last) / 1000);
  last = now;

  // 演示模式：没有后端时用正弦波假装音量，方便检查动效
  if (!tauri && smoother.name !== 'S0') {
    const t = now / 1000;
    targetEnergy = 0.35 + 0.3 * Math.sin(t * 3.1) + 0.15 * Math.sin(t * 7.7);
    for (let i = 0; i < 64; i++) {
      const v = 0.5 + 0.5 * Math.sin(t * 2.4 + i * 0.31);
      bins[i] = Math.max(0, v) * (0.35 + 0.65 * Math.abs(Math.sin(t * 1.7 + i * 0.11)));
    }
  }

  // 能量平滑（攻击快、释放慢，像真的听感）
  const k = targetEnergy > energy ? 0.35 : 0.12;
  energy += (targetEnergy - energy) * k;

  const vis = smoother.update(dt);
  renderer.render(dt, vis, energy, bins);
  requestAnimationFrame(frame);
}

boot().finally(() => requestAnimationFrame(frame));
