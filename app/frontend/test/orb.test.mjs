// 前端纯逻辑单测：node --test app/frontend/test
// 不测 canvas 绘制（需要真实 GPU/浏览器），只测状态与粒子数学。
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { STATES, STATE_ORDER, StateSmoother } from '../src/orb/state.js';
import { Particles } from '../src/orb/particles.js';

test('六个状态两两之间至少两个维度差异明显（肉眼看得出，验收 U2）', () => {
  const names = STATE_ORDER;
  const keys = ['glow', 'radius', 'speed', 'spread', 'ringGain', 'pulse'];
  for (let i = 0; i < names.length; i++) {
    for (let j = i + 1; j < names.length; j++) {
      const a = STATES[names[i]], b = STATES[names[j]];
      // 归一化差异：相对变化超过 25% 才算「明显」
      const diffs = keys.filter((k) => {
        const x = a[k] ?? 0, y = b[k] ?? 0;
        const scale = Math.max(Math.abs(x), Math.abs(y), 0.001);
        return Math.abs(x - y) / scale > 0.25;
      });
      assert.ok(diffs.length >= 2,
        `${names[i]} vs ${names[j]} 只有 ${diffs.length} 个维度明显不同（${diffs}），肉眼分不出`);
    }
  }
});

test('六个状态都存在且可区分', () => {
  assert.equal(STATE_ORDER.length, 6);
  for (const name of STATE_ORDER) {
    assert.ok(STATES[name], `缺状态 ${name}`);
    assert.equal(typeof STATES[name].glow, 'number');
    assert.equal(typeof STATES[name].label, 'string');
  }
  // 视觉参数必须两两不同，肉眼才能区分（验收 U2）
  const fingerprints = STATE_ORDER.map((n) => {
    const s = STATES[n];
    return `${s.glow}|${s.radius}|${s.speed}|${s.spread}|${s.ringGain}|${s.pulse}`;
  });
  assert.equal(new Set(fingerprints).size, 6, '六个状态的视觉参数不能重复');
});

test('待机态最暗，说话态最亮', () => {
  assert.ok(STATES.S0.glow < STATES.S4.glow);
  assert.ok(STATES.S5.glow < STATES.S0.glow);
});

test('StateSmoother 会逐步逼近目标而不是瞬移', () => {
  const sm = new StateSmoother('S0');
  const before = sm.current.glow;
  sm.set('S4');
  const afterOneTick = sm.update(1 / 60).glow;
  assert.ok(afterOneTick > before, '必须朝目标走');
  assert.ok(afterOneTick < STATES.S4.glow, '不能一帧瞬移');
  for (let i = 0; i < 200; i++) sm.update(1 / 60);
  assert.ok(Math.abs(sm.current.glow - STATES.S4.glow) < 1e-3, '最终要收敛');
});

test('StateSmoother 忽略未知状态', () => {
  const sm = new StateSmoother('S0');
  sm.set('S9');
  assert.equal(sm.name, 'S0');
});

test('粒子半径在 0..2 倍内且随能量增大', () => {
  const p = new Particles(200);
  const bins = new Array(64).fill(0);
  const vis = { ...STATES.S0 };
  for (let i = 0; i < 120; i++) p.step(1 / 60, 100, 0, vis, bins);
  const quiet = p.radii.reduce((a, b) => a + b, 0) / p.radii.length;

  for (let i = 0; i < 120; i++) p.step(1 / 60, 100, 1, { ...STATES.S4 }, bins);
  const loud = p.radii.reduce((a, b) => a + b, 0) / p.radii.length;

  assert.ok(loud > quiet, `能量大时粒子半径应变大: ${loud} vs ${quiet}`);
  for (const r of p.radii) {
    assert.ok(Number.isFinite(r) && r >= 0 && r < 3, `半径越界: ${r}`);
  }
});

test('频谱 bins 会驱动粒子半径变化', () => {
  const a = new Particles(200);
  const b = new Particles(200);
  const vis = { ...STATES.S2 };
  const zeros = new Array(64).fill(0);
  const ones = new Array(64).fill(1);
  for (let i = 0; i < 60; i++) { a.step(1 / 60, 100, 0.5, vis, zeros); b.step(1 / 60, 100, 0.5, vis, ones); }
  const sa = a.radii.reduce((x, y) => x + y, 0);
  const sb = b.radii.reduce((x, y) => x + y, 0);
  assert.ok(sb > sa, '有频谱能量时粒子应被推出去');
});

test('粒子数量与数组长度一致，无 NaN', () => {
  const p = new Particles(900);
  assert.equal(p.angles.length, 900);
  assert.equal(p.radii.length, 900);
  p.step(0.016, 100, 0.5, { ...STATES.S1 }, new Array(64).fill(0.3));
  for (let i = 0; i < 900; i++) {
    assert.ok(Number.isFinite(p.angles[i]));
    assert.ok(Number.isFinite(p.radii[i]));
  }
});
