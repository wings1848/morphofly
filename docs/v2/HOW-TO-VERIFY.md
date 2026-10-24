# 怎么验证蝶梦（操作手册）

**给后续接手的 agent 或人看。最后更新：2026-10-25 05:50**

## 环境分工

| 设备 | 地址 | 用途 | 限制 |
|---|---|---|---|
| 本机（云主机）| 100.99.51.5 | 开发、单测、CI 触发 | 无桌面、无音频 |
| wings-gpd | `wings@100.107.9.102` | **真机全流程**（桌面 + 麦克风 + 虚拟音源注入） | N6000 慢；**没有能出声的扬声器** |
| wings-pc | `wings@100.110.222.0` | 第二条真机桌面车道（已登录桌面） | 有麦克风接口、**无可用扬声器**；需先起服务 |

用 `tools/pc_ssh.sh` / `tools/gpd_ssh.sh` 拿目标地址（有兜底逻辑）。

> **编译一律走 GitHub Actions**（用户要求）。设备上只部署 CI 产物：
> `tools/gpd_deploy_ci.sh`。

## 验收脚本清单（一条命令一项）

| 验什么 | 命令 | 说明 |
|---|---|---|
| 本机单测 + 跨平台逻辑 | `cargo test -p morphofly-core --lib` | 271 条；`--features kws` 是 282 条 |
| 前端渲染单测 | `node --test app/frontend/test/` | 13 条 |
| 悬浮球几何（位置/置顶/尺寸/截图/点击）| `tools/verify_float_ui.sh all` | 在 gpd 上跑 |
| **悬浮球拖动** | `tools/verify_orb_drag.sh` | A/B 两条用例，误差 1px |
| **打断（barge-in）** | `tools/verify_bargein.sh` | 注入「停」串，量本地反应 <300ms |
| **音频流防自打断** | `tools/verify_audio_topology.sh [--live]` | `--live` 跑真会话断言 0 打断 |
| 语音全链路 | `tools/gpd_voice_e2e.sh` | 唤醒→派活→DSH→主动汇报 |
| 并行多车道 | `tools/e2e_matrix.sh [--with-pc]` | local ∥ gpd ∥ pc |
| 部署 CI 产物 | `tools/gpd_deploy_ci.sh --run <id>` | 下载 + 装 + 重启服务 |
| 看真机画面 | `python3 tools/gpd_shot.py /tmp/x.png` | 唯一能落盘的截图路径 |

## 一、本地验证（最快，不依赖设备）

```bash
cd ~/Code_Project/morphofly
source tools/dev-env.sh    # 本机必需（无 root，ALSA 解包在 ~/.local/sysroot）

# 单元测试（271 个）
cargo test -p morphofly-core --lib

# 含唤醒词的测试（282 个）
cargo test -p morphofly-core --features kws --lib

# 前端（8 个）
node --test 'app/frontend/test/*.test.mjs'

# e2e（mock 模式，不需要网络）
python3 tools/e2e/run.py --mode mock

# 格式与 lint
cargo fmt --all --check
cargo clippy -p morphofly-core --features kws --lib --tests
```

**本机踩坑**：`--features kws` 需要 `libstdc++.so`（不是 `.so.6`）。
已建软链接：`~/.local/sysroot/usr/lib/x86_64-linux-gnu/libstdc++.so`。

## 二、真机语音链路 e2e（本机能跑，需要网络 + key）

```bash
source tools/dev-env.sh
set -a; . ~/.diemeng/credentials; set +a

# 单条用例
python3 tools/e2e/run.py --mode real --case wake-01

# 全部 14 条
python3 tools/e2e/run.py --mode real
```

它会：
1. 用 edge-tts 合成语音 → miniaudio 解成裸 PCM
2. 调 `core/examples/e2e_drive.rs`（带 `--features kws`，走真实 KWS 前置门）
3. 比对期望，输出 `/tmp/e2e-report.json`

**期望：13/14 通过**（唯一失败是 edge-02「小蝶帮我」KWS 漏检）。

## 三、真机部署（CI 产物，一条命令）

### 3.1 部署

```bash
# 1) 本地改完 push，等 CI 的 build-app-linux 绿
gh run list --workflow=ci.yml --limit 3

# 2) 下载最新成功产物 → 传到设备 → 解到 target/release → 重启服务
tools/gpd_deploy_ci.sh                    # 取 main 上最近一次成功 CI
tools/gpd_deploy_ci.sh --run <run-id>     # 指定某次
tools/gpd_deploy_ci.sh --no-restart       # 只装不重启

# 3) 确认跑的是哪个提交
ssh "$(tools/gpd_ssh.sh)" 'cat ~/Code_Project/morphofly/target/release/build.txt'
```

产物解到 `~/Code_Project/morphofly/target/release/`（二进制 + `resources/`），
服务的 `MORPHOFLY_BUNDLE_DIR` 已经指向那里，**不用改服务配置**。

### 3.2 启动方式（关键：不能用 nohup）

