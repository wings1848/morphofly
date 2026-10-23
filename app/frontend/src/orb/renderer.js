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
    const ctx = this.ctx;
    const { size, dpr } = this;
    const cx = (size * dpr) / 2;
    const cy = (size * dpr) / 2;
    const baseR = (size * dpr) * 0.19 * vis.radius;
    const R = baseR * (1 + energy * 0.14);

    ctx.clearRect(0, 0, this.canvas.width, this.canvas.height);

    // 1) 远处体积光：多层同心径向渐变，不是单层大 blur
    const halos = [
      { mul: 4.6, alpha: 0.055, stop: 0.0 },
      { mul: 3.1, alpha: 0.085, stop: 0.12 },
      { mul: 2.1, alpha: 0.14, stop: 0.25 },
    ];
    for (const h of halos) {
      const g = ctx.createRadialGradient(cx, cy, R * 0.2, cx, cy, R * h.mul);
      const a = h.alpha * (0.25 + vis.glow * 1.5);
      g.addColorStop(0, `hsla(${vis.hue}, ${(vis.sat * 100) | 0}%, 62%, ${a})`);
      g.addColorStop(h.stop + 0.45, `hsla(${vis.hue}, ${(vis.sat * 100) | 0}%, 48%, ${a * 0.35})`);
      g.addColorStop(1, `hsla(${vis.hue}, ${(vis.sat * 100) | 0}%, 40%, 0)`);
      ctx.fillStyle = g;
      ctx.beginPath();
      ctx.arc(cx, cy, R * h.mul, 0, TAU);
      ctx.fill();
    }

    // 2) 核心体：白心 → 青边，边缘衰减
    const core = ctx.createRadialGradient(cx - R * 0.22, cy - R * 0.26, R * 0.04, cx, cy, R * 1.04);
    const sat = (vis.sat * 100) | 0;
    core.addColorStop(0, `hsla(${vis.hue}, 96%, ${86 + vis.glow * 10}%, ${0.55 + vis.glow * 0.45})`);
    core.addColorStop(0.42, `hsla(${vis.hue}, ${sat}%, 62%, ${0.30 + vis.glow * 0.45})`);
    core.addColorStop(0.78, `hsla(${vis.hue}, ${sat}%, 46%, ${0.10 + vis.glow * 0.22})`);
    core.addColorStop(1, `hsla(${vis.hue}, ${sat}%, 40%, 0)`);
    ctx.fillStyle = core;
    ctx.beginPath();
    ctx.arc(cx, cy, R * 1.04, 0, TAU);
    ctx.fill();

    // 3) 粒子
    this.particles.step(dt, R, energy, vis, bins);
    this.particles.draw(ctx, cx, cy, R, vis);

    // 4) 频谱环：64 根短弧，贴在球外侧
    this.drawSpectrumRing(cx, cy, R * 1.55, bins, vis);
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
      ctx.strokeStyle = `hsla(${vis.hue}, ${sat}%, ${64 + v * 22}%, ${0.10 + v * 0.55 * vis.glow})`;
      ctx.lineWidth = Math.max(1, this.size * this.dpr * 0.0022);
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
    }
    ctx.restore();
  }
}
