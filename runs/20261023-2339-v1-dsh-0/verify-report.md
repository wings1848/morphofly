# verify-report · 独立核验

**日期**：2026-10-24
**范围**：阶段 0（7 个接缝）+ 阶段 1（双工链路、小球视觉）
**核验者**：待填（subagent 独立上下文）

## 1. 复现命令与结果

| # | 命令 | 期望 | 实测 | 结论 |
|---|---|---|---|---|
| V1 | `source tools/dev-env.sh && cargo test -p diemeng-core --lib` | 0 failed | 160 passed | ✅ |
| V2 | `node --test app/frontend/test/` | 0 failed | 8 passed | ✅ |
| V3 | `tools/pcrun.sh 'cargo run -q -p diemeng-core --example mic_level -- --seconds 3'` | 150 帧 / 48000 采样 | 150 帧 | ✅ |
| V4 | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example ws_roundtrip -- testdata/audio/04_ack_zai.mp3'` | 收到音频+字幕 | first_audio 429ms / 88320 字节 / 「在呢，有什么事儿？」 | ✅ |
| V5 | 同上 `--example delegate` | end_turn + 非空文本 | end_turn / 3.6s / 151 字 | ✅ |
| V6 | 同上 `--example duplex -- --seconds 9 --input-file testdata/audio/04_ack_zai.mp3` | 播到云端音频 | 38400 帧播放 | ✅ |
| V7 | `tools/pc_states.sh` | 六态可见且不同 | 六态截图可辨 | ✅ |
| V8 | `tools/pcrun.sh 'cd app/src-tauri && cargo build'` | 0 error | Finished | ✅ |
| V9 | `rg -F "$STEPFUN_KEY" app/ core/ protocol/` | 0 命中 | 待核 | |
| V10 | 桌面端无 `getUserMedia` | 0 命中 | 待核 | |

## 2. 不变量核对

| # | 不变量 | 判定方式 | 结果 |
|---|---|---|---|
| I1 | 语音模型只有 4 个工具 | `tools::assert_tool_invariant` + 单测 | ✅ |
| I2 | API Key 不在前端产物 | grep 前端源码与二进制 | 待核 |
| I3 | 语音/任务两条链路独立 | 杀 WS 后任务仍完成 | 待核（阶段 2） |
| I4 | 播放队列可即时清空 | `stop_now() 773ns` | ✅ |
| I5 | 上行 16k/mono/i16/20ms | 单测断言 640 字节/帧 | ✅ |

## 3. 未完成项（诚实列出）

- 阶段 1 的「点击小球」只做了命令与状态机，**没有做真实鼠标点击的自动化验证**
- 真麦克风 + 真扬声器的 100 次延迟采样没做（只用文件回放）
- 打断（barge-in）没有在真实播放中测过
- 任务表 / 4 工具 → DSH 的桥接还没接起来（阶段 2）
- AEC、唤醒词常驻集成、打包分发 未做（按计划属于后续阶段）
