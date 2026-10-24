# 本地代派：模型只说不做时，用用户原话直接派活

**2026-10-25**

## 问题

真机反复出现：用户说「帮我看看蝶梦项目的 README 第一行」，
模型回一句「好，查着呢」**却不调 `delegate_task`** —— 假装干活。

## 走到这一步之前排除过的假设（都有探针证据）

| 假设 | 结果 |
|---|---|
| 服务端不给用户转写，拿不到原话 | ❌ 错，一直在发，是我们丢了 |
| `tool_choice: "required"` 能强制调工具 | ❌ 不支持，返回空响应 |
| `commit` 后立刻 `create_response` 会抢跑 | ❌ 不会，两轮都正常 |

## 现在的做法

```
用户说话 → 云端转写（conversation.item.input_audio_transcription）
        → 记住原话
模型回复 → 若「一个工具都没调」+「回复像在办事」
        → 直接拿用户原话调 delegate_task（本地代派）
        → 不再求模型第二次，也不用多等 5 秒
```

只有拿不到原话时（云端没发转写），才退回「注入纠正提醒再求一次」。

## 代码位置

- `core/src/realtime/protocol.rs`：`RealtimeEvent::UserTranscript { text, is_final }`
- `core/src/realtime/providers/stepfun.rs`：delta 追加 / completed 整句替换
- `app/src-tauri/src/voice.rs`：`user_said` 累积 + `本地用用户原话代派` 分支

## 判据为什么是「回复像在办事」

`dispatch::looks_like_action` 只认「好，」「查着」「这就」「马上」这类措辞。
闲聊（「我叫小蝶」）不满足 → 不会误派活。

## 还没验证的

真机还没跑过这条新路径（包在构建）。要看的日志：
```
WARN 模型只说不做 → 本地用用户原话代派 said=...
INFO 本地代派成功 speech=好，干完说
```
