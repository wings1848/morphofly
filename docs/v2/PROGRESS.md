# MorphoFly V2 进展总览

**最后更新**：2026-10-25 00:20

## 一句话状态

**麦克风修好了 —— 第一次在真机上完成「喊小蝶 → 说任务 → 派活 → 主动汇报」全链路。现在在做 V3 悬浮窗 UI。**

## 当前指标

| 项目 | 数值 |
|---|---|
| 单元测试（默认） | **242 passed** |
| 单元测试（含 kws） | **243 passed** |
| 前端测试 | **13 passed** |
| e2e mock | 14/14 |
| **e2e 真机（语音链路）** | **13/14** |
| 编译警告 | 0 |
| clippy 警告 | 0 |
| CI | ✅ **绿（修好了长期红灯）** |
| GitHub Actions 构建 | ✅ 产出 deb 24MB / AppImage 97MB |

## 已完成（全部有实测证据）

### 0. 今天最硬的一块：麦克风（P0，卡了很久）

两个原因叠加，都修了：
1. 服务跑的是**旧 deb 里的二进制**（没有 PulseAudio 后端代码）—— 改完代码要
   先 `ls -la /proc/$(pgrep -x morphofly-app)/exe` 确认跑的是哪个文件。
2. `.asoundrc` 里 `device "default"` 在 pipewire-pulse 上不存在（`No such entity`）。

现在：`音频后端：PulseAudio` + `KWS 常驻监听已启动`。
详见 `docs/v2/gpd-voice-e2e.md`。

### 1. 修复 12 个 bug（其中 7 个是「假通过」）

| # | 问题 | 类型 |
|---|---|---|
| 1 | 模型编造目录列表（6 个全不存在）| 🔴 致命 |
| 2 | 模型凭空捏造路径（Code_Project/M1）| 🔴 致命 |
| 3 | 模型假装干活（说「好查着呢」不调工具）| 🔴 致命 |
| 4 | cancel_task 不真停 DSH | 🟠 欺骗 |
| 5 | append 不发给运行中 DSH | 🟠 欺骗 |
| 6 | set_context(model) 只改字段 | 🟠 假功能 |
| 7 | set_context(agent) 只改字段 | 🟠 假功能 |
| 8 | ACP 不按 sessionId 过滤 | 🟠 多agent地基 |
| 9 | 打断不停云端生成 | 🟡 浪费token |
| 10 | 每任务 spawn（慢 2.8s）| 🟡 性能 |
| 11 | 兜底纠正判据错 → 一句话派 4 次任务 | 🔴 致命 |
| 12 | App 重启把历史任务结果重念一遍 | 🟠 体验 |

「假通过」的共同模式：**写了状态变更但没通知外部系统**，只改本地副本。

### 2. P0 体验
- ✅ 系统托盘（左键显示/隐藏、右键退出）—— 之前只能 pkill
- ✅ 小球自动贴右下角
- ✅ 拖动 + 松手自动贴边
- ✅ 文案极简（23 字 → 5 字）
- ✅ 复杂任务先问确认

### 3. 多 agent
- ✅ `set_context` 第 5 个工具（白名单+黑名单不变量）
- ✅ 项目别名表（中文 → 真实路径）
- ✅ agent 真实切换（dsh 可用；codex 需登录）
- ✅ 模型切换（JSON 数组格式，DSH 校验）
- ⚠️ 语音切换不可用（模型不调工具，协议无用户转写）

### 4. AEC
- ✅ sonora AEC3 接入音频链路
- ✅ 19 个测试全绿，合成信号 34.5dB
- ✅ 双讲人声保留 43-55%
- ⏳ 真机听觉验证（等麦克风桥接修好）

### 5. 进程复用
- ✅ 冷启动 3698ms → 复用后 **1102ms**
- ✅ 每任务省 2.6 秒

### 6. 测试基础设施
- ✅ 15 条 e2e 用例 + TTS 合成 + 裸 PCM 注入
- ✅ 真机驱动 `e2e_drive.rs`
- ✅ KWS 前置门打通（本机建 libstdc++ 软链接解决）

