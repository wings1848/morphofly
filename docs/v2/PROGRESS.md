# MorphoFly 进展总览

**最后更新**：2026-10-25 06:35

## 一句话状态

**V3 真机全链路跑通**：喊「小蝶」→ 说任务 → 派活 → DSH 真干活 → **语音会话关掉了它也能自己重开回来汇报**。
悬浮窗（置顶/右下角/拖动/六态动效）完成；编译改走 GitHub Actions；
本机 ∥ gpd ∥ pc 三台设备并行验收。
**音频设备可显式指定**（托盘菜单/配置文件/环境变量），不再被系统默认设备劫持。

## 当前指标

| 项目 | 数值 |
|---|---|
| 单元测试（默认） | **277 passed** |
| 单元测试（含 kws） | **288 passed**（CI 实测确认） |
| 前端测试 | **13 passed** |
| CI | ✅ **全绿**（测试 / 含唤醒词 / Windows 检查 / 密钥扫描 / build-app-linux）|
| 真机全链路 | ✅ 唤醒 1 / 派活 1 / DSH 会话 1 / 主动汇报 1 |
| 悬浮球拖动 | ✅ A/B 两条真机用例，误差 1px |
| 打断（barge-in） | ✅ 真机抓到完整打断（interruptions=1；本地反应 <10ms）|
| 音频流防自打断 | ✅ `tools/verify_audio_topology.sh` PASS（`--live` 实测 0 打断）|
| 并行车道 | **3 条**：local（无头）/ gpd（真桌面+真麦）/ pc（真桌面）|
| 编译方式 | 全部 GitHub Actions（单次 3–4 分钟），CI 产物 sha 可追溯 |
| 内存 | ≈294MB PSS（目标 <150MB，❌ 待用户拍板）|

## 里程碑（新 → 旧）

### V3.4 音频设备可选（2026-10-25 06:35）
- 用户要求「让 morphofly 支持设置输入输出声音的设备」
- 托盘新增 **输入设备（麦克风）/ 输出设备（耳机/音箱）** 两个子菜单，选完写进配置
- 配置 `[audio] input_device/output_device` + 环境变量 `MORPHOFLY_INPUT_DEVICE/OUTPUT_DEVICE`
- 名字匹配：不区分大小写子串（`USB` / `KTMicro` / `录音1` 都行），找不到回退默认并告警
- 输入设备改完**自动重启唤醒监听**（否则常驻采集流还录老设备）
- 真机踩坑总结见 `docs/v2/audio-device.md`（默认设备会被外部改回虚拟声卡/不出声喇叭）

### V3.3 打断验收 + 音频流排查（2026-10-25 05:40）
- 首次拿到打断真机证据：停播 + `response.cancel` + 计数 + 回聆听态
- 修 3 个 bug：打断路径 `block_on` panic、journal 写 ANSI 导致脚本失真、
  「说完」判定 200ms 太短导致 `ongoing response already exists`
- 按用户要求改语义：回复中收到新一句 → 先取消再干净开新轮；
  打断计数两条路径都算 + 2 秒去重（一次开口只记一次）
- 音频输入/输出流实测：App 播放→物理 sink、采集←虚拟 sink monitor，**结构上不会自打断**
- 新增：`tools/verify_bargein.sh`、`tools/verify_audio_topology.sh`、
  `docs/v2/bargein.md`、`docs/v2/audio-topology.md`

### V3.2 语音全链路修复（2026-10-25 04:40）
- 真机时间线：20:35 唤醒 → 20:36 DSH 会话 → 20:37 会话已关 → **自己重开 → 回报**
- 修 5 个真 bug：汇报不响（两个 TaskStore 实例）、只读任务被误解为复杂、
  ASR 听错项目名、ASR 听错专有名词、DSH 握手超时（高负载实测 22.8s > 30s 上限）
- 新增：ASR 模糊匹配 + 本地纠正表；`tools/gpd_voice_e2e.sh` 计数修正
- Windows 车道：pc 接成第二条真机桌面车道（`--with-pc`）
- 证据：`runs/20261024-2208-ui-ux-360/verify-round12.md`

### V3.1 拖动 + 编译走 CI + 并行 e2e（2026-10-25 03:45）
- 拖动：原生 `startDragging()` 在 KWin/XWayland 实测无效 → Rust 位置跟随；
  松手只吸水平边，垂直位置保留
