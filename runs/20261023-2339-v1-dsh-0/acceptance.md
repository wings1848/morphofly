# acceptance · 本轮验收表

规矩：每行必须有判定命令（没有命令的行不成立），最后一列由开发过程填实测值。
上限 10 行。

| # | 验收标准 | 判定命令 | 期望输出 | 实测 |
|---|---|---|---|---|
| A0-1 | Tauri 能弹窗 | `tools/pcrun.sh "cd app && cargo tauri build --debug \|\| cargo build"` + `tools/pcrun.sh "DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_bqxIvn ./target/debug/diemeng-app &"` 后 `import -window root /tmp/x.png` | 截图里有小球窗口 | |
| A0-2 | cpal 原生采集跑通 | `tools/pcrun.sh "cargo run -p diemeng-core --example mic_level -- --seconds 3"` | 打印 16kHz/mono/i16 帧数与 RMS>0 | |
| A0-3 | rodio 播放跑通 | `tools/pcrun.sh "cargo run -p diemeng-core --example play_wav -- testdata/audio/01_wake_short.mp3"` | 播放完成，退出码 0 | |
| A0-4 | Rust WS 连实时语音 | `tools/pcrun.sh "cargo run -p diemeng-core --example ws_roundtrip -- testdata/audio/04_ack_zai.mp3"` | 收到 response.audio.delta 与 transcript | |
| A0-5 | 频谱推给 WebView | `tools/pcrun.sh "cargo run -p diemeng-core --example spectrum_json -- --seconds 2"` | 输出 64 个 float 的 JSON 且非常量 | |
| A0-6 | DSH ACP 能建会话 | `python3 tools/probe_acp.py` | `sessionId` 非空 | ✅ 已实测（evidence/g0.6-g0.7-acp.md） |
| A0-7 | DSH 能收外部任务 | 同上探针 | `stopReason: end_turn` 且 agent_text 非空 | ✅ 已实测 |
| A1-1 | 双工链路：说话→听到回应 | `tools/pcrun.sh "cargo run -p diemeng-core --example duplex -- testdata/audio/01_wake_short.mp3"` | 延迟数字 < 800ms，收到音频回放 | |
| A1-2 | 小球随音量动 | 截图两帧对比 | 球亮度/尺寸随 RMS 变化 | |
| A1-3 | 点击进出通话 | 自动化点击 + 日志 | 状态 S0→S2→S0 | |
| A1-4 | 语音派活能给 DSH | `tools/pcrun.sh "cargo run -p diemeng-core --example delegate -- 'list files'"` | DSH 返回文本并入任务表 | |
| A1-5 | API Key 不进前端产物 | `rg -F "$STEPFUN_KEY" app/frontend app/dist 2>/dev/null; echo rc=$?` | rc=1（0 命中） | |
| A1-6 | 两条链路独立 | 杀掉语音 WS 后任务仍完成 | 任务 `done` | |
