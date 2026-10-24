# 并行 e2e 可行性：wings-gpd × wings-pc

**日期**：2026-10-25 02:40–02:50 CST
**方式**：只读探查。没有 stop/start/restart 任何服务，没有启动 app，没有跑打 StepFun 的用例。
**相关脚本**：`tools/e2e_matrix.sh`（本轮新增）
**下文简写**：`ssh gpd` = `ssh wings@100.107.9.102`；`ssh pc` = `ssh wings@100.110.222.0`。

## 结论

**能并行：本机（无头）∥ gpd（真桌面）∥ pc（真桌面）三条车道，pc 已于 2026-10-25 04:18 接通（加 `--with-pc`）。**

- 现状：`wings-gpd` 负责「真桌面 + 真麦 + 截图/点击」，`wings-tx-us1` 负责「无头 core 测试」，两条可以同时跑。**pc 现在跑不了任何 e2e**：它停在登录界面（只有 greeter），没有用户图形会话。
- pc 登录桌面后可以加第三条车道，但「真桌面类」永远只有 gpd 一条，所以并行度上限不是 3 倍。
- 打分：**现状 6/10；pc 登录并验证后 7/10；天花板 8/10**（受单台 gpd 桌面 + 同一把 StepFun key 限制）。
- 最大阻碍：不是工具链，是**只有 gpd 一块能操作的桌面**，以及 pc 上没人登录。其次才是 StepFun key 的并发额度（未确认）。

---

## 更新：pc 车道已接通（2026-10-25 04:18）

- wings-pc 已登录桌面（`loginctl` 有 seat0 用户会话，`wayland-1` + XWayland 都在），
  用 CI 产物部署 + user service 起 app 后，`ui-pos-pc / ui-top-pc / ui-size-pc` 三项 **PASS，rc=0**。
- pc 车道**默认关**（`--with-pc` 才跑）：pc 没有扬声器，app 的主动汇报念不出来，
  会每 ~30 秒重开一次 StepFun 实时会话（计费 + 抢 key）。所以服务不常驻，
  用前 `systemctl --user start`、用完 stop。
- pc 的 X11 缩放是 2（`Xft.dpi=264`，窗口 296=148×2），阈值按 `PC_SCALE=2` 折算，
  不是放松标准 —— 是同一个验收标准的物理像素换算。
- 「截图 / 点击」仍然只有 gpd 能用（computer-use MCP 只连 gpd），所以并行上限不是 3 倍。
- 细节与实测输出：[`pc-lane.md`](pc-lane.md)

---

## 一、事实核对

### 1.1 图形会话

| 设备 | 结论 | 实测 |
|---|---|---|
| gpd | ✅ 有 | `ls /run/user/1000/wayland-*` → `wayland-1`；`ls /tmp/.X11-unix` → `X0 X1`；`pgrep -a Xwayland` → `Xwayland :0 -auth /run/user/1000/xauth_YRAalP` |
| pc | ❌ 没有 | `/tmp/.X11-unix` 为空；`ls /run/user/1000/wayland-*` → 不存在；`loginctl list-sessions` 里只有 `c1 959 plasmalogin seat0 ... greeter tty1`；`systemctl --user show-environment` 里没有 `DISPLAY`/`WAYLAND_DISPLAY` |
| tx-us1 | ❌ 没有 | `DISPLAY`/`WAYLAND_DISPLAY`/`XDG_SESSION_TYPE` 全空；`/tmp/.X11-unix` 为空 |

pc 的 `loginctl show-session c1 -p Type` 是 `wayland`，但那是 **greeter 自己的会话**，不是用户桌面。SSH 登录后 `XDG_SESSION_TYPE=tty`。

gpd 的逻辑分辨率（注意不是物理分辨率）：

```
$ XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 kscreen-doctor -o | grep -E 'Geometry|priority'
Geometry: 0,0 2048x1152        priority 1
Geometry: 417,1152 1280x800    priority 2
```

物理面板是 3072×1728 + 1920×1200；四个维度的比值都是 1.5，对得上 KDE 的 150% 缩放，所以 XWayland 里看到的是 2048×1152 + 1280×800。**e2e 的坐标判定用的是逻辑坐标**，写验收标准时别用物理值。

### 1.2 麦克风与扬声器