```bash
# ❌ nohup/setsid 在 SSH 会话退出时会被杀，App 立刻消失、日志全空
# ✅ systemd user service，ExecStart 指向启动包装脚本
ssh "$(tools/gpd_ssh.sh)" 'cat ~/.config/systemd/user/morphofly-test.service'
```

服务单元要点：

```ini
[Service]
Type=simple
Environment=XDG_RUNTIME_DIR=/run/user/1000
Environment=DISPLAY=:0
Environment=GDK_BACKEND=x11            # 必须：否则 Wayland 下不能定位/置顶
Environment=MORPHOFLY_BUNDLE_DIR=/home/wings/Code_Project/morphofly/target/release/resources
Environment=RUST_LOG=info,morphofly_app=debug
# ⚠️ 不要写死 XAUTHORITY！每次登录文件名都会变
ExecStart=/usr/bin/bash /home/wings/Code_Project/morphofly/tools/gpd_run_app.sh
Restart=no
```

`tools/gpd_run_app.sh` 会在启动时现找当前会话的 `XAUTHORITY`
（先抄 plasmashell 的 environ，退路取最新的 `xauth_*`）。
写死会导致重新登录后崩在 `Failed to initialize GTK`（status=101）。

### 3.3 确认成功

```bash
ssh "$(tools/gpd_ssh.sh)" 'journalctl --user -u morphofly-test -n 30 --no-pager -o cat | \
  grep -E "托盘|贴边|AEC|KWS|启动自检"'
```

期望看到：

```
系统托盘已就绪（左键显示/隐藏，右键菜单）
小球已贴边 x=2908 y=1516 right=true
AEC 已启用（sonora / AEC3） stream_delay_ms=0
KWS 常驻监听已启动
前端日志 js=启动自检 tauri=true core=true win=true
```

**这五行同时出现 = 蝶梦完整跑起来了。**
（`前端自检` 那行很关键：透明无边框窗口没 devtools，桥有没有通只能看它。）

### 3.4 虚拟音源（语音测试必需）

gpd 没有能出声的扬声器，声学回环做不了，测试改成「注入虚拟音源」：

```bash
ssh "$(tools/gpd_ssh.sh)" '
  pactl load-module module-null-sink sink_name=mfsink || true
  pactl set-default-source mfsink.monitor     # App 从这里录音
  pactl get-default-source'                    # 期望 mfsink.monitor

# 改完默认 source 必须重启 App（采集设备是建会话时打开的）
```

**注意两个坑**：

1. 默认 **sink** 必须留在物理设备，**不要**设成 mfsink ——
   那会让 App 听到自己的 TTS → 自打断（`tools/verify_audio_topology.sh` 会报 FAIL）。
2. 注入链路有 1–2 秒延迟（paplay 启动 + null-sink/monitor + 采集缓冲），
   这是测试台延迟，不是产品延迟。

音频拓扑详见 `docs/v2/audio-topology.md`。

## 四、用 Computer Use 操作 gpd 桌面

已接入 `computer-use-linux`，配置在 `~/.dsh/profiles/web/cordis.patch.yml`。

```python
# 通过 SSH 起 MCP，然后 JSON-RPC 调用
CMD = ("export PATH=$HOME/.bun/bin:$HOME/.local/share/fnm/aliases/default/bin:$PATH && "
       "export XDG_RUNTIME_DIR=/run/user/1000 DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_XXX && "
       "computer-use-linux mcp")
```

18 个工具，常用的：
- `list_windows` — 列窗口（KWin 后端）
- `press_key` — 参数是 `{"key": "Ctrl+Alt+t"}`（**不是 `keys`**）
- `type_text` — `{"text": "...", "app_id": "org.kde.konsole"}`
- `screenshot` — 返回 base64 image
- `activate_window` — 切窗口

**踩坑**：
- `press_key` 的参数名是 `key`，写 `keys` 会报 `missing field`
- 截图分辨率会缩放，要按 `scale` 换算坐标
- 双屏时先确认哪块屏（gpd 是 3072×1728 + 1920×1200）

## 五、语音全链路 / 打断 / AEC

### 5.1 全链路（虚拟音源，一条命令）

```bash
tools/gpd_voice_e2e.sh          # 唤醒 → 说任务 → 派活 → DSH 干活 → 主动汇报
tools/gpd_voice_e2e.sh --wake-only   # 只验唤醒
```

它会：准备音频（edge-tts）→ 设虚拟音源 → 重启 App → 注入唤醒词 →
等会话建立 → 注入任务语音 → 轮询「主动汇报」→ 计数判定。

期望输出：

```
派活次数:          1
DSH 会话新建:      1
主动汇报:          1
✅ 全链路通过：唤醒 1 次、派活 1 次、汇报 1 次
```

**关键点**：语音会话 90 秒空闲会被云端断开，而 DSH 干活要 1–3 分钟 ——
所以「会话已经关了它自己重开会话回来汇报」才是核心验收，不要只看会话内的汇报。

### 5.2 打断（barge-in）

```bash
tools/verify_bargein.sh --dry-run   # 看计划
tools/verify_bargein.sh             # 真机跑一次
```

