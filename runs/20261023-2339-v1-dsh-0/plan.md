# plan · 实施计划（本波 = 阶段 0 + 阶段 1）

## Context

本仓库从零搭建。本机（US 云主机）负责写代码与自动化测试；wings-pc 负责真机音频/GUI 实测。
两条流水：`tools/pcsync.sh` 同步、`tools/pcrun.sh` 远程执行。

## Goals

1. 阶段 0 的 7 个接缝全绿（其中 0.6/0.7 已完成）
2. 阶段 1：双工音频链路 + 小球 + 点击进出 + 字幕 + 延迟数字

## Non-goals

AEC、KWS 常驻、任务表持久化、主动汇报排队、打包、移动端、Windows/macOS。

## Design

```
core/src/
  audio/{capture,playback,ring}.rs   原生采集与可中断播放
  realtime/{protocol,client,providers}.rs  统一事件模型 + StepFun/Qwen 适配
  session/machine.rs                 六态状态机（纯逻辑，可单测）
  tasks/store.rs                     任务表（JSON 持久化）
  dsh/acp.rs                         ACP 客户端（spawn + JSON-RPC）
  spectrum.rs                        频谱（realfft → 64 bin）
  config.rs                          TOML + env 覆盖
  lib.rs                             对外 API（被 Tauri 命令层调用）
app/src-tauri/                       壳 + IPC 命令
app/frontend/                        球渲染（原生 ES module）
protocol/schema.json                 跨端事件契约
```

## 接口冻结（先冻结再并行）

**Tauri IPC（前端 ←→ Rust）**

| 方向 | 命令/事件 | 载荷 |
|---|---|---|
| FE→BE | `orb_click` | `{}` → 返回 `{state}` |
| FE→BE | `orb_state` | `{}` → `{state: "S0".."S5", caption: string}` |
| BE→FE | `spectrum` | `{bins: [f64; 64], rms: f64}` @30fps |
| BE→FE | `state` | `{state, caption}` |
| BE→FE | `caption` | `{text}` |

**Realtime 统一事件（protocol/schema.json）**

| 事件 | 字段 |
|---|---|
| `SessionCreated` | `session_id`, `model` |
| `SpeechStarted` | — |
| `SpeechStopped` | — |
| `AudioDelta` | `pcm: Vec<i16>`（24k? 记录在 `sample_rate`） |
| `AudioDone` | — |
| `TranscriptDelta` | `text` |
| `TranscriptDone` | `text` |
| `TextDelta` | `text` |
| `ToolCall` | `call_id`, `name`, `arguments` |
| `ResponseDone` | `status` |
| `Error` | `message`, `code` |

**DSH（core → dsh --profile acp）**

| 调用 | 方法 | 结果 |
|---|---|---|
| 建会话 | `session/new {cwd}` | `sessionId` |
| 派活 | `session/prompt {sessionId, prompt:[{type:"text",text}]}` | `stopReason` + 流式 `agent_message_chunk` |
| 取消 | `session/cancel {sessionId}` | — |

## 任务 DAG

见 `tasks.yaml`。第一波并行 3 个任务（互不写同一文件）：
- T1 音频层（`core/src/audio/**`）
- T2 实时语音协议层（`core/src/realtime/**`）
- T3 状态机 + 任务表 + 配置（`core/src/session/**`、`core/src/tasks/**`、`core/src/config.rs`）

第二波（依赖第一波接口）：
- T4 DSH ACP 客户端（`core/src/dsh/**`）
- T5 频谱（`core/src/spectrum.rs`）
- T6 Tauri 壳 + IPC（`app/src-tauri/**`）

第三波：
- T7 前端小球（`app/frontend/**`）
- T8 examples（各接缝的可运行探针）
- T9 集成 + 真机验证

## 不变量（本次必须保持不变）

| # | 不变量 | 检查方式 |
|---|---|---|
| I1 | 语音模型只有 4 个工具 | 单测断言 tools 数组长度与名字 |
| I2 | API Key 不出现在前端产物 | `rg -F <key> app/frontend app/dist` rc=1 |
| I3 | 语音链路与任务链路独立 | 杀 WS 后任务仍完成 |
| I4 | 播放队列可即时清空 | 单测：play→clear 后 buffer 长度为 0 |
| I5 | 上行格式恒为 16k/mono/i16/20ms | 单测断言帧字节数 = 640 |

## 风险

| 风险 | 对策 |
|---|---|
| wings-pc 远程执行断连 | 每步写证据文件到仓库；断线可重跑 |
| StepFun 限免下线 | provider 配置化，Qwen 适配预留 |
| 无 KVM 下截图失败（kwin 无 screencapture 协议） | 用 `spectacle -b -n -f -o` 可行（已实测 10MB png） |
| 音频设备被占用 | 用 PipeWire 默认设备；失败则回落到 null sink 做链路测试 |