| 设备 | 麦克风 | 扬声器 |
|---|---|---|
| gpd | ✅ 有（内置 `sofessx8336`） | ⚠️ 有 sink，但声学回环未成立 |
| pc | ✅ **有**（前面板已插，`Front Mic Jack: on`） | ❌ 没有可用输出（面板 jack 全 off，默认 S/PDIF） |
| tx-us1 | ❌ 无（`/dev/snd` 只有 `seq`/`timer`） | ❌ 无 |

**pc 的「没麦」传言不成立**，证据：

```
$ amixer -c 1 cget numid=36        # Front Mic Jack
  ; type=BOOLEAN, access=r-------, values=1
  : values=on
$ arecord -l
card 1: PCH [HDA Intel PCH], device 0: ALC1220 Analog [ALC1220 Analog]
card 1: PCH [HDA Intel PCH], device 2: ALC1220 Alt Analog [ALC1220 Alt Analog]
$ parecord --device=alsa_input.pci-0000_00_1f.3.analog-stereo ... | RMS
rms=133.0 max=525        # 第一次 2 秒
rms=126.7 max=423        # 第二次 2 秒
```

- 主板是 ALC1220，`Front Mic Jack = on` 说明前面板**物理上插了麦克风**；PipeWire 里也有 `alsa_input.pci-0000_00_1f.3.analog-stereo`，未静音、100%。
- 两段各 2 秒的底噪 RMS 稳定在 127–133（s16、16 kHz 单声道），不是数字静音，但没有可控声源，**拾音质量未验证**。
- 更正 `docs/v2/hardware-facts.md`：「`/dev/snd` 没有 capture 设备」这句与现状不符（现在有 `pcmC1D0c`、`pcmC1D2c`）。当时的结论大概率来自「没插麦」而非「没采集设备」。

**pc 没有扬声器**（这也是它做不了 AEC 的真正原因）：

```
$ amixer -c 1 cget numid=41   # Front Headphone Jack → values=off
$ amixer -c 1 cget numid=38   # Line Out Front Jack  → values=off
$ pactl list sinks short
50   alsa_output.pci-0000_00_1f.3.iec958-stereo      # 默认 sink，S/PDIF
57   alsa_output.platform-snd_aloop.0.analog-stereo  # 虚拟
3364 alsa_output.pci-0000_01_00.1.hdmi-stereo        # HDMI，显示器有没有喇叭未知
```

gpd 的音频状态：

```
$ pactl list sources short
6660 ...essx8336.stereo-fallback.monitor ... RUNNING
6661 alsa_input...essx8336.stereo-fallback  ... SUSPENDED   # 内置麦
7274 mfsink.monitor                         ... IDLE        # 虚拟源，当前是默认源
$ pactl get-default-source  → mfsink.monitor
$ pactl get-sink-volume @DEFAULT_SINK@ → 100%, Mute: no
```

要点：gpd 的默认录音源现在被改成了 `mfsink.monitor`（数字注入的产物），**任何「录默认源」的测试都会录到虚拟源**，不是麦克风。`tools/gpd_voice_e2e.sh` 的注释里记录过：放音时麦克风里只有底噪（RMS≈266），所以声学回环在这台机器上**未成立**。

### 1.3 构建与运行依赖

| 项 | gpd | pc | tx-us1 |
|---|---|---|---|
| cargo | ✅ 1.99.0 | ✅ 1.98.1（`/usr/bin/cargo`，系统包） | ⚠️ 1.99.1，在 `~/.cargo/bin`，**不在 SSH 默认 PATH** |
| gtk3 / webkit2gtk-4.1 | ✅/✅ | ✅/✅（webkit 2.54.1、gtk 3.24.52） | ❌/❌（连 `pkg-config` 都没有） |
| libsoup-3.0 / JSC | ✅/✅ | ✅/✅ | ❌ |
| ALSA 开发 | ✅ | ✅ 1.2.16.1 | ⚠️ 走 `tools/dev-env.sh` 的 sysroot（`alsa.pc` 在 `~/.local/sysroot`） |
| release 二进制 | ✅ `target/release/morphofly-app`（02:23 构建） | ✅ 10-24 18:44 构建，`ldd` 缺库数 = 0 | ❌ 编不了 app crate |
| KWS/VAD 资源 | ✅ `resources/{keywords.txt,kws-model,vad-model}` | ✅ 同左 | — |
| core 例子二进制 | — | ❌ 没有 `target/debug/examples/e2e_drive` | ✅ 有（148 MB，可直接跑） |

命令：

