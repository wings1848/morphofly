# adr · 方案决策记录

**status**: accepted
**approvers**: [lead, 用户（inspect 模式默认值）]
**date**: 2026-10-23

| # | 决策 | 备选 | 选择理由 | 被否原因 |
|---|---|---|---|---|
| ADR-1 | **Rust core + Tauri 2 壳 + WebView 画球** | Go+Wails；纯 Electron；纯 Web | 音频必须原生（AEC 参考信号）；Tauri 体积最小；core 可用 UniFFI 复用到移动端 | Go+gobind 与 cgo 音频库冲突（golang/go#35426）；Electron 内存 150MB+ 直接超标；Web 采音 AEC 断链 |
| ADR-2 | **实时语音先用 StepFun `stepaudio-3-realtime-preview`** | Qwen 百炼（首选但无 key）；Gemini（大陆不可用） | key 已在手、限免、协议与 OpenAI Realtime 同构、实测可用（facts F14–F17） | Qwen 需用户提供 DashScope key，暂缺；Gemini 需代理且违反 ToS |
| ADR-3 | **开发机用 wings-pc（Linux/CachyOS）** | 本机 US 云主机 | 本机无音频设备、无桌面（F1/F2）；wings-pc 有麦克风/扬声器/显示器/Tauri 依赖（F3–F8） | 本机无法跑音频与 GUI 实测 |
| ADR-4 | **前端不引入打包器**，原生 ES module + 静态目录 | Vite + TS；Svelte | 只画一个球，无需构建链；少一层故障面；`cargo tauri build` 直接吃静态目录 | 增加 3–5 分钟构建与版本漂移风险，收益不足 |
| ADR-5 | **音频格式定死 16kHz/mono/i16/20ms 上行，pcm16 下行** | 48k 原生；f32 | AEC/唤醒词/模型端全部要 16k；引擎侧只重采样一次；ESP32 可照抄 | 多格式分支会让 AEC 延迟估计崩 |
| ADR-6 | **provider 配置化**（TOML + 环境变量覆盖），协议层做统一事件枚举 | 直接写死 StepFun | 验收 A5「provider 可切换、0 行代码改动」 | — |
| ADR-7 | **DSH 接入走 ACP（`dsh --profile acp`）** | SDK profile；直接调工具 API | 已实测 initialize/session.new/session.prompt 全通（F10–F12） | SDK profile 是 stdio JSON-RPC，语义更偏 SDK 客户端，ACP 是编辑器/代理的稳定契约 |
| ADR-8 | **测试策略：可测逻辑与硬件隔离** | 全部靠真机手测 | 状态机/任务表/协议解析/频谱纯函数在 CI 可测；硬件路径用 example 二进制 + 远程命令留证据 | 全手测无法在 US 本机复现 |

## 被否方案（备查）

- **Go + Wails v3**：`gomobile bind` 对 cgo 有已知问题，而 cpal/rodio/webrtc 全要 cgo。
- **WebView 采音 + `echoCancellation: true`**：拿不到 AEC 参考信号；Linux WebKitGTK 默认关 media-stream；iOS AnalyserNode 返回全 0。
- **直接在云端做 ASR 再给 DSH**：多一跳延迟（+300–800ms），且失去打断能力。
