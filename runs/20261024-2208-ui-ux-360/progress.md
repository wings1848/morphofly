# 进展（S4 实现 → S5 验证）

**最后更新**：2026-10-24 23:10

## 三条流的状态

| 流 | 分支 | 状态 |
|---|---|---|
| A 前端视觉（ui-visual） | `wt/ui-float` | ✅ 已合并（commit 963d836 → 515162b） |
| B 窗口行为（lead） | main | 🔄 修到第 3 轮（启动时序 / KWin 越界） |
| C 验收脚本（verify-harness） | `wt/verify-float` | ✅ 已合并（790f111），默认底距阈值改 80 |

## 已实测通过

- `top` 置顶：`_NET_WM_STATE_ABOVE` 存在（X11 后端）
- `size` 尺寸：148×148（之前 200×200，是旧前端的 360 布局把窗口顶大）
- 前端单测：13 passed（原 8 + 新 5）
- 语音链路回归：唤醒 → **恰好 1 次** delegate_task → 0 次纠正
- 任务汇报不重复：`reported` 落盘生效（4 个历史任务升级后补报一次，之后不再念）
- 截图能力：`tools/gpd_shot.py` 打通（MCP 协议直连取图，portal 截图落盘）

## 修到第 3 轮的东西（都在真机上暴露）

1. `visible:false` 时 `outer_size()` 为 0 → 贴边公式算出屏幕外的坐标
2. 未映射时 `set_position` 被 KWin 丢弃 → 窗口弹在正中
3. 气泡收起后偏 196px → KWin 不允许右边缘越界，会拉回 x；顺序要按方向区分

## 新发现（未修，记下来）

- **云端 ASR 会把 "morphofly" 听成 "move_forward"**：注入 TTS「帮我看看 morphofly 项目的 README」，
  模型派活时写的 dir 是 `/home/wings/Code_Project/move_forward`（不存在）。
  → DSH 会话建不起来，也没有主动汇报。这是 ASR 谐音问题，不是派活逻辑问题。
  缓解方向：dispatch 层校验 dir 是否存在，不存在就回落到默认项目并记日志。
- 视觉上球体内部的白色粒子有点像「灰尘」，可调淡；用户看过再定。
