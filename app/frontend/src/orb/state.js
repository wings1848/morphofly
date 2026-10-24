// 六态 → 视觉参数 + 平滑混合。
//
// 视觉铁律（docs/UI设计提示词.md）：
//   - 单一色相：HUE 固定 188（≈ #22D3EE hsl(187, 85%, 53%)），只靠明度/饱和度分层。
//   - 安静、无硬边、发光靠多层叠加。
//
// 六态至少要在两个维度上明显不同，肉眼才能一眼区分（验收 A4）。
// 状态切换不能硬切：渲染层拿的是「六态权重」，所有视觉参数都是六态参数的
// 加权和；环形态（呼吸/涟漪/频谱/虚线/断续）也按同一组权重交叉淡入淡出。

export const HUE = 188;

export const STATES = {
  // 待机：暗、慢呼吸（4s 周期）、环只留一圈很淡的边。不抢注意力。
  S0: {
    glow: 0.16, radius: 1.00, speed: 0.22, spread: 0.82,
    ringGain: 0.30, pulse: 0.00, sat: 0.62, core: 0.30,
    jitter: 0, dash: 0, label: '待机',
  },
  // 唤醒：环全亮 + 向外扩散的涟漪，明确「我在」。
  S1: {
    glow: 0.95, radius: 1.10, speed: 1.55, spread: 1.02,
    ringGain: 1.00, pulse: 0.90, sat: 0.95, core: 1.00,
    jitter: 0, dash: 0, label: '在',
  },
  // 聆听：频谱环张开，粒子铺在球壳上（像竖起耳朵）。
  S2: {
    glow: 0.60, radius: 1.05, speed: 0.55, spread: 1.08,
    ringGain: 0.72, pulse: 0.10, sat: 0.88, core: 0.72,
    jitter: 0, dash: 0, label: '听你说',
  },
  // 思考：收紧、高速内旋 + 环上旋转虚线弧。
  S3: {
    glow: 0.52, radius: 0.93, speed: 2.60, spread: 0.60,
    ringGain: 0.28, pulse: 0.00, sat: 0.70, core: 0.58,
    jitter: 0, dash: 1, label: '想一下',
  },
  // 说话：最亮最大，脉动 + 频谱。
  S4: {
    glow: 1.00, radius: 1.12, speed: 1.00, spread: 1.16,
    ringGain: 1.00, pulse: 0.65, sat: 1.00, core: 1.05,
    jitter: 0, dash: 0, label: '说话中',
  },
  // 异常：几乎熄灭、去饱和、环断成几段并轻微抖动。
  S5: {
    glow: 0.10, radius: 0.87, speed: 0.10, spread: 0.44,
    ringGain: 0.08, pulse: 0.00, sat: 0.20, core: 0.16,
    jitter: 1, dash: 0, label: '连不上',
  },
};

export const STATE_ORDER = ['S0', 'S1', 'S2', 'S3', 'S4', 'S5'];

/** 参与平滑混合的数值参数。label 是字符串，单独处理。 */
export const BLEND_KEYS = [
  'glow', 'radius', 'speed', 'spread', 'ringGain', 'pulse',
  'sat', 'core', 'jitter', 'dash',
];

/** 按权重把六态参数混合成一份当前视觉参数（权重和应为 1）。 */
export function blendStates(weights, keys = BLEND_KEYS) {
  const out = {};
  for (const key of keys) {
    let v = 0;
    for (const name of STATE_ORDER) {
      v += (weights[name] || 0) * (STATES[name][key] ?? 0);
    }
    out[key] = v;
  }
  out.hue = HUE; // 单一色相，不参与混合
  return out;
}

/**
 * 目标态 → 当前态的平滑逼近。
 *
 * 内部维护六个状态的权重（始终和为 1），每帧做指数逼近；
 * `current` 是权重加权后的参数。这样状态切换是「淡入淡出」而不是硬切，
 * 也让渲染层能直接拿到 weights 去做环形态的交叉过渡。
 */
export class StateSmoother {
  constructor(initial = 'S0') {
    const name = STATES[initial] ? initial : 'S0';
    this.name = name;
    this.target = STATES[name];
    this.weights = {};
    for (const n of STATE_ORDER) this.weights[n] = n === name ? 1 : 0;
    this.current = blendStates(this.weights);
  }

  set(name) {
    if (!STATES[name]) return;
    this.name = name;
    this.target = STATES[name];
  }

  update(dt) {
    // 时间常数 ~180ms：状态切换 300ms 左右肉眼完成，不拖沓也不突兀
    const k = 1 - Math.exp(-Math.max(0, dt) / 0.18);
    for (const n of STATE_ORDER) {
      const goal = n === this.name ? 1 : 0;
      this.weights[n] += (goal - this.weights[n]) * k;
    }
    this.current = blendStates(this.weights);
    return this.current;
  }
}

// ── 悬停 ──
// 鼠标移上来：放大 6%、环变亮。也用指数逼近（~100ms），避免指针来回时闪烁。

export const HOVER = {
  scale: 1.06,      // 球体放大倍数
  ringGain: 1.45,   // 描边环亮度倍数
  glow: 0.10,       // 外发光额外增量
};

export class HoverEase {
  constructor() {
    this.value = 0; // 0 = 没悬停，1 = 完全悬停
    this.target = 0;
  }

  set(on) {
    this.target = on ? 1 : 0;
  }

  update(dt) {
    const k = 1 - Math.exp(-Math.max(0, dt) / 0.10);
    this.value += (this.target - this.value) * k;
    return this.value;
  }
}

/** 当前视觉参数 + 悬停进度 → 实际放大倍数与环亮度（纯函数，方便单测）。 */
export function hoverVisual(vis, h) {
  const t = Math.max(0, Math.min(1, h || 0));
  return {
    scale: 1 + (HOVER.scale - 1) * t,
    ringGain: (vis?.ringGain ?? 0) * (1 + (HOVER.ringGain - 1) * t),
    glow: (vis?.glow ?? 0) + HOVER.glow * t,
  };
}
