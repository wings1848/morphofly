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
