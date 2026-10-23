# 证据索引

按阶段/验收项列出所有证据。

## 阶段 0 · 七接缝

| 文件 | 内容 |
|---|---|
| `g0.1-tauri-window.md` | Tauri 弹窗（含 KWin/Wayland 坑） |
| `g0.2-g0.3-audio.md` | cpal 采集 150帧/3s + rodio 播放 |
| `g0.3-playback-physical.md` | **真的出声**（物理 sink + monitor 录音峰值 32000） |
| `g0.4-ws-roundtrip.md` / `.json` | Rust WS 往返（首音频 429ms 那次） |
| `g0.4-g0.5-local.md` | 频谱 + 前端单测 |
| `g0.6-g0.7-acp.md` / `g0.6-acp-probe.json` / `g0.7-delegate.json` | DSH ACP 建会话 + 派活 |

## 阶段 1 · 双工 + 视觉

| 文件 | 内容 |
|---|---|
`g1.1-duplex.md` | 双工链路（上行 186 帧 / 下行 38400 帧）
`g1.2-orb-visual.md` | 六态小球（含 S1/S5 补拍）
`screenshots/states-grid-all6.png` | 六态拼图（3×2）
`screenshots/state-S0..S5.png` | 六态单张
`screenshots/e2e-S2-real-session.png` | 真实会话中的 S2

## 阶段 2 · 派活

| 文件 | 内容 |
|---|---|
`g2.1-delegate-speech.md` | 派活 → DSH 执行 → 可朗读汇报
`g2.2-voice-tool-call.md` | 语音 → 工具调用 → 派活 → 语音确认（最有价值的一条）

## 阶段 3 · 主动汇报

| 文件 | 内容 |
|---|---|
`g3.1-active-report.md` | **不等用户问，自己开口**（产品定义里最硬的一条）

## 阶段 4 · 唤醒词

| 文件 | 内容 |
|---|---|
`g4.1-kws-recall.md` | Python 侧：召回 9/9，误唤醒 0/8
`g4.2-wake-seam.md` | 状态机接缝 S0→S1→S2→S0
`g4.3-rust-kws.md` | **Rust 侧**：召回 9/9，误唤醒 0/8（含 3 个坑）
`g4.4-wake-integration.md` | 完整接缝 + App 常驻监听

## 阶段 5 · 打包与端到端

| 文件 | 内容 |
|---|---|
`g5.1-e2e-app.md` | App 端到端（点击 → 建会话 → S2，126ms）
`p2-release-metrics.md` | 二进制 18MB / 冷启动 1476ms / 内存 190MB / CPU 3.7%
`p2-release-package.md` | deb 6.20MB（AppImage 112MB 超重已弃用）

## 性能（含不达标项，如实记录）

| 文件 | 内容 |
|---|---|
`p1-vad-parameter-sweep.md` | **决定性实验**：StepFun VAD 参数完全不生效，1.6s 硬下限
`p1-latency-HONEST.md` | 诚实版延迟数据（推翻之前的「22ms 达标」误读）
`p1-latency-bench.md` | 早期错误口径的测量（保留但**不要用于下结论**）
`p2-bargein-analysis.md` | 打断失败分析（依赖云端事件的架构冲突）
`p2-bargein-local-vad.md` | 打断成功（本地 VAD：停播 <0.01ms，端到端 441ms）

## 验红记录

| 文件 | 内容 |
|---|---|
`red-dispatch.log` | 工具分发的 5 个红（含 3 个真机才暴露的 bug）
`red-session-tasks.log` | 状态机/任务表的重红→修绿
