// 悬浮小球的几何：窗口是 148×148，球主体直径约 84px（≈窗口 57%，验收要求 ~60%）。
//
// 只依赖比例，窗口 128–176 之间变化时球跟着缩放，不会顶到边也不会变成小点。
// 纯函数，方便单测；不碰 DOM。

/** 球半径 / 窗口边长。0.285 → 148 窗口下直径 84.4px。 */
export const BALL_RATIO = 0.285;

/**
 * @param {number} size 窗口边长（CSS px）
 * @returns {{size:number, radius:number, diameter:number, haloRadius:number,
 *            spectrumRadius:number, hitRadius:number}}
 */
export function orbGeometry(size) {
  // 极窄/极宽窗口都兜住（正常是 148）
  const s = Number.isFinite(size) && size > 0 ? size : 148;
  const radius = s * BALL_RATIO;
  return {
    size: s,
    radius,
    diameter: radius * 2,
    // 外发光最大到 1.68R，148 窗口下 ≈71px，正好在画布内（半窗 74px）
    haloRadius: radius * 1.68,
    // 频谱环的起始半径
    spectrumRadius: radius * 1.055,
    // 悬停命中半径：比球略大一点，鼠标擦边也算悬停
    hitRadius: radius * 1.18,
  };
}
