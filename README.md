# 蝶梦 · 语音智能体

> 喊一声「小蝶」，用嘴指挥 DSH 干活，活干完它主动回来说。

**状态**：V1 核心体验跑通，**延迟未达标**（详见下方「已知问题」）
**日期**：2026-10-24
**开发流程**：devflow（run `20261023-2339-v1-dsh-0`，已 done）

---

## 一句话

手柄占着的时候（做饭、修东西、躺着），喊一声就能派活，干完它自己回来汇报。
**全程零键鼠。**

## 现在能做到什么（都实测过）

| 能力 | 实测 |
|---|---|
| **端到端延迟** | **802ms**（说完到听见回答） |
| **打断延迟** | **300ms**（说「停」到声音停止） |
| 喊「小蝶」唤醒 | 召回 **9/9**，误唤醒 **0/8**（含「小蝶儿」「蝴蝶」「小的」谐音陷阱） |
| 点小球进通话 | **150ms** 进到聆听态 |
| 说话派活给 DSH | 真调工具、真执行（实测跑 `ls` 返回 8 个目录） |
| 干完主动汇报 | 不等用户问，自己开口说「干完了：…」 |
| 六态小球 | S0–S5 肉眼可辨 |
| 安装包 | deb **6.20MB**，冷启动 **1476ms** |

## 已知问题（重要，别忽略）

### 1. 空闲内存 488MB（要求 <150MB）

| 进程 | RSS |
|---|---|
| `diemeng-app` 主进程 | 191 MB |
| `WebKitWebProcess` | 161 MB |
| `WebKitNetworkProcess` | 63 MB |
| **合计** | **488 MB** |

**原因**：Tauri 2 在 Linux 上必须用 WebKitGTK，这玩意儿本身 200MB+。
这是「用 WebView 做 UI」的固有代价（Electron 同量级），不是我们代码的问题。

要压下去只能重写 UI 层为原生渲染，工作量 3–5 天。
详见 `runs/*/evidence/p4-memory-honest.md`。

### 2. 未做真人完整验收

需要本人戴耳机跑一遍「喊小蝶 → 派活 → 关屏 → 听汇报」。

### 3. 打断参数偏激进

`min_speech_duration=0.08`（为了达到 300ms 延迟）。
嘈杂环境下可能需要退回 0.12s（延迟变 340ms）避免误打断。

## 快速开始

```bash
# 1. 准备（一次性）
mkdir -p ~/.diemeng
# 密钥（不要提交！）
echo 'STEPFUN_KEY=你的key' > ~/.diemeng/credentials && chmod 600 ~/.diemeng/credentials
# 唤醒词模型
cd ~/.diemeng && wget https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2
tar xf sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2 && mv sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20 kws-model
cp <repo>/tools/kws/kw_a.txt ~/.diemeng/keywords.txt
# VAD 模型（打断用）
mkdir -p vad-model && curl -sL -o vad-model/silero_vad.onnx \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx

# 2. 跑测试
source tools/dev-env.sh          # 本机无 root，ALSA 解包在 ~/.local/sysroot
cargo test -p diemeng-core --lib                    # 177 passed
cargo test -p diemeng-core --features kws --lib     # 188 passed
node --test app/frontend/test/                      # 8 passed

# 3. 真机（wings-pc）
tools/pcsync.sh
tools/pcrun.sh 'cargo run -p diemeng-core --example mic_level -- --seconds 3'
tools/pc_e2e.sh 20               # 端到端：启动 → 唤醒 → 建会话 → S2

# 4. 打包
cd app && cargo tauri build      # → target/release/bundle/deb/蝶梦_0.1.0_amd64.deb
```

## 代码结构

```
core/src/
  audio/        cpal 采集 + rodio 播放 + 环形缓冲（60 单测）
  realtime/     统一事件 + StepFun WS + provider 适配（59 单测）
  session/      六态状态机（15 单测）
  tasks/        任务表（原子写、重启不丢）
  tools.rs      4 工具不变量（物理隔离，不是提示词纪律）
  dispatch.rs   工具分发 + 结果清洗（去掉 markdown/路径，可直接朗读）
  dsh/          ACP 客户端 + 任务桥（一次任务一个会话）
  kws.rs        唤醒词常驻监听（Rust 侧，9/9 召回）
  vad.rs        本地 VAD（打断用）
  spectrum.rs   realfft 64bin + RMS
app/src-tauri/  Tauri 2 壳 + IPC + 会话守护
app/frontend/   六态蓝色小球（原生 ESM，无打包器）
core/examples/  13 个可运行探针（每个接缝一个）
tools/          11 个运维脚本
```

## 架构铁律（三条，破了就是设计失败）

1. **WebView 永不碰麦克风** —— 音频全在 Rust 侧用 cpal/rodio 原生处理
2. **语音模型只有 4 个函数** —— 没有文件/命令能力，这是物理隔离不是提示词
3. **语音链路与任务链路独立** —— 语音轮次先结束，任务后台跑完再主动汇报

## 文档索引

| 想知道 | 看这份 |
|---|---|
| 怎么换语音服务商 | `docs/切换语音服务商.md` |
| 唤醒词怎么配 | `docs/唤醒词.md` |
| 音频路由（为什么声音听不见） | `docs/音频路由.md` |
| 本机环境限制 | `docs/本机环境.md` |
| 需求/技术选型/开发方案（原始） | `docs/必看/`（从 Obsidian 仓复制） |
| 验收表（27 条可判定） | `runs/*/acceptance.md` |
| 验收报告 + 审计整改 | `runs/*/verify-report.md` |
| 复盘 | `runs/*/retro.md` |

## 开发这台机器的注意事项

- 本机是 **Debian 13 云主机**：无音频设备、无桌面、无 root、磁盘只有 59G
- 真机是 **wings-pc**（CachyOS）：用 `tools/pcrun.sh` / `tools/pcsync.sh` 远程操作
- **改前端后要 `cargo tauri build` 或清 build 缓存**，`cargo build` 有时不重新嵌入
- **截图必须用 `spectacle` 截全屏**，`import -window` 在 KWin 下会拿到过期图层
