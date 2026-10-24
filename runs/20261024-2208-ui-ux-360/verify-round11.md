# 第 11 轮验证：悬浮球拖动（真机）

**2026-10-25 03:24** ｜ 触发：用户反馈「悬浮球无法正常拖动」
**被测：** CI 产物 `morphofly-app` sha=e786d34（GitHub Actions run 40123992473，`build-app-linux`）
**设备：** wings-gpd（KWin + XWayland，3072×1728 逻辑物理同为 1:1 的 X11 根）

## 根因（实测，不是猜）

| 断言 | 怎么测的 | 结果 |
|---|---|---|
| 系统原生 `startDragging()` 在这台机器无效 | computer-use MCP `drag` 做真系统级拖动 + python-xlib 每 100ms 采样窗口位置 | 8 秒采样窗口位置**零变化**，松手后也没变 |
| 前端到 Rust 的桥是好的 | `orb_log` 启动自检写 journal | `tauri=true core=true win=true getCurrentWindow=true` |
| 原生请求会吞 pointerup | 拖动后没有 `贴边计算` 日志 | 确认（改手动后 pointerup 正常到达） |
| 松手贴边把 y 强行贴底，加剧「拖不动」感 | 读代码 `snap_to_edge` | 确认，已改为只吸水平边 |

## 修法与验证

- 拖动全程在 Rust：`orb_drag_begin` 起后台循环（16ms）读全局指针搬窗口，
  `orb_drag_end` 收尾；前端只判断「开始/结束」。
- 验收脚本：`tools/verify_orb_drag.sh`

```
[drag] 初始位置=(2908,1516) 显示器=3072x1728+0+0
[drag] A PASS 期望≈(16,502) 实得=(16,501) 误差=(0,1)
[drag] B PASS 期望≈(2908,790) 实得=(2908,789) 误差=(0,1)
```

## 顺带修掉

- `tools/gpd_run_app.sh`：服务启动时现找 XAUTHORITY（换登录不再崩）
- `tools/verify_float_ui.sh`：自动发现 XAUTHORITY（重登后不用手改）
- 崩溃恢复：启动时把上次没跑完的 `running` 任务标成中断并被主动汇报
  （实测 5 个任务，含卡了很久的 `task_20261024_1827_025`）

## 判定

- 拖动：**PASS**（两条用例，误差 1px）
- 竖屏贴边回归：**PASS**（`verify_float_ui.sh pos` 右距 16px / 底距 64px，阈值内）
- 启动/主动汇报：**PASS**（journal 有 5 条主动汇报）

## 第二轮：补最后一帧 + 固定物理像素（CI sha=0990b83）

第一次自动化验收虽然 PASS，但另一次人工触发的快速拖动暴露了两个问题：

1. 拖动结束没有补最后一帧 → 合成/脚本的瞬时拖动会停在采样点（实测停在半路）。
   修法：`orb_drag_end` 按当前指针再摆一次窗口。
2. 自校准 k 偶尔把噪声当真（日志实测 k=0.76）→ 拖动距离只有 76%。
   修法：`cursor_position()` 在 X11 上就是物理像素，固定 k=1.0，删掉反推。

重跑验收（sha=0990b83）：

```
[drag] A PASS 期望≈(16,502) 实得=(16,501) 误差=(0,1)
[drag] B PASS 期望≈(2908,790) 实得=(2908,789) 误差=(0,1)
```

重启后默认位置回归：`pos PASS 右距 16px / 底距 64px`，重启没有重复主动汇报。
