# verify-report · 独立核验

**日期**：2026-10-24
**范围**：阶段 0（7 个接缝）+ 阶段 1（双工链路、小球视觉）+ 阶段 2（派活）+ 阶段 3（主动汇报）+ 阶段 4（唤醒词实测）
**核验者**：lead 自测 + 待 subagent 独立复核

## 1. 复现命令与结果（全部本机/真机亲跑）

| # | 命令 | 期望 | 实测 | 结论 |
|---|---|---|---|---|
| V1 | `source tools/dev-env.sh && cargo test -p diemeng-core --lib` | 0 failed | **169 passed** | ✅ |
| V2 | `node --test app/frontend/test/` | 0 failed | **8 passed** | ✅ |
| V3 | `tools/pcrun.sh 'cargo run --example mic_level -- --seconds 3'` | 150 帧 / 48000 采样 | 150 帧 / 48000 | ✅ |
| V4 | `--example ws_roundtrip -- testdata/audio/04_ack_zai.mp3` | 收到音频+字幕 | first_audio 429ms / 88320 字节 /「在呢，有什么事儿？」 | ✅ |
| V5 | `--example delegate` | end_turn + 非空 | end_turn / 3.6s / 151 字 | ✅ |
| V6 | `--example duplex -- --input-file 04_ack_zai.mp3` | 播到云端音频 | 38400 帧播放 | ✅ |
| V7 | `--example voice_task -- --file 17_cmd_chinese_project.mp3` | 派活 + 主动汇报 | 「干完了：app，core，docs…」 | ✅ |
| V8 | `--example wake_word -- --file 01_wake_short.mp3` | S0→S1→S2→S0 | 完整循环 | ✅ |
| V9 | `tools/pcrun.sh 'cargo run ---example delegate'` + ACP 探针 | sessionId | 9059de17-… | ✅ |
| V10 | `tools/pc_states.sh` | 六态可辨 | 截图 6 张 + 差异测试 | ✅ |
| V11 | KWS Python 实测 | 召回 ≥95%、误唤醒 <1 | 9/9、0/8 | ✅ |
| V12 | `tools/pcrun.sh 'cd app/src-tauri && cargo build'` | 0 error | Finished | ✅ |

## 2. 不变量核对

| # | 不变量 | 判定方式 | 结果 |
|---|---|---|---|
| I1 | 语音模型只有 4 个工具 | `tools::assert_tool_invariant` + 单测 + 运行时断言 | ✅ |
| I2 | API Key 不在前端产物 | 密钥只从 env/credentials 读；前端源码 0 命中 | ✅ |
| I3 | 语音/任务两条链路独立 | 语轮 10865ms 先结束、任务后完成并主动汇报 | ✅ |
| I4 | 播放队列可即时清空 | `stop_now() 773ns` | ✅ |
| I5 | 上行 16k/mono/i16/20ms | 每帧 320 采样 = 640 字节（单测 + 真机） | ✅ |

## 3. 已完成（有证据）

阶段 0：0.1–0.7 全部 ✅（`evidence/g0.*`）
阶段 1：双工链路、小球六态、点击进出、字幕、延迟记录 ✅（`evidence/g1.*`）
阶段 2：4 工具 + 任务表 + DSH 桥 + 结果清洗 ✅（`evidence/g2.*`）
阶段 3：主动汇报 ✅（`evidence/g3.1`）
阶段 4：KWS 实测 + 状态机接缝 ✅（`evidence/g4.*`，约 50% 完成度）

## 4. 未完成项（诚实列出）

### 必须在 V1 前补的
1. **Rust 侧常驻 KWS**：现在只有 Python 侧验证，`sherpa-onnx` Rust 绑定未引入
2. **真麦克风 + 真扬声器的 100 次延迟采样**（验收 P1 正式口径，现在只有单次/文件回放）
3. **打断实测**：播放时对着麦克风喊停，量 stop 延迟（P2）
4. **点击小球的真实鼠标验证**（现在只有命令与状态机单测）
5. **AEC**（阶段 4.5）：外放时自我打断问题未验证
6. **打包分发**（阶段 5）：`cargo tauri build` 没跑过 release

### 已知缺陷
1. **英文项目名对中文 ASR 不友好**：`diemeng` 被听成 `daimian`/`diamond`/`aimeng`（实测 3 次都没对）
   - 已有缓解：指令加「已知项目表 + 不确定必须追问」+ dispatcher 目录校验
   - 根本解法：中文别名（说「蝶梦」），或 ASR 热词
2. **26ms 的「说完到开口」延迟口径存疑**：StepFun 是全双工边想边说模型，
   不能和单向轮次模型直接比。真实端到端要用真人对话采样。
3. **任务超时 3600s 是拍脑袋值**，没有实测过长任务。

### 未验证的环境
- Windows / macOS 完全没碰（计划阶段 5）
- Linux 只在 wings-pc 单机上验证过

## 5. 结论

**阶段 0 闸门：通过。方案骨架成立。**
**阶段 1–3 核心体验：通过（文件回放版），真人版待补。**
**阶段 4：可行，未完成。**

**是否可交付 V1：否。** 距离「打开电脑、不按键、喊一声、关屏、听汇报」还差：
releease 打包、真机 100 次采样、Rust 侧 KWS 常驻。
