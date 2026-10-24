// 悬浮小球渲染（Canvas2D，148×148 画布上直径约 84px）。
//
// 结构从下到上：
//   1. 外发光：3 层很淡的径向渐变叠加（不用大 shadowBlur，N6000 集显要 60fps）
//   2. 深色玻璃圆盘
//   3. 盘内光核 + 盘内粒子
//   4. 边缘辉光 + 内缘反光 + 左上高光（玻璃质感）
//   5. 青色描边环（画在盘内侧 0.965R，避免被白底/桌面吃掉）+ 顶部亮弧
//   6. 按状态交叉淡入的环形态：
//        S0 4s 呼吸 / S1 扩散涟漪 / S2·S4 频谱环 / S3 旋转虚线弧 / S5 断续抖动环
//
// 铁律：单一色相 HUE=188；没有大面积 blur；所有形态切换走状态权重，不硬切。

import { Particles } from './particles.js';
import { orbGeometry } from './layout.js';
import { HUE, hoverVisual } from './state.js';

const TAU = Math.PI * 2;

const clamp01 = (v) => (v < 0 ? 0 : v > 1 ? 1 : v);

/** hsla 字符串：色相锁死，只调饱和度/明度/透明度 */
function hsla(hue, sat, light, alpha) {
  return `hsla(${hue}, ${Math.round(Math.max(0, Math.min(1, sat)) * 100)}%, ${light}%, ${Math.max(0, Math.min(1, alpha)).toFixed(3)})`;
}

