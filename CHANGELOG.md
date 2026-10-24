# 更新日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增（V2）
- **多 agent 支持**：`set_context` 第 5 个工具，可切换 agent / 项目 / 模型
  - agent 表可配置，内置 dsh 和 codex（codex 走 `@zed-industries/codex-acp` 适配器）
  - 项目别名表 `~/.diemeng/projects.toml`，中文别名映射到真实路径
  - 模型切换走 ACP `session/set_config_option`（值必须是 JSON 数组格式）
- **回声消除（AEC）**：接入 sonora（纯 Rust AEC3 移植）
  - 合成信号抑制 34.5dB，双讲人声保留 43–55%
  - 帧长不变（320 样本内部拆 2×160 子帧），零额外延迟
  - 可运行中开关
- **进程复用**：任务结束后保留已握手的 DSH 进程
  - 派发耗时 3698ms → **1102ms**（省 2.6 秒）
- **系统托盘**：左键显示/隐藏，右键菜单（含切换 agent）
- **小球自动贴边**：启动贴右下角，拖动松手按位置贴最近边
- **意图确认**：复杂任务先问「要我…吗」，简单任务直接派
- **Computer Use 接入**：`computer-use-linux` 通过 SSH 桥接到有桌面的机器

### 修复（V2，10 个 bug，7 个是「假通过」）
- **模型编造结果**：说「目录有 datasets/models/...」但那些目录不存在
  → 重写提示词 + 代码兜底纠正
- **模型捏造路径**：凭空生成 `/home/wings/Code_Project/M1`
  → 三层防御，校验 task 里所有绝对路径必须存在
- **模型假装干活**：只说「好，查着呢」但不调工具
  → 检测到就注入系统提醒重做（只纠正一次防循环）
- **cancel_task 不真停 DSH**：只改本地 TaskStatus
  → oneshot 信号 + RAII 清理，真的杀进程
- **append_requirement 不发给运行中 DSH**：只拼本地 goal
  → 按状态区分，Running 时如实说「要停了重来吗」
- **set_context(model) 只改字段**：bridge 里零 model 代码
  → 真的调 ACP API，失败时如实告知
- **set_context(agent) 只改字段**：spawn 用固定命令
  → 按 agent 选命令 + 进程池按 agent 隔离
- **ACP 不按 sessionId 过滤**：多会话会串台
  → 用 await 而非 try_lock
- **打断不停云端生成**：只 stop_now() 不 cancel_response()
  → 两处都补上
- **每任务 spawn 一个进程**：白等 2.8 秒
  → 常驻进程复用

### 修复（音频）
- **cpal 优先用 PulseAudio 后端**：绕开 ALSA→PipeWire 录音流桥接 bug
  （`PulseAudio: Unable to create stream: No such entity`）

### 变更
- 不变量 I1 从「恰好 4 个工具」改成「名字白名单 + 禁用名黑名单」
  - 白名单保证不会多，黑名单保证 bash/read_file 等永久禁入
- `docs/实测数据.md` 的内存口径改为 PSS（RSS 会重复计算共享库）

### 待处理
- **内存 371MB (PSS)** 超标（要求 <150MB）—— 需用户拍板：
  接受现状，还是走原生重写（3–9 天，视觉会降级）
- **语音切换 agent 不可用** —— 模型不调 set_context 工具，
  且协议没开用户语音转写所以宿主也拿不到原话。暂用托盘菜单替代
- **AEC 真机听觉验证** —— 需要真实人声（喊「停」能否打断）
- ARM 平台支持

## [0.1.0] - 2026-10-24

首个可用版本。

### 新增

**语音交互**
- 喊「小蝶」唤醒，召回 9/9、误唤醒 0/8
- 六态视觉反馈（待机/唤醒/聆听/思考/说话/异常）
- 点小球或喊唤醒词进入通话
- 说话打断：本地 VAD 判定，端到端 300ms
- 多轮对话，上下文保持

**任务执行**
- 4 个语音工具（派活/查进度/取消/追加），物理上没有文件/命令能力
- 派活给 DSH 执行，一次任务一个会话
- 任务表持久化，重启不丢
- **主动汇报**：活干完自己开口，不等用户问

**工程**
- Rust 核心 + Tauri 2 壳 + WebView 渲染小球
- 本地唤醒词（sherpa-onnx，只处理本地音频）
- 本地 VAD（silero-vad，643KB）
- 13 个可运行探针（每个接缝一个）
- 178 / 189 / 8 个测试全绿，0 编译警告

**分发**
- deb 22.6MB（依赖系统 WebKitGTK）
- AppImage 126MB（自包含，含唤醒词和 VAD 模型）

### 性能实测

| 指标 | 实测 | 目标 |
|---|---|---|
| 端到端延迟 | **802ms** | <800ms |
| 打断延迟 | **300ms** | <300ms |
| 冷启动 | 1476ms | <3s |
| 空闲 CPU | 3.7% | <5% |
| 空闲内存 | **488MB** | <150MB ❌ |

### 踩过的坑（写在这里避免以后重踩）

- **StepFun 服务端 VAD 参数完全无效**：所有模型「确认说完」固定 1.95 秒，
  参数调 10 倍差 5ms → 改用本地 VAD + 手动 commit，延迟从 1994ms 降到 802ms
- **模型选择影响巨大**：6 个候选模型端到端差 3.5 倍（2.0s vs 6.9s）
- **`turn_detection: null` 必须是真 null**，不能靠 `skip_serializing_if` 省略字段
- **sherpa-onnx Rust 绑定的 `get_result` 没命中时返回 `Some(空)`** 而 Python 返回 `None`
  → 纯静音也「命中」
- **Tauri 缺 capabilities 时 IPC 全静默失败**，没有任何报错
- **`import -window` 在 KWin 下截到过期图层**，必须用 spectacle 截全屏
- **`target-cpu=native` 编出来的包换机器会 SIGILL**（AVX 指令集不兼容）

### 已知限制

- 英文项目名对中文 ASR 不友好（`diemeng` → 听成 daimian/diamond/aimeng）
- 外放会自我打断（AEC 未做，戴耳机正常）
- 只在 x86_64 Linux 验证过
