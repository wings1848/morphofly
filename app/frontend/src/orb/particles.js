// 粒子系统。
// 视觉铁律：只用一个蓝色色相；发光靠多层叠加（不是大 blur）；没有硬边，边缘衰减到透明。
// 实现要点：每个粒子有 3D 感（z 决定大小与亮度），画点不画线——连线会变成乱麻。

export const PARTICLE_COUNT = 720;

const TAU = Math.PI * 2;

export class Particles {
  constructor(count = PARTICLE_COUNT) {
    this.count = count;
    // 球面均匀分布（Fibonacci 之外用随机+归一，够用且便宜）
    this.angles = new Float32Array(count);
    this.baseRadii = new Float32Array(count);
    this.radii = new Float32Array(count);
    this.speeds = new Float32Array(count);
    this.seeds = new Float32Array(count);
    this.depths = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      this.angles[i] = Math.random() * TAU;
      // 球体积均匀：r = cbrt(u)
      this.baseRadii[i] = Math.cbrt(Math.random());
      this.radii[i] = this.baseRadii[i];
      this.speeds[i] = 0.35 + Math.random() * 0.9;
      this.seeds[i] = Math.random() * TAU;
      // z 从 -1（背后）到 +1（面前）
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

    for (let i = 0; i < this.count; i++) {
      // 角速度：越外层越慢（体积感）
      const layer = 0.35 + this.baseRadii[i] * 0.9;
      this.angles[i] += (dt * vis.speed * this.speeds[i] * 0.55) / layer;

      // 深度循环：粒子在球面上由前到后地流动
      this.depths[i] = Math.sin(this.time * 0.7 * this.speeds[i] + this.seeds[i]);

      // 频谱驱动：把 64 bin 映射到粒子的径向带
      const band = Math.min(bins.length - 1, (this.baseRadii[i] * bins.length) | 0);
      const bandVal = bins[band] || 0;

      let target = this.baseRadii[i] * (0.70 + energy * 0.50 + bandVal * 0.40);
      if (outward) target *= 1.0 + 0.30 * Math.sin(this.time * 2.2 + this.seeds[i]);
      // 呼吸（待机时也能看出活着）
      target *= 1 + 0.035 * Math.sin(this.time * 1.1 + this.seeds[i]);

      this.radii[i] += (target - this.radii[i]) * Math.min(1, dt * 6);
    }
  }

  /** 画：两遍叠加（背后暗、前面亮），单色相，边缘自动衰减 */
  draw(ctx, cx, cy, R, vis, alphaMul = 1) {
    const { hue, sat, glow } = vis;
    const satPct = (sat * 100) | 0;
    ctx.save();
    ctx.globalCompositeOperation = 'lighter';

    for (let i = 0; i < this.count; i++) {
      const z = this.depths[i];            // -1..1
      const front = (z + 1) / 2;           // 0..1
      const rr = this.radii[i];
      // 边缘衰减：越靠外越淡，避免硬边
      const edge = Math.max(0, 1 - rr * 0.86);
      const alpha = (0.05 + front * 0.42) * (0.25 + glow) * edge * alphaMul;
      if (alpha <= 0.004) continue;

      const x = cx + Math.cos(this.angles[i]) * rr * R;
      const y = cy + Math.sin(this.angles[i]) * rr * R * 0.94;
      const size = Math.max(0.6, R * (0.006 + front * 0.010));

      ctx.beginPath();
      ctx.fillStyle = `hsla(${hue}, ${satPct}%, ${74 + front * 18}%, ${alpha})`;
      ctx.arc(x, y, size, 0, TAU);
      ctx.fill();
    }

    // 前层再叠一次，做出「体积光」而不是「雾」
    for (let i = 0; i < this.count; i += 3) {
      const z = this.depths[i];
      if (z < 0.35) continue;
      const rr = this.radii[i];
      const x = cx + Math.cos(this.angles[i]) * rr * R;
      const y = cy + Math.sin(this.angles[i]) * rr * R * 0.94;
      ctx.beginPath();
      ctx.fillStyle = `hsla(${hue}, ${Math.min(100, satPct + 8)}%, 88%, ${0.10 * glow})`;
      ctx.arc(x, y, Math.max(0.8, R * 0.012), 0, TAU);
      ctx.fill();
    }
    ctx.restore();
  }
}
