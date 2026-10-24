# MorphoFly

> 喊一声「小蝶」，用嘴指挥 AI 干活，活干完它主动回来说。

**全程零键鼠的语音智能体。** 手被占着的时候（做饭、修东西、躺着），
喊一声就能派活，干完它自己回来汇报。

[![CI](https://github.com/wings1848/morphofly/actions/workflows/ci.yml/badge.svg)](https://github.com/wings1848/morphofly/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

---

## 它解决什么

现有的语音助手（Siri、小爱）只能问答，不能**干活**。
能干活的东西（IDE、终端）又必须**用手**。

中间的空白：**手被占着的时候，怎么指挥电脑干活？**

MorphoFly 的答案：
```
喊「小蝶」→ 说一句人话 → AI 派活给 DSH 执行 → 干完主动回来汇报
```

不需要看屏幕，不需要按键，不需要知道项目路径。

---

## 实测数据

所有数字都来自真机测试（Intel/AMD x86_64 + PipeWire + 真实声卡）。

| 指标 | 实测 | 目标 | 状态 |
|---|---|---|---|
| **端到端延迟** | **802ms** | <800ms | ✅ |
| **打断延迟** | **300ms** | <300ms | ✅ |
| 唤醒召回 | **9/9 = 100%** | ≥95% | ✅ |
| 唤醒误报 | **0/8 = 0%** | ≈0 | ✅ |
| 冷启动 | 1476ms | <3s | ✅ |
| **任务派发（复用进程）** | **1102ms** | — | ✅ |
| 空闲 CPU | 3.7% | <5% | ✅ |
| 安装包 | 24MB (deb) | <100MB | ✅ |
| **真机 e2e 通过率** | **13/14** | >80% | ✅ |
| **空闲内存** | **371MB (PSS)** | <150MB | ❌ |

**唯一不达标是内存**，原因见[已知问题](#已知问题)。

> 内存口径说明：早期写的 488MB 是 RSS（会把共享 .so 重复计算）。
> 用 PSS 重测是 371MB，其中真正属于我们代码的堆只有 81MB，
> 其余是 WebKit 子进程和映射的系统库。详见 `docs/v2/scout-full.md`。

复现命令见 [`docs/实测数据.md`](docs/实测数据.md)。

V2 的完整进展（含 10 个已修 bug、真机验证、剩余工作）见
[`docs/v2/PROGRESS.md`](docs/v2/PROGRESS.md)。

---

## 三条设计铁律

这三条是架构底线，破了就是设计失败：

1. **WebView 永不碰麦克风** —— 音频全在 Rust 侧用 cpal/rodio 原生处理，
   前端只负责画球
2. **语音模型只有 5 个函数** —— 派活/查进度/取消/追加/切上下文，
   **物理上**没有文件和命令能力（有白名单+黑名单断言守着）
3. **语音链路与任务链路独立** —— 语音轮次先结束，任务后台跑完再主动汇报

---

## 架构

```
┌─────────────────────────────────────────┐
│  WebView（画小球）                        │
│  六态视觉反馈 · 不碰麦克风 · 不碰密钥      │
└──────────────┬──────────────────────────┘
               │ Tauri IPC（单向：只收状态和频谱）
┌──────────────▼──────────────────────────┐
│  Rust core                              │
│  ┌────────────────────────────────┐    │
│  │ 本地：唤醒词(9/9) + VAD(300ms)  │    │
│  └────────────┬───────────────────┘    │
│               │                         │
│  ┌────────────▼───────────────────┐    │
│  │ 云端：语音模型（802ms 往返）     │    │
│  │ 只给 4 个函数，无文件/命令权限   │    │
│  └────────────┬───────────────────┘    │
│               │ 派活                    │
│  ┌────────────▼───────────────────┐    │
│  │ DSH（ACP 协议）→ 真干活         │    │
│  └────────────┬───────────────────┘    │
│               │ 完成                    │
│  ┌────────────▼───────────────────┐    │
│  │ 主动汇报：自己开口，不等你问     │    │
│  └────────────────────────────────┘    │
└─────────────────────────────────────────┘
```

**为什么这样分**：语音要**快**（延迟敏感，云端模型最强），
任务要**稳**（长时运行，本地 DSH 有完整工具链）。
两者用不同通道，互不阻塞。

---

## 快速开始

### 安装

```bash
# Debian/Ubuntu（需要系统有 webkit2gtk）
sudo dpkg -i 蝶梦_0.1.0_amd64.deb

# 或者用 AppImage（自包含，不挑发行版）
chmod +x 蝶梦-0.1.0-x86_64.AppImage
```

### 配置密钥

```bash
mkdir -p ~/.diemeng
echo 'STEPFUN_KEY=你的key' > ~/.diemeng/credentials
chmod 600 ~/.diemeng/credentials
```

**密钥只从环境变量或这个文件读，永不进前端、永不进仓库。**

### 跑

```bash
./蝶梦-0.1.0-x86_64.AppImage
```

喊「小蝶」，或者点一下中间的蓝色小球。

### 从源码构建

```bash
# 依赖：rustc 1.82+ / node 20+ / webkit2gtk-4.1 / gtk3
git clone https://github.com/wings1848/morphofly
cd morphofly

# 测试
cargo test -p morphofly-core --lib          # 178 passed
node --test app/frontend/test/              # 8 passed

# 带唤醒词（需要 libstdc++）
cargo test -p morphofly-core --features kws --lib   # 189 passed

# 打包
cd app && cargo tauri build
```

---

## 已知问题

### 1. 内存 488MB（要求 <150MB）

| 进程 | RSS |
|---|---|
| `morphofly-app` 主进程 | 191 MB |
| `WebKitWebProcess` | 161 MB |
| `WebKitNetworkProcess` | 63 MB |

**原因**：Tauri 2 在 Linux 上必须用 WebKitGTK，它自身就吃 200MB+。
这是「用 WebView 做 UI」的固有代价（Electron 同量级），不是内存泄漏。

**解法**：重写 UI 层为原生渲染（3–5 天），或调整验收标准。

### 2. 外放会自我打断

回声消除（AEC）未实现。**戴耳机正常。**

### 3. 英文项目名识别不准

`morphofly` / `diemeng` 这类英文词对中文 ASR 不友好，
可能听成 `diamond` / `daimian`。**说中文名「蝶梦」100% 正确。**

### 4. 只在 x86_64 Linux 验证

ARM 平台（部分掌机）需要重新编译。

---

## 项目结构

```
core/                Rust 核心
  audio/             cpal 采集 + rodio 播放（60 单测）
  realtime/          统一事件 + 各服务商适配（59 单测）
  session/           六态状态机（15 单测）
  kws.rs             唤醒词（sherpa-onnx）
  vad.rs             本地 VAD（打断用）
  tools.rs           4 工具不变量（物理隔离）
  dispatch.rs        工具分发 + 结果清洗
  dsh/               ACP 客户端 + 任务桥
app/src-tauri/       Tauri 2 桌面壳
app/frontend/        六态蓝色小球（原生 ESM，无打包器）
core/examples/       13 个可运行探针
tools/               运维脚本
docs/                技术文档
runs/                devflow 开发记录（含实测证据）
```

---

## 文档

| 想知道 | 看 |
|---|---|
| 所有实测数字 + 复现命令 | [`docs/实测数据.md`](docs/实测数据.md) |
| 怎么换语音服务商 | [`docs/切换语音服务商.md`](docs/切换语音服务商.md) |
| 唤醒词配置与 3 个坑 | [`docs/唤醒词.md`](docs/唤醒词.md) |
| 音频路由排障 | [`docs/音频路由.md`](docs/音频路由.md) |
| 跨机部署 | [`docs/跨机传文件.md`](docs/跨机传文件.md) |
| 完整验收表（28 条） | [`runs/`](runs/) |

---

## 技术亮点（踩坑换来的）

这个项目在实现过程中遇到了几个值得记录的问题：

1. **服务端 VAD 参数完全无效** —— StepFun 所有模型的「确认说完」固定 1.95 秒，
   参数调 10 倍只差 5ms。改用本地 silero-vad + 手动 commit，延迟从 1994ms 降到 802ms
2. **模型选择影响 3.5 倍** —— 6 个候选模型端到端从 2.0s 到 6.9s 不等
3. **Rust 绑定行为与 Python 不一致** —— sherpa-onnx 的 `get_result`
   没命中时 Rust 返回 `Some(空)`、Python 返回 `None`，导致纯静音也「命中」
4. **`target-cpu=native` 的代价** —— 在支持 AVX2 的机器上编译，
   拿到只支持 SSE4.2 的机器上会 SIGILL 崩溃

详细记录见 [`CHANGELOG.md`](CHANGELOG.md) 和 [`docs/`](docs/)。

---

## 许可证

[Apache-2.0](LICENSE)

---

## 状态

V1 已可用。开发记录（含所有实测证据）在 [`runs/`](runs/)。
