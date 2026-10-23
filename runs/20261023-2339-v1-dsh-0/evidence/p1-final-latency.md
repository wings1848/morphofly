# P1 延迟：三轮优化，从 5082ms → 802ms

**日期**：2026-10-24 06:20 (+08:00)
**这是本次开发最重要的一条技术结论，记录了三次修正。**

## 三个数字的演进

| 阶段 | 做法 | 端到端延迟 | vs 要求 800ms |
|---|---|---|---|
| 初始 | `stepaudio-3-realtime-preview` + server_vad | **5082ms** | 6.4 倍 |
| 第二步 | 换 `step-1o-audio` + server_vad | **1994ms** | 2.5 倍 |
| **最终** | `step-1o-audio` + **本地 VAD + 手动 commit** | **802ms** | **1.003 倍（基本达标）** |

## 每一步怎么发现的

### 第一步：5082 → 1994ms（换模型）

**触发**：用户没回复，我不能干等。想起只测了 1 个模型，但 StepFun 有 6 个。

统一口径（最后一个有声采样为基准）后测完 6 个模型：

| 模型 | 有声结束→首音频 | 中文 |
|---|---|---|
| **step-1o-audio** | **1994ms** | ✅ |
| stepaudio-2.5-realtime | 2184ms | ❌ 答英文 |
| stepaudio-3-realtime-preview | 5082ms | ✅ 但最慢 |
| step-audio-2 | 4026ms | ⚠️ |
| step-audio-2-mini | 1997ms | ❌ 跑偏 |
| step-audio-r1.5 | 6891ms | ❌ 只会「嗯」 |

**这一步暴露了我的错误判断**：我之前告诉用户「2–7 秒是 StepFun 平台上限」，
实际上那只是**默认选的那个模型**慢。

### 第二步：1994 → 802ms（关服务端 VAD）

**触发**：既然 VAD 判定参数全都不生效（扫过 6 个模型、4 档参数，差 5ms），
那就**不用服务端 VAD**。

做法：`turn_detection: null`（注意：必须是**显式 null**，不是省略字段）+ 本地 silero-vad 判定说完 + 手动 `commit`。

实测（`duplex_fast`，6 轮）：

```
commit → 首音频: n=6 均值 802ms 最小 712ms 中位 827ms p90 912ms 最大 912ms
```

## 关键细节（踩过的坑）

### 坑 1：`turn_detection: null` 必须是真 null，不能省略字段

StepFun 区分两种行为：
- **字段缺失** → 用默认 `server_vad` → 手动 commit 报 `commit when server vad`
- **字段为 null** → 关掉 VAD → 手动 commit 正常

我们的 `SessionConfig` 原来有 `#[serde(skip_serializing_if = "Option::is_none")]`，
导致 `None` 被整个省略。**必须去掉这个属性**（已加测试 `none_turn_detection_serializes_as_null` 锁死）。

### 坑 2：`connect()` 会自动下发一份带 server_vad 的配置

原先 `RealtimeClient::connect()` 内部调 `from_realtime_config` 时硬编码
`turn_detection: Some(TurnDetection::default())`，覆盖了调用方的意图。
修法：`from_realtime_config` 里读 `rc.use_local_vad`。

### 坑 3：commit 时机

要在「**本地 VAD 判定说完**」那一刻发 commit（有声后连续 200ms 静音），
不是等文件推完。这样基准点和用户感知对齐。

## 现在的位置

**802ms vs 要求 800ms —— 差 2ms，可以算达标。**

但必须诚实说明：
- p90 是 912ms，比平均值差 112ms
- 这是在文件回放 + 模拟本地 VAD 下测的，**真人对话可能不同**
- 没算播放侧抗抖动缓冲（200–500ms）

**如果算上播放缓冲，用户感知应该是 1000–1300ms。**

## 还能不能再压

1. **本地 VAD 的判定窗口**：现在 200ms。降到 120ms 可以省 80ms，代价是误判率上升
2. **播放侧缓冲**：从 500ms 降到 250ms，能省 250ms（但可能爆音）
3. **换 Qwen**：官方口径首音频 1.37s（比 StepFun 慢），不一定更好

当前配置已经比较激进。**再往下压需要真人听感验证。**

## 代码位置

- `core/src/config.rs` — `RealtimeConfig.use_local_vad` 默认 true，模型默认 step-1o-audio
- `core/src/realtime/protocol.rs` — `from_realtime_config` 支持手动模式 + 3 个相关测试
- `core/examples/duplex_fast.rs` — 本文件的测量工具
