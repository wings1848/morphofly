# 启动瞬间干净（黑框 / 「待机」黑气泡压球）

**最后更新**：2026-10-25
**run**：`runs/20261025-0520-startup-clean/`
**验收**：`tools/verify_startup_clean.sh`（T1–T5）

## 用户看到的现象

Linux KDE 桌面上启动 morphofly 时：

1. 屏幕先弹出一个黑框 UI；
2. 球左边出现黑色圆角气泡 +「待机」两个字，和球叠在一起；
3. 4.2 秒后气泡消失，一切恢复正常。

## 真机复现（wings-gpd）

重启 `morphofly-test`，看 journal（2026-10-25 05:09 实测）：

```
05:09:52.875  贴边计算 from=Some((1461, 756)) to_x=2908 to_y=1516   ← 窗口先出现在屏幕中间
05:09:52.935  小球已贴边 x=2908 y=1516
05:09:53.352  前端日志 js=启动自检 ... dpr=1.5                        ← 页面这时才加载完
05:09:53.499  气泡窗口已调整 show=true target_w=360 target_x=2696    ← 窗口被向左撑开
05:09:57.701  气泡窗口已调整 show=false                               ← 4.2 秒后才收起
```

用 KWin ScreenShot2 以 8fps 连拍（`tools/gpd_startup_frames.py`），
球左侧确实出现黑气泡 +「待机」：见 `runs/.../evidence/red-visible-bubble/mfirst.jpg`。

## 根因（两条，叠在一起）

### 1. `orb_state` 把状态名当字幕

`main.rs` 的 `orb_state` 原来在「没有显式字幕」时回退成 `m.state().label()`，
S0 就是「待机」。前端 `boot()` 拿到后直接 `showCaption`，于是**每次启动必显示一次字幕**，
再经 `orb_tip(show: true)` 把窗口向左撑到 360 宽，形成「黑框 + 字」。
4.2 秒后定时器收起，看起来就像「字和球黏了一会儿」。

> 竞态：如果页面加载比贴边早，`orb_tip` 会被 `ORB_READY` 拦下、窗口不撑开，
> 但字幕元素仍然被塞了文字；反之就是可见黑气泡。所以这个 bug 时隐时现。

### 2. 先 `show()` 再等页面画完

启动顺序是 `show()` → 等映射 → 定尺寸 → 贴边。`show()` 时 WebKit 还没画第一帧，
窗口在 KDE 下就是一个空的、无内容的黑框；等页面画出来才变成球。
贴边也是在 `show()` 之后做的，所以球还会先从屏幕中间跳一下。

## 修法

- `orb_state`：没有字幕就返回空串，不再拿状态名顶替。
- 新增 `orb_ready` 命令：前端渲染循环跑过一帧后通知 Rust「可以显形了」。
- Linux 启动时先用 GTK 把窗口设成 **全透明**（`linux_set_window_opacity(0.0)`）：
  - 透明状态下的 `show()`、贴边、跳位用户都看不见；
  - 等 `小球已贴边` + 前端 `orb_ready` 之后，主线程恢复不透明度 → 第一帧可见画面
    就是画好的球、且已经在右下角。
  - 兜底：等前端超过 3 秒也照常显示，避免前端异常时球永远不可见。
- 前端：`boot()` 不再显示初始字幕，只打一行
  `启动状态 state=S0 caption=无` 进 journal 当证据；`showCaption` 对空字幕直接 return。

平台范围：只有 Linux 走透明这步（Windows/macOS 没有对应窗口 API，
本轮不改它们的行为）。

## 验收怎么跑

```bash
# 真机重启 + 连拍 + journal 判据（T1–T5）
tools/verify_startup_clean.sh

# 只跑日志判据（快，不录屏）
tools/verify_startup_clean.sh --no-frames
```

判据（对应 `runs/20261025-0520-startup-clean/acceptance.md`）：

| # | 判据 | 修前 | 修后 |
|---|---|---|---|
| T1 | 启动日志 `caption=无`，且无 `气泡窗口已调整 show=true` | FAIL | PASS |
| T2 | 「启动首帧就绪」出现在「小球已贴边」之后 | FAIL（无此日志） | PASS |
| T3 | 首帧就绪 `elapsed_ms < 3000` | 不可测 | PASS |
| T4 | 启动连拍帧：球左侧气泡区域差分 < 1.5 | FAIL（实测 5.95，气泡停 4.2s） | PASS |
| T5 | 贴边位置不回归 | PASS | PASS |

证据：`runs/20261025-0520-startup-clean/evidence/`（red.log / 修后 summary / 连拍帧）。

## 踩坑记录

- **spectacle 截图不能用来抓启动**：`-b -n -f -o` 一次 10–15 秒（走 portal），
  4.2 秒的气泡根本拍不到。KWin 自己的 `org.kde.KWin.ScreenShot2` D-Bus 一次约 40–90ms，
  8–10fps 连拍才够。
- **ScreenShot2 的 pipe 不能等 EOF**：KWin 写完图也不一定关 write end，会白卡 5–10 秒。
  必须按 props 里的 `width/height/stride` 读满字节数就停。
- **`xdpyinfo` 的 dimensions 不是当前显示器**：XWayland 下是 3072x2928（多屏拼起来的 root），
  不是 3072x1728 的活动桌面。位置验收要用 `xrandr --listactivemonitors`（verify_float_ui.sh 已这么做）。
- **这个 bug 有竞态**：页面比贴边早就绪时气泡不展开（只闪一下），
  所以只看「有没有 show=true」会漏判；T1 用前端启动日志 `caption=无` 才稳定。
