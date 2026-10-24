# AEC 真机验证方案（用 snd_aloop 回环设备）

**发现**：wings-pc 有 `snd_aloop` 内核模块（ALSA loopback），
可以**物理上把输出接回输入**，精确控制回声路径。

**这比真实房间测试更好**，因为：
- 回声路径完全确定（无混响、无非线性）
- 可以精确控制延迟和增益
- 可重复

## 测试设计

```
TTS 语音 → 播放到 aloop 输入 → aloop 输出（= 回声）
                                    │
                          ┌─────────┴─────────┐
                          ▼                   ▼
                    直接录音（有回声）    喂给 AEC
                          │                   │
                          └──── 对比 RMS ─────┘
```

## 具体步骤

1. 生成一段真实中文语音（edge-tts）
2. 播放到 `alsa_output.platform-snd_aloop.0.analog-stereo`
3. 同时从 `alsa_input.platform-snd_aloop.0.analog-stereo` 录音
4. 把录音分成两半：一半直接量 RMS（有回声），一半过 AEC 再量
5. 计算抑制率

## 但有个前提问题

**真实产品里 reference 是「即将播放的音频」**，
aloop 场景下 reference 就是播放的内容本身。
这个测试能验证 AEC 的核心能力，但**不验证「参考信号时序对不对」**。

要验证时序，需要真实的播放+录音同时进行，那就得跑完整的 App（需要桌面）。

## 结论

aloop 能验证 AEC 的**抑制能力**，这是最关键的部分。
