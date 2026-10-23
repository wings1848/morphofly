# 进度 · 蝶梦语音智能体 V1

更新：2026-10-24 04:00 (+08:00)
run: `20261023-2339-v1-dsh-0`（standard 档 / desktop-app / inspect 模式）
**锚定 commit**：`87ad000`

## 一句话现状

**核心体验全部跑通**（说句话 → 派活 → DSH 执行 → 主动汇报），
**但端到端延迟 2–7 秒不达标**（要求 <800ms），卡在 StepFun 服务端 VAD 硬下限 1.6s。

## 五个阶段

| 阶段 | 状态 | 证据 |
|---|---|---|
| 0 打通接缝 | ✅ 7/7（含「真的出声」） | `evidence/g0.*` |
| 1 能说话 | ✅ 双工 + 六态小球全有 | `evidence/g1.*`、`screenshots/states-grid-all6.png` |
| 2 能派活 | ✅ 工具调用 → 任务 → 结果 | `evidence/g2.*` |
| 3 能主动汇报 | ✅ 不等用户问 | `evidence/g3.1` |
| 4 唤醒词 | ⏳ 识别率已验证，Rust 常驻未接 | `evidence/g4.*` |
| 5 打磨打包 | ❌ 未开始 | — |

## 关键结论：P1 延迟不达标，且是 provider 上限

**决定性实验**（`evidence/p1-vad-parameter-sweep.md`）：

| silence_duration_ms | 语音结束→speech_stopped |
|---|---|
| 100 | 1604ms |
| 300 | 1608ms |
| 500 | 1608ms |
| 1000 | 1612ms |

参数拉 10 倍，结果差 4ms。**StepFun 的 VAD 参数完全不生效，1.6 秒是硬下限。**
手动模式（turn_detection=null + commit）更慢：commit 要 5 秒。

端到端 = 1.6s(VAD) + 0.4~5.4s(推理) = **2–7 秒**，要求 800ms，差 3–9 倍。

## 代码结构

```
core/src/
  audio/        cpal 采集 + rodio 播放 + 环形缓冲         60 单测
  realtime/     统一事件 + StepFun WS + provider 适配     59 单测
  session/      六态状态机（含 S5 缺口修复）              15 单测
  tasks/        任务表（原子写、重启不丢）                 8 单测
  tools.rs      4 工具不变量 + 运行时断言                  6 单测
  dispatch.rs   工具分发 + 结果清洗（markdown/路径/URL）   6 单测
  dsh/          ACP 客户端 + 任务桥                       10 单测
  spectrum.rs   realfft 64bin + RMS                        9 单测
app/src-tauri/  Tauri 2 壳 + IPC + 状态推送 + 巡检器
app/frontend/   六态蓝色小球（原生 ESM，无打包器）         8 单测
core/examples/  11 个可运行探针
tools/          10 个运维脚本（pc_*/audio_route/states/...）
```

## 审计整改记录（8 项全处理）

见 `verify-report.md` §6。其中 3 个是真 bug：
1. S5 状态机缺口（待机断线不提示）
2. Tauri 2 缺 capabilities → IPC 全静默失败
3. 音频路由到虚拟设备（声音听不见）

## 下一步（等用户决策）

**阻塞项：P1 延迟怎么办？**（已发 TG 三次）

| 选项 | 需要 |
|---|---|
| A. 换阿里云百炼 Qwen | DashScope key（代码已准备好，`docs/切换语音服务商.md`） |
| B. 改产品定位为「语音留言」 | 无成本，但改变交互模型 |
| C. 接受 2–7 秒 | 无成本 |

**用户回复后立刻能做**：
- 选 A → 配 key + 跑参数扫描 + 可能要写本地 VAD
- 选 B/C → 调验收标准 + 做 release 打包

**不依赖决策的工作**：
- Rust 侧常驻 KWS（阶段 4）
- release 打包（阶段 5）
- 真人完整验收脚本
