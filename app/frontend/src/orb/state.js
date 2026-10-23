// 六态 → 视觉参数。与 core/src/session/machine.rs 的 OrbState::visual() 保持一致。
// 每个状态在**至少两个维度**上明显不同，肉眼才能区分（验收 U2）。
// 只调亮度不够——实测六态截图几乎一样，必须配合半径、速度、粒子分布形态。
export const STATES = {
  // 待机：暗、小、慢呼吸。不抢注意力（验收 U4）
  S0: { glow: 0.16, radius: 0.62, speed: 0.20, label: '待机', hue: 190, sat: 0.45, spread: 0.78, ringGain: 0.35, pulse: 0.0 },
  // 唤醒：突然变大变亮 + 强脉冲，明确「我在」
  S1: { glow: 1.00, radius: 1.18, speed: 1.45, label: '在', hue: 190, sat: 0.95, spread: 0.98, ringGain: 1.0, pulse: 0.9 },
  // 聆听：中等亮度、粒子铺开在球壳上（像张开耳朵）
  S2: { glow: 0.58, radius: 1.06, speed: 0.55, label: '听你说', hue: 187, sat: 0.85, spread: 1.08, ringGain: 0.75, pulse: 0.15 },
  // 思考：收紧 + 快速内旋（粒子挤到内核，转得快）
  S3: { glow: 0.66, radius: 0.74, speed: 2.40, label: '想一下', hue: 202, sat: 0.70, spread: 0.60, ringGain: 0.25, pulse: 0.35 },
  // 说话：最亮最大，粒子向外扩散
  S4: { glow: 1.05, radius: 1.26, speed: 0.95, label: '说话中', hue: 184, sat: 1.00, spread: 1.14, ringGain: 1.0, pulse: 0.6 },
  // 异常：几乎熄灭、收缩、几乎不动
  S5: { glow: 0.06, radius: 0.45, speed: 0.06, label: '连不上', hue: 214, sat: 0.18, spread: 0.42, ringGain: 0.05, pulse: 0.0 },
};

export const STATE_ORDER = ['S0', 'S1', 'S2', 'S3', 'S4', 'S5'];

/** 目标态 → 当前态的平滑逼近（避免状态切换时画面跳变） */
export class StateSmoother {
  constructor(initial = 'S0') {
    this.target = STATES[initial] || STATES.S0;
    this.current = { ...(STATES[initial] || STATES.S0) };
    this.name = initial;
  }
  set(name) {
    if (!STATES[name]) return;
    this.name = name;
    this.target = STATES[name];
  }
  update(dt) {
    // 指数逼近，时间常数 ~180ms
    const k = 1 - Math.exp(-dt / 0.18);
    for (const key of ['glow', 'radius', 'speed', 'hue', 'sat', 'spread', 'ringGain', 'pulse']) {
      this.current[key] += (this.target[key] - this.current[key]) * k;
    }
    return this.current;
  }
}