判定：① 出现打断日志 ② 打断后回到 S2 ③ 本地反应（VAD 判定人声 → 停播）<300ms。

**已知不稳定**：虚拟音源注入有 1–2 秒延迟，模型对短问题只答 2–4 秒，
经常错过「正在说话」的窗口。真机手感请人工验证（接耳机说话时喊「停」）。

### 5.3 音频流 / 防自打断

```bash
tools/verify_audio_topology.sh         # 只查结构（只读）
tools/verify_audio_topology.sh --live  # 真跑一次「只说话不插话」，断言 0 打断
```

会检查：默认 sink/source 是什么、采集源是不是默认输出的 monitor（自打断高危配置）、
App 的播放/录音流是否存在。

### 5.4 AEC（仍然没做真机听觉验证）

代码里已接 sonora / AEC3，但两台设备（gpd、pc）都**没有能出声的扬声器**，
测试台用虚拟音源注入 → App 的输出不进采集 → **没有回声可消**，
所以 AEC 效果在这套环境里测不出来。

能做的验证：
1. 接实体音箱 + 麦克风，外放音量 70%，让它说话时喊「停」
2. 期望 300ms 内停播、且不自我打断
3. 观察日志 `AEC 已启用（sonora / AEC3）` 与打断记录

### 5.5 生成注入音频（手工时用）

```bash
uvx --from edge-tts edge-tts --voice zh-CN-XiaoxiaoNeural \
  --text "小蝶" --write-media /tmp/xiaodie.mp3
scp /tmp/xiaodie.mp3 "$(tools/gpd_ssh.sh):/tmp/"
ssh "$(tools/gpd_ssh.sh)" 'ffmpeg -y -loglevel error -i /tmp/xiaodie.mp3 -ar 16000 -ac 1 /tmp/xiaodie.wav'
ssh "$(tools/gpd_ssh.sh)" 'paplay --device=mfsink /tmp/xiaodie.wav'
```

## 六、常见问题

| 症状 | 原因 | 解决 |
|---|---|---|
| `KWS 需要麦克风` | 采集设备/后端问题 | 确认 `音频后端：PulseAudio`，并确认默认 source 是 mfsink.monitor |
| App 启动后日志全空、进程消失 | nohup 被杀 | 用 systemd service（3.2）|
| `Failed to initialize GTK` + `Authorization required` | 服务里写死了旧的 XAUTHORITY | 用 `tools/gpd_run_app.sh` 作 ExecStart（3.2）|
| `PulseAudio: Unable to create stream: No such entity` | ALSA→PipeWire 录音流 bug | 用 cpal 的 PulseAudio 后端（代码已改）|
| 脚本 grep 不到 `state="S4"` | 老版本二进制把 ANSI 颜色码写进 journal | 升级到 ≥ d4f6ee1；脚本侧也做了去 ANSI |
| `ongoing response already exists` | 一话被切成多段，没取消上一轮就开新轮 | 已修（说完判定 400ms + 开新轮前取消）|
| 打断计数变多/重复 | 本地 VAD 与云端 speech_started 各看见一次 | 已加 2 秒去重（一次开口记一次）|
| 没人说话却记了打断 | 默认 sink 被设成了虚拟 sink（自打断）| 默认 sink 保持物理设备；跑 `verify_audio_topology.sh` |
| 窗口跑到屏幕外 | 贴边坐标按主屏算 | 查 `小球已贴边 x=? y=?`，确认哪块屏 |
| 双屏截图对不上坐标 | 截图缩放 + 多屏 | 先 `xrandr` 确认布局 |
| CI 产物在 gpd SIGILL | AppImage 打包了 AVX 库 | 用 CI 的原生二进制（现在默认就是）|

## 看真机效果（截图）

**最重要的一条：光看日志不算看见，必须看图。**

```bash
# 整个桌面（走 app 自己的 MCP 协议，portal 截图，能落盘）
python3 tools/gpd_shot.py /tmp/desktop.png

# 只截一块（x,y,w,h，按真实分辨率）
python3 tools/gpd_shot.py /tmp/orb.png --crop 2900,1450,172,172
```

为什么不用别的办法（都试过，都不行）：

| 办法 | 结果 |
|---|---|
| `ffmpeg -f x11grab` | 抓不到被合成的窗口，整张图只有鼠标指针 |
| computer-use MCP 的 `screenshot` | 图能拿到，但本会话网关只转发文本 |
| `computer-use-linux screenshot` CLI | 只打印元数据，不落盘 |
| **`tools/gpd_shot.py`** | ✅ 直连 MCP stdio 协议，base64 取回来写文件 |

前端视觉**不用编译**就能看（秒级）：

```bash
ssh gpd 'cd ~/Code_Project/morphofly/app/frontend && nohup python3 -m http.server 8899 &'
ssh gpd 'chromium --headless=new --disable-gpu --no-sandbox --window-size=148,148 \
  --virtual-time-budget=1600 --screenshot=/tmp/ui-S2.png \
  "http://127.0.0.1:8899/index.html?state=S2&demo=1"'
```

⚠️ 判断观感要在**深色背景**上合成，白底会让你误判「太淡」。
