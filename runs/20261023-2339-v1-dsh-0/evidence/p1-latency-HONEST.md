# 验收 P1 · 延迟实测（诚实版，推翻之前的「22ms 达标」）

**日期**：2026-10-24 03:00 (+08:00)
**背景**：独立审计质疑「首音频 429ms」是否稳定。我重测后确认审计是对的，而且实际情况比审计看到的更糟。

## 一句话结论

**P1 不达标。** 真实端到端延迟 **1.8–3.3 秒**，远高于验收要求 <800ms。

之前的「22ms」是**口径错误**：它只量了 `speech_stopped → 第一个音频 delta`，
但漏掉了真正的耗时大头——**服务端 VAD 判定「用户说完了」本身要 1.8–3.3 秒**。

## 实测数据（wings-pc，cargo example vad_latency）

### 04_ack_zai.mp3（音频 1.87s，人声在 0.47s 结束）
```
[vad] 第 1/5 轮：最后有声帧 → speech_stopped 3231ms；→ 首个音频 3295ms
[vad] 第 2/5 轮：最后有声帧 → speech_stopped 3230ms；→ 首个音频 3254ms
[vad] 第 3/5 轮：最后有声帧 → speech_stopped 3227ms；→ 首个音频 3250ms
[vad] 第 4/5 轮：最后有声帧 → speech_stopped 3226ms；→ 首个音频 3247ms
[vad] 第 5/5 轮：最后有声帧 → speech_stopped 3211ms；→ 首个音频 3234ms

最后有声帧 → speech_stopped（VAD 判定）: n=5 均值 3225ms
最后有声帧 → 首个音频（端到端）:        n=5 均值 3256ms
```

### 06_cmd_progress.mp3（音频 2.9s）
```
最后有声帧 → speech_stopped（VAD 判定）: n=5 均值 1842ms
最后有声帧 → 首个音频（端到端）:        n=5 均值 1863ms
```

## 排除法：不是我们这侧的锅

| 假设 | 实验 | 结果 |
|---|---|---|
| 是我注入的尾部静音太长 | 把 `DIEMENG_FILE_PAD_MS` 从 500 改到 3000 | 延迟**不变**（3224ms）→ 不是 |
| 是 `silence_duration_ms` 设太大 | 从 300 改到 5000 | 延迟**不变**（3228ms）→ StepFun 没按这个值执行 |
| 是网络/连接慢 | 连接耗时单独量 | 均值 103ms / p90 118ms → 不是 |

**结论**：`speech_stopped` 事件来的时间由 **StepFun 服务端**决定，
我们传的 `silence_duration_ms` 对它没有可见影响。
配置项写了但**实际不生效**——这是一个必须记录的发现。

## 对验收的影响

| 项 | 要求 | 实测 | 结论 |
|---|---|---|---|
| P1 端到端延迟 | <800ms（理想 <500ms） | **1842–3256ms** | ❌ **不达标** |
| P3 唤醒响应 | <500ms | 未测（Rust 侧 KWS 未接） | ⏳ |

## 之前为什么会得出 22ms

原先的 `latency_bench` 只统计 `speech_stopped → 第一个 delta`，
而 StepFun 是**全双工边想边说**模型——它在 VAD 判定「说完」时，回复其实早就生成好了。
所以那一段只有 22ms，是**模型架构的红利**，但它**不是用户感知的延迟**。

用户感知的延迟是：
```
我说完 → [VAD 判定 1800~3300ms] → [生成 22ms] → [播放缓冲 200~500ms] → 听见声音
         ↑ 真正的大头，之前完全没算
```

## 可选的对策（需要用户决策，涉及花时间和可能换 provider）

1. **关掉服务端 VAD，改客户端 VAD + 手动 commit**
   用本地 silero VAD 检测说完（可以做到 300ms），立刻 commit。
   但实测 StepFun **只接受 server_vad**，传 `none` 会被拒 → 这条路可能走不通，需要再查文档/问厂商。
2. **换回 Qwen**（阿里云百炼）：docs/技术选型.md 记录它「首音频 1.37s」，
   但那是官方口径，需要真实测。且 Qwen 支持手动 commit（不上 server_vad）。
   **需要用户提供 DashScope key。**
3. **接受 2–3 秒延迟**，把验收标准从 800ms 放宽到 3000ms。
   代价：体验上会明显「慢半拍」，与「像打电话」的产品定位冲突。
4. **用本地 VAD 做「说完」判定 + `response.create` 立即触发**（不依赖服务端 VAD）。
   需要验证 StepFun 在 server_vad 开启时是否接受主动 `response.create`（实测会报错）。

## 我的建议

**先问用户**（这是产品级决策）：
- 如果能拿到百炼 key → 优先试 Qwen（支持手动 commit，可能压到 1s 内）
- 如果拿不到 → 建议把 V1 验收标准改成 <3s，并在产品文案上回避「实时」这个词

## 相关代码

- `core/examples/vad_latency.rs` — 本证据的测量工具（口径正确）
- `core/examples/latency_bench.rs` — 旧的错误口径工具，保留但**不要用来下结论**
- `core/src/realtime/protocol.rs` `TurnDetection::default()` — 注释里有 VAD 参数的实测记录
