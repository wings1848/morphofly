# 交接状态（2026-10-25 05:50，V3）

## 一句话

**V3 真机全链路已跑通**（喊「小蝶」→ 说任务 → 派活 → DSH 干活 →
**语音会话关了自己重开回来汇报**）；悬浮窗/拖动/CI 构建/三设备并行验收都已完成。
剩下的主要是**需要用户拍板的事**和**两件缺硬件的验证**。

## 当前指标

| 项目 | 数值 |
|---|---|
| 单元测试（默认 / 含 kws） | **271 / 282 passed** |
| 前端测试 | 13 passed |
| CI | ✅ 全绿（含 Windows 检查、可下载 Linux 产物）|
| 真机全链路 | ✅ 唤醒/派活/DSH/汇报各 1 次 |
| 悬浮球拖动 | ✅ A/B 两条真机用例，误差 1px |
| 打断 | ✅ 真机抓到完整打断（interruptions=1）|
| 音频流防自打断 | ✅ `verify_audio_topology.sh` PASS |
| 内存 | ≈294MB PSS（目标 <150MB，❌）|

## 怎么接手（30 分钟上手）

1. **看状态**：`docs/v2/PROGRESS.md`（一页看完）
2. **跑验收**：`docs/v2/HOW-TO-VERIFY.md` 里的「验收脚本清单」
   - 本机：`cargo test -p morphofly-core --lib`（271 条）
   - 真机：`tools/e2e_matrix.sh`（local ∥ gpd，可选 `--with-pc`）
3. **改代码 → 编译**：**一律 push 让 GitHub Actions 编**
   （用户明确要求），`tools/gpd_deploy_ci.sh` 部署到设备
4. **看真机画面**：`python3 tools/gpd_shot.py /tmp/x.png`

## 三条不能破的不变量

1. WebView 永不碰麦克风（音频全在 Rust）
2. 语音模型只有 5 个函数，物理上没有文件/命令能力
3. 语音链路与任务链路独立（语音先结束，任务后台跑完再汇报）

## 设备

| 设备 | 用途 | 限制 |
|---|---|---|
| wings-tx-us1（本机 100.99.51.5）| 开发/单测/CI | 无桌面无音频 |
| wings-gpd（100.107.9.102）| **真机全流程** | N6000 慢；无扬声器；靠虚拟音源注入测试 |
| wings-pc（100.110.222.0）| 第二条桌面车道 | 无可用扬声器；有麦克风接口 |

## 需要用户拍板的事（别自己决定）

1. **内存 ≈294MB**（要求 <150MB）：接受现状，还是投入 3–9 天把 UI 改原生渲染（视觉会降级）？
2. **仓库 `.git` 245MB**：历史里有早期构建产物（AppImage/squashfs）。
   彻底瘦身要改写历史（不可逆），要不要做？
3. **StepFun key**：canvas 里那份是明文，建议轮换（凭据的事不代办）。
4. **gpd 是否配自动登录**：配了 pc 车道随时可用，但会少一层登录保护。

## 硬件卡住的两件验证

1. **AEC 真机听觉验证**：两台设备都没有能出声的扬声器 → 没有回声可消。
   接实体音箱 + 麦克风后，按 `docs/v2/audio-topology.md` 5.4 节验。
2. **Windows 运行时**：没有 Windows 机器。能编、能打 NSIS，但没真机跑过；
   上 Windows 前建议先把 `productName` 从「蝶梦」改成 ASCII。

## 已知不稳定项

- **打断自动化复现**：机制通（有真机证据），但虚拟音源注入延迟 1–2s、
  模型短回答只有 2–4s，脚本经常抓不到「正在说话」的窗口。
  真机手感建议接耳机人工喊「停」。
- **ASR 内容词听错**（README→limit）：本地救不了；项目名这类有限集合已用
  模糊匹配 + 本地纠正表兜住。

## 关键文件

| 想知道什么 | 看哪个 |
|---|---|
| 一页状态 | `docs/v2/PROGRESS.md` |
| 怎么验证 | `docs/v2/HOW-TO-VERIFY.md` |
| 拖动（为什么原生拖不动）| `docs/v2/orb-drag.md` |
| 编译走 CI + 部署 | `docs/v2/ci-build.md` |
| 并行 e2e / pc 车道 | `docs/v2/parallel-e2e.md`、`docs/v2/pc-lane.md` |
| 打断 | `docs/v2/bargein.md` |
| 音频流（防自打断）| `docs/v2/audio-topology.md` |
| ASR 听错 | `docs/v2/asr-project-mishearing.md` |
| Windows | `docs/v2/windows.md` |
| DSH 握手 | `docs/v2/dsh-handshake.md` |
| 每轮验收记录 | `runs/20261024-2208-ui-ux-360/verify-round*.md` |
| 变更历史 | `CHANGELOG.md` |