### 7. Computer Use
- ✅ 接入 `computer-use-linux`（SSH 桥接到 gpd）
- ✅ 18 个工具可用：列窗口/点击/输入/截图
- ✅ 注册到 DSH profile

### 8. CI/CD
- ✅ GitHub Actions 构建成功（deb 24MB）
- ✅ 产物在 gpd（N6000 无 AVX）能运行

## 关键发现（推翻了 6 个原有判断）

| 我说过 | 实际 |
|---|---|
| AEC 只有 1.9dB，方案不行 | 测试信号太规则，实际 34.5dB |
| codex 实测可用 | 复用 dsh 进程池的假象 |
| 谐音误唤醒是产品 bug | KWS 干净，e2e 绕过了它 |
| wings-pc 桌面已就绪 | 是登录器进程 |
| 缺陷 #4 需 300 行重构 | 50 行省 2.6 秒 |
| wings-pc 能测 AEC | **它没有麦克风硬件** |

## 第 2 轮（2026-10-24 深夜）又修了什么

| 问题 | 真因 |
|---|---|
| 右下角又歪 212px | 启动时字幕抢在贴边前调 orb_tip，归位点拿屏幕正中当基准 |
| 模型只回「好，查着呢」不派活，且没被纠正 | 纠正判据只认云端 `speech_started`，本地 VAD 提前 commit 时那条不来 |
| 项目名被 ASR 听错就直接放弃 | `delegate` 里 dir 不存在只追问，没先查别名表 |
| **Windows 上仓库根本检不出来** | 根目录有个被跟踪的空文件叫 `...`（Windows 文件名不能以点结尾） |
| **仓库 423MB 是构建产物撑的** | `ci-dist/` 把 AppImage + 解开的 squashfs-root 提交了 |

## Windows 兼容（用户 2026-10-25 要求）

**能编、能打 NSIS 安装包、三个必然挂的地雷已拆；运行时没在真 Windows 上验过。**

1. 数据目录只读 `HOME` → `USERPROFILE` 回退（+3 测试）
2. `Command::new("dsh")` 拉不起 npm 装的 `dsh.cmd` → `spawn_program` 按 PATHEXT 试 + `cmd /C` 兜底
3. 平台代码未隔离 → `force_x11_backend` cfg 到 linux；cpal 的 pulseaudio 依赖本身是 target 门控的

闸门：CI 加 `build-windows` job；release 加 Windows NSIS 打包 job。
详见 `docs/v2/windows.md`（含 5 条没验证的风险）。

## 剩余工作

### P0 阻塞：麦克风桥接
gpd 上 ALSA→PipeWire 录音流报 `No such entity`。
已改 cpal 用 PulseAudio 后端（代码已提交），**待 gpd 编译验证**。

### P1 待做
1. AEC 真机听觉验证（喊「停」能否打断）
2. 语音切换 agent（需协议加用户转写，或改托盘菜单）

### P2 可选
1. 完整多 session 进程池
2. 内存优化（等用户拍板）
3. ESP32 / 安卓 / Web 端

## 硬件与设备

| 设备 | 用途 | 限制 |
|---|---|---|
| wings-tx-us1（本机）| 开发、单测、CI | 无桌面无音频 |
| wings-pc | GUI 验证 | **无麦克风硬件** |
| **wings-gpd** | **真机全流程** | 有麦克风+桌面，但 N6000 慢 |

## 文档索引

| 想知道什么 | 看哪个 |
|---|---|
| 侦察结论 | `docs/v2/scout-full.md` |
| 体验问题清单 | `docs/v2/review.md` |
| AEC 状态 | `docs/v2/aec-status.md` |
| AEC 真机测试 | `docs/v2/evidence/aec-realtime-test.md` |
| e2e 发现 | `docs/v2/e2e-status.md` |
| 不会误唤醒的证明 | `docs/v2/no-false-wake-proof.md` |
| 工具真实性审计 | `docs/v2/tool-audit.md` |
| 教训总结 | `docs/v2/lessons.md` |
| 硬件事实 | `docs/v2/hardware-facts.md` |
| Computer Use 接入 | `docs/v2/computer-use-setup.md` |
| 语音切换设计 | `docs/v2/voice-switch-design.md` |
| KWS 边界情况 | `docs/v2/kws-edge-case.md` |
