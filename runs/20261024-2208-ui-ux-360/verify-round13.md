# 第 13 轮验证：打断（barge-in）+ 合成语音测试的音频流排查

**2026-10-25 05:35** ｜ 触发：用户反馈三条（说完判定太短 / 回复中收到新一句被服务端拒 /
打断计数别让日志看起来正常），以及「检查合成语音测试的音频输入输出流，确保不会有冲突、自打断」。

## 一、打断：机制验证 + 三个 bug

真机实测抓到过一次完整打断（CI 产物 11dc82e 之前那版）：

```
20:48:10.078  本地判定开口
20:48:10.254  ⚡ 本地 VAD 打断（已通知云端停止生成） queued=9952
20:48:10.358  推状态给前端 state="S2" prev=S4
SessionStats { interruptions: 1 }
```

同一次运行里暴露并修掉：

| # | Bug | 后果 | 修法 |
|---|---|---|---|
| 1 | 打断路径里 `block_on` | `Cannot start a runtime from within a runtime` → 推流任务死掉、麦克风不再工作 | 改 `spawn` |
| 2 | journal 写入 ANSI 颜色码 | 脚本 grep 不到 `state="S4"`；journald 存成字节数组，`-o json` 解析踩坑 | 只在终端上色 |
| 3 | 「说完」判定 200ms | 换气就切句 → 第二段 `create_response` 被拒 `ongoing response already exists` | 改 400ms + 开新轮前兜底取消 |

按用户要求改的语义：

- 打断判定 `queued>0` → **`queued>0 || 状态机 S3/S4`**（回复进行中就算打断）
- 打断动作为：停播 + `response.cancel` + 记一次打断 + 通知状态机回 S2
- 计数去重：本地 VAD 与云端 `speech_started` 会各看见一次，用 2 秒窗口只记一次
- 日志区分「记一次打断」/「同一次开口的重复信号（不重复计数）」

## 二、音频输入输出流排查（防冲突 / 自打断）

实测拓扑（wings-gpd）：

```
默认 sink   = alsa_output...essx8336.stereo-fallback（物理，id 48）
默认 source = mfsink.monitor（虚拟，id 1015）
App 录音流  = cpal-pulseaudio ← Source 1015
App 播放流  = cpal-pulseaudio → Sink 48
注入通道    = paplay --device=mfsink → mfsink → mfsink.monitor → App 采集
```

- App 的播放**不进**采集 → **结构上不可能自打断**
- 实测：模型说话 20 秒、不注入任何语音 → `interruptions: 0`，无打断日志
- 高危配置（脚本会 FAIL）：默认 sink 设成 mfsink，或默认 source 设成默认 sink 的 monitor
- 新增脚本 `tools/verify_audio_topology.sh`（+ `--live` 真会话断言 0 打断）
- 已知限制：AEC 在这套测试台上完全没被验证（没有回声可消）；gpd/pc 没有扬声器，
  人耳听不到 TTS —— 只能用日志证明「播了」。

## 三、判定

- 打断机制：**PASS**（一次完整真机证据：停播 + 取消云端 + 计数 + 回 S2）
- 打断自动化复现：**不稳定**（虚拟音源注入有 1–2s 延迟，模型回答只有 2–4s，见 bargein.md）
- 音频流结构：**PASS**（`tools/verify_audio_topology.sh` 通过，无自打断路径）
- CI：改动已推（d4f6ee1），等 CI 绿后部署
