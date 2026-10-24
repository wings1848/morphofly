# wings-gpd：唯一能做 AEC 真机测试的设备（2026-10-24）

## 硬件确认

| 项 | 值 |
|---|---|
| CPU | Intel Pentium Silver N6000 @ 1.10GHz（4 核）|
| 内存 | 7677MB |
| 声卡 | `sof-essx8336`（SOF 驱动）|
| 麦克风 | ✅ 有（`alsa_input...stereo-fallback`）|
| 扬声器 | ✅ 有（`alsa_output...stereo-fallback`）|
| 桌面 | ✅ wayland-1（Plasma）|
| 已装蝶梦 | ✅ `~/.local/opt/morphofly` |

## 实测：麦克风能收到扬声器

```
默认输出: alsa_output.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback
默认输入: alsa_input.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback

播放 4 秒测试音 + 同时录音：
录音 160000 样本 RMS=0.00987 峰值=3293
✅ 有声音
```

**这是 AEC 真机验证的前提条件，wings-gpd 满足。**

## 对比

| 设备 | 麦克风 | 能测 AEC |
|---|---|---|
| wings-tx-us1（云主机）| ❌ | ❌ |
| wings-pc | ❌ **无麦克风硬件** | ❌ |
| **wings-gpd** | ✅ | ✅ **唯一选择** |

## 踩到的坑

**坑 1：`pw-cat` 录到纯静音**
```
pw-cat --record --format=s16 --rate=16000 --channels=1 /tmp/x.wav
→ RMS=0.00000
```
但设备状态显示正常（80% 音量、未静音）。

**坑 2：`parecord`/`paplay` 显式指定设备才行**
```
parecord --device="$SRC" --format=s16le --rate=16000 --channels=1 --file-format=wav /tmp/x.wav
paplay --device="$SINK" /tmp/x.wav
→ RMS=0.00987 峰值=3293 ✅
```

推测原因：`pw-cat` 走了 PipeWire 的默认节点解析，
而设备处于 SUSPENDED 状态时没有正确 resume。
`parecord`/`paplay` 走 PulseAudio 兼容层，显式指定设备能绕过。

**教训**：PipeWire 时代 `pw-cat` 不一定是最可靠的，
遇到静音先用 `parecord`/`paplay` 试。

## 下一步（AEC 真机验收）

需要跑完整 App（有 GUI + 音频），步骤：

```bash
# 1. 启动蝶梦
~/启动蝶梦.sh

# 2. 外放音量调到 70%
# 3. 喊「小蝶」唤醒
# 4. 让它说一段话（说「小蝶，介绍一下你自己」）
# 5. 在它说话时喊「停」
# 6. 期望：300ms 内停播，且不自我打断
# 7. 观察日志：AEC stats 的 render_blocks / capture_blocks 应同步增长
```

关键观察点：
- **自我打断**：它说话时麦克风听到自己的声音，VAD 不应该触发
- **打断能力**：用户喊「停」应该能打断（AEC 不能把人声也消掉）
