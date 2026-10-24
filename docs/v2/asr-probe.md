# 探针：StepFun 支持「用户语音转写」吗？

**结论：不支持（至少不支持 OpenAI 那套 `input_audio_transcription`）—— 静默忽略。**

## 为什么问这个

真机反复出现：用户说「帮我看看蝶梦项目的 README」，模型只回一句「好，查着呢」
**却不调 `delegate_task`**。宿主拿不到用户原话（协议层把用户转写事件丢了），
就没法在本地"确定性地"帮它派活。

如果服务端支持开会话级转写，这个问题就能根治。否则只能另找办法。

## 怎么测的

`core/examples/asr_probe.rs`：直连 WS，下发
```json
{"type":"session.update","session":{"input_audio_transcription":{"model":"whisper-1"}}}
```
再推 3.86 秒真实语音，补 1.5 秒静音，收集 20 秒事件。

## 实测（2026-10-25 00:10）

```
[探针] 连 wss://api.stepfun.com/v1/realtime?model=step-1o-audio
[探针] 连上后收到: ["session.created"]
[探针] ✅ session.updated；回显 transcription = null      ← 关键
[探针] 配置结果: Some(true)

[探针] 收到的事件类型统计：
    1 × session.created
    1 × session.updated

[探针] 结论：⚠️ 配置被接受，但没收到转写事件
```

两个事实：
1. **`session.updated` 回显里 `input_audio_transcription` 是 `null`** ——
   服务端没接受这个字段，只是没报错（静默忽略）。
2. 全程没有 `conversation.item.input_audio_transcription.*` 事件。

## 所以怎么办

放弃「靠服务端转写拿用户原话」这条路。改用**协议级强制调工具**：

`response.create` 带 `{"tool_choice": "required"}` —— 兜底纠正时用这一档，
这一轮模型**必须**调工具，没有「只说话」这个选项。

已实现（`ClientCommand::RawCreateResponse` + `RealtimeClient::create_response_with`），
若服务端也不认这个字段，会被忽略 → 退化成普通一轮，无害。

## 复现

```bash
source tools/dev-env.sh
cargo run -q -p morphofly-core --example asr_probe -- /tmp/task1.wav
```
报告写到 `/tmp/morphofly-asr-probe.json`。

---

# 探针二：`tool_choice: "required"` 有效吗？

**结论：无效。强制轮返回空响应（既不调工具也不说话），比普通重试更糟 —— 不要用。**

## 背景

探针一证明「拿不到用户原话」，退而求其次想用协议级硬约束逼模型调工具。

## 实测（2026-10-25 00:35，`core/examples/toolchoice_probe.rs`）

```
[探针] 已下发 5 个工具
[探针] 已 commit + create_response，等第一轮…
  🔧 工具调用: delegate_task
  ✓ 一轮结束（completed）
[探针] 第一轮（默认 tool_choice=auto）：工具 1 次

[探针] 发 response.create + tool_choice=required
  ✓ 一轮结束（completed）
[探针] 强制轮：工具 0 次，模型说「」

[探针] 结论：❌ 不支持：强制轮返回空响应
```

## 两个副产品（都有价值）

1. **第一轮 1 次工具调用** —— 说明「模型不调工具」不是模型本身不会，
   而是**时机问题**：探针是「音频推完 + 1.5s 静音 + commit + create_response」，
   一气呵成；App 里是本地 VAD 200ms 判完就 commit。
   这指向「commit 太早 / 音频还没送完」这条线，比怪模型更可信。
2. **探针第一版漏了 `commit()`**（因为我们的会话 turn_detection=null，
   服务端不会自己判「说完了」），结果第一轮什么都没发生 ——
   写探针也会踩自己的坑，记在这省下一个人。

## 复现

```bash
source tools/dev-env.sh
cargo run -q -p morphofly-core --example toolchoice_probe -- /tmp/task1.wav
```

---

# 探针三：`commit()` 之后立刻 `create_response()` 会抢跑吗？

**结论：不会。这个假设被否证。**

## 假设

App 里「说完 → 200ms 静音 → commit + create_response」是一口气发的。
如果服务端先处理 `response.create`、后处理 `commit`，这一轮就会基于不完整输入，
模型只能敷衍一句「好，查着呢」而不调工具 —— 这能解释真机那个「时好时坏」。

## 实测（2026-10-25 00:45，`core/examples/commit_race_probe.rs`）

同一段音频放两遍，只改一个变量：

```
[A 轮] 推音频 → commit → **立刻** create_response
    🔧 delegate_task
[A 轮] 工具 1 次
[B 轮] 推音频 → commit → **等 300ms** → create_response
    🔧 delegate_task
[B 轮] 工具 1 次

结论：⚠️ 两轮都调了工具，看不出来 —— 这个场景不复现 App 的问题
```

**所以不是抢跑。** 在干净音频 + 单会话的条件下，模型每次都老实调工具。

## 剩下的差异（下一轮要查的）

探针和 App 唯一的区别是**音频来源**：
- 探针：直连推干净的 wav
- App：虚拟音源 → PulseAudio 采集（48k）→ 重采样 16k → **AEC（sonora）** → 推流

AEC 动了音频，这是最可疑的一环。已做 A/B（`MORPHOFLY_AEC=0` 开关）：
```
docs/v2/aec-ab.md
```
