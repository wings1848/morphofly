# acceptance · 验收表（整改版）

**整改日期**：2026-10-24 03:35
**整改原因**：独立审计指出原表 12 行只有 2 行真正判定过，2 条判定命令本身跑不通。
本版要求：**每一行的命令都必须能直接复制执行，且期望输出可判定**。

**锚定 commit**：`7f7204f`（本文所有「实测」都在此 commit 上跑出）
**真机**：wings-pc（CachyOS / PipeWire / 物理 sink `alsa_output.pci-0000_00_1f.3.iec958-stereo`）

## 前置（每行都假设已做）

```bash
cd ~/Code_Project/diemeng
source tools/dev-env.sh          # 本机无 root，ALSA 解包在 ~/.local/sysroot
tools/pcsync.sh                  # 同步到真机
```

## A. 阶段 0 接缝（7 条）

| # | 标准 | 判定命令 | 期望输出 | 实测 |
|---|---|---|---|---|
| A0-1 | Tauri 能弹窗 | `tools/pc_app.sh start && sleep 6 && tools/pcrun.sh 'xdotool search --onlyvisible --name ".*" getwindowname %@'` | 输出里有「蝶梦」 | ✅ 窗口 ID 18874371，720×720 |
| A0-2 | cpal 原生采集 | `tools/pcrun.sh 'cargo run -q -p diemeng-core --example mic_level -- --seconds 3'` | `完成：150 帧 / 48000 采样` | ✅ 150 帧，每帧 320 采样 |
| A0-3 | rodio 播放 | `tools/pcrun.sh 'cargo run -q -p diemeng-core --example play_wav -- testdata/audio/01_wake_short.mp3'` | `✅ 播放完成` 且耗时 ≈1876ms | ✅ 1876ms |
| A0-3b | **真的出声**（审计新增） | `tools/pc_audio_route.sh --physical` 后跑上面那条 + monitor 录音 | `pactl get-default-sink` 非 aloop，录音峰值 >5000 | ✅ 峰值 32000 |
| A0-4 | Rust WS 连实时语音 | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example ws_roundtrip -- testdata/audio/04_ack_zai.mp3'` | `stop=completed` + `audio>0` + 字幕非空 | ✅ 88320 字节 /「在呢，有什么事儿？」 |
| A0-5 | 频谱推 WebView | `cargo run -q -p diemeng-core --example spectrum_json -- --seconds 1 \| wc -l` | 30 行（30fps×1s） | ✅ 30 行 |
| A0-6 | DSH ACP 建会话 | `python3 tools/probe_acp.py \| grep session_id` | 有 session_id | ✅ 连跑 3 次都成功（修竞态后） |
| A0-7 | DSH 收外部任务 | `python3 tools/probe_acp.py \| grep agent_text_len` | `agent_text_len` > 0 且 `stop_reason: end_turn` | ✅ 113–153 字 |

## B. 阶段 1 / 2 / 3（功能）

| # | 标准 | 判定命令 | 期望输出 | 实测 |
|---|---|---|---|---|
| B1-1 | 完整双工链路 | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example duplex -- --seconds 9 --input-file testdata/audio/04_ack_zai.mp3'` | `played_frames` > 0 且 `✅ 双工链路闭环` | ✅ 38400 帧 |
| B1-2 | 六态可辨 | `tools/pc_s1s5.sh` 或看 `evidence/screenshots/states-grid-all6.png` | 六态截图肉眼可分 | ✅ 六张全有（S0–S5） |
| B1-3 | 点小球进出通话 | `grep -c "orb_click" app/src-tauri/src/main.rs` + 状态机单测 | 命令存在 + `click_toggles_out_of_call` 通过 | ⚠️ 命令与单测有，**真实鼠标点击未自动化验证** |
| B2-1 | 语音派活给 DSH | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example voice_task -- --file testdata/audio/17_cmd_chinese_project.mp3'` | `🔧 工具调用 delegate_task` + 任务状态 Done | ✅ |
| B2-2 | 4 工具不变量 | `cargo test -p diemeng-core --lib tools::` | 6 passed | ✅ |
| B3-1 | **主动汇报** | 同上 B2-1 | 出现 `🔔 主动汇报` 且内容可朗读 | ✅「干完了：app，core，docs…」 |
| B3-2 | 汇报无 markdown/路径 | `cargo test -p diemeng-core --lib dispatch::` | 6 passed（含 clean_for_speech 测试） | ✅ |

