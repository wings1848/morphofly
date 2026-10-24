# 合成语音测试的音频输入/输出流（防冲突、防自打断）

**2026-10-25 ｜ 用户要求**：「检查合成语音测试时的音频输入输出流，确保不会有冲突，自打断等问题」

## 结论（先看这个）

- 测试台上 **结构上不可能自打断**：App 的播放写「物理 sink」，采集读「虚拟 sink 的 monitor」，
  两者不是同一个设备 —— App 听不到自己的 TTS。
- 已实测：让模型说话 20 秒、不注入任何语音 → `interruptions: 0`，没有任何打断日志。
- 会自动检查这个结构的脚本：`tools/verify_audio_topology.sh`（可加 `--live` 跑一次真会话验证）。
- **唯一的自打断高危配置**：把默认 **sink** 设成 mfsink（或把默认 source 设成默认 sink 的 monitor）。
  这时 App 的 TTS 会灌进自己的采集 → 本地 VAD 判「用户开口」→ 自己打断自己。
  脚本会把这种情况直接报 FAIL。

## 实测的流拓扑（wings-gpd）

```
默认 sink   = alsa_output.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback（id 48，物理）
默认 source = mfsink.monitor（id 1015，虚拟）

注入唤醒/任务语音：paplay --device=mfsink  →  mfsink(1015)  →  mfsink.monitor(1015)
                                                                    ↓
App 采集（PulseAudio）：source-output #1317  ←──────────────────────┘
App 播放（cpal-pulseaudio）：sink-input → 物理 sink 48  →  无声卡喇叭（听不到，但不回灌）
```

用 `pactl list sink-inputs / source-outputs` 实测到的两条 App 流：

| 流 | 应用名 | 设备 |
|---|---|---|
| 录音 | `cpal-pulseaudio-24896` | Source 1015（mfsink.monitor） |
| 播放 | `cpal-pulseaudio-24896` | Sink 48（物理） |

## 三条测试纪律

1. **默认 sink 保持物理设备**，永远不要把 sink 设成 mfsink。注入只走 `paplay --device=mfsink`。
2. **默认 source 要设成 mfsink.monitor，然后重启 App** —— 采集设备是建会话时打开的，
   改完不重启还是老设备（真机踩过）。
3. 多次 `paplay` 注入可以重叠，PipeWire 会混音，不会互相吞；但**别同时改默认设备**。

## 已知限制（诚实）

- **AEC 在这套测试台上完全没被验证**：App 的输出根本不进采集，没有回声可消。
  AEC 只能等接入「实体音箱 + 真麦克风」的机器，或者人工戴耳机喊话验证。
- gpd 没有喇叭：App 把 TTS 播到物理 sink 去了，但**人听不到**。
  「主动汇报念出来」在 gpd 上只能从日志（`🔔 主动汇报` + `played_frames`）证明，
  不能靠耳朵验收。pc 同理（没扬声器），而且 pc 上没声卡可用时还会反复重开会话。
- 注入链路本身有 1–2 秒延迟（paplay 启动 + null-sink/monitor + 采集缓冲，高负载下更糟）。
  这是**测试台延迟**，不是产品延迟：真实麦克风场景没有这一段。
  打断验收脚本因此把「本地反应时间（VAD 判定人声 → 停播）」和「注入 → 判定」分开量。

## 怎么复跑

```bash
tools/verify_audio_topology.sh         # 只查结构（只读）
tools/verify_audio_topology.sh --live  # 真跑一次「只说话不插话」的会话，断言 0 打断
```

输出示例：

```
[audio] 默认 sink   = alsa_output...stereo-fallback
[audio] 默认 source = mfsink.monitor
[audio] ✅ 采集源与默认输出不是同一个设备（无扬声器回灌路径）
[audio] ✅ App 录音流存在（cpal-pulseaudio ← source）
[audio] ✅ 测试口径：默认采集源 = mfsink.monitor（注入的语音能被 App 录到）
[audio] PASS 音频输入输出结构正常
```
