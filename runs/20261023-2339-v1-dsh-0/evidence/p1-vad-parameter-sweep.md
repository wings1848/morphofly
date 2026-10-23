# 决定性实验：StepFun 的 VAD 参数完全不生效，延迟存在硬下限

**日期**：2026-10-24 03:50 (+08:00)
**方法**：原始 WebSocket 脚本直接对 StepFun 扫参数（绕开我们自己的代码，排除本地实现问题）

## 实验结果

| silence_duration_ms | energy_threshold | 语音结束→speech_stopped | 语音结束→首音频 |
|---|---|---|---|
| 100 | 500 | **1604ms** | 7032ms |
| 300 | 500 | **1608ms** | 1998ms |
| 500 | 500 | **1608ms** | 6993ms |
| 1000 | 500 | **1612ms** | 6954ms |
| 300 | 100 | **1606ms** | 7004ms |
| 300 | 1000 | **1613ms** | 1973ms |

**`speech_stopped` 恒定在 1604–1613ms，与参数完全无关。**

- `silence_duration_ms` 从 100 拉到 1000（10 倍）→ 结果差 4ms（噪声）
- `energy_awakeness_threshold` 从 100 拉到 1000（10 倍）→ 结果差 7ms（噪声）

**结论：这两个参数对 StepFun 的 VAD 不起作用。1.6 秒是服务端的硬下限。**

## 第二组实验：手动 commit 更慢

StepFun 文档说「把 turn_detection 设为 null 可以禁用 VAD，然后手动 commit」。
我们验证了 —— **确实可行**（不会报错），但结果更差：

| 阶段 | 耗时 |
|---|---|
| commit → input_audio_buffer.committed | **5027~5028ms** |
| committed → 首个 audio.delta | 356~388ms |
| **合计** | **约 5.4 秒** |

手动模式下服务端要花 5 秒处理完 1.87 秒的音频才回 committed。
**比 server_vad 还慢 3 倍。**

## 第三组：连续推静音，VAD 何时触发

```
2711ms  input_audio_buffer.speech_started
4982ms  input_audio_buffer.speech_stopped     ← 语音结束（约 3.37s）后 1.6s
4982ms  input_audio_buffer.committed
5349ms  response.audio.delta                   ← 首个音频
6503ms  response.done
```

## 最终结论

**P1 端到端延迟 = 1.6s（VAD 硬下限）+ 0.4~5.4s（推理波动）= 2~7 秒。**

| 环节 | 耗时 | 能否优化 |
|---|---|---|
| VAD 判定说完 | **1.6s** | ❌ 参数无效，服务端固定 |
| 推理生成 | 0.4~5.4s | ❌ 波动大，双峰分布 |
| 播放缓冲 | 0.2~0.5s | ✅ 可调 |

**验收要求 <800ms，实测 2000~7000ms，差 3–9 倍。**

这不是「再优化一下」能解决的问题，是 **provider 能力上限**。

## 对我们代码的影响

我们的实现没有问题：
- 连接耗时 103ms（快）
- 收到 committed 后到首音频只有 356ms（快）
- 问题 100% 在服务端的 VAD + 推理

## 需要用户决策（已发 TG）

| 选项 | 说明 |
|---|---|
| A. 换阿里云百炼 Qwen | 官方口径首音频 1.37s；支持手动 commit。**需要 DashScope key** |
| B. 换其他 provider | 需要用户提供凭据 |
| C. 放宽验收标准到 <3s | 与「像打电话」定位冲突，且实测最坏 7s |
| D. 接受现状 | 不推荐 |

**我的建议**：先拿 key 试 Qwen。如果没有 key，只能选 C，并在产品上明确「它不是实时通话，是语音留言式交互」。
