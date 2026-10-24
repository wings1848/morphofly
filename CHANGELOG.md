# 更新日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 新增（V3，2026-10-24～25）

**悬浮窗（用户要求：默认置顶 / 启动右下角 / 参考 360 悬浮窗）**
- 148×148 透明置顶小球，六态视觉（S0–S5），启动贴右下角
- 气泡字幕向左展开，收起时右边缘不偏（KWin 不允许右边缘越界）
- **拖动**：Rust 每 16ms 读全局指针搬窗口（原生 `startDragging()` 在
  KWin/XWayland 上实测无效），松手只吸左右边、垂直位置保留
- 前端日志桥 `orb_log` / `orb_metrics`：透明无边框窗口没 devtools，排障靠 journal

**编译与交付**
- CI 新增 `build-app-linux`：产出 `morphofly-app-linux-x64.tar.gz`
  （二进制 + 唤醒词模型 + `build.txt` 版本标记）
- `tools/gpd_deploy_ci.sh`：下载 CI 产物 → 部署到设备 → 重启服务（一条命令）
- **掌机不再本地编译**（用户要求），CI 单次约 3–4 分钟

**并行验收**
- `tools/e2e_matrix.sh`：本机（无头）∥ gpd（真桌面+真麦）∥ pc（真桌面）三条车道，
  `--with-pc` 启用 pc 车道，默认不花钱、不重启服务
- gpd 上新增 `tools/verify_orb_drag.sh`（拖动 A/B 两条用例，真机误差 1px）
- 新增 `tools/verify_bargein.sh`（打断）、`tools/verify_audio_topology.sh`（音频流防自打断）

**语音链路**
- 用户转写事件接上（`conversation.item.input_audio_transcription.*`），
  模型只说不做时本地拿用户原话代派
- ASR 听错兜底三层：项目名模糊匹配、任务文本纠正、专有名词本地纠正表
- 启动自愈：上次没跑完的 `running` 任务标成「被中断」并被主动汇报
- DSH 握手超时按慢机器放宽（initialize 30s→75s，建会话 50s→100s）

### 修复（V3）
- **干完了不汇报**：会话与汇报守卫各自 `TaskStore::open(同一路径)` 建了两个实例，
  守卫看不到会话派出的任务 → 改成共用同一份 store/dispatcher
- **只读任务被反问确认**：「查看 /path 的 README」因路径太长（57 字）被判复杂任务
  → 只读动词 + 无改动动词时直接派
- **拖动**：原生拖动无效 + 松手强拉回底部 → 改位置跟随 + 只吸水平边
- **打断路径 panic**：async 任务里 `block_on` → 推流任务死掉、麦克风静默失效 → 改 `spawn`
- **「说完」判定 200ms 太短**：换气切句 → `ongoing response already exists` → 改 400ms
- **打断判定**：从「播放队列有音频」扩成「队列有音频 或 状态机在思考/说话」，
  并把两条打断路径的计数都补上（2 秒去重，一次开口只记一次）
- **journal 写入 ANSI 颜色码**：脚本 grep 不到状态、`-o json` 解析踩坑 → 只在终端上色
- **服务启动崩在 GTK**：单元里写死 `XAUTHORITY`，重新登录后失效
  → `tools/gpd_run_app.sh` 启动时现找；验收脚本也会自动发现
- **Windows 构建失败**：兜底取消用了 kws 门控变量 → 改未门控的 `m_machine`

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

### 修复（V2.3，2026-10-25）

- **KDE 启动弹黑框 +「待机」黑气泡压球**：`orb_state` 没有显式字幕时回退成状态名，
  前端启动就把它当字幕显示 4.2 秒（窗口同时向左撑开，看起来就是一个黑框）。
  → 后端不再拿状态名当字幕；前端启动不再显示初始字幕、空字幕一律不渲染。
  真机回归：`tools/verify_startup_clean.sh`（T1–T4），见 `docs/v2/startup-clean.md`

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
- **内存 ≈294MB (PSS)** 超标（要求 <150MB）—— 需用户拍板：
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
