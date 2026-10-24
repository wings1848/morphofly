# 悬浮小球窗口（置顶 / 右下角 / 360 风格）

**最后更新**：2026-10-24

## 一句话

把蝶梦从「360×360 窗口里一个暗淡小光点」做成「紧凑的桌面悬浮小球：默认置顶、启动就在右下角」。

## 三个硬限制（都实测过，别绕）

### 1. Wayland 下既不能定位也不能置顶

Wayland 协议不给客户端全局坐标，所以：
- `set_position`（贴右下角）被合成器忽略
- `alwaysOnTop` 不产生 `_NET_WM_STATE_ABOVE`

实测（改之前），窗口是 `client_type: "wayland"`，位置停在屏幕正中 `(844, 374)`，
而启动日志还写着「小球已贴边 x=3344 y=1456」——**日志说成功，实际没动**。

### 2. 解法：在 Wayland 会话里强制 GTK 走 XWayland

`main.rs` 里的 `force_x11_backend()`：只要同时有 `WAYLAND_DISPLAY` 和
`DISPLAY`（说明有 XWayland），就在 GTK 初始化前设 `GDK_BACKEND=x11`。

实测改之后：
```
_NET_WM_STATE(ATOM) = _NET_WM_STATE_ABOVE, _NET_WM_STATE_STAYS_ON_TOP,
                      _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER
```
位置也对上了：`小球已贴边 x=2696 y=1304`（3072−360−16，1728−360−64）。

想不动它就设环境变量 `MORPHOFLY_NO_X11=1` 或自己给 `GDK_BACKEND`。

### 3. 窗口透明区照样吃鼠标事件

这是「小球不能做大」的真正原因：窗口是矩形，透明的地方点不到下面的窗口。
之前 360×360 的窗口、球只有 122px，等于在桌面盖了一张 360×360 的隐形膜。

现在窗口 **148×148**，球直径约 84px（占窗口 ~57%）。

## 字幕/气泡怎么放

小球窗口放不下字。做法是**按需把窗口向左撑开**（`orb_tip` 命令）：

- 收起：148×148
- 展开：360×148，同时 `x -= 212`，**右边缘钉住** → 屏幕上看球没动，左边多出一块气泡
- 前端把画布钉在窗口右下角（`align-items:flex-end; justify-content:flex-end`），
  这样撑开/收起时球视觉上不跳

为什么不另开一个窗口：多一个 WebView 大约多 30–50MB 内存，而内存是本项目的老大难（PSS 371MB）。

## 验收怎么跑

```bash
tools/verify_float_ui.sh all     # 位置/置顶/尺寸/6状态截图/点击
```

逐条说明见 `runs/20261024-2208-ui-ux-360/acceptance.md`。

## 视觉怎么快速迭代（不用重新编译）

前端是原生 ES module，直接用无头 Chromium 渲染：

```bash
ssh wings@100.107.9.102 'cd app/frontend && nohup python3 -m http.server 8899 &'
ssh wings@100.107.9.102 'chromium --headless=new --disable-gpu --no-sandbox \
  --window-size=148,148 --virtual-time-budget=1500 \
  --screenshot=/tmp/ui-S2.png "http://127.0.0.1:8899/index.html?state=S2&demo=1"'
```

`?state=S2` 强制状态、`?demo=1` 喂假频谱数据，都是给截图用的。
**必须走 http**：`file://` 下 ES module 会被 CORS 拦掉。

## 启动瞬间的黑框是什么（2026-10-25 修正）

用户反馈启动时闪一个黑框 +「待机」黑气泡。8fps 连拍结论：
**黑框就是字幕气泡本身**（窗口撑到 360 宽时左边那块深色圆角气泡），
不是「未画内容的空窗口」——`transparent: true` 的窗口在 KDE 下首帧前就是透明的。

修法不在窗口层，而是 `orb_state` 不再把状态名当字幕、前端启动不再显示字幕。
详见 [startup-clean.md](startup-clean.md)。

之前考虑过「启动时把窗口设成全透明、等首帧再显形」，评估后放弃：

1. 没有复现到需要它的空黑框；
2. `gtk_widget_set_opacity` 对 RGBA 透明窗口走的是 Cairo alpha，不等于合成器不透明度
   （`_NET_WM_WINDOW_OPACITY`），WebKit 的 native 子窗口也遮不住；
3. 恢复不透明度失败会让球永远不可见，风险大于收益。

## 启动时序：两个必踩的坑（真机实测）

### 坑 1：`visible:false` 时窗口尺寸是 0

`outer_size()` 在窗口还没映射时返回 0，贴边公式拿 0 当尺寸算：
`x = 3072 - 0 - 16 = 3056`，窗口右边缘跑到屏幕外（3204 > 3072）。
同时 `set_position` 在未映射时会被 KWin 直接丢掉，窗口最后还是弹在正中。

**修法**：`show()` → 轮询等 `outer_size()` 非 0（最多 1.5s）→ 显式 `set_size(148)`
→ 再贴边。显式设尺寸也是必须的：WebView 内容会把窗口顶大，
旧的 360 布局前端就把它顶成了 200×200（新的 148 布局才收得住）。

### 坑 2：气泡展开/收起用「当前尺寸」反算会漂

`set_size` 和 `set_position` 都是异步的，用「当前尺寸/当前位置」算新坐标，
两次调用之间读到的可能是上一个状态的值。实测收起后窗口停在
(2712,1516)，比应有的 (2908,1516) 差 196px。

**修法**：贴边成功时把归位点记进静态 `ORB_HOME`；
展开 = 归位点左移 `360-148`，收起 = 回到归位点；先 `set_position` 再 `set_size`。

## 踩过的坑

| 坑 | 事实 |
|---|---|
| 用 `ffmpeg -f x11grab` 抓桌面看小球 | 抓不到：Xwayland 客户端被合成器拿走了，整张图只有鼠标指针 |
| 用 computer-use MCP 的 screenshot 看图 | 图片回不到本机（网关只转发文本），CLI 也只打印元数据不落盘 |
| 以为日志「已贴边」就是贴上了 | 坐标算的是物理像素，窗口根本没动（Wayland 下 set_position 无效） |
| 直接改前端就重新编译看效果 | 一次 release 编译 6 分钟；用无头 Chromium 是秒级 |
