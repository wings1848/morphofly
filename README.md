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

所有数字都来自真机测试（wings-gpd：Intel N6000 + PipeWire；语音用虚拟音源注入，
除「空气」那一段外整条链路都是真的）。

| 指标 | 实测 | 目标 | 状态 |
|---|---|---|---|
| **端到端延迟** | **802ms** | <800ms | ✅（早期实测）|
| **打断（本地反应）** | **<10ms**（VAD 判定人声 → 停播） | <300ms | ✅ 真机抓到完整打断 |
| 唤醒召回 | **9/9 = 100%** | ≥95% | ✅ |
| 唤醒误报 | **0/8 = 0%** | ≈0 | ✅ |
| 冷启动 | 1476ms | <3s | ✅ |
| **任务派发（复用进程）** | **1102ms** | — | ✅ |
| 空闲 CPU | 3.7% | <5% | ✅ |
| **悬浮球拖动** | 真机 A/B 两条用例，误差 **1px** | — | ✅ |
| 单元测试 | **271**（含唤醒词 **282**） | — | ✅ |
| 前端测试 | **13** | — | ✅ |
| CI | ✅ 全绿（含 Windows 检查、可下载 Linux 产物）| — | ✅ |
| 安装包 | 24MB (deb) / 97MB (AppImage) | <100MB | ✅ |
| **空闲内存** | **≈294MB (PSS)** | <150MB | ❌ |

