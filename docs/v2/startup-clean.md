# 启动瞬间干净（「待机」黑气泡压球）

**最后更新**：2026-10-25
**run**：`runs/20261025-0520-startup-clean/`
**验收**：`tools/verify_startup_clean.sh`（T1–T4）

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

**这就是用户说的「黑框」**：它是字幕气泡本身的深色圆角背景 + 文字。

## 根因

`main.rs` 的 `orb_state` 在「没有显式字幕」时回退成 `m.state().label()`，
S0 就是「待机」。前端 `boot()` 拿到后直接 `showCaption`，于是**每次启动必显示一次字幕**，
再经 `orb_tip(show: true)` 把窗口向左撑到 360 宽，形成「黑框 + 字」。
4.2 秒后定时器收起，看起来就像「字和球黏了一会儿」。

> 竞态：如果页面加载比贴边早，`orb_tip` 会被 `ORB_READY` 拦下、窗口不撑开，
> 但字幕元素仍然被塞了文字；反之就是可见黑气泡。所以这个 bug 时隐时现。

### 关于「启动瞬间的空窗口是不是黑框」

一开始怀疑第二个原因：先 `show()` 再等页面画完，首帧前窗口是空的。
但真机 8fps 连拍（覆盖 t=0.5–13s、屏幕中间 + 右下角两块区域）**没有拍到黑框窗口**——
`transparent: true` 的窗口在 KDE 下首帧前就是透明的。用户看到的黑框就是上面的字幕气泡。

因此本轮**没有**加「窗口先设不透明度、等首帧再显形」这类过渡。原因是：

- 没有复现到需要它的现象；
- `gtk_widget_set_opacity` 在 RGBA 透明窗口上走的是 Cairo alpha，和「窗口合成不透明度」
  不是一回事（WebKit 的 native 子窗口也遮不住）；
- 并且一旦恢复失败，用户会得到「进程在、球永远不可见」这种更糟的故障。

保持窗口行为不变，只修「启动不该有字幕」这一条，风险最小。

## 修法

- `orb_state`：没有字幕就返回空串，不再拿状态名顶替。
- 前端 `boot()`：不再显示初始字幕，只打一行
  `启动状态 state=S0 caption=无` 进 journal 当证据；`showCaption` 对空字幕直接 return。
- `spawn_pump`：`last_state` 初始化为当前状态，避免 demo/QA 模式启动时从状态事件
  再推一次「待机」字幕。
- 新增 `app/frontend/src/orb/caption.js`（字幕清洗）+ 单测。

## 验收怎么跑

```bash
# 真机重启 + 连拍 + 判据（T1–T4）
tools/verify_startup_clean.sh

# 只跑日志/单测，不录屏（快）
tools/verify_startup_clean.sh --no-frames
```

| # | 判据 | 修前 | 修后 |
|---|---|---|---|
| T1 | 启动日志 `caption=无`，且重启后无 `气泡窗口已调整 show=true` | FAIL | PASS |
| T2 | 真机连拍：球左侧气泡区差分 < 1.5 | FAIL（黑气泡停 4.2s，实测 5.95） | PASS（0.00） |
| T3 | 位置 / 尺寸 / 置顶不回归 | PASS | PASS |
| T4 | 前端单测全绿 | 13 passed（无回归测试） | 16 passed |

证据：`runs/20261025-0520-startup-clean/evidence/`（red.log / startup-clean-summary.md / 连拍帧）。

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
- **验收会混进「重启前旧实例」的证据**：`systemctl --user restart` 之前的旧进程可能正在
  显示正常的运行期字幕（本次就遇到旧实例被环境声唤醒弹了 S2 字幕），
  journal 和连拍帧都会带上它 → 必须先按「最后一次 `Started` 标记 / restart 返回的 epoch」
  切掉，否则会误判启动不干净（脚本已处理）。
