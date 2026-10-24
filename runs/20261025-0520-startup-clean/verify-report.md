# 验证报告：启动瞬间不再弹「待机」黑气泡（KDE）

**run**：20261025-0520-startup-clean（轻量档，巡检模式）
**日期**：2026-10-25
**修后二进制**：main CI run [40136977533](https://github.com/wings1848/morphofly/actions/runs/40136977533)
（main merge commit `071bd45`，已部署到 wings-gpd；`build.txt` 可查）
> 注：PR 期间先在 `2724ae4`（PR merge preview）上验过一轮，app 代码与 `071bd45` 完全一致；
> 合并后又用 main 的 CI 产物全量复跑了一次，见下表。
**真机**：wings-gpd（KDE + KWin/XWayland，`morphofly-test` 用户服务）

## 结论

用户报的「启动弹黑框 UI、字和球叠一会儿」已修复，并通过真机回归（`tools/verify_startup_clean.sh`）：

| 判据 | 修前（旧二进制） | 修后（CI 产物） |
|---|---|---|
| T1 启动不显示初始字幕 | **FAIL**（旧二进制无 `caption=无` 判据；journal 有启动后 1.3s 的 `show=true`） | **PASS**（`启动状态 state=S0 caption=无`，启动窗口无 `show=true`） |
| T2 真机连拍：球左侧气泡区差分 | **FAIL**（修复前留存帧复算 **3.73** > 阈值 1.5，见 `evidence/red-pixel-analyze.json`） | **PASS，0.00** |
| T3 位置 / 尺寸 / 置顶不回归 | PASS（基线） | **PASS**：(2908,1516)、148×148、右距 16 底距 64、`_NET_WM_STATE_ABOVE` |
| T4 前端单测 | 13 passed（无回归测试） | **PASS，16 passed** |

一条命令重跑：`tools/verify_startup_clean.sh`。

## 视觉证据

| 修前 | 修后 |
|---|---|
| 黑气泡「待机」贴在球左边 4.2 秒<br>`evidence/red-visible-bubble/mfirst.jpg`、单帧 `bubble-frame-t2.6.jpg` | 同一区域连拍（t≈3s）只有球<br>`evidence/startup-frames/morphofly-startup-frames/f0025.jpg` |

## 修后真机 journal 时间线（main 产物，一次冷启动）

```
22:19:01.970  蝶梦启动
22:19:02.771  贴边计算 from=Some((1461, 756)) to_x=2908 to_y=1516
22:19:02.832  小球已贴边 x=2908 y=1516 right=true
22:19:02.896  前端日志 js=启动自检 ... dpr=1.5
22:19:02.924  前端日志 js=启动状态 state=S0 caption=无       ← 启动没有字幕
（启动窗口内没有「气泡窗口已调整 show=true」；+5s 后确有运行期 S2 字幕，
 是外部语音注入触发的正常产品行为，T1/T2 已按「第一次状态变化」切掉）
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

## 独立验收（subagent，2026-10-25 06:05）

独立 subagent 在不知道结论的前提下自己复跑 + 交叉核对，报告要点：

- **T1–T4 独立复跑 4/4 PASS**（exit 0）；T1 启动窗口内只有 `caption=无`。
- **T2 独立复算**：自己用 PIL 重算 72 帧——region 最大差 0.0，整图最大 2.085、
  球区最大 8.798（证明帧在动，不是冻结/截错屏）；抽看的帧里没有黑气泡。
- **T3**：xprop + python-xlib 各独立确认 (2908,1516)/148x148/_NET_WM_STATE_ABOVE。
- **T4**：16 passed / 0 failed。
- **部署链**：`origin/main = 071bd45`，修复 commit 是其祖先；`app/` 与 main 无差异。
- **它同时指出本报告初版有 2 处红证据不实**（5.95 没归档、旧二进制不打 caption 日志）。
  本版已修正：删掉不可复核的 5.95，改用归档复算的 3.73（帧包 + `recompute.py` 已提交），
  红证据叙述改为「journal 4.202s + 截图 + 像素复算」。
- 它还发现验收期间**有别的进程反复重启 gpd 服务、注入语音**；脚本已加
  「只取第一次启动窗口 / 第一次运行期状态变化之前」的裁剪（本次 main 复跑因此稳定全绿）。

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
4. 验收期间如果 gpd 上有别的 agent 在注入语音/重启服务，脚本会自动裁掉运行期部分；
   但仍建议复跑时确认环境安静（脚本会打印「验收期间重启次数」）。

## 复跑命令

```bash
cd /home/wings/Code_Project/_worktrees/startup-clean   # 或合并后的主仓库
tools/verify_startup_clean.sh               # 全量：重启服务 + 连拍 + T1–T4
tools/verify_startup_clean.sh --no-frames   # 只看日志 + 单测（快）
```

## 回滚

改动只涉及启动字幕显示，无数据迁移/无配置变更。需要回退时：

```bash
git revert -m 1 071bd45        # 撤销 PR #2 的合并
# 或直接把 gpd 上的二进制换回上一个 CI 产物：tools/gpd_deploy_ci.sh --run <旧 run-id>
```
