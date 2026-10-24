// 字幕文本清洗 —— 「启动时黑气泡压在球旁边」这个 bug 的修复点之一。
//
// 背景（2026-10-25 用户反馈）：morphofly 在 Linux KDE 桌面启动时，屏幕会弹出一个
// 黑框 UI，字和悬浮球叠一会儿才恢复正常。
//
// 真因：启动时前端问 Rust 当前状态（orb_state），后端在「没有显式字幕」的时候
// 拿状态名顶替（S0 → "待机"）；前端 boot 时直接 showCaption，于是：
//   1. 黑色字幕气泡弹出来，停在球旁边 4.2 秒；
//   2. 窗口还会被 orb_tip 向左撑到 360 宽，看起来就是「弹出个黑框」。
//
// 现在的约定：只有真实、非空的字幕才显示；空串/纯空白/非字符串都当作「没有字幕」。

/** @param {unknown} text @returns {string} 清洗后的字幕，空串表示「没有字幕」 */
export function normalizeCaption(text) {
  if (typeof text !== 'string') return '';
  return text.trim();
}
