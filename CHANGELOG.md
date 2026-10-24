# 更新日志

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### 待处理
- **内存 488MB** 超标（要求 <150MB）—— WebKitGTK 固有开销，
  要么接受（改标准到 600MB），要么重写 UI 层为原生渲染（3–5 天）
- 真人完整验收（需要用户戴耳机跑一遍）
- 回声消除（AEC），当前外放会自我打断
- ARM 平台支持（wings-gpd 是 x86_64，但其他掌机可能是 ARM）

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
