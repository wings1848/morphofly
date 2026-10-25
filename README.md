# MorphoFly · 蝶梦

> 喊一声「小蝶」，用嘴指挥 AI 干活；活干完，它自己回来汇报。

手被占着的时候（做饭、修东西、躺着），不用看屏幕、不用敲键盘，
把事说给它，它派给干活 agent 去办，办完主动开口告诉你。

[![CI](https://github.com/wings1848/morphofly/actions/workflows/ci.yml/badge.svg)](https://github.com/wings1848/morphofly/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-1.82%2B-orange?logo=rust)
![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri)
![Platform](https://img.shields.io/badge/Linux-x86__64-success)
![Tests](https://img.shields.io/badge/core%20tests-284-brightgreen)

---

## 它是怎么工作的

```
   你：小蝶                          ← 本地唤醒词（sherpa-onnx，9/9 召回）
   你：今天天气怎么样                 ← 云端语音模型听懂（说完 → 她开口 386ms）
   她：好，查着呢                     ← 立刻派给干活 agent（DSH，ACP 协议）
   …（后台真的在干）…
   她：干完了：今天天气不错 ☀️         ← 不等你问，主动回来汇报
```

三个角色，各管一摊：

| 角色 | 干什么 | 不干什么 |
|---|---|---|
| **小蝶**（语音前台） | 听、说、把话整理成任务 | 不读文件、不跑命令、**不编造结果** |
| **干活 agent**（DSH） | 真正执行任务（读代码、跑测试、查资料） | 不直接和用户说话 |
| **App**（Tauri 壳） | 悬浮球、托盘、设备管理、会话生命周期 | 不碰密钥、不碰麦克风 |

---

## 实测数字

全部来自真机（wings-gpd：Intel N6000 + PipeWire + 真实麦克风）。

| 指标 | 实测 | 说明 |
|---|---|---|
| **说完 → 她开口** | **353–386ms** | 服务端判定说完后测到首段音频 |
| 判定「你说完了」 | **250ms 静音** | 服务端 VAD，可用环境变量调 |
| 打断（判定人声 → 停播） | **<10ms** | 本地 silero VAD 先停，不等云端 |
| 播放丢帧 | **0** | 10 秒播放缓冲，整段回复不再被截断 |
| 唤醒召回 / 误报 | **9/9 · 0/8** | 唤醒词「小蝶」 |
| 单元测试 | **284 passed** | `cargo test -p morphofly-core --lib` |
| 前端测试 | **13 passed** | `node --test app/frontend/test/` |
| 空闲内存 | ≈294MB PSS | 目标 <150MB，见[已知问题](#已知问题) |

数字怎么复现：[`docs/v2/HOW-TO-VERIFY.md`](docs/v2/HOW-TO-VERIFY.md) ·
语音链路细节：[`docs/v2/turn-taking-latency.md`](docs/v2/turn-taking-latency.md)

---

## 快速开始

### 1. 准备

- Linux x86_64（已在 GPD 掌机的 Arch/CachyOS 上验证）
- 一个阶跃星辰的 API Key（[platform.stepfun.com](https://platform.stepfun.com)）
- 想让它干活的话，还需要装 [DSH](https://www.npmjs.com/package/@deepseek-ai/dsh)（`npm i -g @deepseek-ai/dsh`）

### 2. 配置密钥

```bash
mkdir -p ~/.diemeng
echo 'STEPFUN_KEY=你的key' > ~/.diemeng/credentials
chmod 600 ~/.diemeng/credentials
```

密钥只从环境变量或这个文件读，**永不进前端、永不进仓库**。

### 3. 运行

```bash
# 从 CI 产物（推荐：不在本机编译）
tools/gpd_deploy_ci.sh --run <run-id>     # 下载 + 装机 + 重启服务

# 或者本机编译（需要 webkit2gtk-4.1 / gtk3 / libasound2）
cd app && cargo tauri build               # 产出 deb / AppImage
```

启动后屏幕右下角出现一个蓝色小球 —— 喊「小蝶」，或者点它一下。

### 4. 想改设备 / 改参数

右键托盘图标：

- **输入设备（麦克风）**、**输出设备（耳机/音箱）**：选完立刻生效并写进配置
- 换麦克风后唤醒监听会自动重启（否则还在录老设备）

---

## 可调参数（都能用环境变量改，不用重新编译）

| 变量 | 默认 | 作用 |
|---|---|---|
| `STEPFUN_KEY` | — | 阶跃 API Key |
| `MORPHOFLY_MODEL` | `step-1o-audio` | 语音模型 |
| `MORPHOFLY_VAD_SILENCE_MS` | `250` | 多少毫秒静音算「说完了」 |
| `MORPHOFLY_LOCAL_VAD` | `0` | `1` = 退回本地 VAD + 手动 commit |
| `MORPHOFLY_NO_AGC` | `0` | `1` = 关掉上行自动增益 |
| `MORPHOFLY_INPUT_DEVICE` | 系统默认 | 麦克风（名字子串匹配，如 `USB` / `录音1`） |
| `MORPHOFLY_OUTPUT_DEVICE` | 系统默认 | 扬声器（如 `播放1`） |
| `MORPHOFLY_BUNDLE_DIR` | — | 唤醒词/VAD 模型目录 |

---

## 架构

```
┌────────────────────────────────────────────┐
│ WebView：画六态小球 + 气泡字幕              │
│ 不碰麦克风 · 不碰密钥                       │
└───────────────┬────────────────────────────┘
                │ Tauri IPC（只收状态与频谱，单向）
┌───────────────▼────────────────────────────┐
│ Rust core                                  │
│  本地：唤醒词 KWS · VAD · 播放缓冲 · AGC     │
│  云端：realtime 语音模型（只有 5 个函数）    │
│  桥接：DSH（ACP 协议）· 任务表 · 主动汇报    │
└────────────────────────────────────────────┘
```

三条设计铁律（破了就算设计失败）：

1. **WebView 永不碰麦克风** —— 音频全在 Rust 侧用 cpal/rodio 原生处理
2. **语音模型只有 5 个函数** —— 派活 / 查进度 / 取消 / 追加 / 切上下文，
   **物理上**没有文件和命令能力（白名单 + 黑名单断言守着）
3. **语音链路与任务链路独立** —— 语音这一轮先结束，任务后台跑完再主动汇报

---

## 目录结构

```
core/                 Rust 核心
  audio/              采集 / 播放 / 环形缓冲 / 上行增益 / AEC
  realtime/           统一事件模型 + 服务商适配
  session/            六态状态机
  kws.rs  vad.rs      本地唤醒词与 VAD
  dispatch.rs         5 个工具的分发与结果清洗
  dsh/                ACP 客户端 + 任务桥
app/src-tauri/        Tauri 2 桌面壳（托盘 / 设备 / 会话）
app/frontend/         六态蓝色小球（原生 ESM，无打包器）
core/examples/        可运行探针（延迟 / VAD / AEC / 打断…）
tools/                运维与验收脚本
docs/                技术文档（v2/ 下有渐进式历史）
runs/                devflow 开发记录（含实测证据）
```

---

## 文档

| 想了解 | 看这里 |
|---|---|
| 语音链路怎么做到 386ms | [`docs/v2/turn-taking-latency.md`](docs/v2/turn-taking-latency.md) |
| 怎么验这个项目 | [`docs/v2/HOW-TO-VERIFY.md`](docs/v2/HOW-TO-VERIFY.md) |
| 进展与里程碑 | [`docs/v2/PROGRESS.md`](docs/v2/PROGRESS.md) |
| 变更记录 | [`CHANGELOG.md`](CHANGELOG.md) |
| 音频路由排障 | [`docs/音频路由.md`](docs/音频路由.md) · [`docs/v2/audio-device.md`](docs/v2/audio-device.md) |
| 唤醒词配置与坑 | [`docs/唤醒词.md`](docs/唤醒词.md) |
| 打断（barge-in） | [`docs/v2/bargein.md`](docs/v2/bargein.md) |
| 换语音服务商 | [`docs/切换语音服务商.md`](docs/切换语音服务商.md) |
| 编译走 CI + 部署 | [`docs/v2/ci-build.md`](docs/v2/ci-build.md) |
| 开发方案与验收标准 | [`docs/开发方案.md`](docs/开发方案.md) · [`docs/验收标准.md`](docs/验收标准.md) |

---

## 开发与验收

```bash
# 单元测试（本机无桌面也能跑）
cargo test -p morphofly-core --lib                   # 284 passed
cargo test -p morphofly-core --features kws --lib    # 含唤醒词

# 前端测试
node --test app/frontend/test/                       # 13 passed

# 门禁（CI 同款）
cargo fmt --check
cargo clippy -p morphofly-core --lib --tests -- -D warnings

# 编译：一律走 GitHub Actions（掌机不本地编译）
tools/gpd_deploy_ci.sh --run <run-id>
```

三台设备并行验收：本机（无头）∥ gpd（真桌面 + 真麦）∥ pc（真桌面），
见 [`docs/v2/parallel-e2e.md`](docs/v2/parallel-e2e.md)。

---

## 已知问题

<details>
<summary><b>1. 内存 ≈294MB（要求 &lt;150MB）</b></summary>

Tauri 2 在 Linux 上必须用 WebKitGTK，它自己就吃 200MB+，和内存泄漏无关。
已做过一轮优化（关掉演示音频、跳过低频 emit），371MB → ≈294MB。
想达标只能把 UI 换成原生渲染（3–5 天），或调整验收标准 —— 待拍板。
</details>

<details>
<summary><b>2. 外放场景的自打断风险</b></summary>

代码里已接 AEC（sonora / AEC3），但**真机听觉验证不完整**：
耳麦场景正常；外放时如果回声没被消干净，她可能被自己的声音打断。
结构上不会自打断的配置已固化并在 `tools/verify_audio_topology.sh` 里断言。
</details>

<details>
<summary><b>3. 英文项目名识别不准（已有三层兜底）</b></summary>

`morphofly` / `diemeng` 这类英文词对中文 ASR 不友好
（实测：`mosofly` / `魔方fly` / `move_forward`）。**说中文名「蝶梦」100% 正确。**

兜底：项目别名模糊匹配 → 任务文本纠正 → 专有名词本地纠正表。
试过但无效：给 StepFun 转写传热词 prompt（服务端静默忽略）。
</details>

<details>
<summary><b>4. Windows 只做到「能编」</b></summary>

能编译、能打 NSIS 安装包，**运行时没在真 Windows 上验过**。
已知坑：数据目录只读 HOME、拉不起 npm 装的 `dsh.cmd`、平台代码未隔离，
详见 [`docs/v2/windows.md`](docs/v2/windows.md)。建议先把 productName 改成纯 ASCII。
</details>

<details>
<summary><b>5. 干活 agent 冷启动约 8 秒</b></summary>

DSH 的 ACP 进程第一次 `initialize` 实测 8.04s。
所以「派活 → 听到结果」通常 10 秒以上；这段时间是它在启动和干活，不是语音链路卡。
</details>

---

## 许可证

[Apache-2.0](LICENSE)
