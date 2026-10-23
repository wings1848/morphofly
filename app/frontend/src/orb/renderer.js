// 小球渲染：核心体 + 体积光晕 + 粒子 + 频谱环。
// 铁律：只用一个蓝色色相；发光靠多层叠加（不是大 blur）；边缘衰减到透明。
import { Particles } from './particles.js';

const TAU = Math.PI * 2;

export class OrbRenderer {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext('2d', { alpha: true });
    this.particles = new Particles();
    this.dpr = window.devicePixelRatio || 1;
    this.resize();
    window.addEventListener('resize', () => this.resize());
  }

  resize() {
    const size = Math.min(window.innerWidth, window.innerHeight);
    const dpr = window.devicePixelRatio || 1;
    this.dpr = dpr;
    this.canvas.width = Math.floor(size * dpr);
    this.canvas.height = Math.floor(size * dpr);
    this.canvas.style.width = `${size}px`;
    this.canvas.style.height = `${size}px`;
    this.size = size;
  }

  render(dt, vis, energy, bins) {
    this.time = (this.time || 0) + dt;
    const ctx = this.ctx;
    const { size, dpr } = this;
    const cx = (size * dpr) / 2;
    const cy = (size * dpr) / 2;
    // 球半径占窗口的 ~17%（原来 30% 会让 S4 撑满整屏，失去「深海一点光」的感觉）
    const baseR = (size * dpr) * 0.17 * vis.radius;
    const R = baseR * (1 + energy * 0.14);

    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);

    // 1) 远处体积光：多层同心径向渐变，不是单层大 blur
    // 体积光：多层叠加但每层都很淡；最外层只负责「有一点空气感」，不能铺满窗口
    const halos = [
      { mul: 2.9, alpha: 0.028, stop: 0.12 },
      { mul: 2.1, alpha: 0.048, stop: 0.22 },
      { mul: 1.55, alpha: 0.075, stop: 0.34 },
      { mul: 1.20, alpha: 0.11, stop: 0.48 },
    ];
    for (const h of halos) {
      const g = ctx.createRadialGradient(cx, cy, R * 0.2, cx, cy, R * h.mul);
      // glow 只放大中心，不放大最外层（否则整屏发青）
      const a = h.alpha * (0.25 + vis.glow * 1.15) * (h.mul > 3 ? 0.55 : 1.0);
      const hs = (vis.sat * 100) | 0;
      g.addColorStop(0, `hsla(${vis.hue}, ${hs}%, 54%, ${a})`);
      g.addColorStop(h.stop + 0.35, `hsla(${vis.hue}, ${hs}%, 44%, ${a * 0.45})`);
      g.addColorStop(0.75, `hsla(${vis.hue}, ${hs}%, 34%, ${a * 0.12})`);
      g.addColorStop(1, `hsla(${vis.hue}, ${hs}%, 28%, 0)`);
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(cx, cy, R * h.mul, 0, TAU);
      ctx.fill();
    }

    // 2) 核心体：白心 → 青边，边缘衰减
    const core = ctx.createRadialGradient(cx - R * 0.22, cy - R * 0.26, R * 0.04, cx, cy, R * 1.04);
    const sat = (vis.sat * 100) | 0;
    // 目标色 #22D3EE ≈ hsl(187, 85%, 53%)。核心最亮也只到 ~74%，别冲到白色。
    core.addColorStop(0, `hsla(${vis.hue}, 100%, ${52 + vis.glow * 12}%, ${0.42 + vis.glow * 0.38})`);
    core.addColorStop(0.42, `hsla(${vis.hue}, ${sat}%, 52%, ${0.30 + vis.glow * 0.45})`);
    core.addColorStop(0.78, `hsla(${vis.hue}, ${sat}%, 38%, ${0.12 + vis.glow * 0.26})`);
    core.addColorStop(1, `hsla(${vis.hue}, ${sat}%, 30%, 0)`);
    ctx.fillStyle = core;
    ctx.beginPath();
    ctx.arc(cx, cy, R * 1.04, 0, TAU);
    ctx.fill();

    // 3) 粒子
    this.particles.step(dt, R, energy, vis, bins);
    this.particles.draw(ctx, cx, cy, R, vis);

    // 4) 频谱环：64 根短弧，贴在球外侧
    this.drawSpectrumRing(cx, cy, R * 1.55, bins, vis);
    // 唤醒/说话的脉冲光环（S0/S5 没有）
    if (vis.pulse > 0.05) this.drawPulseRing(cx, cy, R, vis);
  }

  drawPulseRing(cx, cy, R, vis) {
    const ctx = this.ctx;
    const t = (this.time || 0);
    const dur = 2.2;
    const phase = (t % dur) / dur;
    const r = R * (1.35 + phase * 1.8);
    const alpha = (1 - phase) ** 2 * 0.34 * (vis.pulse ?? 0);
    if (alpha <= 0.004) return;
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.beginPath();
    ctx.strokeStyle = `hsla(${vis.hue}, ${(vis.sat * 100) | 0}%, 76%, ${alpha})`;
    ctx.lineWidth = Math.max(1, this.size * this.dpr * 0.0035);
    ctx.arc(cx, cy, r, 0, TAU);
    ctx.stroke();
    ctx.restore();
  }

  drawSpectrumRing(cx, cy, r, bins, vis) {
    const ctx = this.ctx;
    const n = bins.length;
    if (!n) return;
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';
    ctx.lineCap = 'round';
    const sat = (vis.sat * 100) | 0;
    for (let i = 0; i < n; i++) {
      // 只画上半圈 + 左右，留出下半球空白（安静感）
      const t = i / (n - 1);
      const ang = -Math.PI * 0.92 + t * Math.PI * 1.84;
      const v = Math.max(0.02, bins[i] || 0);
      const len = r * 0.06 + r * 0.22 * v;
      const x1 = cx + Math.cos(ang) * r;
      const y1 = cy + Math.sin(ang) * r;
      const x2 = cx + Math.cos(ang) * (r + len);
      const y2 = cy + Math.sin(ang) * (r + len);
      ctx.beginPath();
      const gain = vis.ringGain ?? 1;
      ctx.strokeStyle = `hsla(${vis.hue}, ${sat}%, ${64 + v * 22}%, ${(0.06 + v * 0.55 * vis.glow) * gain})`;
      ctx.lineWidth = Math.max(1, this.size * this.dpr * 0.0022);
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
    }
    ctx.restore();
  }
}