**唯一不达标是内存**，原因见[已知问题](#已知问题)。

> 内存口径说明：早期写的 488MB 是 RSS（会把共享 .so 重复计算）。
> 用 PSS 重测从 371MB 降到 ≈294MB，其中真正属于我们代码的堆只有几十 MB，
> 其余是 WebKit 子进程和映射的系统库。详见 `docs/v2/scout-full.md`。

复现命令见 [`docs/实测数据.md`](docs/实测数据.md)，
验收脚本清单见 [`docs/v2/HOW-TO-VERIFY.md`](docs/v2/HOW-TO-VERIFY.md)。

V3 的完整进展（悬浮窗/拖动/CI/并行 e2e）见
[`docs/v2/PROGRESS.md`](docs/v2/PROGRESS.md)。

---

## V3 · 悬浮窗与交付方式（2026-10-25）

用户提的要求：**默认置顶 / 启动在屏幕右下角 / 参考 360 悬浮窗 / UI 和动效优化 /
构建用 GitHub Actions / 用 computer use 看真机效果**。全部落地：

- **148×148 透明置顶小球**，六态视觉（待机/唤醒/聆听/思考/说话/异常），
  启动贴右下角，气泡字幕向左展开、收起后右边缘不偏（KWin 右边缘越界会拉回窗口）。
- **拖动**：系统原生 `startDragging()` 在 KWin/XWayland 上实测**完全不生效**
  （8 秒采样零位移），改成 Rust 每 16ms 读全局指针搬窗口；松手只吸左右边、
  上下位置保留。真机验收误差 1px，见 [`docs/v2/orb-drag.md`](docs/v2/orb-drag.md)。
- **编译走 GitHub Actions**：`ci.yml` 的 `build-app-linux` 产出
  `morphofly-app-linux-x64.tar.gz`（二进制 + 唤醒词模型 + 版本标记），
  部署用 `tools/gpd_deploy_ci.sh`，掌机不再本地编译。
  见 [`docs/v2/ci-build.md`](docs/v2/ci-build.md)。
- **三台设备并行验收**：本机（无头单测）∥ gpd（真桌面+真麦）∥ pc（真桌面），
  `tools/e2e_matrix.sh --with-pc` 一条命令跑多车道。
  见 [`docs/v2/parallel-e2e.md`](docs/v2/parallel-e2e.md)、[`docs/v2/pc-lane.md`](docs/v2/pc-lane.md)。
- **被动升级**：CI 产物 sha 可追溯（`build.txt`），`systemctl --user restart` 即生效。

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

# 测试（本机无桌面也能跑）
cargo test -p morphofly-core --lib                    # 271 passed
cargo test -p morphofly-core --features kws --lib     # 282 passed（唤醒词，需 libstdc++）
node --test app/frontend/test/                        # 13 passed

# 编译应用：**推荐走 CI**（用户要求；本机/掌机都不本地编）
#   push 后 GitHub Actions 的 build-app-linux 产出可下载产物
tools/gpd_deploy_ci.sh --run <run-id>     # 下载 + 部署到掌机 + 重启服务

# 本机想本地编也行（需要 webkit2gtk-4.1 / gtk3 / libasound）
cd app && cargo tauri build               # 产出 deb / AppImage
```

---

## 已知问题

### 1. 内存 ≈294MB（要求 <150MB）

| 进程 | 说明 |
|---|---|
| `morphofly-app` 主进程 | Rust core + GitHub 出的二进制 |
| `WebKitWebProcess` | 画小球与气泡字幕 |
| `WebKitNetworkProcess` | WebView 的网络进程 |

**原因**：Tauri 2 在 Linux 上必须用 WebKitGTK，它自身就吃 200MB+。
这是「用 WebView 做 UI」的固有代价（Electron 同量级），不是内存泄漏。
已经做过一轮优化（关掉前端演示音频、跳过低频 emit），从 371MB 降到 ≈294MB。

**解法**：重写 UI 层为原生渲染（3–5 天），或调整验收标准 —— **等用户拍板**。

### 2. 自打断风险（AEC 在测试台上验不了）

代码里已接 AEC（sonora / AEC3），但**真机听觉验证仍然没做**：
gpd 和 pc 都没有能出声的扬声器，测试用虚拟音源注入，App 自己的输出
根本不会回灌到采集里 —— 没有回声可消，也就测不出 AEC 好坏。

- 结构上不会自打断的配置已固化并在 `tools/verify_audio_topology.sh` 里断言
  （默认 sink 必须是物理设备，不能是虚拟 sink）。
- 真实场景（外放 + 麦克风）**戴耳机正常，外放待验证**。

详见 [`docs/v2/audio-topology.md`](docs/v2/audio-topology.md)。

### 3. 英文项目名识别不准（已加三层兜底）

`morphofly` / `diemeng` 这类英文词对中文 ASR 不友好
（真机实测：`morphofly` → `mosofly` / `魔方fly` / `move_forward`）。
**说中文名「蝶梦」100% 正确。**

已经做的兜底（都不依赖供应商）：
1. 项目别名**模糊匹配**（编辑距离，听错一两个字也能对上）
2. 任务文本**纠正**（`mosofly` → `morphofly` 再交给 DSH）
3. 专有名词**本地纠正表**（`vimi`/`微米` → `README`）

试过但没用：给 StepFun 转写加 OpenAI 风格 `prompt` 热词提示，
服务端**静默忽略**（探针实测，见 [`docs/v2/asr-project-mishearing.md`](docs/v2/asr-project-mishearing.md)）。

### 4. 平台支持现状

| 平台 | 状态 |
|---|---|
| Linux x86_64 | ✅ 真机验证（wings-gpd：唤醒/派活/汇报/悬浮窗/拖动/打断全链路；wings-pc 作为第二条桌面车道） |
| Windows x86_64 | ⚠️ **能编、能打 NSIS 安装包，运行时没在真 Windows 上验过**。已修三个必然挂的问题（数据目录只读 HOME、拉不起 npm 装的 `dsh.cmd`、平台代码未隔离），详见 [docs/v2/windows.md](docs/v2/windows.md) |
| 其它 | ❌ 未验证（ARM 需要重新编译） |

Windows 上最大的未知是**中文安装路径 + sherpa-onnx 加载唤醒词模型**：
`productName` 是「蝶梦」，安装目录带中文，模型加载可能失败。要上 Windows 建议先把
productName 改成纯 ASCII。

### 5. 打断的自动化复现有已知限制

机制是通的（真机抓到过完整打断：停播 + 取消云端 + 计数 + 回聆听态），
但自动化脚本在虚拟音源上**不稳定**：注入链路有 1–2 秒延迟，
而模型对短问题的回答只有 2–4 秒，经常错过「正在说话」的窗口。
真机手感建议接耳机人工说「停」验证。见 [`docs/v2/bargein.md`](docs/v2/bargein.md)。

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
| **验收脚本清单（怎么验这个项目）** | [`docs/v2/HOW-TO-VERIFY.md`](docs/v2/HOW-TO-VERIFY.md) |
| 悬浮球拖动（为什么原生拖不动） | [`docs/v2/orb-drag.md`](docs/v2/orb-drag.md) |
| 编译走 CI + 一键部署 | [`docs/v2/ci-build.md`](docs/v2/ci-build.md) |
| 三台设备并行 e2e | [`docs/v2/parallel-e2e.md`](docs/v2/parallel-e2e.md) / [`docs/v2/pc-lane.md`](docs/v2/pc-lane.md) |
| 打断（barge-in）实现与限制 | [`docs/v2/bargein.md`](docs/v2/bargein.md) |
| 测试台音频流（防自打断） | [`docs/v2/audio-topology.md`](docs/v2/audio-topology.md) |
| ASR 听错项目名怎么兜底 | [`docs/v2/asr-project-mishearing.md`](docs/v2/asr-project-mishearing.md) |
| Windows 兼容现状与风险 | [`docs/v2/windows.md`](docs/v2/windows.md) |
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

**V3 可用**：真机全链路（喊「小蝶」→ 说任务 → 派活 → DSH 干活 → 会话关了自己重开回来汇报）
实测通过；悬浮窗置顶/右下角/拖动/六态动效完成；编译走 GitHub Actions；
本机 ∥ gpd ∥ pc 三台设备并行验收。

当前开发记录（含所有实测证据）在 [`runs/`](runs/)，
最新一轮：`runs/20261024-2208-ui-ux-360/verify-round13.md`。

**未完成/待拍板**：Windows 运行时真机验证、AEC 真机听觉验证、内存优化（≈294MB）、
仓库历史瘦身（.git 245MB，改写历史不可逆）。
