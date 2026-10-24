# 强制 X11 后端的副作用核查（2026-10-24 23:30）

改 XWayland（`GDK_BACKEND=x11`）是「用兼容性换能力」，所以专门核了三件事。

## 1. 系统托盘还在 ✅

```
$ gdbus call --session --dest org.kde.StatusNotifierWatcher \
    --object-path /StatusNotifierWatcher \
    --method org.freedesktop.DBus.Properties.Get \
    org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems

[..., ':1.6211/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray', ...]
```
`main_tray` 就是 `TrayIconBuilder::with_id("main-tray")` 注册的那一个 —— 托盘没被 X11 后端搞坏。

## 2. 内存没变差，反而降了 ✅

```
$ python3 /tmp/mem.py <app> <WebKitNetworkProcess> <WebKitWebProcess>
pid 1380653: 141.4 MB PSS   ← morphofly-app
pid 1380679:  20.6 MB PSS   ← WebKitNetworkProcess
pid 1380693: 131.9 MB PSS   ← WebKitWebProcess
合计 293.9 MB PSS
```

对比 V2 记录（docs/v2/PROGRESS.md 里写的 371MB）→ **少了约 77MB（−21%）**。
原因大概率是球体小了一圈、粒子 720→130、窗口 360×360→148×148（WebView 合成面积变小）。

## 3. 缩放/输入法 ⚠️ 没核

gpd 是 3072×1728 @ Scale 1.5 的屏，X11 后端下 GTK 走的是 Xwayland 缩放，
肉眼看不出问题但不等于没问题。**这条留人工**。

## 结论

两条关键副作用（托盘、内存）都没问题；缩放/输入法待人工。
