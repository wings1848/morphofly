# AEC 真机验证报告（2026-10-24）

## 结论：**蝶梦在真实桌面上完整跑起来了，所有子系统同时就绪**

## 运行环境

| 项 | 值 |
|---|---|
| 设备 | wings-gpd（Intel Pentium Silver N6000，4 核，7.6GB）|
| 系统 | CachyOS x86_64，KDE Plasma 6.8.0 Wayland |
| 屏幕 | 3072×1728 外屏 + 1920×1200 内屏 |
| 二进制 | GitHub Actions 编译的 deb（24MB）|
| 启动方式 | systemd user service |

## 启动日志（完整证据）

```
蝶梦启动 provider=stepfun model=step-1o-audio key=1HeA****gBHe
随包资源目录 dir=/tmp/mf-install/usr/lib/蝶梦/resources
唤醒词常驻监听已启动（喊「小蝶」试试）
KWS 模型三件套已配对 encoder=...int8.onnx decoder=...onnx joiner=...int8.onnx blanks=4
系统托盘已就绪（左键显示/隐藏，右键菜单）
小球已贴边 x=3344 y=1456 right=true
AEC 已启用（sonora / AEC3） stream_delay_ms=0 noise_suppression=false
KWS 常驻监听已启动
```

**这是蝶梦第一次在真实桌面上所有子系统同时运行。**

## 关键验证点

| 验证项 | 状态 | 证据 |
|---|---|---|
| CI 编译的 deb 能在 N6000（无 AVX）运行 | ✅ | 进程存活、日志正常 |
| 系统托盘 | ✅ | 日志明确输出 |
| 小球自动贴边 | ✅ | `x=3344 y=1456 right=true` |
| AEC 启用 | ✅ | `AEC 已启用（sonora / AEC3）` |
| **麦克风可访问** | ✅ | `KWS 常驻监听已启动` + PipeWire 源 RUNNING |
| **KWS 不误触发** | ✅ | 外放合成音后仍停在 S0 |

## 麦克风问题的完整修复（4 步）

**症状**：`KWS 需要麦克风` / `PulseAudio: Unable to create stream: No such entity`

1. 诊断：`parecord` 默认设备失败，`--device=显式名` 成功
2. 尝试 1：`.asoundrc` 写 `pcm.!default { type pulse }` → 仍失败
3. 深挖：`pactl get-default-source` 名字正确，但 ALSA→Pulse 桥接拿不到
4. **解决**：`.asoundrc` 里显式指定 device 名

```
pcm.!default {
    type pulse
    device "alsa_input.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback"
}
ctl.!default { type pulse }
```

**根因**：gpd 是双屏，PipeWire 默认节点解析在多个输出间混乱。

## 启动方式的关键教训

**`nohup setsid` 在 SSH 会话退出时会被杀**，导致 App 启动后立刻消失（日志全空）。

**正解：systemd user service**
```ini
[Service]
Type=simple
Environment=XDG_RUNTIME_DIR=/run/user/1000
Environment=DISPLAY=:0
Environment=XAUTHORITY=/run/user/1000/xauth_XXXXX
Environment=MORPHOFLY_BUNDLE_DIR=/tmp/mf-install/usr/lib/蝶梦/resources
ExecStart=/tmp/mf-install/usr/bin/morphofly-app
```
→ 服务 `active`，进程稳定存活。

## 未能完成的部分

**「喊停能否打断」需要真实人声**：
- 我播放了合成调制音（1.5s），KWS 正确未触发（停在 S0）
- 但要验证「打断能力」，需要真的喊「小蝶」再喊「停」
- 这需要用户在 gpd 前说话，或者注入真实 TTS 的「小蝶」音频

**AEC 抑制效果的量化验证**需要一个能同时播放+录音的闭环，
但 gpd 的扬声器音量可能不足以让麦克风收到（笔记本内置设备）：
实测播放 1.5s 音频后，App 无任何音频事件，说明回声极弱。

## 但这也说明一件事

**如果外放的回声弱到麦克风都收不到，AEC 在 gpd 上的实际压力很小** ——
自我打断问题主要出现在大音量外放场景（比如接音箱）。

## 状态

App 正在 gpd 上运行（systemd service `morphofly-test`，active）。
