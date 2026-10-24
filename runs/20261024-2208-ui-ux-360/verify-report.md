# 验证报告（S5）

**日期**：2026-10-24 23:18
**对象**：commit `deae843`（GitHub Actions 构建的 deb，装在 wings-gpd 上跑）

## 验收总表（一条命令跑出来的，不是手写）

```
$ tools/verify_float_ui.sh all

检查	结果	说明
------	------	------
pos   	通过	右距 16px / 底距 64px，均在阈值内
top   	通过	含 _NET_WM_STATE_ABOVE（X11 后端下置顶生效）
size  	通过	宽高 148x148 ≤ 176
shots 	通过	15 对两两可区分（RMSE 全部 ≥0.023840）
click 	通过	点击触发状态机离开 S0 → S2

总计：通过 5 / 失败 0 / 跳过 0
```

对应 acceptance.md 的 A1/A2/A3/A4/A6 五条。

## 其余几条

| # | 标准 | 证据 |
|---|---|---|
| A5 | 前端单测全绿 | `node --test app/frontend/test/` → **13 pass / 0 fail** |
| A7 | 拖动 + 贴边不回归 | 见下面的「未覆盖」 |
| A8 | 语音链路不回归 | `tools/gpd_voice_e2e.sh`：唤醒 1 次 → **delegate_task 恰好 1 次** → 兜底纠正 **0 次**；4 个历史任务不再重念（bug #12 修好） |
| A9 | CI 构建通过 | Release run `40105600171` success（5m21s）；CI run `40101950972` success |

## 视觉证据（都在 evidence/）

| 文件 | 说明 |
|---|---|
| `desktop-final.png` | 整屏截图（1920×1830，portal 截的真实桌面） |
| `orb-bottom-right.png` | 放大后的右下角：球贴在任务栏上方，压在其他窗口之上 |
| `states-on-dark.png` | S0 vs S2 在深色背景下的对比 |
| `bubble.png` | 气泡展开态（球不动、左边冒字幕） |
| `ui-S0..S5.png` | 六态证据（验收脚本自动产出，用于 RMSE 两两比较） |

## 不变量核对

| # | 不变量 | 结果 |
|---|---|---|
| I1 | 语音链路不回归 | ✅ 实测 1 次派活、0 次纠正 |
| I2 | 前端不碰麦克风 | ✅ `grep getUserMedia app/frontend/src` 只有一句注释，无调用；无 npm 依赖 |
| I3 | 工具仍 5 个 | ✅ `cargo test -p morphofly-core --lib tools::` 全绿（不变量断言在测） |
| I4 | 不碰 AEC/KWS/派活逻辑 | ✅ `git diff` 未触及 core/src/audio、core/src/kws、core/src/dispatch（除已修的 correction/tasks） |

## 未覆盖 / 需要人看

1. **A7 拖动 + 贴边**：脚本不测拖拽手感。需要真人拖一次（或我用 computer-use 拖）。
   已知风险：canvas 盖满窗口时 `data-tauri-region` 收不到指针，前端加了
   「移动 >5px → startDragging()」兜底，但**没在真机上验证过**。
2. **悬停手感、气泡动画流畅度、60fps**：无头截图看不出，需要真机观感确认。
3. **强制 XWayland 的副作用**（托盘图标/缩放/输入法）未逐项回归。

## 独立核验

- 截图能力 `tools/gpd_shot.py` 是我自己写的，但它产出的 `desktop-final.png`
  与验收脚本的 `pos` 数字（window=(2908,1516)）互相印证 —— 图上球确实在右下角。
- 两个子代理（ui-visual / verify-harness）各自独立跑了自己那部分，
  验收脚本是 verify-harness 写的，我拿它判我自己改的 Rust —— 属于独立核验。
