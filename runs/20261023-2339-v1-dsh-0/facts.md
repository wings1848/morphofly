# facts · 事实核对（每条 = 断言 + 可复现命令 + 实测输出）

> 规矩：关于框架/宿主/依赖行为的断言，必须有命令和实测。没实测的写「未验证」。

| # | 断言 | 命令 | 实测输出 | 结论 |
|---|---|---|---|---|
| F1 | 本机（US 云主机）没有音频设备，无法做音频实测 | `ls /dev/snd` | `seq`, `timer`（无 pcm/control） | ✅ 无采集/播放能力 |
| F2 | 本机没有桌面/显示器 | `echo $DISPLAY $WAYLAND_DISPLAY`；`ls /tmp/.X11-unix` | 空；目录不存在 | ✅ 无 GUI 能力 |
| F3 | wings-pc（Linux/CachyOS）具备音频采集+播放 | `ls /dev/snd`（wings-pc） | `controlC0..2 hwC1D0 hwC2D0 pcmC0D0c pcmC0D0p ... pcmC1D2c` | ✅ 有麦克风+扬声器 |
| F4 | wings-pc 有活动图形会话 | `loginctl show-session 2 -p Type -p Active` | `Type=wayland Active=yes`，Xwayland `:0`，`systemctl --user show-environment` → `DISPLAY=:0 WAYLAND_DISPLAY=wayland-1` | ✅ 可跑 GUI |
| F5 | wings-pc 具备 Tauri 构建依赖 | `pkg-config --modversion webkit2gtk-4.1 gtk+-3.0 libsoup-3.0` | `2.54.1` / `3.24.52` / `3.8.0` | ✅ 可构建 |
| F6 | wings-pc 有 clang/ninja（webrtc-audio-processing 需要） | `which clang ninja` | `/usr/bin/clang` `/usr/bin/ninja`；meson 缺 | ⚠️ 后续装 meson |
| F7 | wings-pc 有 ALSA 头文件（cpal 编译必需） | `pkg-config --modversion alsa` | `1.2.16.1` | ✅ |
| F8 | wings-pc 能访问 crates.io | `cargo new t && cargo fetch` | `Updating mirror index ... Locking 1 package` | ✅ 有镜像 |
| F9 | DSH 提供 ACP stdio 接口 | `dsh --profile acp --help` | `Serve automation clients over Agent Client Protocol stdio.` | ✅ |
| F10 | ACP 能 initialize | `python3 tools/probe_acp.py` | `protocolVersion:1, agentInfo.name=deepseek-harness-acp` | ✅ |
| F11 | ACP 能建会话 | 同上 | `sessionId=9059de17-...` | ✅ |
| F12 | ACP 能派活并拿回结果 | 同上 | `stopReason=end_turn`，`prompt_ms=3229`，回「目录里共有四项…」 | ✅ |
| F13 | ACP 的 promptCapabilities.audio = false | 同上 initialize response | `"promptCapabilities": {"image": false, "audio": false, "embeddedContext": false}` | ✅ 音频不能直传 DSH |
| F14 | StepFun Realtime WS 可用 | `uv run --with websockets python ws_probe.py` | `session.created`，`model=stepaudio-3-realtime-preview`，`voice=jingdiannvsheng` | ✅ |
| F15 | StepFun voice 参数不合法会返回 error 事件而不中断会话 | 同上（发 voice:"default"） | `{"type":"error","error":{"message":"voice default is not valid","code":"400"}}` | ✅ 会话继续保留 |
| F16 | StepFun 输入输出音频格式固定 pcm16 | 官方文档 `https://platform.stepfun.com/docs/zh/api-reference/realtime` | 「input_audio_format / output_audio_format 当前仅支持 pcm16」 | ✅ |
| F17 | StepFun 事件名与 OpenAI Realtime 一致 | 同上文档 | `session.update` / `input_audio_buffer.append` / `response.audio.delta` / `response.done` | ✅ 可复用现成协议层 |
| F18 | 本机到 StepFun 端点网络可达 | `getent hosts api.stepfun.com`（wings-pc） | `api.stepfun.com` → TCP 443 OK | ✅ |
| F19 | 最新 crate 版本 | `curl crates.io/api/v1/crates/{cpal,rodio,tokio-tungstenite}` | cpal 0.18.2 / rodio 0.23.0 / tokio-tungstenite 0.30.0 | ✅ 与 docs/开发方案.md 一致 |
| F20 | 本机无 ffmpeg/sox，测试音频需在 wings-pc 或 Rust 侧解码 | `which ffmpeg sox`（本机） | 均无；wings-pc 有 `/usr/bin/ffmpeg` | ⚠️ 音频转换在 wings-pc 做 |

## 对方案的影响

1. **开发机 = wings-pc**，不是本机（本机无音频/无 GUI）。仓库在本机编辑，用 `tools/pcsync.sh` 同步，`tools/pcrun.sh` 远程执行。
2. **StepFun 协议与 OpenAI Realtime 同构**，Rust 侧写一份协议层即可，provider 换 Qwen 时只改事件适配。
3. **音频不能直传 DSH**（F13），与架构一致：语音模型负责听懂，DSH 只收文字。