- 编译：CI 新增 `build-app-linux` + `tools/gpd_deploy_ci.sh`（掌机不再本地编译）
- 并行：`tools/e2e_matrix.sh`，本机 ∥ gpd（+pc）多车道
- 证据：`docs/v2/orb-drag.md`、`runs/.../verify-round11.md`

### V3 悬浮窗（2026-10-24 23:20）
- 148×148 置顶透明小球、启动右下角、360 风格、六态动效，机器验收 5/5

### V2（2026-10-24）
- 麦克风打通（PulseAudio 后端 + 去掉坏的 `.asoundrc`）
- 修 12 个 bug（7 个「假通过」：写了状态变更但没通知外部系统）
- 多 agent / 项目别名表 / 模型切换 / AEC 接入 / 进程复用 / 托盘 / 意图确认
- 首次真机全链路（喊小蝶 → 派活 → 主动汇报）

## 当前架构要点（别破的三条不变量）

1. **WebView 永不碰麦克风**：音频全在 Rust，前端只画球
2. **语音模型只有 5 个函数**（派活/查进度/取消/追加/切上下文），无文件与命令能力
3. **语音链路与任务链路独立**：语音轮次先结束，任务后台跑完再主动汇报

## 已知限制 / 风险

| # | 限制 | 现状 |
|---|---|---|
| 1 | Windows 运行时没在真 Windows 验过 | 能编、能打 NSIS；`productName` 还是中文，安装路径有风险 |
| 2 | AEC 真机听觉验证没做 | 代码已接 sonora/AEC3；两台设备都没有能出声的扬声器，测试台没有回声可消 |
| 3 | 打断自动化复现不稳 | 虚拟音源注入延迟 1–2s，模型短回答只有 2–4s；建议接耳机人工验手感 |
| 4 | 内存 ≈294MB（要求 <150MB） | 待用户拍板：接受 / 原生重写 UI |
| 5 | 仓库 `.git` 245MB | 历史里有早期构建产物；改写历史不可逆，待用户点头 |
| 6 | ASR 内容词听错（README→limit） | 本地救不了，靠模型直接听音频；项目名这类有限集合已兜底 |
| 7 | 无扬声器机器上「汇报念不出」 | 会反复重开会话（计费）；退避策略未做（P1）|

## 剩余工作

### P1
1. 无扬声器/无输出设备时给主动汇报加**退避与上限**（避免反复开计费会话）
2. AEC 真机验证（接实体音箱 + 麦克风；或人工戴耳机喊「停」验打断手感）
3. Windows 真机试装（先把 `productName` 改 ASCII）
4. 打断自动化复现稳定化（用更长的模型回答或双注入）

### P2
1. 完整多 session 进程池
2. 内存优化（等用户拍板）
3. ESP32 / 安卓 / Web 端

## 硬件与设备

| 设备 | 地址 | 用途 | 限制 |
|---|---|---|---|
| wings-tx-us1（本机）| 100.99.51.5 | 开发、单测、CI 触发 | 无桌面、无音频 |
| wings-gpd | `wings@100.107.9.102` | **真机全流程**（桌面+麦克风+虚拟音源注入） | N6000 慢；无扬声器 |
| wings-pc | `wings@100.110.222.0` | 第二条真机桌面车道（已登录桌面） | 有麦克风接口、**无可用扬声器**；需先起服务 |

> 编译一律走 GitHub Actions（用户要求），设备上只部署 CI 产物：
> `tools/gpd_deploy_ci.sh --run <run-id>`。

## 文档索引

| 想知道什么 | 看哪个 |
|---|---|
| **怎么验证这个项目** | `docs/v2/HOW-TO-VERIFY.md` |
| 悬浮球拖动（为什么原生拖不动）| `docs/v2/orb-drag.md` |
| 编译走 CI + 一键部署 | `docs/v2/ci-build.md` |
| 并行 e2e / pc 车道 | `docs/v2/parallel-e2e.md`、`docs/v2/pc-lane.md` |
| 打断实现与限制 | `docs/v2/bargein.md` |
| 测试台音频流（防自打断）| `docs/v2/audio-topology.md` |
| ASR 听错兜底 | `docs/v2/asr-project-mishearing.md` |
| Windows 兼容 | `docs/v2/windows.md` |
| DSH 握手 | `docs/v2/dsh-handshake.md` |
| 硬件事实 | `docs/v2/hardware-facts.md` |
| 教训总结 | `docs/v2/lessons.md` |
| 所有实测数字 | `docs/实测数据.md` |
| 每轮验收记录 | `runs/20261024-2208-ui-ux-360/verify-round*.md` |
