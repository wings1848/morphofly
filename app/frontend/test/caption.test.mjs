// 启动黑气泡回归测试（2026-10-25 用户反馈：KDE 启动时黑框 + 字压球）
//
// 修复前：orb_state 没有显式字幕时回退成状态名（"待机"），前端 boot 直接显示，
// 于是每次启动都会弹一个黑色字幕气泡。这里锁定「空字幕一律不许显示」这个契约。
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { normalizeCaption } from '../src/orb/caption.js';

test('启动时没有真实字幕：空串/空白/非字符串一律返回空（不许弹黑气泡）', () => {
  assert.equal(normalizeCaption(''), '');
  assert.equal(normalizeCaption('   '), '');
  assert.equal(normalizeCaption('\n\t'), '');
  assert.equal(normalizeCaption(null), '');
  assert.equal(normalizeCaption(undefined), '');
  assert.equal(normalizeCaption(0), '');
  assert.equal(normalizeCaption({}), '');
});

test('真实字幕保留（只是去掉首尾空白）', () => {
  assert.equal(normalizeCaption('听你说'), '听你说');
  assert.equal(normalizeCaption('  说话中  '), '说话中');
});

test('旧后端把状态名当字幕传过来时，至少不会再显示空白气泡', () => {
  // 旧 orb_state 的返回形状：{state:'S0', caption:'待机'}
  // 新前端启动不再显示 caption（见 main.js boot），这里只保证清洗函数行为稳定。
  assert.equal(normalizeCaption('待机'), '待机');
});