export class OrbRenderer {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: true });
    this.particles = new Particles();
    this.time = 0;
    this.resize();
    window.addEventListener('resize', () => this.resize());
  }

  /** 画布跟着窗口走；dpr 封顶 2，避免 2.5/3 倍屏白烧像素 */
  resize() {
    const size = Math.max(96, Math.min(window.innerWidth || 148, window.innerHeight || 148));
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    this.size = size;
    this.dpr = dpr;
    const px = Math.round(size * dpr);
    if (this.canvas.width !== px) {
      this.canvas.width = px;
      this.canvas.height = px;
    }
    this.canvas.style.width = `${size}px`;
    this.canvas.style.height = `${size}px`;
    this.geometry = orbGeometry(size);
  }

  /**
   * @param {number} dt 秒
   * @param {object} vis 平滑后的视觉参数（state.js 的 current）
   * @param {number} energy 0..1 音量
   * @param {number[]} bins 64 bin 频谱
   * @param {object} weights 六态权重（用于环形态交叉淡入）
   * @param {number} hover 0..1 悬停进度
   */
  render(dt, vis, energy, bins, weights, hover) {
    this.time += dt;
    const t = this.time;
    const ctx = this.ctx;
    const dpr = this.dpr;
    const W = this.canvas.width;
    const H = this.canvas.height;
    const geo = this.geometry || orbGeometry(this.size);
    const w = weights || { S0: 1 };
    const hv = hoverVisual(vis, hover);
    const e = clamp01(energy || 0);
    const glow = clamp01(hv.glow);
    const sat = clamp01(vis.sat ?? 0.8);
    const ringGain = Math.max(0, hv.ringGain ?? 0.3);
    const core = clamp01(vis.core ?? 0.4);
    const hov = clamp01(hover || 0);

    // 缓慢呼吸（所有状态都有，很轻；S0 的环还会额外呼吸）
    const breath = 1 + 0.008 * Math.sin((t * TAU) / 4);
    const R = geo.radius * dpr * (vis.radius ?? 1) * hv.scale * breath;

    // S5 故障抖动：整个球轻微位移（权重驱动，切出 S5 时自然消失）
    const jw = clamp01(w.S5 ?? 0);
    const jx = jw * 0.7 * dpr * Math.sin(t * 17.3);
    const jy = jw * 0.7 * dpr * Math.cos(t * 13.1);
    const cx = W / 2 + jx;
    const cy = H / 2 + jy;

    ctx.clearRect(0, 0, W, H);

    // ── 1. 外发光：三层叠加，边缘衰减到 0（最大 1.68R ≈ 71px，不出画布）──
    const haloBoost = (0.20 + 0.95 * glow) * (1 + 0.25 * hov);
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    for (const [mul, a] of [[1.68, 0.055], [1.40, 0.065], [1.16, 0.085]]) {
      const g = ctx.createRadialGradient(cx, cy, R * 0.42, cx, cy, R * mul);
      const al = a * haloBoost;
      g.addColorStop(0, hsla(HUE, 0.92, 58, al));
      g.addColorStop(0.55, hsla(HUE, 0.92, 46, al * 0.45));
      g.addColorStop(1, hsla(HUE, 0.92, 40, 0));
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(cx, cy, R * mul, 0, TAU);
      ctx.fill();
    }
    ctx.restore();

    // ── 2. 深色玻璃圆盘（近黑带蓝，不是纯黑，桌面透一点冷色）──
    const disc = ctx.createRadialGradient(cx - R * 0.20, cy - R * 0.24, R * 0.05, cx, cy, R);
    disc.addColorStop(0, 'rgba(12, 27, 42, 0.96)');
    disc.addColorStop(0.52, 'rgba(7, 15, 25, 0.97)');
    disc.addColorStop(0.86, 'rgba(4, 9, 17, 0.98)');
    disc.addColorStop(1, 'rgba(3, 6, 12, 0.99)');
    ctx.fillStyle = disc;
    ctx.beginPath();
    ctx.arc(cx, cy, R, 0, TAU);
    ctx.fill();

    // ── 3. 盘内光核：状态决定亮度/大小；S5 会快速闪烁 ──
    const flicker = 1 - 0.35 * jw * (0.5 + 0.5 * Math.sin(t * 23.0));
    const innerR = R * (0.50 + 0.28 * core) * (1 + 0.08 * e);
    const innerA = (0.05 + 0.42 * glow) * (0.45 + 0.50 * core) * flicker;
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    const inner = ctx.createRadialGradient(cx, cy, 0, cx, cy, innerR);
    inner.addColorStop(0, hsla(HUE, Math.max(0.5, sat), 76, innerA));
    inner.addColorStop(0.42, hsla(HUE, sat, 58, innerA * 0.5));
    inner.addColorStop(1, hsla(HUE, sat, 50, 0));
    ctx.fillStyle = inner;
    ctx.beginPath();
    ctx.arc(cx, cy, innerR, 0, TAU);
    ctx.fill();

    // 白心：只占很小一块，别把球冲成白色
    const hotR = R * 0.22;
    const hotA = (0.06 + 0.30 * core) * glow * (0.6 + 0.4 * e) * flicker;
    const hot = ctx.createRadialGradient(cx, cy, 0, cx, cy, hotR);
    hot.addColorStop(0, `rgba(224, 242, 254, ${clamp01(hotA).toFixed(3)})`);
    hot.addColorStop(1, 'rgba(224, 242, 254, 0)');
    ctx.fillStyle = hot;
    ctx.beginPath();
    ctx.arc(cx, cy, hotR, 0, TAU);
    ctx.fill();
    ctx.restore();

    // ── 4. 盘内粒子（裁到圆盘内，跑不出去）──
    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.985, 0, TAU);
    ctx.clip();
    this.particles.step(dt, R, e, vis, bins);
    this.particles.draw(ctx, cx, cy, R, vis, 1, dpr);
    ctx.restore();

    // ── 5. 边缘辉光 + 内缘反光 + 左上高光：玻璃的「厚度」和「反光」──
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';

    // 贴边的一圈光晕（往外衰减）：给描边环垫底，也像 360 悬浮窗的亮边
    const edgeGlow = ctx.createRadialGradient(cx, cy, R * 0.90, cx, cy, R * 1.22);
    edgeGlow.addColorStop(0, hsla(HUE, 0.95, 66, 0.025 + 0.09 * glow + 0.11 * Math.min(1, ringGain / 1.2)));
    edgeGlow.addColorStop(0.55, hsla(HUE, 0.95, 58, 0.015 + 0.05 * glow + 0.05 * Math.min(1, ringGain / 1.2)));
    edgeGlow.addColorStop(1, hsla(HUE, 0.95, 50, 0));
    ctx.fillStyle = edgeGlow;
    ctx.beginPath();
    ctx.arc(cx, cy, R * 1.22, 0, TAU);
    ctx.fill();

    const rim = ctx.createRadialGradient(cx, cy, R * 0.62, cx, cy, R * 1.01);
    rim.addColorStop(0, hsla(HUE, 0.8, 62, 0));
    rim.addColorStop(0.72, hsla(HUE, 0.9, 64, 0.03 + 0.07 * Math.min(1, ringGain)));
    rim.addColorStop(1, hsla(HUE, 1, 74, 0.08 + 0.18 * Math.min(1, ringGain)));
    ctx.fillStyle = rim;
    ctx.beginPath();
    ctx.arc(cx, cy, R * 1.01, 0, TAU);
    ctx.fill();

    // 左上高光（漫反射玻璃反光）
    const sx = cx - R * 0.34;
    const sy = cy - R * 0.38;
    const spec = ctx.createRadialGradient(sx, sy, 0, sx, sy, R * 0.58);
    spec.addColorStop(0, 'rgba(224, 242, 254, 0.17)');
    spec.addColorStop(0.45, 'rgba(224, 242, 254, 0.06)');
    spec.addColorStop(1, 'rgba(224, 242, 254, 0)');
    ctx.fillStyle = spec;
    ctx.beginPath();
    ctx.arc(sx, sy, R * 0.58, 0, TAU);
    ctx.fill();
    ctx.restore();

    // ── 6. 青色描边环：画在 R*0.965（盘内侧），保证在深色盘上看得见 ──
    const breatheW = clamp01(w.S0 ?? 0);
    // S0：4s 周期呼吸，环亮度的 35% 跟着起伏
    const ringBreath = 1 - 0.35 * breatheW * (0.5 + 0.5 * Math.sin((t * TAU) / 4));
    const ringA = (0.16 + 0.42 * ringGain) * ringBreath;
    const rimR = R * 0.965;
    const rimW = Math.max(1.6 * dpr, R * 0.042);

    ctx.save();
    ctx.lineCap = 'round';
    ctx.globalCompositeOperation = 'lighter';

    if (jw > 0.01) {
      // S5：断续环（4 段，缺口大），带一点角度抖动；去饱和、偏灰
      const rot = 0.06 * Math.sin(t * 7.3) * jw;
      ctx.strokeStyle = hsla(HUE, Math.max(0.30, sat), 74, 0.20 + 0.45 * jw);
      ctx.lineWidth = rimW;
      for (let k = 0; k < 4; k++) {
        const a0 = rot + k * (TAU / 4) + 0.16;
        ctx.beginPath();
        ctx.arc(cx, cy, rimR, a0, a0 + 0.72);
        ctx.stroke();
      }
    }

    // 底盘环（S5 时按权重变淡，让断续环接管）
    ctx.strokeStyle = hsla(HUE, 0.92, 76, ringA * (1 - 0.85 * jw));
    ctx.lineWidth = rimW;
    ctx.beginPath();
    ctx.arc(cx, cy, rimR, 0, TAU);
    ctx.stroke();

    // 外侧细环：给一点空气感
    ctx.strokeStyle = hsla(HUE, 0.95, 72, ringA * 0.30);
    ctx.lineWidth = Math.max(1 * dpr, rimW * 0.55);
    ctx.beginPath();
    ctx.arc(cx, cy, rimR + Math.max(1.6 * dpr, R * 0.055), 0, TAU);
    ctx.stroke();

    // 亮弧：左上方到右上方，模拟环境光
    ctx.strokeStyle = hsla(HUE, 1.0, 86, Math.min(0.95, ringA * 1.55));
    ctx.lineWidth = Math.max(1.2 * dpr, rimW * 0.85);
    ctx.beginPath();
    ctx.arc(cx, cy, rimR, -2.55, 0.55);
    ctx.stroke();

    // 玻璃球顶部的内高光弧：让盘看起来是球，不是平圆
    ctx.strokeStyle = `rgba(224, 242, 254, ${(0.10 + 0.10 * Math.min(1, ringGain)).toFixed(3)})`;
    ctx.lineWidth = Math.max(1 * dpr, R * 0.028);
    ctx.beginPath();
    ctx.arc(cx, cy, R * 0.80, -2.45, -1.10);
    ctx.stroke();
    ctx.restore();

    // ── 7. 状态环形态（权重交叉淡入）──
    this.drawSpectrum(ctx, cx, cy, R, e, bins, w, ringGain, t);
    this.drawRipples(ctx, cx, cy, R, t, w, ringGain);
    this.drawThinkArc(ctx, cx, cy, R, t, w, ringGain, rimW);
  }

  /** S2/S4：环外 64 根短弧，跟着频谱呼吸 */
  drawSpectrum(ctx, cx, cy, R, e, bins, w, ringGain, t) {
    const specW = clamp01((w.S2 ?? 0) * 0.95 + (w.S4 ?? 0) * 1.15);
    const n = bins?.length || 0;
    if (specW < 0.01 || !n) return;
    const r0 = R * 1.02;
    const maxLen = R * (0.31 + 0.07 * clamp01(w.S4 ?? 0));
    const lineW = Math.max(1.2 * (this.dpr || 1), R * 0.026);

    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.lineCap = 'round';
    ctx.lineWidth = lineW;
    for (let i = 0; i < n; i++) {
      const ang = -Math.PI / 2 + (i / n) * TAU;
      const v = clamp01(bins[i] || 0);
      const len = R * 0.035 + maxLen * v * (0.55 + 0.45 * e);
      const a = (0.16 + 0.80 * v) * specW * (0.5 + 0.5 * ringGain);
      if (a < 0.006) continue;
      const ca = Math.cos(ang);
      const sa = Math.sin(ang);
      ctx.strokeStyle = hsla(HUE, 0.95, 64 + v * 24, a);
      ctx.beginPath();
      ctx.moveTo(cx + ca * r0, cy + sa * r0);
      ctx.lineTo(cx + ca * (r0 + len), cy + sa * (r0 + len));
      ctx.stroke();
    }
    // S4 专属：盘内 32 根短频谱，像声音在球面上震荡
    const talkW = clamp01(w.S4 ?? 0);
    if (talkW > 0.25) {
      const innerR = R * 0.66;
      const innerLen = R * 0.11;
      ctx.lineWidth = Math.max(1 * (this.dpr || 1), R * 0.020);
      for (let i = 0; i < 32; i++) {
        const ang = (i / 32) * TAU + t * 0.35;
        const v = clamp01(bins[(i * 2) % n] || 0);
        const a = (0.10 + 0.35 * v) * talkW;
        if (a < 0.006) continue;
        const ca = Math.cos(ang);
        const sa = Math.sin(ang);
        ctx.strokeStyle = hsla(HUE, 0.95, 72, a);
        ctx.beginPath();
        ctx.moveTo(cx + ca * innerR, cy + sa * innerR);
        ctx.lineTo(cx + ca * (innerR + innerLen * v), cy + sa * (innerR + innerLen * v));
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  /** S1 唤醒涟漪（两圈错开）；S4 说话时也叠一圈随音量起伏的脉动 */
  drawRipples(ctx, cx, cy, R, t, w, ringGain) {
    const wakeW = clamp01(w.S1 ?? 0);
    const talkW = clamp01(w.S4 ?? 0);
    if (wakeW < 0.01 && talkW < 0.01) return;

    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.lineWidth = Math.max(1.6 * (this.dpr || 1), R * 0.034);

    if (wakeW > 0.01) {
      // 唤醒时整颗球「亮起来」：一圈大而柔的光晕（S4 没有）
      const halo = ctx.createRadialGradient(cx, cy, R * 0.92, cx, cy, R * 1.52);
      halo.addColorStop(0, hsla(HUE, 0.95, 66, 0.22 * wakeW));
      halo.addColorStop(0.55, hsla(HUE, 0.95, 58, 0.09 * wakeW));
      halo.addColorStop(1, hsla(HUE, 0.95, 50, 0));
      ctx.fillStyle = halo;
      ctx.beginPath();
      ctx.arc(cx, cy, R * 1.52, 0, TAU);
      ctx.fill();

      const period = 1.8;
      for (let k = 0; k < 3; k++) {
        const p = ((t / period) + k / 3) % 1;
        const rr = R * (1.03 + 0.48 * p);
        const a = (1 - p) ** 2 * 0.80 * wakeW * (0.5 + 0.5 * Math.min(1, ringGain));
        if (a < 0.006) continue;
        ctx.strokeStyle = hsla(HUE, 0.95, 78, a);
        ctx.beginPath();
        ctx.arc(cx, cy, rr, 0, TAU);
        ctx.stroke();
      }
    }

    if (talkW > 0.01) {
      const period = 1.3;
      const p = (t / period) % 1;
      const rr = R * (1.03 + 0.44 * p);
      const a = (1 - p) ** 2 * 0.70 * talkW;
      if (a >= 0.006) {
        ctx.strokeStyle = hsla(HUE, 1, 82, a);
        ctx.beginPath();
        ctx.arc(cx, cy, rr, 0, TAU);
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  /** S3 思考：环外旋转的虚线弧 + 内圈反向短弧 */
  drawThinkArc(ctx, cx, cy, R, t, w, ringGain, rimW) {
    const thinkW = clamp01(w.S3 ?? 0);
    if (thinkW < 0.01) return;
    const dpr = this.dpr || 1;
    const rot = t * 1.5;
    const dashR = R * 1.10;

    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.lineCap = 'round';
    ctx.lineWidth = Math.max(1.6 * dpr, R * 0.034);
    for (let k = 0; k < 3; k++) {
      const a0 = rot + k * (TAU / 3);
      const a = 0.65 * thinkW * (0.55 + 0.45 * ringGain);
      ctx.strokeStyle = hsla(HUE, 0.92, 76, a);
      ctx.beginPath();
      ctx.arc(cx, cy, dashR, a0, a0 + 0.92);
      ctx.stroke();
    }

    // 内圈反向短弧：让「想」有向内收的感觉
    ctx.lineWidth = Math.max(1 * dpr, rimW * 0.7);
    for (let k = 0; k < 2; k++) {
      const a0 = -rot * 1.6 + k * Math.PI;
      ctx.strokeStyle = hsla(HUE, 0.9, 70, 0.36 * thinkW);
      ctx.beginPath();
      ctx.arc(cx, cy, R * 0.82, a0, a0 + 0.55);
      ctx.stroke();
    }
    ctx.restore();
  }
}