```
$ ssh gpd 'cargo --version'                         → cargo 1.99.0
$ ssh pc  'cargo --version; pkg-config --modversion webkit2gtk-4.1 gtk+-3.0'
                                                    → 1.98.1 / 2.54.1 / 3.24.52
$ ssh pc  'ldd target/release/morphofly-app | grep -c "not found"'  → 0
$ bash tools/e2e_matrix.sh --preflight              → 见 §四
```

pc 的仓库 `/home/wings/Code_Project/morphofly` **不是 git 仓库**（`fatal: not a git repository`），是用 `tools/pcsync.sh`（tar over ssh，排除了 `.git`/`target`/凭据）传过去的副本；`app/` 下是 `frontend` + `src-tauri`，工作区成员是 `core` 和 `app/src-tauri`。

### 1.4 StepFun key 与 DSH

```
$ ssh pc  'sed "s/=.*//" ~/.diemeng/credentials'   → STEPFUN_KEY（值长 65）
$ ssh gpd 'sed "s/=.*//" ~/.diemeng/credentials'   → STEPFUN_KEY（值长 65）
$ sha256sum 两边文件 → 哈希相同
$ ssh pc/gpd 'dsh --version'                        → 0.2.1-rc.2（两台一致）
```

结论：**两台用的是同一把 StepFun key**，DSH 各自独立（`dsh --profile acp` 在各自机器上起子进程，`~/.dsh` 也是各自的）。

### 1.5 computer-use MCP：只绑在 gpd，且是单实例

`~/.dsh/profiles/web/cordis.patch.yml:744` 只有一条 entry：

```yaml
- serverName: computer-use-linux
  transport: stdio
  command: ssh
  args: ['-o','BatchMode=yes','-o','ConnectTimeout=15','wings@100.107.9.102',
         'export ... DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_YRAalP && computer-use-linux mcp']
```

- 目标写死 gpd，`DISPLAY=:0` 和 `XAUTHORITY` 也是写死的。
- `XAUTHORITY` 文件名每次登录会变（`xauth_XXXXXX`），现在恰好还是 `xauth_YRAalP`；**重新登录 gpd 后 MCP 会静默失效**，这是已知坑。
- pc 上其实也装了 `~/.local/bin/computer-use-linux`，但没有对应的 server 条目，且没有桌面，用不了。

### 1.6 服务与端口

```
$ systemctl --user is-enabled morphofly-test   → static（不是 enable）
$ unit: ExecStart=.../target/release/morphofly-app, Restart=no,
        Environment=... DISPLAY=:0 GDK_BACKEND=x11 XAUTHORITY=/run/user/1000/xauth_YRAalP
        MORPHOFLY_BUNDLE_DIR=.../target/release/resources
$ systemctl --user is-active morphofly-test    → inactive（02:44:28 被停掉，不是本轮操作）
```

- 端口：gpd 上 8901 空闲（`verify_float_ui.sh shots` 会占用它）；app 不监听端口，只有到 `198.18.0.64:443` 的出站连接（StepFun WS）。
- pc 上 3180 是 DSH web、3080 是 socat 隧道，8901 空闲，与本项目 e2e 无冲突。
- 负载快照：gpd 4 核、load 6.9–8.6（有人开着 21 个 chrome 进程）；pc load 0.10–0.18。

---

## 二、并行矩阵

| 测试类别 | gpd | pc | tx-us1 | 能并行吗 | 冲突/前置 |
|---|---|---|---|---|---|
| 单元/集成（`cargo test -p morphofly-core`） | ✅ | ✅ | ✅（`dev-env.sh`） | ✅ 天然并行 | 各机器各自 CPU/`target` |
| core 级语音链路（`tools/e2e/run.py --mode real` / `e2e_drive`） | ✅ 不需要桌面 | ✅ 需先编一次 | ✅ 已编好 | ✅ 并行 | **共用同一把 StepFun key，并发额度未确认** |
| 悬浮球几何 `pos/top/size` | ✅ | ⚠️ 登录后可试（未验证） | ❌ 无桌面 | ⚠️ 同机串行 | 窗口必须已存在（服务在跑） |
| 视觉 `shots`（无头 chromium） | ✅ | ⚠️ 未验证 | ❌ 无 chromium 需求 | ❌ 同机不可并行 | gpd:8901 端口、gpd 临时目录 |
| 点击 `click`（computer-use MCP） | ✅ | ❌ | ❌ | ❌ 全局独占 | MCP 单实例、单指针 |
| 整机语音（`gpd_voice_e2e.sh`：重启服务 + `mfsink` 注入） | ✅ | ❌ 无服务/无桌面 | ❌ 无音频 | ❌ 同机独占 | 会 `systemctl --user restart` |
| 真麦输入（KWS/ASR 实录） | ✅ | ✅（登录后） | ❌ | ✅ 两台可各跑 | pc 无服务，要手工起 app |
| AEC 真声学回环 | ❌* | ❌ | ❌ | — | *脚本注释记录放音进不了麦（本轮未复测）；pc 没扬声器；只能数字注入 |

