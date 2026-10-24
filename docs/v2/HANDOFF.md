# V2 交接状态（2026-10-24 20:30）

## 一句话

**能做的都做完了。剩下一件事卡在硬件上：AEC 真机验证，需要 wings-gpd（唯一有麦克风的设备），但它离线 1 小时了。**

## 当前指标

| 项目 | 数值 |
|---|---|
| 单元测试（默认） | 232 passed |
| 单元测试（含 kws） | **243 passed** |
| 前端测试 | 8 passed |
| e2e mock | 14/14 |
| **真机 e2e** | **13/14** |
| 编译警告 | 0 |
| clippy 警告 | 0 |
| cargo fmt | 干净 |
| CI | ✅ 通过（e0befed）|

## 已完成（按用户 11 条需求）

### P0 体验
- ✅ 系统托盘（左键显示/隐藏、右键退出）—— 之前只能 pkill
- ✅ 小球贴右下角，不挡中间
- ✅ 拖动 + 松手自动贴边
- ✅ 文案极简（派活 23 字 → 5 字）
- ✅ 意图确认（复杂任务先问「要我…吗」）

### P1 能力
- ✅ 多 agent：set_context + 真实切换
  - dsh 可用；codex 需 `codex login`
- ✅ 项目别名表（中文别名 → 真实路径）
- ✅ 模型切换（JSON 数组格式，DSH 校验）
- ✅ 目录切换（实测生效）

### P0.5 现存缺陷（5 个全修）
- ✅ cancel_task 真的停 DSH
- ✅ append_requirement 如实告知
- ✅ ACP 按 sessionId 过滤
- ✅ 常驻进程复用（3698ms → 1102ms）
- ✅ 打断时通知云端停止生成

### AEC
- ✅ sonora AEC3 接入音频链路
- ✅ 19 个测试全绿，合成信号 34.5dB
- ⏳ **真机效果未验证**（硬件限制）

### 测试基础设施
- ✅ e2e 框架（15 条用例）
- ✅ TTS 合成 + 裸 PCM 注入
- ✅ 真机驱动 `core/examples/e2e_drive.rs`
- ✅ KWS 前置门打通

## 关键发现（推翻了之前的判断）

| 我说过 | 实际 | 证据 |
|---|---|---|
| 「AEC 只有 1.9dB，方案不行」 | 测试信号太规则，实际 34.5dB | 换基频滑动信号后实测 |
| 「codex 实测可用」 | 复用了 dsh 进程池的假象 | clippy unused variable 警告 |
| 「谐音误唤醒是产品 bug」 | KWS 工作正常，e2e 绕过了它 | KWS 实测 3 条谐音全 0 命中 |
| 「wings-pc 桌面已就绪」 | 那是登录器进程 | 没有 plasmashell/wayland socket |
| 「缺陷 #4 需 300 行重构」 | 50 行简化版省 2.6 秒 | pool_bench 实测 |
| 「wings-pc 能测 AEC」 | **它根本没有麦克风** | /proc/asound/cards 只有 2 张纯输出声卡 |

## 剩下一件事

### AEC 真机验证

**为什么做不了**：
- 本机（云主机）：无音频设备
- wings-pc：**没有麦克风硬件**（声卡只有 PCH 和 NVidia，都是纯输出）
- wings-gpd：有麦克风，但**离线 1 小时**了

**怎么验**（设备上线后）：
```bash
# 1. 等它上线（后台已在等）
tail -f /tmp/gpd-wait.log

# 2. 或手动跑全套
./tools/gpd_e2e.sh

# 3. 或只测音频
./tools/gpd_e2e.sh --audio

# 4. 真机 AEC 活体测试（需要外放 + 麦克风）
./tools/aec_live.sh
```

**验收标准**：
- 外放音量 70%，让蝶梦说话
- 中间喊「停」
- 期望：300ms 内停播，且不自我打断
- 观察日志里 AEC stats 的 render_blocks / capture_blocks 是否同步增长

## 已知限制

1. **「小蝶帮我」KWS 漏检**（4 字无停顿）
   - 不能降 num_trailing_blanks（有硬断言，降了谐音就回来）
   - 正解：加唤醒词变体，需装 sherpa-onnx-cli
   - 见 `docs/v2/kws-edge-case.md`

2. **codex 需要登录**
   - `codex login` 或设置 OPENAI_API_KEY
   - 按 AGENTS.md，凭据的事不代做

3. **多任务并发未实测**
   - sessionId 过滤做了，但没有真机并发验证
   - 需要 GUI 环境

## 文件索引

| 想知道什么 | 看哪个文件 |
|---|---|
| 侦察结论 | `docs/v2/scout-full.md` |
| 体验问题清单 | `docs/v2/review.md` |
| AEC 状态 | `docs/v2/aec-status.md` |
| 真机 e2e 发现 | `docs/v2/e2e-status.md` |
| 不会误唤醒的证明 | `docs/v2/no-false-wake-proof.md` |
| 工具真实性审计 | `docs/v2/tool-audit.md` |
| 教训总结 | `docs/v2/lessons.md` |
| 硬件事实 | `docs/v2/hardware-facts.md` |
