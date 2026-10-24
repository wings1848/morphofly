# V2 端到端测试基础设施（已验证可用）

**日期**：2026-10-24 16:32

## 结论：wings-pc 的 DSH 可以远程驱动，用于完整端到端测试

### 实测

```bash
# 从 wings-tx-us1 通过 SSH 跑 wings-pc 上的 dsh --profile acp
1. initialize: OK
2. session/new: dc68c3c5-2667-4953-a7db-f5761f9f28f4
3. prompt: 0.7s, stop=end_turn, 文本='收到'
```

**三步全通，派活到拿到结果 0.7 秒。**

### 这意味什么

蝶梦的「派活 → DSH 执行 → 汇报」这条链路，
**可以在 wings-pc 上真实跑，不需要在本机装 DSH**。

对比之前：`core/examples/voice_task.rs` 只能在本机跑（本机也有 DSH）。
现在多了一条路：`DETERMINE_CWD` 指向远程，或者用 SSH 隧道。

## 三种端到端测试架构

### A. 完全本地（当前）
```
蝶梦 app (wings-pc) → dsh --profile acp (wings-pc 本地)
```
**优点**：延迟最低，最接近真实使用
**缺点**：要有屏幕、音频设备、GUI

### B. 本机蝶梦 + 远程 DSH（新增能力）
```
蝶梦 core (tx-us1) --SSH--> dsh --profile acp (wings-pc)
```
**优点**：不用 GUI 也能测任务链路
**缺点**：多一层 SSH 延迟（实测 0.7s 里含这个）

### C. 混合：GUI 在 wings-pc，测试驱动在 tx-us1
```
tx-us1: 测试脚本 → SSH → wings-pc: 蝶梦 app + DSH
```
**优点**：最接近真实，且能自动化
**缺点**：要处理 SSH 会话的显示/音频权限（踩过）

## 推荐的测试分层

| 层 | 在哪测 | 测什么 |
|---|---|---|
| 单元测试 | tx-us1（CI） | 纯逻辑，178 个已有 |
| 集成测试 | tx-us1 | 协议层、DSH 桥（用远程 DSH） |
| 组件 e2e | wings-pc | 音频链路、KWS、VAD（要真麦克风） |
| 宿主 e2e | wings-pc（GUI） | 完整用户流程（要屏幕） |
| 人工验收 | **用户本人** | 真实使用感受 |

## 待建：自动化 e2e 脚本

需要一个脚本能：
1. 在 wings-pc 启动蝶梦 app
2. 用 TTS 合成音频代替人说话
3. 通过虚拟音频设备注入（或者用 `--input-file` 模式）
4. 采集输出音频，验证内容
5. 截图验证 UI 状态

**关键难点**：怎么把音频喂进去。
- 方案 1：`PULSE_SINK` 虚拟设备（pactl load-module module-null-sink）
- 方案 2：蝶梦加 `--audio-file` 测试模式（代码里已有 `file_source`）
- **方案 2 更可控**，且 `core/src/audio/capture.rs` 已有 `file_source()`

## 下一步

1. 建 `tools/e2e/` 目录
2. 写 TTS 合成脚本（用 edge-tts，免费）
3. 写测试用例 YAML（场景 → 输入音频 → 期望行为）
4. 写驱动脚本（启动 app → 注入音频 → 采集结果 → 判定）
