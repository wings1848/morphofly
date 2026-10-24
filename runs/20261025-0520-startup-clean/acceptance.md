# 验收单：启动瞬间干净（「待机」黑气泡压球）

关联 run：20261025-0520-startup-clean（轻量档）

| # | 验收标准 | 判定命令 | 期望输出 | 实测 |
|---|---|---|---|---|
| A1 | 启动不再显示「待机」字幕黑气泡 | `tools/verify_startup_clean.sh` → T1 | PASS T1 | 修前 FAIL / 修后 PASS |
| A2 | 真机连拍：启动过程中球左侧无黑框/文字（差分 < 1.5） | 同上 → T2 | PASS T2 | 修前 FAIL（气泡停 4.2s, 5.95）/ 修后 0.00 |
| A3 | 位置 / 尺寸 / 置顶不回归 | 同上 → T3（复用 verify_float_ui pos/top/size） | PASS T3 | 修后 PASS |
| A4 | 前端单测全绿 | 同上 → T4（node --test） | PASS T4（16 passed） | 修后 PASS |
| A5 | Rust core 测试 + 全部 CI 绿 | GitHub Actions `ci.yml` | success | 修后 |
| A6 | 真机视觉证据留档（修前/修后帧） | 人工看 evidence/ | 球左边无黑气泡 | 修后 |

## 不做清单

- 不做「窗口先透明/等首帧再显形」的过渡：真机连拍没复现到「空窗口黑框」，
  且该机制在 RGBA 透明窗口上不可靠、失败会让球永远不可见（review 结论）。
- 不改内存优化路线（PSS 371MB → 150MB 是独立议题）
- 不动 Windows/macOS 启动行为
- 不重做字幕气泡视觉（运行期状态字幕保留）

## 假设（巡检模式默认值）

- H1 自动通过：目标就是「启动瞬间只剩球，没有黑框/字幕」，无歧义
- 真机 = wings-gpd（KDE + KWin/XWayland，和用户报告的环境同类）
- 验收用 `morphofly-test` 服务重启来复现「启动」