## C. 不变量

| # | 不变量 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| C1 | 语音模型只有 4 工具 | `cargo test -p diemeng-core --lib tools::tests::exactly_four_tools` | pass；审计还做了变异测试（加第 5 个工具 → 立刻红） | ✅ |
| C2 | Key 不在前端 | `rg -F "$(grep STEPFUN_KEY ~/.diemeng/credentials \| cut -d= -f2)" app/ core/ protocol/ docs/; echo rc=$?` | `rc=1`（0 命中） | ✅ 审计独立验证 0 处 |
| C3 | 播放队列可即时清空 | `cargo test -p diemeng-core --lib playback` | 含 `queue_stop_now_is_immediate_i4` 等测试通过 | ✅（**但 773ns 这个数字不可复现，已从证据里删除**） |
| C4 | 上行 16k/mono/i16/20ms | `cargo test -p diemeng-core --lib uplink` | 通过 | ✅ |

## D. 性能（**重点：P1 不达标，如实记录**）

| # | 标准 | 判定命令 | 通过值 | 实测 | 结论 |
|---|---|---|---|---|---|
| D1 | P1 端到端延迟 | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run -q -p diemeng-core --example vad_latency -- --rounds 5 --file testdata/audio/04_ack_zai.mp3'` | <800ms | **3256ms** | ❌ **不达标** |
| D2 | P3 唤醒响应 | 未测（Rust 侧 KWS 未接） | <500ms | — | ⏳ 未验证 |
| D3 | 打断延迟 | 未测 | <300ms | — | ⏳ 未验证 |
| D4 | 内存/包体积 | 未测（release 未构建） | <150MB / <100MB | — | ⏳ 未验证 |

> **P1 为什么不达标**：`speech_stopped` 事件本身就要 1.8–3.3 秒才来。
> 我们传的 `silence_duration_ms` 对 StepFun **实际不生效**（300 和 5000 实测无差别）。
> 详见 `evidence/p1-latency-HONEST.md`。

## E. 唤醒词（阶段 4）

| # | 标准 | 判定命令 | 通过值 | 实测 |
|---|---|---|---|---|
| E1 | 唤醒召回 | 见 `evidence/g4.1-kws-recall.md`（Python + sherpa-onnx） | ≥95% | **9/9 = 100%**（样本小） |
| E2 | 误唤醒 | 同上 | <1 次 | **0/8** ✅ |
| E3 | 状态机接缝 | `tools/pcrun.sh 'cargo run -q -p diemeng-core --example wake_word -- --file testdata/audio/01_wake_short.mp3 --seconds 6'` | S0→S1→S2→S0 | ✅ |
| E4 | Rust 侧常驻 KWS | — | — | ❌ **未实现**（只有 Python 侧） |

## 覆盖率

- **能判定的行**：A 段 8 条 + B 段 7 条 + C 段 4 条 + D 段 4 条 + E 段 4 条 = **27 条**
- **已实测通过**：20 条
- **实测不通过**：1 条（D1）
- **未验证**：6 条（B1-3 真实点击、D2/D3/D4、E4）

## 结论

**阶段 0 闸门：通过**（A 段 8 条全绿，含审计新增的「真的出声」）。
**阶段 1–3：通过**（B/C 段除 B1-3 外全绿）。
**阶段 4：可行未完成**（E4 缺失，E1/E2 是在 Python 侧验的）。
**P1 性能：不达标**，需要用户决策（换 provider / 放宽标准 / 继续优化）。