**能并行到几分**：现状 2 条车道（gpd ∥ tx-us1），约 **6/10**；pc 登录桌面后 3 条车道，约 **7/10**；因为「真桌面 + 截图 + 点击」永远只有 gpd 一条，「整机语音」也要独占 gpd，所以 **天花板 8/10**。

---

## 三、哪些会互相打架

1. **同一把 StepFun key（已证实）**。两台同时跑语音测试 = 同一个 key 上两条 realtime WS。StepFun 的并发上限**未确认**（仓库文档只写了 Qwen 60 RPM、豆包 QPM 60，没有 StepFun 的数）。最坏情况是第二条被限流/拒绝，表现成「握手失败」而不是「测试失败」，很容易误判。→ 见 §六 的 30 秒探针。

2. **computer-use MCP 只有一个实例，且锁死 gpd**。两路 UI 测试同时用 MCP → 点击会互相插队（同一个 uinput 指针）。所以「UI 点击」在任何时候只能有一个人用。

3. **gpd 的 8901 端口**。`verify_float_ui.sh shots` 会在 gpd 上起静态服务器（默认 8901），同机第二次跑会撞端口并判 SKIP/FAIL。跨机不冲突（pc 用自己的 8901）。

4. **`mfsink` 虚拟源是全局状态**。`gpd_voice_e2e.sh` 会 `pactl load-module module-null-sink sink_name=mfsink` 并改默认录音源。同机第二个语音测试会改同一份默认源，两个测试互相喂错音频。

5. **服务重启会打断一切 UI 测试**。`gpd_voice_e2e.sh` 第 64 行 `systemctl --user restart morphofly-test`，重启后窗口重建，正在跑的 `pos/top/size/click` 全部失真。

6. **截图/证据目录**。`verify_float_ui.sh` 固定写 `RUN_DIR/evidence`，两个 run 同时跑会互相覆盖；`gpd_shot.py` 的输出路径由调用方给，必须每次不同。

7. **gpd 只有 4 核且当前 load 6.9–8.6**。并行任务一多，超时阈值会被误伤（截图/点击本来就要 8 秒级等待）。

8. **同机不要起两个 app 实例**。仓库里没找到 single-instance 插件，两个实例会抢托盘和窗口，且都连 StepFun。

---

## 四、方案：两条车道 + 一个脚本

### 车道划分

| 车道 | 跑什么 | 独占资源 |
|---|---|---|
| `local`（tx-us1，无头） | 单元测试；core 级语音 e2e（可选，花钱） | 无（只吃本机 CPU） |
| `gpd`（真桌面） | 悬浮球几何、截图、点击、整机语音 | gpd 桌面、8901、mfsink、MCP |
| `pc`（未开通） | 登录桌面后可跑几何/真麦类 | pc 桌面 |

### `tools/e2e_matrix.sh`

- 默认只跑「不花钱、不重启服务」的项：`unit` ∥ `ui-pos/top/size`。
- `--with-cost` 才跑打 StepFun 的 `voice-core`；`--allow-restart` 才跑会重启 gpd 服务的 `voice-app`。
- gpd 车道持 `flock`，拿不到锁整体 SKIP，避免两个人同时点同一块桌面。
- 每项都有 `timeout`，不 sleep 轮询、不重试；结果按 PASS/FAIL/SKIP/TIMEOUT 汇总，退出码 `0/1/2`。
- 服务没在跑时，需要窗口的 `ui-*` 自动 SKIP，而不是跑出一堆假 FAIL。

**语法检查**：

```
$ bash -n tools/e2e_matrix.sh
bash -n: OK
```

**`--dry-run`**：

