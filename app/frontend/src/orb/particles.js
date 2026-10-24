// 盘内粒子（球壳上的浮尘）。
//
// 视觉铁律：单一色相；发光靠叠加；没有硬边。
// 这是 148×148 窗口里直径约 84px 的球：130 颗 1–2px 的小点，
// 比 720 颗亚像素点既省 GPU（N6000 集显要 60fps）又更像浮尘。

export const PARTICLE_COUNT = 130;

const TAU = Math.PI * 2;

export class Particles {
  constructor(count = PARTICLE_COUNT) {
    this.count = count;
    // 球内均匀分布（r = cbrt(u)）
    this.angles = new Float32Array(count);
    this.baseRadii = new Float32Array(count);
    this.radii = new Float32Array(count);
    this.speeds = new Float32Array(count);
    this.seeds = new Float32Array(count);
    this.depths = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      this.angles[i] = Math.random() * TAU;
      this.baseRadii[i] = Math.cbrt(Math.random());
      this.radii[i] = this.baseRadii[i];
      this.speeds[i] = 0.35 + Math.random() * 0.9;
      this.seeds[i] = Math.random() * TAU;
      // z 从 -1（背后）到 +1（面前），决定大小和亮度
      this.depths[i] = Math.random() * 2 - 1;
    }
    this.time = 0;
  }

  /**
   * @param {number} dt 秒
   * @param {number} R 球半径（像素）
   * @param {number} energy 0..1 音量能量
   * @param {object} vis 来自 StateSmoother 的视觉参数
   * @param {number[]} bins 64 个频谱值
   */
  step(dt, R, energy, vis, bins) {
    this.time += dt * vis.speed;
    const outward = vis.speed > 0.9 && vis.glow > 0.8; // 说话态外扩
    const spread = vis.spread ?? 1;
    const e = Math.max(0, Math.min(1, energy || 0));
    const nBins = bins?.length || 0;

    for (let i = 0; i < this.count; i++) {
      // 角速度：越外层越慢（体积感）
      const layer = 0.35 + this.baseRadii[i] * 0.9;
      this.angles[i] += (dt * vis.speed * this.speeds[i] * 0.55) / layer;

      // 深度循环：粒子在球面上由前到后地流动
      this.depths[i] = Math.sin(this.time * 0.7 * this.speeds[i] + this.seeds[i]);

      // 频谱驱动：把 64 bin 映射到粒子的径向带
      const band = nBins ? Math.min(nBins - 1, (this.baseRadii[i] * nBins) | 0) : 0;
      const bandVal = nBins ? (bins[band] || 0) : 0;

      // spread <1 收紧到内核，>1 铺开成球壳 —— 状态差异的主要来源之一
      let target = this.baseRadii[i] * (0.92 + e * 0.30 + bandVal * 0.30) * spread;
      if (outward) target *= 1.0 + 0.28 * Math.sin(this.time * 2.2 + this.seeds[i]);
      // 呼吸（待机时也能看出活着）
      target *= 1 + 0.035 * Math.sin(this.time * 1.1 + this.seeds[i]);
      // 别跑出圆盘
      target = Math.min(target, 0.94);

      this.radii[i] += (target - this.radii[i]) * Math.min(1, dt * 6);
    }
  }

  /**
   * 画点：单色相、小圆点、边缘衰减。
   * @param {number} dpr 设备像素比（让点的视觉大小不随屏幕缩放变化）
   */
  draw(ctx, cx, cy, R, vis, alphaMul = 1, dpr = 1) {
    const { hue, sat, glow } = vis;
    const satPct = Math.round((sat ?? 0.8) * 100);
    const gain = 0.14 + Math.max(0, glow) * 1.0;
    const baseR = Math.max(1.15 * dpr, R * 0.020);
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';

    for (let i = 0; i < this.count; i++) {
      const z = this.depths[i];            // -1..1
      const front = (z + 1) / 2;           // 0..1
      const rr = this.radii[i];
      // 边缘衰减：越靠外越淡，避免硬边
      const edge = Math.max(0, 1 - rr * 0.78);
      const alpha = (0.05 + front * 0.40) * gain * edge * alphaMul;
      if (alpha <= 0.004) continue;

      const x = cx + Math.cos(this.angles[i]) * rr * R;
      const y = cy + Math.sin(this.angles[i]) * rr * R;
      const size = Math.max(1.15 * dpr, baseR * (0.75 + front * 0.85));

      ctx.beginPath();
      ctx.fillStyle = `hsla(${hue}, ${satPct}%, ${78 + front * 14}%, ${alpha})`;
      ctx.arc(x, y, size, 0, TAU);
      ctx.fill();
    }

    ctx.restore();
  }
}
