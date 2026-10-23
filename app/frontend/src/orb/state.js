// 六态 → 视觉参数。与 core/src/session/machine.rs 的 OrbState::visual() 保持一致。
export const STATES = {
  S0: { glow: 0.18, radius: 0.82, speed: 0.25, label: '待机', hue: 190, sat: 0.55 },
  S1: { glow: 0.85, radius: 1.12, speed: 1.20, label: '在',   hue: 190, sat: 0.90 },
  S2: { glow: 0.62, radius: 1.00, speed: 0.75, label: '听你说', hue: 188, sat: 0.80 },
  S3: { glow: 0.70, radius: 0.92, speed: 1.60, label: '想一下', hue: 200, sat: 0.70 },
  S4: { glow: 0.92, radius: 1.15, speed: 1.05, label: '说话中', hue: 186, sat: 0.95 },
  S5: { glow: 0.10, radius: 0.70, speed: 0.10, label: '连不上', hue: 210, sat: 0.25 },
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
    for (const key of ['glow', 'radius', 'speed', 'hue', 'sat']) {
      this.current[key] += (this.target[key] - this.current[key]) * k;
    }
    return this.current;
  }
}
