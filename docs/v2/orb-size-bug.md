# 小球渲染尺寸异常（computer use 发现，2026-10-24）

## 现象

在 wings-gpd 上启动蝶梦，窗口**本应是 360×360 的透明小球**，
实际渲染成一个**覆盖屏幕下半部分的巨大蓝色矩形**。

证据：原截图含私人界面内容，开源前已移除（现象与根因见下方分析）。

## 环境

| 项 | 值 |
|---|---|
| 屏幕 | 2560×1440 @ 1.25x（外接）/ 1920×1200 @ 1.5x（内屏）|
| **KWin Xwayland Scale** | **1.5** |
| 会话 | Plasma 6.8.0 Wayland |
| tauri.conf.json | `width: 360, height: 360, resizable: true, transparent: true` |

## 可能的根因

### 假设 1：HiDPI 逻辑尺寸 vs 物理尺寸混淆
Tauri 的 `width/height` 是**逻辑尺寸**，实际渲染是 `逻辑 × scale_factor`。
360 × 1.5 = 540，但截图里的矩形明显超过 1000px。

**这个假设不足以解释。**

### 假设 2：`transparent: true` + KWin 合成问题
透明窗口在 Wayland 下需要合成器正确支持 alpha。
如果 WebView 的背景不是真透明，会渲染成不透明的大色块。

**更可能**：`app/frontend/src/style.css` 里 body 有背景色，
且 `#drag-region` 是 `position: fixed; inset: 0`，铺满窗口。

### 假设 3：CSS 尺寸单位问题
`canvas` 是 `width=360 height=360`（HTML 属性），
但如果 CSS 里没约束，可能被拉伸。

需要查 `style.css`。

## 待验证

1. 查 `style.css` 里 body/canvas 的尺寸约束
2. 查 Tauri 是否在 HiDPI 下把窗口放大
3. 在 1.0x 缩放下测试

## 为什么这个 bug 之前没发现

之前所有测试都在**无 GUI 环境**跑：
- 单测只测 Rust 逻辑
- e2e 只测音频链路
- **没有人真正看过窗口长什么样**

这是 computer use 接入后立刻发现的第一个真实 UI bug。

## 严重性

**高** —— 用户看到的是一个巨大色块挡住桌面，
不是设计中的「深海一点光」的小球。
这直接违反用户要求「默认显示在桌面合适位置」。
