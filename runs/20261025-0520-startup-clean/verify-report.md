# 验证报告：启动瞬间干净（黑框 / 「待机」黑气泡压球）

**run**：20261025-0520-startup-clean（轻量档，巡检模式）
**日期**：2026-10-25
**修后二进制**：CI run [40134296273](https://github.com/wings1848/morphofly/actions/runs/40134296273)（sha `1fdf456`，PR #2 的 merge commit）
**真机**：wings-gpd（KDE + KWin/XWayland，`morphofly-test` 用户服务）

## 结论

用户报的「启动弹黑框 UI、字和球叠一会儿」已修复并通过真机回归：

| 判据 | 修前（旧二进制） | 修后（CI 产物） |
|---|---|---|
| T1 启动不显示初始字幕 | **FAIL**（journal `caption=待机` → `show=true`） | **PASS**（`caption=无`，重启后无 `show=true`） |
| T2 首帧就绪在贴边之后 | **FAIL**（无此日志，窗口先 show） | **PASS** |
| T3 首帧就绪耗时 | 不可测 | **PASS，840ms**（< 3000ms） |
| T4 真机连拍：球左侧气泡区域差分 | **FAIL，实测 5.95**（黑气泡在 t≈5.15s，持续 4.2s） | **PASS，0.00** |
| T5 贴边位置不回归 | PASS（2908,1516 右距16 底距64） | **PASS**（同坐标） |

验收脚本：`tools/verify_startup_clean.sh`（一条命令重跑）。
证据：`evidence/red.log`（修前红）、`evidence/startup-clean-summary.md`（修后绿）、
`evidence/startup-frames/`（修后连拍帧）、`evidence/red-visible-bubble/`（修前黑气泡截图）。

## 视觉证据

- 修前（2026-10-25 05:09，连拍放大）：黑气泡「待机」贴在球左边 4.2 秒
  `evidence/red-visible-bubble/mfirst.jpg`
- 修后（05:33，同一区域连拍，t=3.3s）：只有球，没有黑框/字幕
  `evidence/startup-frames/morphofly-startup-frames/f0026.jpg`

## 修后真机 journal 时间线（一次冷启动）

```
21:31:49.669  启动：小球窗口先全透明（等前端首帧画完再显示）
21:31:50.067  前端日志 js=启动自检 ...
21:31:50.166  前端日志 js=启动状态 state=S0 caption=无        ← 启动没有字幕
21:31:50.217  小球已贴边 x=2908 y=1516 right=true
21:31:50.237  前端首帧已画（通知 Rust 显示窗口）
21:31:50.270  启动首帧就绪：小球可见 elapsed_ms=929 frontend_ready=true
```

## 不变量（本次必须保持不变，已核对）

| 不变量 | 判定 | 实测 |
|---|---|---|
| 窗口尺寸仍是 148×148 | `verify_float_ui.sh size` / pos 探测 | 148x148 ✅ |
| 仍在右下角（右距 16 / 底距 64） | `verify_float_ui.sh pos` | (2908,1516) ✅ |
| 置顶仍生效 | `verify_float_ui.sh top`（上一轮同一构建体系基线） | `_NET_WM_STATE_ABOVE`（CI 产物未改置顶代码）✅ |
| 运行期字幕仍能显示 | 代码审查：`state`/`caption` 事件路径未改，仅 boot 不再显示 | 首轮部署启动时旧实例曾正常弹出 S2 字幕（反证运行期路径没坏）✅ |
| 前端单测不回归 | `node --test` | 16 passed（13 旧 + 3 新）✅ |
| Rust core 单测 / CI | GitHub Actions | 全 job success ✅ |

## 改动清单

- `app/src-tauri/src/main.rs`：`orb_ready` 命令、`FRONTEND_READY`、`PROCESS_START`、
  `linux_set_window_opacity()`（GTK）、启动流程改为「先透明 → show → 贴边 → 等首帧 → 显形」、
  `orb_state` 不再把状态名当字幕。
- `app/src-tauri/Cargo.toml` + `Cargo.lock`：Linux target 加 `gtk = "0.18"`（与 tauri/tao 同版本）。
- `app/frontend/src/main.js`：boot 不显示初始字幕（打 `caption=无` 日志）、空字幕不渲染、首帧后调 `orb_ready`。
- `app/frontend/src/orb/caption.js` + `test/caption.test.mjs`：字幕清洗 + 回归测试。
- `tools/verify_startup_clean.sh` + `tools/gpd_startup_frames.py`：真机启动回归验收（T1–T5）。
- 文档：`docs/v2/startup-clean.md`、`docs/v2/float-window.md`、`CHANGELOG.md`。

## 已知限制（诚实写清）

1. **透明显形只做在 Linux**：Windows/macOS 没有对应窗口不透明度 API，本轮不改它们的行为
   （Windows CI 的 `cargo check` 已通过，cfg 隔离开了）。
2. **依赖合成器**：`gtk_widget_set_opacity` 在 KDE/KWin 有合成时生效；没有合成器的 X11 桌面上
   透明窗口本身也不成立，不是本次引入的问题。
3. **T4 判据有环境依赖**：差分区域是「球左侧气泡区」，若该区域背景本身在动（例如下面的
   聊天窗口在流式刷新）会抬高噪声；脚本已用「稳定后两帧的噪声」做动态阈值，但换桌面主题/壁纸
   后建议先跑一次确认读数（正常应 ≈0）。
4. **启动时球仍会先从屏幕中间被搬到右下角**，只是整个搬运过程在 opacity=0 下不可见；
   如果以后 KWin 对「透明度 0 的窗口」也做打开动画，需要再看一眼（当前实测无感）。

## 复跑命令

```bash
cd /home/wings/Code_Project/_worktrees/startup-clean   # 或合并后的主仓库
tools/verify_startup_clean.sh          # 全量：重启 + 连拍 + 5 条判据
tools/verify_startup_clean.sh --no-frames   # 只看 journal（快）
```
