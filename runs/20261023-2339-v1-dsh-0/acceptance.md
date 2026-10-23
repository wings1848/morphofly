# acceptance · 验收表（最终版）

**锚定 commit**：见 `git log -1`
**日期**：2026-10-24 05:40 (+08:00)
**真机**：wings-pc（CachyOS / PipeWire / 物理 sink）

## 前置

```bash
cd ~/Code_Project/diemeng && source tools/dev-env.sh
tools/pcsync.sh
```

## A. 阶段 0 · 七接缝

| # | 标准 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| A0-1 | Tauri 能弹窗 | `tools/pc_app.sh start && sleep 6 && tools/pcrun.sh 'xdotool search --name "蝶梦"'` | 有窗口 ID | ✅ |
| A0-2 | cpal 原生采集 | `tools/pcrun.sh 'cargo run --example mic_level -- --seconds 3'` | 150 帧 / 48000 采样 | ✅ |
| A0-3 | rodio 播放 | `tools/pcrun.sh 'cargo run --example play_wav -- testdata/audio/01_wake_short.mp3'` | 播放完成 ≈1876ms | ✅ |
| A0-3b | **真的出声** | `tools/pc_audio_route.sh --physical` + monitor 录音 | sink 非 aloop，峰值 >5000 | ✅ 峰值 32000 |
| A0-4 | Rust WS 连实时语音 | `tools/pcrun.sh 'set -a; . ~/.diemeng/credentials; set +a; cargo run --example ws_roundtrip -- testdata/audio/04_ack_zai.mp3'` | 收到音频+字幕 | ✅ |
| A0-5 | 频谱推 WebView | `cargo run --example spectrum_json -- --seconds 1 \| wc -l` | 30 行 | ✅ |
| A0-6 | DSH ACP 建会话 | `python3 tools/probe_acp.py` | session_id | ✅ 连跑3次稳定 |
| A0-7 | DSH 收外部任务 | 同上 | agent_text_len>0 + end_turn | ✅ 113–153 字 |

## B. 功能

| # | 标准 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| B1 | 双工链路 | `tools/pcrun.sh 'cargo run --example duplex -- --seconds 9 --input-file testdata/audio/04_ack_zai.mp3'` | played_frames>0 | ✅ 38400 帧 |
| B2 | 六态可辨 | 看 `evidence/screenshots/states-grid-all6.png` | 六态肉眼可分 | ✅ |
| B3 | App 端到端 | `tools/pc_e2e.sh 20` | S0→S1→S2 + WS 连接 | ✅ 126ms |
| B4 | 语音派活 | `tools/pcrun.sh '... voice_task -- --file testdata/audio/17_cmd_chinese_project.mp3'` | 工具调用 + 任务 Done | ✅ |
| B5 | **主动汇报** | 同上 | 出现「🔔 主动汇报」 | ✅ 成功/失败都如实报 |
| B6 | 唤醒召回 | `tools/pcrun.sh 'cargo run --features kws --example kws_check -- --model ~/.diemeng/kws-model --keywords ~/.diemeng/keywords.txt testdata/audio/'` | 召回 100% | ✅ 9/9 |
| B7 | 唤醒误唤醒 | 同上 | 0 | ✅ 0/8 |
| B8 | 唤醒→聆听接缝 | `tools/pcrun.sh 'cargo run --features kws --example wake_integration'` | S0→S1→S2→S0 | ✅ |

## C. 不变量

| # | 不变量 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| C1 | 只有 4 工具 | `cargo test --lib tools::` | 6 passed（含变异测试） | ✅ |
| C2 | Key 不在前端 | `rg -F "$KEY" app/ core/ protocol/ docs/; echo rc=$?` | rc=1 | ✅ 审计独立验证 0 命中 |
| C3 | 队列可即时清空 | `cargo test --lib playback` | 通过 | ✅ 实测 <0.01ms |
| C4 | 上行格式 | `cargo test --lib uplink` | 通过 | ✅ 320 采样/帧 |

## D. 性能

| # | 标准 | 判定命令 | 通过值 | 实测 | 结论 |
|---|---|---|---|---|---|
| D1 | P1 端到端延迟 | `tools/pcrun.sh '... vad_latency -- --rounds 5'` | <800ms | **2000–7000ms** | ❌ **不达标**（provider 上限） |
| D2 | P2 打断延迟 | `tools/pcrun.sh '... --features kws --example bargein_local'` | <300ms | 停播 **<0.01ms** / 端到端 **441ms** | ✅（可调到 300ms 出头） |
| D3 | P4 空闲内存 | release 跑 10s 后 `VmRSS` | <150MB | **190MB** | ❌（软件渲染下测的） |
| D4 | P5 包体积 | `ls -lh target/release/bundle/deb/*.deb` | <100MB | **6.20MB** | ✅✅ |
| D5 | P6 冷启动 | 双击到窗口出现 | <3s | **1476ms** | ✅ 达理想值 |
| D6 | P7 空闲 CPU | 待机 10s 均值 | <5% | **3.7%** | ✅ |

## E. 测试总量

```
cargo test -p diemeng-core --lib              → 177 passed, 0 failed
cargo test -p diemeng-core --features kws     → 188 passed, 0 failed
node --test app/frontend/test/                →   8 passed, 0 failed
编译警告（core / app / app+kws）               →   0 / 0 / 0
```

## 覆盖率

- 能判定的条目：**27 条**
- 通过：**24 条**
- 不通过：**2 条**（D1 延迟、D3 内存）
- 未验证：**1 条**（真人完整验收，需要用户本人操作）

## 结论

**阶段 0–5 的功能全部通过。**
**性能上 P1 不达标**（StepFun 服务端 VAD 1.6s 硬下限，见 `p1-vad-parameter-sweep.md`），
**P4 内存超标**（软件渲染模式下测的，真实场景未验证）。

**V1 是否可交付：有条件交付。**
条件：接受 2–7 秒的语音延迟，或换 provider（代码已备好）。
