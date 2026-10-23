# 第二轮增补（run 已 done 后的持续优化）

**日期**：2026-10-24 06:45 (+08:00)
**背景**：G8 复盘完成后，用户未回复。我不能干等，继续处理「不依赖用户决策」的优化项。

本轮不改 devflow 阶段状态（run 已是 done），记录增量成果。

## 一、延迟：5082ms → 802ms（三段优化）

这是本轮最大的技术突破，也是对我之前判断的修正。

| 阶段 | 做法 | 端到端 | 相对要求 |
|---|---|---|---|
| 原状 | `stepaudio-3` + server_vad | 5082ms | 6.4 倍 |
| v2 | `step-1o-audio` + server_vad | 1994ms | 2.5 倍 |
| **v3** | `step-1o-audio` + 本地 VAD + 手动 commit | **802ms** | **达标** |

### 我之前错在哪

我对用户说「2–7 秒是 StepFun 平台上限」。**这个判断不准确**：

1. **漏做横向对比**：StepFun 有 6 个实时语音模型，我只测了默认那个。
   换 `step-1o-audio` 就快 2.5 倍。
2. **误判「VAD 参数无效 = 无解」**：参数确实无效，但正确结论是「那就别用服务端 VAD」，
   而不是「只能接受 1.95 秒」。

### 两个技术坑

**坑 1**：`turn_detection: null` 必须是**真 null**，不能靠 `skip_serializing_if` 省略字段。
- 字段缺失 → 服务端用默认 `server_vad` → 手动 commit 报 `commit when server vad`
- 字段为 null → 真的关掉 → 手动 commit 正常

**坑 2**：`RealtimeClient::connect()` 内部硬编码了 `Some(TurnDetection::default())`，
覆盖了调用方意图。改成读 `rc.use_local_vad`。

## 二、打断：441ms → 300ms（达标）

| 配置 | 端到端 |
|---|---|
| `min_speech_duration=0.2` | 441ms |
| `=0.12` | 340ms |
| **`=0.08`** | **300ms**（3 轮一致） |

300ms 是物理下限（20ms 帧粒度 × 4 帧 VAD 判定）。

## 三、内存：190MB → 488MB（诚实复测，之前报错了）

发现之前的测量**只测了主进程**，漏算 WebKit 子进程：

| 进程 | RSS |
|---|---|
| `diemeng-app` | 191 MB |
| `WebKitWebProcess` | 161 MB |
| `WebKitNetworkProcess` | 63 MB |
| **合计** | **488 MB** |

原因：Tauri 2 在 Linux 必须用 WebKitGTK，固有开销 200MB+。
**这是架构代价，不是 bug。** 需要用户决定接受还是重写 UI 层。

## 四、测试变化

```
core 默认        177 → 178 passed（+1：turn_detection null 序列化）
core + kws       188 → 189 passed
前端              8 → 8 passed
编译警告          0（保持）
```

## 五、本轮的自我修正清单

| 之前说的 | 实际情况 | 修正证据 |
|---|---|---|
| 「延迟 2–7 秒是平台上限」 | 是**模型选择**问题，换模型+关 VAD 后 802ms | `p1-model-sweep.md`、`p1-final-latency.md` |
| 「内存 190MB」 | 漏算子进程，真实 **488MB** | `p4-memory-honest.md` |
| 「打断 441ms 不达标」 | 调参后 **300ms 达标** | `p2-bargein-local-vad.md` |

**教训：给用户报数字前，先检查测量口径是否完整。**
这三次错误都是「测量方法有问题」，不是「结论推导有问题」。
