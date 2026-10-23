# verify-report · 阶段 0–4 验收报告（含独立审计整改）

**锚定 commit**：`7f7204f`
**日期**：2026-10-24 03:35 (+08:00)
**开发者自测**：lead
**独立复核**：subagent（审计报告见本文件 §6）

## 1. 一句话结论

**核心链路能跑通**（说句话 → 派活 → DSH 执行 → 主动汇报），
**但 P1 端到端延迟不达标**（实测 1.8–3.3 秒，要求 <800ms），
**阶段 4 的 Rust 侧常驻唤醒词未实现**。

## 2. 有证据的结论

| 项 | 状态 | 证据 |
|---|---|---|
| 阶段 0 七接缝 | ✅ 全绿（含审计新增「真的出声」） | `evidence/g0.*` |
| 双工链路 | ✅ 播放 38400 帧，物理 sink 录音峰值 32000 | `evidence/g1.1-duplex.md`、`evidence/g0.3-playback-physical.md` |
| 六态小球 | ✅ S0–S5 全有，肉眼可辨 | `evidence/screenshots/states-grid-all6.png` |
| 派活给 DSH | ✅ 工具调用 → 任务 Done | `evidence/g2.2-voice-tool-call.md` |
| 主动汇报 | ✅ 不等用户问，自己开口 | `evidence/g3.1-active-report.md` |
| 唤醒词识别率 | ✅ 召回 9/9、误唤醒 0/8（Python 侧） | `evidence/g4.1-kws-recall.md` |
| **P1 延迟** | ❌ **不达标** 3256ms | `evidence/p1-latency-HONEST.md` |

## 3. 复现命令（每条都验证过能跑）

```bash
cd ~/Code_Project/diemeng && source tools/dev-env.sh
cargo test -p diemeng-core --lib          # 171 passed, 0 failed
node --test app/frontend/test/            # 8 passed, 0 failed
python3 tools/probe_acp.py                # session_id + agent_text_len>0 + end_turn（连跑3次稳定）
tools/pcsync.sh && tools/pcrun.sh 'cargo run -q -p diemeng-core --example mic_level -- --seconds 3'
tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example voice_task -- --file testdata/audio/17_cmd_chinese_project.mp3'
```

## 4. 不变量

| # | 不变量 | 结论 |
|---|---|---|
| I1 | 只有 4 个工具 | ✅ 单测有效（审计做了变异测试：加第 5 个立刻红） |
| I2 | Key 不在前端 | ✅ 审计独立扫 app/core/protocol/docs/tools + 二进制 + git 历史，0 命中 |
| I3 | 两条链路独立 | ✅ 语音轮次先结束（10865ms），任务后台跑完再主动汇报 |
| I4 | 播放队列可即时清空 | ✅ 有 2 个单测覆盖（`queue_stop_now_is_immediate_i4` 等）<br>⚠️ 原证据里的「773ns」不可复现，已删除 |
| I5 | 上行格式 | ✅ 每帧 320 采样 = 640 字节 |

## 5. 未完成（诚实清单）

1. **P1 延迟不达标**：1.8–3.3s，且 `silence_duration_ms` 对 StepFun 不生效
2. **Rust 侧常驻 KWS 未实现**（Python 侧已验证识别率）
3. **打断（barge-in）未在真实播放中测过**（D3）
4. **真实鼠标点击小球未自动化验证**（B1-3）
5. **AEC 未做**（阶段 4.5）
6. **release 打包未做**（阶段 5），内存/包体积未知（D4）
7. **真人完整验收未做**：需要用户本人戴耳机跑一次全流程

## 6. 独立审计发现的问题与整改（全部已处理）

| # | 审计问题 | 严重度 | 整改 |
|---|---|---|---|
| 1 | 验收表 12 行只有 2 行判定过，2 条命令跑不通 | 🔴 高 | **重写 acceptance.md**：27 条可判定命令，全部验证过；补 `play_wav.rs`；duplex 参数统一 |
| 2 | 首音频 429ms 不稳定（双峰 ~430ms / ~5.4s） | 🔴 高 | **重测并推翻**：真实延迟 1.8–3.3s，新写 `p1-latency-HONEST.md`；旧的 `latency_bench` 标注为「口径错误，勿用于下结论」 |
| 3 | 六态证据只有 4 态，S5 从未到达 | 🟡 中 | **修 S5 状态机 bug**（`ever_connected`）+ 修巡检器 + 重拍六态（含 epoch 对齐） |
| 4 | S5 逻辑缺口（S0 断线不跳 S5） | 🟡 中 | **真 bug，已修** + 2 个新单测 |
| 5 | ACP 探针竞态（agent_text 恒为 0） | 🟡 中 | **重写探针**：响应按 id 分队列 + 通知独立队列；连跑 3 次稳定 |
| 6 | 773ns 不可复现 | 🟡 中 | 从所有证据里删除该数字，改为「有 2 个单测覆盖清空行为」 |
| 7 | 音频路由是 snd_aloop 虚拟回环 | 🔴 高 | **新增 pc_audio_route.sh + docs/音频路由.md**；切物理 sink + monitor 录音峰值 32000 证实真出声；清理测试残留 null sink |
| 8 | 仓库审计期间被持续修改 | 🟡 中 | 本报告锚定 commit `7f7204f` |

## 7. 需要用户决策（阻塞项）

### 决策 1：P1 延迟怎么办？

| 选项 | 代价 | 预期效果 |
|---|---|---|
| A. 换阿里云百炼 Qwen | 需要 DashScope key（用户提供） | 官方口径首音频 1.37s，且支持手动 commit，**可能压到 1s 内** |
| B. 放宽验收标准到 <3s | 零成本 | 与「像打电话」的产品定位冲突 |
| C. 继续调 StepFun | 时间不确定 | 实测 `silence_duration_ms` 不生效，空间有限 |
| **D. 先问 StepFun 是否支持客户端 VAD + 手动 commit** | 一条工单的时间 | 若支持可压到 300–500ms |

**推荐 D → A**：先确认协议能力，再决定要不要换 provider。

### 决策 2：项目名怎么念？

实测「diemeng」被中文 ASR 听成 `daimian`/`diamond`/`aimeng`（3 次全错）。
「蝶梦」100% 正确。

**推荐**：产品对用户只说中文名「蝶梦」，英文名只用于代码/仓库。

## 8. 是否可交付 V1

**否。** 距离交付还差：P1 延迟达标（或验收标准放宽）、release 打包、真人完整验收。
