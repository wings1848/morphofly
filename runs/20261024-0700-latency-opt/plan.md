# 方案

## 步骤 1：模型横向对比（统一口径）

**问题**：之前几次测量基准点不一致，数字不可比。

**正确口径**：算出音频里最后一个 RMS>0.02 的采样位置（= 用户真正说完的时刻），
所有模型都从这一刻算起。

**测 6 个模型**：`step-1o-audio` / `stepaudio-2.5-realtime` / `stepaudio-3-realtime-preview`
/ `step-audio-2` / `step-audio-2-mini` / `step-audio-r1.5`

## 步骤 2：如果 VAD 是瓶颈，就绕过它

StepFun 支持 `turn_detection: null` + 客户端手动 `commit`。

**关键细节**（踩过的坑）：
- 必须是**显式 null**，不能靠 `skip_serializing_if` 省略字段
- 「字段缺失」= 用默认 server_vad；「字段为 null」= 真关掉

本地用 silero-vad（643KB）判定「说完了」，立刻 commit。

## 步骤 3：多轮对话验证

单轮通过不等于可用。要验证：
- 上下文是否保持
- 工具调用后能否继续
- 多轮延迟是否稳定

## 风险

| 风险 | 应对 |
|---|---|
| 手动 commit 可能不被支持 | 先测，失败就回退到 server_vad |
| 本地 VAD 误判「说完了」 | 可调 `min_speech_duration`，且服务端会兜底 |
| 换模型可能中文变差 | 实测字幕质量，不只看延迟 |
