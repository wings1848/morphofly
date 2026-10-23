# 进度 · 蝶梦语音智能体 V1

更新：2026-10-24 02:30 (+08:00)
run: `20261023-2339-v1-dsh-0`（standard 档 / desktop-app / inspect 模式）

## 一句话现状

**V1 的核心体验已经在真机上跑通**：说句话 → 派活给 DSH → 后台真执行 → 干完主动开口汇报。
还差 release 打包、真人 100 次采样、Rust 侧常驻唤醒词。

## 五个阶段的状态

| 阶段 | 状态 | 关键证据 | 说明 |
|---|---|---|---|
| 0 打通接缝 | ✅ 7/7 全绿 | `evidence/g0.*` | 方案骨架成立，不需换架构 |
| 1 能说话 | ✅ 核心通 | `evidence/g1.*` | 双工链路 + 六态小球 |
| 2 能派活 | ✅ | `evidence/g2.*` | 4 工具 + 任务表 + DSH 桥 |
| 3 能自动汇报 | ✅ | `evidence/g3.1` | 最硬的一条验收 |
| 4 唤醒词 | ⏳ 约 50% | `evidence/g4.*` | KWS 实测通过，Rust 常驻未接 |
| 5 打磨打包 | ❌ 未开始 | — | |

## 单测

```
cargo test -p diemeng-core --lib  → 169 passed, 0 failed
node --test app/frontend/test/    →   8 passed, 0 failed
```

## 代码结构

```
core/src/
  audio/        cpal 采集 + rodio 播放 + 环形缓冲     （60 单测）
  realtime/     统一事件 + StepFun WS + provider 适配 （55 单测）
  session/      六态状态机（纯逻辑）                  （13 单测）
  tasks/        任务表（原子写、重启不丢）            （ 8 单测）
  tools.rs      4 工具不变量 + 运行时断言             （ 6 单测）
  dispatch.rs   工具分发 + 结果清洗                   （ 6 单测）
  dsh/          ACP 客户端 + 任务桥                   （10 单测）
  spectrum.rs   realfft 64bin + RMS                   （ 9 单测）
app/src-tauri/  Tauri 2 壳 + IPC 命令 + 状态推送
app/frontend/   六态蓝色小球（原生 ESM，无打包器）
core/examples/  8 个可运行探针（每个接缝一个）
tools/          pc_ssh/pcsync/pcrun/pc_app/pc_states/pcscreenshot/dev-env
```

## 真机踩坑记录（全部已写进代码注释或文档）

| # | 坑 | 现象 | 修法 |
|---|---|---|---|
| 1 | StepFun `energy_awakeness_threshold` 默认 2500 太高 | 正常音量完全不触发 VAD，等 45s 无 response | 改 500 |
| 2 | StepFun 只支持 `server_vad` | 传 `none` 报错；开 VAD 时手动 commit 报错 | 只能推流让服务端切轮 |
| 3 | StepFun tools 要 OpenAI 嵌套格式 | 扁平格式报 `function.name is required` | `normalize_tools()` |
| 4 | 同一次工具调用被报两遍 | 会导致重复派活 | 按 call_id 去重 |
| 5 | **Tauri 2 缺 capabilities 时 IPC 全静默失败** | listen/invoke 全无反应、零报错 | 补 `capabilities/default.json` |
| 6 | CSP 没放行 ipc: | 同上 | `connect-src 'self' ipc: http://ipc.localhost` |
| 7 | 改前端后不重编译 | 画面不变 | `touch build.rs && cargo build` |
| 8 | KWin/Wayland 下 Tauri 窗口建不起来 | `Error 71 Protocol error` | `GDK_BACKEND=x11` + 关 dmabuf |
| 9 | 本机 MagicDNS 失效 | `ssh wings-pc` 解析不到 | `tools/pc_ssh.sh` 回落 Tailscale IP |
| 10 | 本机无 root、无 ALSA 头 | cpal 编译失败 | deb 解包到 `~/.local/sysroot` + `dev-env.sh` |
| 11 | 英文项目名对中文 ASR 不友好 | `diemeng` → `daimian`/`diamond`/`aimeng` | 已知项目表 + 追问 + 目录校验 |

## 下一步（按优先级）

1. **真麦克风 + 真扬声器 100 次延迟采样**（P1 正式口径）
2. **打断实测**：播放中喊「停」，量 stop 延迟（P2）
3. **Rust 侧接 sherpa-onnx 常驻 KWS**（验收 K1 严格测法）
4. **release 打包**（阶段 5）：`cargo tauri build`，量包体积（P5）
5. **真人完整验收**：打开电脑 → 不按键 → 喊小蝶 → 派活 → 关屏 → 听汇报