```
$ ./tools/e2e_matrix.sh --dry-run
=== dry-run：只打印，不执行 ===
REPO_ROOT=/home/wings/Code_Project/morphofly
GPD=wings@100.107.9.102  lock=/tmp/morphofly-e2e-gpd.lock  with-cost=0  allow-restart=0
LANE     TASK       TIMEOUT GATE    COMMAND
local    unit       1800s  -       . tools/dev-env.sh && cargo test -q -p morphofly-core
local    voice-core 900s   cost    . tools/dev-env.sh && python3 tools/e2e/run.py --mode real --case wake-01
gpd      ui-pos     180s   -       tools/verify_float_ui.sh pos
gpd      ui-top     120s   -       tools/verify_float_ui.sh top
gpd      ui-size    120s   -       tools/verify_float_ui.sh size
gpd      ui-shots   900s   -       tools/verify_float_ui.sh shots
gpd      ui-click   240s   -       tools/verify_float_ui.sh click
gpd      voice-app  360s   restart tools/gpd_voice_e2e.sh --wake-only

执行顺序：local 车道与 gpd 车道同时启动（&）；各自车道内串行；gpd 车道持 flock。
```

**`--preflight`（只读探测，实测输出）**：

```
$ ./tools/e2e_matrix.sh --preflight
HOST     CAP         OK? DETAIL
gpd      ssh         OK  wings@100.107.9.102
gpd      session     OK  wayland=2 x11=2
gpd      pipewire    OK  sources=3 default_source=mfsink.monitor
gpd      capture     OK  alsa_capture_cards=1
gpd      cargo       OK  cargo 1.99.0 (5f94df478 2026-08-27)
gpd      gtk-deps    OK  gtk3+webkit2gtk-4.1
gpd      app-bin     OK  target/release/morphofly-app
gpd      resources   OK  keywords.txt,kws-model,vad-model,
gpd      stepfun-key OK  ~/.diemeng/credentials
gpd      dsh         OK  0.2.1-rc.2
gpd      service     NO  没有 morphofly-test        ← 当前 inactive
pc       ssh         OK  wings@100.110.222.0
pc       session     NO  wayland=0 x11=0（只有 greeter，无用户桌面）
pc       pipewire    OK  sources=5 default_source=alsa_input.pci-0000_00_1f.3.analog-stereo
pc       capture     OK  alsa_capture_cards=4
pc       cargo       OK  cargo 1.98.1 (797e8a9bc 2026-08-05)
pc       gtk-deps    OK  gtk3+webkit2gtk-4.1
pc       app-bin     OK  target/release/morphofly-app
pc       resources   OK  keywords.txt,kws-model,vad-model,
pc       stepfun-key OK  ~/.diemeng/credentials
pc       dsh         OK  0.2.1-rc.2
pc       service     NO  没有 morphofly-test
```

（`session wayland=2` 里含 `wayland-1.lock`，实际 socket 只有 `wayland-1`。）

**服务没跑时的跳过行为（实跑）**：

```
$ ./tools/e2e_matrix.sh --only ui-pos
  [SKIP   ] ui-pos     服务没在跑，跳过

=== 汇总 ===
TASK       LANE   STATUS   RC     NOTE
ui-pos     gpd    SKIP     0      morphofly-test 没在跑；先 systemctl --user start morphofly-test（本脚本不替你起服务）
日志：/tmp/morphofly-e2e/20261025-024704    FAIL/TIMEOUT=0  SKIP=1
$ echo $?   → 2
```

### 日常命令

```bash
tools/e2e_matrix.sh --preflight                  # 先看两台设备状态
tools/e2e_matrix.sh                              # 单元测试 ∥ 几何三项（不花钱）
tools/e2e_matrix.sh --only unit,ui-pos           # 只跑指定项
tools/e2e_matrix.sh --with-cost                  # 加 core 级语音（打 StepFun）
tools/e2e_matrix.sh --with-cost --allow-restart  # 加整机语音（会重启 gpd 服务）
```

### pc 转正后怎么接

pc 登录桌面后，`verify_float_ui.sh` 可以用环境变量改目标（脚本已支持 `GPD`/`GPD_XAUTHORITY`/`GPD_SERVICE` 覆盖），但**从未在 pc 上验证过**；另外 `click` 依赖 journal 里的状态日志，pc 没有 `morphofly-test`，这一项会退化成 SKIP。需要一次人工试跑：

```bash
# 未验证，仅供下一步验证
GPD=wings@100.110.222.0 \
GPD_XAUTHORITY=/run/user/1000/xauth_XXXXXX \
tools/verify_float_ui.sh pos
```

pc 没有 `morphofly-test` 服务，整机语音得手工起 app（`tools/pc_app.sh start` 那套环境），不能直接复用 `gpd_voice_e2e.sh`。

---

## 五、阻碍清单

