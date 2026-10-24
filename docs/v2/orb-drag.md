# 悬浮球拖动（实测：原生拖动在这台机器上无效）

**2026-10-25** ｜ 用户反馈：「我发现悬浮球无法正常拖动」

## 一句话结论

前端原来在 pointermove 里调 Tauri 的 `startDragging()`（系统原生拖动），
在 wings-gpd 的 **KWin / XWayland** 上**窗口一动不动** —— 不是阈值判断错了，
是原生请求到了合成器那边没生效。另外「松手贴边」原来还把 y 强行吸到底部，
就算拖到了也会弹回底部，两个因素叠在一起就是「拖不动」。

## 怎么确认的（可复现）

1. 用 computer-use MCP 的 `drag` 做真系统级拖动（uinput 绝对指针）：

```bash
ssh wings@100.107.9.102 'python3 /tmp/cu.py drag "{\"start_x\":2982,\"start_y\":1590,\"end_x\":700,\"end_y\":500}"'
```

2. 同时用 python-xlib 每 100ms 采样窗口位置：

```
0.00s pos=(2908,1516) size=148x148
done, final=(2908, 1516, 148, 148)
```

拖动的 8 秒里位置**一次都没变**，松手后也没变。
（两次拖动分别拖到右半屏和左半屏，结果都是原位置。）

## 修法

三层，都在 `app/`：

1. **手动伺服拖动**（主要修法）
   `app/frontend/src/main.js` + `app/src-tauri/src/main.rs: orb_drag_move`
   目标位置 = 当前窗口位置 + (指针在窗口内坐标 − 按下时在窗口内坐标)。
   窗口跟着指针走、指针相对位置不变，等价于「抓着球拖」，不依赖合成器。

2. **原生优先、探针兜底**
   先调 `orb_drag_begin`（内部还是 `start_dragging()`），250ms 后调
   `orb_drag_probe` 读窗口位置；没动就切手动模式。这样在原生拖动正常的机器
   （Windows / 别的桌面）上仍然是系统级平滑拖动。

3. **贴边只吸水平边**
   `snap_to_edge(NearestHorizontal)` 不再把 y 强行贴底，保留用户放下的垂直位置
   （只夹到屏幕内）。启动归位 `BottomRight` 行为不变，仍贴右下角。

4. 顺手加了 `orb_log` 命令：前端日志写进 journal。
   透明无边框窗口没有 devtools，「前端到底有没有桥」以前只能瞎猜。

## 验收（待实测，结果补在这里）

