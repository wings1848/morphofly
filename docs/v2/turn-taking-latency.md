# 说完 → 她开口：语音链路的延迟到底花在哪

**最后更新**：2026-10-25
**适用版本**：`e157194` 之后（服务端 VAD + 上行自动增益为默认）

---

## 一句话结论

真机（wings-gpd）实测：**服务端判定「你说完了」之后 353–386ms，她就开始出声**。
从「你闭嘴」算起，还要加上服务端 250ms 的静音判定，合计约 **600ms 量级**。

```
你闭嘴 ──250ms(服务端判定说完)──▶ speech_stopped ──386ms──▶ 首段音频进声卡
```

日志里就是这两行：

```
INFO 云端判定用户说完（speech_stopped）
INFO 首次音频到达（说完 → 它开口） wait_ms=1456 cloud_wait_ms=386 local_vad=false
```

- `cloud_wait_ms`：**云端判定说完 → 首音频**（真正能优化的那段，386ms）
- `wait_ms`：本地 VAD 判定说完 → 首音频（本地 VAD 天生慢 500ms，只当参考）

---

## 三个坑，按踩到的顺序

### 坑 1：服务端 VAD 根本不触发（根因：我们的麦克风太小）

一度以为「服务端 VAD 失效」，于是改成客户端本地 VAD + 手动 commit，多等 500ms。

真实情况：阶跃的 server VAD 是**能量门控**（`energy_awakeness_threshold`）。
浏览器（官方控制台）自带 WebRTC 自动增益，所以它一直正常；我们把原始麦克风音量送上去，
服务端压根不认 —— 连 `input_audio_buffer.speech_started` 都不发。

探针实测（`core/examples/vad_probe.rs`，对真实 API 跑，同一段真人语音录音）：

| 送进去的音频 | 服务端 VAD | 自动回复 |
|---|---|---|
| 测试用 TTS mp3（响） | ✅ 触发 | ✅ 自动开一轮 |
| 真实耳机麦录音（峰值 ~0.09 / RMS ~0.013） | ❌ 一声不响 | ❌ |
| 同一段录音 ×2（+6dB） | ✅ 触发 | ✅ |
| 同一段录音 ×4（+12dB） | ✅ 触发 | ✅ 且转写更准（"小蝶，你好，你能听见吗？"） |

**修法**：上行加自动增益（`core/src/audio/gain.rs`），峰值跟踪 + 平滑，1×–8×，
底噪（峰值 < 0.015）不放大；只作用于**送上行的那一路**，本地 VAD 仍用原始帧。

### 坑 2：本地 VAD 的 500ms 是白等的

本地 silero VAD 的 `min_silence_duration = 0.5s`，等于「你闭嘴后 500ms 才 commit」。
服务端 VAD 只要 250ms（还能调），而且它自己就会触发推理 —— 本地这套纯属多等。

**修法**：默认改用服务端 VAD（`use_local_vad: false`）。
本地 VAD 保留，只用来做打断（它比云端快，实测 <10ms 停播）。

### 坑 3：播放缓冲装不下服务端「突发推流」

服务端**不按实时速率**推音频：一次回复 58080 个 24k 采样（2.42 秒）在 **650ms** 内全部到达。
当时播放环只有 500ms 容量 → 来不及播的样本被新样本覆盖 → 用户听到「一卡一卡、说几个字就断」。

**修法**：播放环 500ms → 10s（`JITTER_MAX_MS`），并加溢出计数日志。
环变大**不增加延迟**（播放端仍然是到多少放多少，环只是先进先出水池）。

实测：`回复音频收完 chunks=7 frames=107040 seconds=4.46 dropped_total=0`。

---

## 另外两道保险

| 机制 | 作用 |
|---|---|
| **首包看门狗** | `commit`/触发后 2 秒还没首音频 → 取消并重发一次。以前这条路径**没有任何超时**，服务端不回就永久静默 |
| **打断截断** | 打断时发 `conversation.item.truncate`（告诉服务端用户听到第几毫秒），否则上下文里她以为自己说了没播出去的话 |

---

## 参数与开关

| 项 | 默认 | 改法 |
|---|---|---|
| 说完判定（静音） | 250ms | `MORPHOFLY_VAD_SILENCE_MS=200`（50–2000） |
| 回合判定模式 | 服务端 VAD | `MORPHOFLY_LOCAL_VAD=1` 退回本地 VAD |
| 上行增益 | 开（1–8×） | `MORPHOFLY_NO_AGC=1` 关闭 |
| 首个音频前补的音频 | 300ms | `TurnDetection::prefix_padding_ms` |
| 播放缓冲 | 10s | `audio::JITTER_MAX_MS` |

> 判定值再往下调（<200ms）中文换气容易被切成两轮；往上调（>400ms）等待感明显。

---

## 怎么自己验

```bash
# 1) 对真实 API 验服务端 VAD 契约（需要 STEPFUN_KEY）
set -a; . ~/.diemeng/credentials; set +a
cargo run -q -p morphofly-core --example vad_probe -- testdata/audio/07_cmd_fix_bug.mp3

# 2) 真机跑一轮，然后量日志里的两个数
journalctl --user -u morphofly-test.service --since "-5 min" -o cat \
  | grep -E "云端判定用户说完|首次音频到达|回复音频收完"
```

判读：
- `cloud_wait_ms` 是**服务端干活时间**，>800ms 说明瓶颈在模型侧，客户端调参没用
- `dropped_total > 0` 说明播放环又不够了（会听到卡顿）
- 完全没有 `云端判定用户说完` → 服务端 VAD 没被触发（先查音量 / 是不是关了 AGC）
