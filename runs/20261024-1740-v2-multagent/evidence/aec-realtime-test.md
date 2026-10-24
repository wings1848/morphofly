# AEC 真机验证（进行中，2026-10-24）

## 已打通的前提条件（全部实测）

| 项 | 状态 | 证据 |
|---|---|---|
| App 在 gpd 能启动 | ✅ | `小球已贴边 x=2696 y=1304 right=true` |
| 系统托盘 | ✅ | `系统托盘已就绪（左键显示/隐藏，右键菜单）` |
| AEC 已启用 | ✅ | `AEC 已启用（sonora / AEC3）` |
| **KWS 常驻监听** | ✅ | **`KWS 常驻监听已启动`** ← 麦克风可用了 |
| 麦克风能收到扬声器 | ✅ | RMS 0.00987 峰值 3293 |

## 麦克风问题的完整修复过程（4 步）

**问题**：`KWS 需要麦克风` / `PulseAudio: Unable to create stream: No such entity`

1. **诊断**：`parecord` 默认设备失败，但 `--device=显式名` 成功
2. **第一次尝试**：`.asoundrc` 里 `pcm.!default { type pulse }` → 仍失败
3. **深挖**：`pactl get-default-source` 名字是对的，但 ALSA→Pulse 桥接拿不到设备
4. **解决**：`.asoundrc` 里显式指定 `device "alsa_input.pci-...stereo-fallback"`

```
pcm.!default {
    type pulse
    device "alsa_input.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback"
}
```

**根因**：gpd 是双屏（3072×1728 外屏 + 1920×1200 内屏），
PipeWire 的默认节点解析在两个输出之间混乱，ALSA 桥接拿不到。

## 待完成的验证

**需要人耳/真实声音才能做的部分**：

1. 喊「小蝶」是否唤醒（KWS 命中）
2. 让它说话，观察是否自我打断（AEC 效果）
3. 说话时喊「停」，是否 300ms 内停播（打断能力）

**这些用 computer use 做不了** —— 需要真实发声。
但可以用 audio 注入：把 TTS 音频通过 loopback 播给麦克风。

## 当前 App 状态

进程在跑（PID 存在），日志显示所有子系统就绪：
```
系统托盘已就绪（左键显示/隐藏，右键菜单）
小球已贴边 x=2696 y=1304 right=true
AEC 已启用（sonora / AEC3） stream_delay_ms=0
KWS 常驻监听已启动
```

**这是蝶梦第一次在真实桌面完整跑起来。**
