# 事实核对（S2）— 悬浮窗置顶与视觉迭代

每条都有可复现命令和实测输出。命令在 wings-tx-us1 执行，`ssh gpd` = `ssh wings@100.107.9.102`。

## F1 现在的窗口是 Wayland 客户端，位置在屏幕正中

```bash
# computer-use-linux MCP list_windows
```
实测（2026-10-24 22:03）：
```json
{"app_id":"morphofly-app","client_type":"wayland",
 "bounds":{"height":360,"width":360,"x":844,"y":374}}
```
屏幕逻辑尺寸 2048×1152，居中位置正好是 `(2048-360)/2=844, (1152-360)/2=396`。
**结论：Wayland 下客户端不能自己定位，`tauri.conf.json` 的 `center:true` 生效、`snap_orb` 无效。**

启动日志却写着「小球已贴边 x=3344 y=1456 right=true」—— 它算的是物理像素，
窗口实际没动。**「日志说有」和「实际有」是两件事。**

## F2 Wayland 下也不能强制置顶

同一时刻 `_NET_WM_STATE` 查不到（Wayland 客户端没有 X11 属性），
`alwaysOnTop: true` 在 Wayland 后端等于摆设。

## F3 强制 X11 后端后，置顶真的生效

改 systemd unit 加 `Environment=GDK_BACKEND=x11`，重启：

```bash
ssh gpd 'export DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_YRAalP; \
  xprop -id 0x1200003 _NET_WM_STATE'
```
实测：
```
_NET_WM_STATE(ATOM) = _NET_WM_STATE_ABOVE, _NET_WM_STATE_STAYS_ON_TOP,
                      _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER
```
**结论：走 Xwayland（GDK_BACKEND=x11）就能真置顶 + 真定位。** 这是本次方案的地基。

## F4 强制 X11 后 snap 的位置算对了

日志：`小球已贴边 x=2696 y=1304 right=true`，
X11 屏幕 3072×1728 → `3072-360-16=2696`，`1728-360-64=1304`，对得上。

## F5 X11 后端会让托盘/输入法有差异（风险）

未验证。托盘图标、fcitx 输入、拖拽手感需要真机回归。标「人工核对」。

## F6 x11grab 抓不到被合成的窗口

```bash
ssh gpd 'export DISPLAY=:0 XAUTHORITY=...; ffmpeg -f x11grab -video_size 3072x1728 -i :0 -frames:v 1 /tmp/root.png'
```
实测：整张图只有鼠标指针，小球不在里面（Xwayland 客户端被合成器拿走了）。
**结论：看真实桌面只能用合成器截图（portal / KWin DBus），x11grab 不行。**

## F7 computer-use MCP 的截图回不到本机

MCP `screenshot` 返回图片本体，但本会话网关只转发文本（`gateway returns text only`），
`computer-use-linux screenshot` CLI 也只打印元数据、不落盘。
**结论：视觉核对要自己造落盘管道。**

## F8 无头 Chromium 能渲染前端，秒级出图

```bash
ssh gpd 'cd ~/Code_Project/morphofly/app/frontend && python3 -m http.server 8899 &'
ssh gpd 'chromium --headless=new --disable-gpu --no-sandbox --window-size=360,360 \
  --virtual-time-budget=1500 --screenshot=/tmp/ui-s0.png http://127.0.0.1:8899/index.html'
```
实测输出 `18572 bytes written to file /tmp/ui-s0.png`，图里能看到小球。
**结论：UI 迭代不用重新编译 App，改完前端直接截图，秒级反馈。**
注意：必须走 http（`file://` 下 ES module 会被 CORS 拦），所以要起静态服务器。

## F9 真机 release 编译约 6 分钟

`cargo build --release -p morphofly-app` 在 gpd（N6000/4 核）实测 5m55s ~ 7m03s。
**结论：一次编译很贵，前端迭代必须先无头验证，攒一批再编。**

## F10 当前小球在窗口里只占 17% 半径

`renderer.js:35` `const baseR = (size*dpr) * 0.17 * vis.radius`。
360px 窗口里球直径只有约 122px，四周 300px 全是透明死区 ——
**透明区仍然吃鼠标事件**，等于在桌面上盖了一张 360×360 的隐形膜。
这与「360 悬浮窗」那种紧凑小球的体验差很远，也是本次要改的核心。
