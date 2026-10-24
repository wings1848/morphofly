# 验收单：启动瞬间干净（黑框 / 字压球）

关联 run：20261025-0520-startup-clean（轻量档）

| # | 验收标准 | 判定命令 | 期望输出 | 实测 |
|---|---|---|---|---|
| A1 | 启动不再显示「待机」字幕黑气泡 | `tools/verify_startup_clean.sh` → T1 | PASS T1 | 修前 FAIL / 修后见 verify-report |
| A2 | 窗口内容画好才可见（首帧就绪在贴边之后） | 同上 → T2 | PASS T2 | 修前 FAIL / 修后 |
| A3 | 首帧就绪耗时 < 3000ms（启动没变慢） | 同上 → T3 | PASS T3 | 修前不可测 / 修后 |
| A4 | 真机录屏：启动过程中球左侧无黑框/文字（差分 < 1.5） | 同上 → T4 | PASS T4 | 修前在 05:09 实测到 4.2s 气泡（见 red 证据）|
| A5 | 贴边位置不回归（右下角） | 同上 → T5（复用 verify_float_ui pos） | PASS T5 | 修前 PASS（2908,1516 右距 16 底距 64）|
| A6 | 前端单测全绿 | `node --test 'app/frontend/test/*.test.mjs'` | 16 passed | 修后 |
| A7 | Rust core 测试 + 前端测试 CI 绿 | GitHub Actions `ci.yml` | success | 修后 |

## 不做清单

- 不改内存优化路线（PSS 371MB → 150MB 是独立议题）
- 不动 Windows/macOS 启动行为（没有对应窗口不透明度 API；本轮只修 Linux KDE）
- 不重做字幕气泡视觉（只去掉「启动时不该出现」的那一次）

## 假设（巡检模式默认值）

- H1 自动通过：目标就是「启动瞬间只剩球，没有黑框/字幕」，无歧义
- 真机 = wings-gpd（KDE + KWin/XWayland，和用户报告的环境同类）
- 验收用 `morphofly-test` 服务重启来复现「启动」
