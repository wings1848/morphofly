# 第 3 轮验证（回归 + 可观测性）

**日期**：2026-10-25 00:40
**对象**：commit `e833ecb`（GitHub Actions 构建的 deb，装到 wings-gpd）

## 回归结果（不需要浏览器的部分）

```
pos : PASS  右距 16px / 底距 64px（window=(2908,1516) 148x148）
top : PASS  _NET_WM_STATE_ABOVE 存在
size: PASS  148x148
```

前端没改，所以 6 状态截图（shots）没有重跑 —— gpd 上 Chrome 占了 100%+ CPU，
headless chromium 会拖到 5 分钟以上。上一次 `all` 是 5/5 通过。

## 语音链路（这轮跑了两遍）

| 次 | delegate_task | 纠正注入 | 说明 |
|---|---|---|---|
| 00:17 | **1**（纠正后） | 1 | ✅ 纠正救回来了，dir 也正确 |
| 00:23 | 0 | 1 | ❌ 纠正注入了，服务端再无事件 |

**这暴露了一个可观测性缺陷**：`voice.rs` 的 Error 分支只处理断线，
其它 error **一个字都不打日志**。所以「没发出去 / 发了被拒 / 在慢慢生成」
三种情况在日志里长得一模一样 —— 白猜了三轮。

已修：非断线 error 一律 WARN 带 code + message。
下次就能看见服务端到底报什么。
