// URL 参数的解析（无头截图 / 演示用）。
//
//   ?state=S2  强制状态，忽略 Rust 推来的 state 事件
//   ?demo=1    强制假频谱数据（即使有 Tauri 桥）
//   ?debug=1   显示调试文字（状态名），正常使用时不显示
//   ?hover=1   强制悬停态（截图核对放大 1.06 / 环变亮用）
//   ?caption=好，查着呢  造一条字幕，并强制展开气泡布局（截图用）
//
// 纯函数，不碰 DOM，Node 里可直接单测。
import { STATE_ORDER } from './state.js';

export function parseOrbParams(search = '') {
  const q = new URLSearchParams(search || '');
  const raw = String(q.get('state') || '').trim().toUpperCase();
  const caption = q.get('caption');
  return {
    // 只接受合法状态名；写错就当作没写，不抛错
    state: STATE_ORDER.includes(raw) ? raw : null,
    demo: q.get('demo') === '1',
    debug: q.get('debug') === '1',
    hover: q.get('hover') === '1',
    caption: caption ? caption.slice(0, 80) : null,
  };
}
