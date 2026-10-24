# 交付单（S7）

**日期**：2026-10-24 23:20
**交付物**：debes 由 GitHub Actions 构建（Release run 40105600171），装在 wings-gpd 上验证通过。

## Implementation

| 改了什么 | 文件 |
|---|---|
| 窗口 360×360 → 148×148，启动先 show 等映射再设尺寸再贴边 | `app/src-tauri/tauri.conf.json`、`main.rs` |
| Wayland 会话强制 GDK_BACKEND=x11（否则定位/置顶都无效） | `main.rs::force_x11_backend` |
| 气泡用单窗口按需撑开（`orb_tip`），记住归位点，按方向区分尺寸/位置顺序 | `main.rs` |
| 前端小球重做：玻璃盘 + 青环 + 六态动效 + 悬停 + 气泡 | `app/frontend/**` |
| 验收脚本 | `tools/verify_float_ui.sh` |
| 真机桌面截图工具（MCP 协议直连） | `tools/gpd_shot.py` |
| 语音链路回归脚本 | `tools/gpd_voice_e2e.sh` |
| CI 修红（sherpa 预编译缓存） | `.github/workflows/ci.yml` |

## Rollout

用户手动安装 deb 或用仓库脚本；当前 gpd 上是 `/tmp/mf-install6/`（测试用）。
正式安装：下载 Release 产物 → `ar x` + `bsdtar` 解开（gpd 是 Arch，没有 dpkg-deb）。

## Backout

出问题就回滚到 `9db705b`（V2 的 360×360 小球 + Wayland）：
```
git revert deae843..9db705b
```
窗口配置在 `tauri.conf.json` 一处，前端在 `app/frontend/` 一处，回滚面很小。

## Signoff checklist

- [x] 机器验收 5/5 通过（pos/top/size/shots/click）
- [x] 前端单测 13 通过
- [x] 语音链路回归（1 次派活、0 次纠正）
- [x] CI 绿（修好了长期红灯）
- [x] 不变量 4 条核对
- [ ] 拖动/贴边手感真机确认（需要人）
- [ ] 强制 XWayland 副作用回归（托盘/缩放/输入法）

## 收尾三查

1. **残留进程/临时文件**：gpd 上遗留 `/tmp/mf-install{,2..6}` 六个解包目录和
   `/tmp/debx{,2..5}`、`/tmp/wt-*` 两个 worktree、`http.server 8902`。
   这些**没清**，是故意留的（还在迭代中），交付后要清；已写进 retro。
2. **密钥扫描**：仓库 CI 有 secret-scan job（StepFun key 模式 + 通用模式），本轮 push 全部通过。
   但**画布里 StepFun key 是明文**，仍未轮换 —— 列进待办。
3. **文档数字能复现**：`232 → 242` 单测、`8 → 13` 前端、`148×148`、`右距16/底距64`、
   `RMSE≥0.0238` 都能用本文命令重跑。
