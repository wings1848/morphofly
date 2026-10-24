# 探针：StepFun 支持「用户语音转写」吗？

**结论（2026-10-25 更正）：支持！而且一直在发，是我们以前把它丢了。**

> ⚠️ 这份文档的**前一版结论是错的**（写着「不支持，静默忽略」）。
> 错因是探针本身有两个 bug，见下面「为什么会误判」。留着这段是因为
> 「探针写错会骗自己」这件事比结论本身更值得记住。

## 实测（2026-10-25 00:31，`core/examples/correction_probe.rs`）

把服务端原始事件全打出来：

```
🎤 …帮我看看蝶梦项目。  🎤 …帮我看看蝶梦项目的Vimi第一行是什么。
🎤 用户转写完成: "帮我看看蝶梦项目的Vimi第一行是什么。"
🔧 function_call → {"task": "查看Vimi第一行内容",
                    "dir": "/home/wings/Code_Project/morphofly"}
```

事件类型统计里明确有：
```
1 × conversation.item.input_audio_transcription.completed
3 × conversation.item.input_audio_transcription.delta
```

## 为什么会误判（两个自坑）

1. **漏了 `commit()`**：我们的会话关掉了服务端 VAD（`turn_detection=null`），
   不手动 commit 服务端就不会开一轮 → 根本没有转写事件。
   第一版探针只推了音频就等结果，当然什么都没有。
2. **`tools` 没归一化成嵌套格式**：StepFun 要 `{type:"function", function:{name}}`，
   扁平格式会回
   ```
   error: Request param: session.tools[0].function.name is required (400)
   ```
   工具没注册上 → 模型只能瞎编 → 看起来像「模型不行」，其实是探针写错了。

## 对产品的意义

拿到用户原话之后，「模型只说不做」不再是死结：
App 现在会**直接拿原话调 `delegate_task`**（本地代派），
不用再求模型第二次，也不用多等 5 秒。

## 三个探针的最终结论汇总

| 探针 | 假设 | 结果 |
|---|---|---|
| asr_probe | 能开用户转写吗 | ✅ 能（一直能，是我们丢了） |
| toolchoice_probe | `tool_choice=required` 能强制调工具吗 | ❌ 不支持，返回空响应 |
| commit_race_probe | commit 后立刻 create_response 会抢跑吗 | ❌ 不会，两轮都正常调工具 |
| correction_probe | 纠正提醒后服务端回什么 | ✅ 工具正常；用户转写可用 |

## 复现

```bash
source tools/dev-env.sh
cargo run -q -p morphofly-core --example correction_probe -- /tmp/task1.wav
```
