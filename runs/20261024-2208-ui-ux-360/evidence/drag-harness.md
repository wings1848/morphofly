# 拖动逻辑验证（浏览器内夹具）

**2026-10-24 23:35**

## 为什么要这么测
A7「拖动 + 松手贴边」之前没验证过，而且前端有个已知风险：
canvas 盖满窗口时 `data-tauri-drag-region` 收不到指针，靠 JS 兜底
（移动 >5px → `startDragging()`；pointerup → `snap_orb`）。
这段逻辑在真机上要人手拖才能触发，先把它在浏览器里测死。

## 做法
在 gpd 上放一个夹具页 `drag-harness.html`（不进仓库）：
1. 在 import main.js **之前** 装 `window.__TAURI__` 桩（记录所有 invoke + startDragging）
2. 等 main.js 起来后，在球中心派发 pointerdown → pointermove(+40,+20) → pointerup
3. 把记录写进 `document.body.dataset.result`，再用 `--dump-dom` 取出来

命令：
```bash
chromium --headless=new --disable-gpu --no-sandbox --window-size=148,148 \
  --virtual-time-budget=3000 --dump-dom \
  "http://127.0.0.1:8902/drag-harness.html"
```

## 实测输出
```
data-result="{"done":true,
             "cmds":["orb_state","startDragging","snap_orb"],
             "startDragging":1,"snapOrb":1}"
```

## 结论
- 移动 40px 后 `startDragging()` 被调用 1 次 → 拖动阈值（>5px）判定正确
- 松手（pointerup）后 `snap_orb` 被调用 1 次 → 贴边会被触发
- 首帧还调了 `orb_state` → 与 Rust 的初始状态同步正常

**仍未验证**：系统级拖动本身（鼠标按住能不能真的把窗口拽走）——
需要真人手拖，或 uinput 级别的 press-move-release（本机 computer-use 只有 click，没有 drag）。
