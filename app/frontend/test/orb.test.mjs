// 前端纯逻辑单测：node --test app/frontend/test
// 不测 canvas 绘制（需要真实 GPU/浏览器），只测状态、几何、参数与粒子数学。
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  STATES, STATE_ORDER, StateSmoother, blendStates,
  HOVER, HoverEase, hoverVisual, HUE,
} from '../src/orb/state.js';
import { Particles } from '../src/orb/particles.js';
import { BALL_RATIO, orbGeometry } from '../src/orb/layout.js';
import { parseOrbParams } from '../src/orb/params.js';

test('六个状态两两之间至少两个维度差异明显（肉眼看得出，验收 A4）', () => {
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
  // 视觉参数必须两两不同，肉眼才能区分（验收 A4）
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

// ── 以下为 2026-10-24 悬浮球重构新增 ──

test('球体占窗口约 60%，窗口 128–176 都按比例缩放', () => {
  // 148 窗口：直径 ≈ 84px（验收要求 ~60%）
  const g148 = orbGeometry(148);
  assert.ok(Math.abs(g148.diameter - 84.4) < 1.5, `148 窗口球径应约 84px，实测 ${g148.diameter}`);
  assert.ok(g148.diameter / 148 > 0.52 && g148.diameter / 148 < 0.66);

  for (const size of [128, 148, 176]) {
    const g = orbGeometry(size);
    assert.ok(Math.abs(g.radius / size - BALL_RATIO) < 1e-9, `${size} 窗口比例走样`);
    // 外发光不能超出画布（半窗），否则会被裁掉
    assert.ok(g.haloRadius < size / 2, `${size} 窗口外发光会出界`);
  }

  // 异常输入兜底
  assert.equal(orbGeometry(0).size, 148);
  assert.equal(orbGeometry(NaN).size, 148);
});

test('URL 参数：state/demo/debug/hover，非法 state 当没写', () => {
  const p = parseOrbParams('?state=s2&demo=1&debug=1&hover=1');
  assert.equal(p.state, 'S2', '大小写都要认');
  assert.equal(p.demo, true);
  assert.equal(p.debug, true);
  assert.equal(p.hover, true);

  const none = parseOrbParams('');
  assert.equal(none.state, null);
  assert.equal(none.demo, false);

  assert.equal(parseOrbParams('?state=S9').state, null, '非法状态不能污染渲染');
  assert.equal(parseOrbParams('?state=').state, null);
  assert.equal(parseOrbParams('?demo=0').demo, false);

  // 字幕参数：URL 编码的中文要能解出来；过长截断，不影响渲染
  assert.equal(parseOrbParams('?caption=%E5%A5%BD%EF%BC%8C%E6%9F%A5%E7%9D%80%E5%91%A2').caption, '好，查着呢');
  assert.equal(parseOrbParams('?caption=').caption, null);
  assert.equal(parseOrbParams('?caption=' + 'x'.repeat(200)).caption.length, 80);
});

test('悬停：指数逼近 1.06 倍，环变亮且单调不过冲', () => {
  const h = new HoverEase();
  assert.equal(h.value, 0);
  h.set(true);

  const seen = [];
  for (let i = 0; i < 120; i++) {
    h.update(1 / 60);
    seen.push(h.value);
  }
  assert.ok(seen.every((v) => v >= 0 && v <= 1), '悬停进度必须在 0..1');
  for (let i = 1; i < seen.length; i++) {
    assert.ok(seen[i] >= seen[i - 1], '悬停进度必须单调增（不能过冲回弹）');
  }
  assert.ok(Math.abs(h.value - 1) < 1e-3, '最终要完全悬停');

  const base = { ...STATES.S2 };
  const mid = hoverVisual(base, 0.5);
  const full = hoverVisual(base, 1);
  assert.ok(Math.abs(full.scale - HOVER.scale) < 1e-9, '悬停放大到 1.06');
  assert.ok(mid.scale > 1 && mid.scale < full.scale, '缩放要平滑过渡');
  assert.ok(full.ringGain > base.ringGain * 1.2, '悬停时环必须变亮');
  assert.ok(full.glow > base.glow, '悬停时外发光略增');
  // 松开要能回到原样
  h.set(false);
  for (let i = 0; i < 200; i++) h.update(1 / 60);
  assert.ok(h.value < 1e-3, '移开鼠标要回到 1.0 倍');
  assert.ok(Math.abs(hoverVisual(base, h.value).scale - 1) < 1e-6);
});

test('任意状态切换：权重和为 1、无过冲、误差单调递减（缓动不能硬切）', () => {
  for (const from of STATE_ORDER) {
    for (const to of STATE_ORDER) {
      if (from === to) continue;
      const sm = new StateSmoother(from);
      sm.set(to);
      let prevErr = Infinity;
      for (let i = 0; i < 240; i++) {
        sm.update(1 / 60);
        const sum = STATE_ORDER.reduce((acc, n) => acc + sm.weights[n], 0);
        assert.ok(Math.abs(sum - 1) < 1e-9, `${from}→${to} 权重和跑偏: ${sum}`);

        for (const key of ['glow', 'radius', 'speed', 'spread', 'ringGain', 'pulse', 'sat', 'core']) {
          const lo = Math.min(STATES[from][key], STATES[to][key]);
          const hi = Math.max(STATES[from][key], STATES[to][key]);
          const v = sm.current[key];
          assert.ok(v >= lo - 1e-9 && v <= hi + 1e-9,
            `${from}→${to} 的 ${key} 过冲: ${v} 不在 [${lo}, ${hi}]`);
        }
        // 只有 from/to 两个权重非零时，误差必然单调递减；这里正好是这种情况
        const err = Math.abs(sm.current.glow - STATES[to].glow);
        assert.ok(err <= prevErr + 1e-12, `${from}→${to} 误差没有单调收敛`);
        prevErr = err;
      }
      assert.ok(prevErr < 1e-3, `${from}→${to} 240 帧后还没收敛: ${prevErr}`);
    }
  }
});

test('单一色相：所有状态共用 HUE，混合后也不漂色', () => {
  assert.equal(HUE, 188);
  for (const name of STATE_ORDER) {
    assert.equal(STATES[name].hue, undefined, '色相不该写进状态参数，统一取 HUE');
  }
  const weights = Object.fromEntries(STATE_ORDER.map((n, i) => [n, i === 2 ? 1 : 0]));
  const vis = blendStates(weights);
  assert.equal(vis.hue, HUE);
  // 混合权重和为 1 时参数不越界
  const mixed = blendStates(Object.fromEntries(STATE_ORDER.map((n) => [n, 1 / 6])));
  assert.ok(mixed.sat >= 0.2 - 1e-9 && mixed.sat <= 1 + 1e-9);
});
