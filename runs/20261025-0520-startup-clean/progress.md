# 进度

## 2026-10-25 05:20 起步（S1/S2）

- 用户报告：Linux KDE 启动时弹黑框 UI，字和悬浮球叠一会儿才恢复。
- 真机复现（gpd，重启 morphofly-test）：
  - journal：启动后 ~1.2s 出现 `气泡窗口已调整 show=true`，4.2s 后 `show=false`
  - KWin 连拍：球左侧出现黑色圆角气泡，内含「待机」两字（证据帧见 verify-report）
  - 竞态：页面比贴边先就绪时，orb_tip 被 ORB_READY 拦住 → 气泡不展开但字幕元素仍被塞了文字
- 根因两条：
  1. `orb_state` 在无字幕时回退成状态名「待机」，前端 boot 直接 `showCaption`
  2. 启动顺序先 `show()` 再等页面 → 首帧前是空窗口（KDE 下就是黑框）
- 红证据：`evidence/red.log`（T1/T2/T3 FAIL，旧二进制）。

## 2026-10-25 05:30 实现（S4）

- Rust：新增 `orb_ready` 命令；启动时先用 GTK 把窗口设为全透明，贴边 + 前端首帧就绪后再恢复；
  `orb_state` 不再拿状态名当字幕。
- 前端：boot 不显示初始字幕（只打 `启动状态 ... caption=无` 日志）；空字幕一律不渲染；
  渲染循环跑过一帧后调 `orb_ready`。

## 2026-10-25 05:35 验证（S5）

- 第一次修后跑验收 T1/T4 假失败：journal/录屏里混进了「重启前旧实例」的正常运行期字幕
  （旧实例在我们重启前恰好被环境声唤醒，弹了 S2 字幕）。
  → 修法：journal 只从最后一个 `Started ...` 标记后算；录屏帧带 epoch，只分析
  `restart 返回之后` 的帧。
- 修正后真机全绿：T1–T5 全 PASS，首帧就绪 840ms，T4 差分 0.00。
- 详细报告：`verify-report.md`。
