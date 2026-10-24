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

## 实测验收（2026-10-25 03:24，gpd，CI 产物 e786d34）

自动化脚本：`tools/verify_orb_drag.sh`（两条用例 + 位置断言）

```
[drag] 初始位置=(2908,1516) 显示器=3072x1728+0+0
[drag] A 拖到左半屏：(2982,1590) → (768,576)
[drag] A PASS 期望≈(16,502) 实得=(16,501) 误差=(0,1)
[drag] B 拖回右半屏：(90,575) → (2304,864)
[drag] B PASS 期望≈(2908,790) 实得=(2908,789) 误差=(0,1)
[drag] 全部通过：悬浮球可拖动，且松手贴最近的水平边、垂直位置保留
```

- 拖动是**位置跟随**而不是位移累加：拖动中窗口与指针的偏差稳定在 1px（取整误差）。
- 垂直位置保留了（拖到 y=864 松手 → 落在 789），不再弹回底部。
- 拖动过程中 Rust 日志能对上：
  - `拖动：全局指针坐标单位比例 kx≈… ky≈… guess≈1.0` → 这台机器上 `cursor_position()`
    返回的就是物理像素，自校准会把它归一到 1（别的机器若返回逻辑像素会自动换成 1.5）
  - `拖动开始：后台跟指针搬窗口 win_x=… cursor_x=… offset_x=74 offset_y=74`
  - `拖动结束（后台循环退出）x=… y=…`
  - 松手后 `贴边计算 mode=NearestHorizontal from=Some(...) to_x=16 to_y=…`

## 顺带修掉的两个坑（都是「重登后失效」类）

1. **服务启动崩在 GTK 初始化**：`Authorization required, but no authorization protocol
   specified` → `Failed to initialize GTK`（status=101）。
   原因：服务单元把 `XAUTHORITY=/run/user/1000/xauth_YRAalP` 写死了，gpd 重新登录后
   文件名变成 `xauth_Vvpurg`。改法：新增 `tools/gpd_run_app.sh` 作为 ExecStart，
   启动时现找 XAUTHORITY（先抄 plasmashell 的 environ，退路取最新 `xauth_*`）。
2. **验证脚本同样写死 XAUTHORITY**：`tools/verify_float_ui.sh` 现在启动时自动发现
   并覆盖 `GPD_XAUTHORITY`，重登后不用手工改环境变量。

## 真机 journal 证据（sha=0990b83）

```
拖动：全局指针坐标单位比例 kx=3.88 ky=2.76 guess=3.32   ← 已被丢弃的旧自校准
拖动开始：后台跟指针搬窗口 win_x=2908 win_y=1516
  cursor_x=Some(768.0) cursor_y=Some(576.0) offset_x=74.50 offset_y=74.50 k=1.0
拖动结束（后台循环退出） x=693 y=501
拖动结束：按最后一帧指针补位 x=693 y=501
贴边计算 mode=NearestHorizontal from=Some((693,501)) to_x=16 to_y=501
```

读法与结论：

1. `cursor_position()` 的读数就是**物理像素**（这里 768/576 与目标坐标一致），
   所以 `k` 固定 1.0 即可。
2. `offset = 74.5` 来自前端按下位置（CSS 49.33 × ratio 1.51），比从指针反推准。
3. 后台循环立刻就把窗口摆到位（693,501），说明是**位置跟随**不是位移累加。
4. 曾经加过「用 `指针物理位置 / cursor_position 读数` 反推比例 k」的自校准 ——
   **已经删掉**：拖动开始时指针往往已经离开按下点，反推值是噪声，
   而噪声有时正好落在 `[0.5, 2]` 里被当真（实测 k=0.76，拖到底只走到一半）。
   宁可固定 1.0，也不要随机偏差。
