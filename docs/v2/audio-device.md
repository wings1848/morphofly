# 音频设备选择（输入 / 输出）

**2026-10-25 ｜ 用户要求**：「让 morphofly 支持设置输入输出声音的设备」

## 结论（先看这个）

- App 不再只能跟着「系统默认设备」跑：可以在**托盘菜单**里选麦克风和耳机/音箱，
  也可以写配置文件 `~/.diemeng/config.toml` 或环境变量。
- 设备名匹配是**不区分大小写的子串**：可以写 PipeWire/ALSA 全名，
  也可以只写 `USB`、`KTMicro`、`录音1`。
- 选完立刻生效：
  - 输入设备 → 自动重启常驻唤醒监听，立刻用新麦克风；
  - 输出设备 → 下一轮语音会话用它（播放器是每轮会话开一次的）。
- 设备列表在**启动时枚举**；插拔设备后重启 App 才刷新（托盘菜单不支持动态增删项）。

## 为什么非做不可（真机踩坑）

只信「系统默认设备」会出事，而且**日志看起来一切正常**：

| 现象 | 真因 |
|---|---|
| 用户说话，云端一个字都收不到 | 默认录音源被外部改回虚拟声卡 `mfsink.monitor`（App 录到的是空气） |
| 模型回了 17 秒语音，用户听不到 | 默认输出被改回**不出声**的板载喇叭 |

外部是谁：morphofly 自己的 e2e 测试脚本会建 `mfsink` 虚拟声卡并
`pactl set-default-source mfsink.monitor`；WirePlumber 会把这个虚拟设备记进
`~/.local/state/wireplumber/default-nodes`（`default.configured.audio.source.0=mfsink`），
于是**没有脚本在跑的时候**，只要 App 关流/设备变动，系统也会回落到它。

显式指定设备名后，这些外部改动跟 App 无关了。

## 怎么用

### 方式 1：托盘菜单（推荐）

右键托盘图标 → **输入设备（麦克风）** / **输出设备（耳机/音箱）** →
选「系统默认（不指定）」或具体设备。选完写进配置文件，下次启动还是它。

### 方式 2：配置文件

```toml
# ~/.diemeng/config.toml
[audio]
input_device = "USB"          # 或全名 alsa_input.usb-KTMicro_CDSP_2020-02-20-...mono-fallback
output_device = "KTMicro"     # 或 录音1 / 播放1 这类描述名
```

### 方式 3：环境变量（临时排查用，优先于配置文件）

```bash
MORPHOFLY_INPUT_DEVICE=USB MORPHOFLY_OUTPUT_DEVICE=KTMicro morphofly-app
```

空字符串 = 当作没配（用系统默认）。

## 怎么验证

```bash
# 1) 看 App 选中了哪个设备（启动日志）
journalctl --user -u morphofly-test.service -n 50 --no-pager | grep -E "音频设备配置|输入设备：|输出设备："
#   → 期望看到「输入设备：按配置指定（不跟随系统默认） device=...」

# 2) 看 App 实际挂在哪个设备上（PulseAudio 视角）
pactl list source-outputs | grep -E "Source:|application.name"
pactl list sink-inputs   | grep -E "Sink:|application.name"

# 3) 故意写一个不存在的设备名 → 必须告警并回退默认，不能变哑巴
journalctl ... | grep "指定的输入设备不存在"
```

## 实现位置

| 文件 | 职责 |
|---|---|
| `core/src/audio/device.rs` | 枚举设备、名字匹配（精确优先→子串）、`wanted_input()/wanted_output()` |
| `core/src/audio/capture.rs` | 开麦时按配置选设备，找不到回退默认 |
| `core/src/audio/playback.rs` | `DeviceSinkBuilder::from_device()` 按配置开播放设备 |
| `core/src/config.rs` | `[audio]` 配置段 + 只改这一段的保存函数（不覆盖其它字段） |
| `app/src-tauri/src/main.rs` | 托盘两个子菜单 + 改完重启唤醒监听 |

## 已知限制（诚实）

- 菜单里的设备列表是启动时枚举的，插拔后要重启 App 才刷新。
- `mfsink` / `mfmic` 这些**虚拟设备**也会出现在列表里——它们本来就是测试用的，
  选它们等于录空气。生产使用请选物理设备。
- 名字写错不会报错退出，只会在日志里 `warn` 并回退系统默认（避免「配置写错就哑巴」）。