| # | 阻碍 | 证据 | 补上要做什么 | 重量 | 补完收益 |
|---|---|---|---|---|---|
| 1 | **pc 没有登录的图形会话** | `session NO wayland=0 x11=0`；greeter 在 tty1 | 去机器上登录一次 KDE；或配 autologin（改 `/etc/sddm.conf.d/`，目前该目录不存在） | 登录 5 分钟；autologin 10–15 分钟 | pc 从 0 车道变 1 车道 |
| 2 | **StepFun 并发未知** | 两台 key 哈希相同；仓库无 StepFun 限流记录 | 跑一次 2 路并发 WS 探针（见 §六），或找 StepFun 提额 | 探针 30 秒、几分钱 | 决定语音测试能否两台并行 |
| 3 | **pc 没有可用扬声器** | `Front Headphone/Line Out = off`，默认 S/PDIF | 插一个音箱/耳机，或确认 HDMI 显示器带喇叭 | 5 分钟硬件 | pc 才能做 AEC/打断 |
| 4 | **gpd 声学回环未成立** | `gpd_voice_e2e.sh` 注释实测：放音后麦克风只有底噪；sink 100% 未静音 | 接 USB 音箱，或用 `--speaker` 模式人工喊话验证 | 5–10 分钟 | AEC 真机验证（目前只能数字注入） |
| 5 | **pc 不是 git 仓库、无常驻服务** | `fatal: not a git repository`；`~/.config/systemd/user` 无 morphofly | 加一个和 gpd 同款的 user service + 同步脚本 | 10–20 分钟 | pc 可跑整机语音 |
| 6 | **MCP 写死 gpd + 随机 XAUTHORITY** | `cordis.patch.yml:744`；`xauth_*` 每次登录变 | 加 pc 的 server 条目（10 分钟）；但两实例会抢指针，不建议 | 10 分钟（收益低） | 基本没有，维持单实例 |
| 7 | **gpd 服务 `Restart=no` 且现在是 inactive** | `is-enabled → static`；`02:44:28 Stopped` | 跑 UI 测试前 `systemctl --user start morphofly-test`；或改 `Restart=on-failure` | 1 分钟 | 少踩「窗口不存在」的假 FAIL |
| 8 | **pc 的 app 二进制可能过期** | 10-24 18:44 构建，无 git，无法对比 | 同步源码后重编（4 核，估计 5–15 分钟） | 10–20 分钟 | pc 上跑的是最新代码 |

---

## 六、未确认项（附验证方法）

1. **StepFun 同 key 并发上限**。30 秒探针（会花几分钱，需用户同意；`websockets` 不在本机默认环境里，用 `uvx --with websockets python -` 跑）：

   ```bash
   # 两条 WS 同时握手，看第二条是否 429/403
   . ~/.diemeng/credentials
   python3 - <<'PY'
   import asyncio, os, websockets
   url="wss://api.stepfun.com/v1/realtime?model=stepaudio-3-realtime-preview"
   h={"Authorization":"Bearer "+os.environ["STEPFUN_KEY"]}
   async def one(i):
       try:
           async with websockets.connect(url, additional_headers=h) as ws:
               print(i, "connected"); await asyncio.sleep(2)
       except Exception as e:
           print(i, "FAIL", type(e).__name__, e)
   asyncio.run(asyncio.gather(one(1), one(2)))
   PY
   ```

2. **pc 的麦克风能不能用**。现在只能证明「jack 检测到插了麦 + 底噪稳定」，需要一个人对着 pc 说句话录 3 秒。
3. **pc 的 HDMI 显示器有没有喇叭**。放测试音，人耳确认。
4. **pc 登录后 `verify_float_ui.sh` 是否兼容**（XWayland 环境差异：pc 有 `xdotool`，gpd 没有）。
5. **`e2e_drive` 在 pc 上能否编译**。pc 有 alsa 1.2.16.1 和 cargo 1.98.1（≥ MSRV 1.91），但没编过这个 example。

---

## 七、本轮边界

- 没有 stop/start/restart 任何服务；02:44:28 gpd 上 `morphofly-test` 的 Stopped 是别人操作的。
- 没有启动 app，没有改 `core/`、`app/`、`.github/`，没有 commit。
- 没有跑任何打 StepFun 的用例（`voice-core`/`voice-app` 一次都没执行）。
- 只写了两个文件：本文档和 `tools/e2e_matrix.sh`。
- 顺带发现：`docs/v2/hardware-facts.md` 关于 pc「没有 capture 设备」的说法已过时，建议合并本文件 §1.2 的更正。
