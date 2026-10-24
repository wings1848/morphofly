# 验证报告：启动瞬间不再弹「待机」黑气泡（KDE）

**run**：20261025-0520-startup-clean（轻量档，巡检模式）
**日期**：2026-10-25
**修后二进制**：CI run [40135239790](https://github.com/wings1848/morphofly/actions/runs/40135239790)
（PR #2 merge commit，sha `2724ae4`，已部署到 wings-gpd）
**真机**：wings-gpd（KDE + KWin/XWayland，`morphofly-test` 用户服务）

## 结论

用户报的「启动弹黑框 UI、字和球叠一会儿」已修复，并通过真机回归（`tools/verify_startup_clean.sh`）：

| 判据 | 修前（旧二进制） | 修后（CI 产物） |
|---|---|---|
| T1 启动不显示初始字幕 | **FAIL**（旧日志 `caption=待机` → `show=true`） | **PASS**（`启动状态 state=S0 caption=无`，重启后无 `show=true`） |
| T2 真机连拍：球左侧气泡区差分 | **FAIL，实测 5.95**（黑气泡在 t≈5.15s，持续 4.2s） | **PASS，0.00** |
| T3 位置 / 尺寸 / 置顶不回归 | PASS（基线） | **PASS**：(2908,1516)、148×148、右距 16 底距 64、`_NET_WM_STATE_ABOVE` |
| T4 前端单测 | 13 passed（无回归测试） | **PASS，16 passed** |

一条命令重跑：`tools/verify_startup_clean.sh`。

## 视觉证据

| 修前 | 修后 |
|---|---|
| 黑气泡「待机」贴在球左边 4.2 秒<br>`evidence/red-visible-bubble/mfirst.jpg` | 同一区域连拍（t≈3s）只有球<br>`evidence/startup-frames/morphofly-startup-frames/f0025.jpg` |

## 修后真机 journal 时间线（一次冷启动）

```
21:45:14.728  蝶梦启动
21:45:15.401  前端日志 js=启动自检 ... dpr=1.5
21:45:15.423  贴边计算 from=Some((1461, 756)) to_x=2908 to_y=1516
21:45:15.431  前端日志 js=启动状态 state=S0 caption=无       ← 启动没有字幕
21:45:15.484  小球已贴边 x=2908 y=1516 right=true
（没有任何「气泡窗口已调整 show=true」——启动不再弹气泡）
```

## 不变量（本次必须保持不变，已核对）

| 不变量 | 判定方式 | 实测 |
|---|---|---|
| 窗口尺寸仍 148×148 | T3 / `verify_float_ui.sh size` | 148x148 ✅ |
| 仍贴右下角（右距 16 / 底距 64） | T3 / `verify_float_ui.sh pos` | (2908,1516) ✅ |
| 置顶仍生效 | T3 / `verify_float_ui.sh top` | `_NET_WM_STATE_ABOVE` 等 ✅ |
| 运行期状态字幕仍能显示 | 代码审查：`state` 事件仍带 caption，前端仍 showCaption | 未改这条路径 ✅ |
| 前端单测不回归 | T4 | 16 passed（13 旧 + 3 新）✅ |
| Rust core 单测 / CI | GitHub Actions | 全 job success ✅ |

## 改动清单

- `app/src-tauri/src/main.rs`：`orb_state` 不再把状态名当字幕；`spawn_pump` 的
  `last_state` 初始化为当前状态（避免 demo/QA 模式启动从 state 事件推「待机」字幕）。
- `app/frontend/src/main.js`：`boot()` 不再显示初始字幕（只打 `caption=无` 日志）；
  `showCaption` 对空字幕直接 return。
- `app/frontend/src/orb/caption.js` + `test/caption.test.mjs`：字幕清洗 + 回归测试。
- `tools/verify_startup_clean.sh` + `tools/gpd_startup_frames.py`：真机启动回归验收（T1–T4）。
- 文档：`docs/v2/startup-clean.md`、`docs/v2/float-window.md`、`CHANGELOG.md`。

## 独立 review 与范围收窄（重要）

第一版还加了「窗口先 GTK 设全透明、前端首帧就绪后再显形」。独立 subagent 审查后指出：

1. `gtk_widget_set_opacity` 在 `transparent: true` 的 RGBA 窗口上走 Cairo alpha，
   不是合成器不透明度（`_NET_WM_WINDOW_OPACITY`），WebKit 的 native 子窗口也遮不住；
2. 恢复不透明度失败只 warn，窗口可能永远不可见（比原 bug 更糟）；
3. 8fps 真机连拍（t=0.5–13s，屏幕中间+右下角）根本没拍到「首帧前的空窗口黑框」——
   用户看到的黑框就是字幕气泡。

→ 已退回最小修复（只修「启动不该有字幕」），删除 gtk 依赖与过渡代码。
理由与实测写在 `docs/v2/startup-clean.md`。

## 已知限制（诚实写清）

1. 只修了启动字幕；Windows/macOS 的启动路径本轮没动（CI 的 Windows
   `cargo check` 仍然 green）。
2. T2 判据依赖「球左侧区域背景本身不动」；若下面的聊天窗口正在流式刷新会抬高噪声，
   脚本用「稳定后两帧的噪声」做动态阈值，换主题/壁纸后建议先跑一次看读数（正常 ≈0）。
3. T1 的 `caption=无` 日志是「前端启动时实际拿到的字幕值」的直接证据；
   旧二进制没有这条日志，所以 T1 在旧版本上判 FAIL（这是有意的，不是假失败）。

## 复跑命令

```bash
cd /home/wings/Code_Project/_worktrees/startup-clean   # 或合并后的主仓库
tools/verify_startup_clean.sh               # 全量：重启服务 + 连拍 + T1–T4
tools/verify_startup_clean.sh --no-frames   # 只看日志 + 单测（快）
```
