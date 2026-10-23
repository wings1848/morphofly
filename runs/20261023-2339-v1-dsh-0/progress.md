# 进度 · 蝶梦语音智能体 V1

更新：2026-10-24 01:20 (+08:00)

## 阶段 0（接缝验证）— ✅ 7/7 全绿

| 接缝 | 状态 | 证据 |
|---|---|---|
| 0.1 Tauri 能弹窗 | ✅ | `evidence/g0.1-tauri-window.md`（截图 + 窗口枚举） |
| 0.2 cpal 原生采集 | ✅ | `evidence/g0.2-g0.3-audio.md` — 3s/150帧 精确 20ms |
| 0.3 rodio 播放 | ✅ | 同上 — 真机播放 + stop_now 773ns |
| 0.4 Rust WS 连实时语音 | ✅ | `evidence/g0.4-ws-roundtrip.md` — 首音频 429ms |
| 0.5 频谱推 WebView 画球 | ✅ | `evidence/g0.4-g0.5-local.md` — 30fps/64bin + 小球截图 |
| 0.6 DSH ACP 建会话 | ✅ | `evidence/g0.6-g0.7-acp.md` — sessionId |
| 0.7 DSH 收外部任务 | ✅ | `evidence/g0.7-delegate.json` — end_turn + 结果文本 |

**阶段 0 闸门结论：方案骨架成立，不需要换架构。**

## 阶段 1（能说话）— 部分完成

| 项 | 状态 | 证据 |
|---|---|---|
| 完整双工链路 | ✅ | `evidence/g1.1-duplex.md` — 上行 186帧 / 下行 38400帧真正出声 |
| ring buffer 可中断 | ✅ | stop_now 773ns，队列立即归零（I4） |
| 小球渲染 + 呼吸 | ✅ | 截图（六态视觉参数可区分） |
| 点小球进出通话 | ✅ | 命令 `orb_click` + 状态机单测 |
| 字幕 | ✅ | transcript 打印 |
| 延迟数字记录 | ✅ | timeline 探针：speech_stopped→首音频 26ms |

## 单测现状

```
cargo test -p diemeng-core --lib   → 160 passed, 0 failed
node --test app/frontend/test/     → 7 passed, 0 failed
```

## 踩坑记录（都写进代码注释了）

1. **StepFun `energy_awakeness_threshold` 默认 2500 太高** → 正常音量完全不触发 VAD。改 500。
2. **只能 `server_vad`**，传 `none` 被拒；且开着 VAD 时不能手动 commit。
3. **StepFun 的 tools 要 OpenAI 嵌套格式** `{"type":"function","function":{...}}`，扁平格式报错。
4. **同一次工具调用会被报两遍**（`output_item.done` + `function_call_arguments.done`），需按 call_id 去重。
5. KWin/Wayland 下 Tauri 窗口要 `GDK_BACKEND=x11` + `WEBKIT_DISABLE_DMABUF_RENDERER=1`。
6. 本机 `/etc/resolv.conf` 被 dhcpcd 覆写，MagicDNS 失效 → `tools/pc_ssh.sh` 自动回落 Tailscale IP。
7. 本机无 root：ALSA 头/库/pkg-config 解包到 `~/.local/sysroot`，用 `tools/dev-env.sh` 注入。

## 下一步

1. ~~把 `01_wake_short` 等真实语音走一遍（不要只用一个文件）~~ 
2. 六态小球在真实状态迁移下的截图（S0/S1/S2/S3/S4/S5 各一张）
3. 真麦克风 + 真扬声器做 100 次延迟采样（P1 正式口径）
4. 打断实测（播放时对着麦克风喊「停」，量 stop 延迟）
5. 写 verify-report.md，独立 subagent 复核
