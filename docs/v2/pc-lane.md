# pc 车道：把 wings-pc 接成第二条真机桌面 e2e

**日期**：2026-10-25 04:07–04:18 CST
**执行**：pc-lane（工作目录 `/tmp/wt-pc-lane`，分支 `feat/pc-lane`，未 commit）
**方式**：App 走 GitHub Actions 产物（不在本机/pc 编译）；pc 上只装 user service + 起 app 做验证，验证完已停。
**相关文件**：`tools/e2e_matrix.sh`（本轮加 pc 车道）、`tools/verify_float_ui.sh`（只复用，未改一行）
**简写**：`ssh pc` = `ssh wings@100.110.222.0`（局域网 192.168.31.20）；`ssh gpd` = `ssh wings@100.107.9.102`

## 结论（先看这个）

- ✅ **pc 车道通了**：`bash tools/e2e_matrix.sh --with-pc --only ui-pos-pc,ui-top-pc,ui-size-pc`
  → 3 项全 PASS，退出码 **0**（输出见 §三.4）。
- pc 已用 CI artifact（run `40127175191` / sha `6687a12`）部署到
  `~/Code_Project/morphofly/target/release/`，user service `morphofly-test` 已建好
  （**static，不随登录自启**，避免一登录就连环开计费会话）。
- ⚠️ 两个坑（细节见 §五）：
  1. **pc 的 X11 缩放是 2**（`Xft.dpi=264`，GTK 侧取整为 2）。`verify_float_ui.sh` 的阈值是按
     gpd 的物理像素（scale=1）写死的，直接在 pc 上跑 `pos` / `size` 会**误报 FAIL**（实测右距
     32 / 底距 128 / 窗口 296）。pc 车道把三个阈值乘 `PC_SCALE=2` 后 PASS —— 这是同一验收标准
     的物理像素换算，不是放水。
  2. **pc 没有扬声器**：app 的「主动汇报」念不出来，于是每 ~30s 重开一次 StepFun 实时会话
     （journal 有时间线）。**会持续计费、也抢 key**。所以验证完已 `systemctl --user stop`，
     没让它常驻；要用时手动 start、用完 stop。
- 默认行为没变：不加 `--with-pc` 时 pc 车道不选，`local` / `gpd` 的任务和命令与改前一致。

---

## 一、事实核对（每条都是本机实测）

| 项目 | 实测命令 | 输出摘要 |
|---|---|---|
| ssh 连通 | `ssh -o BatchMode=yes wings@100.110.222.0 hostname` | `wings-pc` |
| 桌面会话 | `loginctl list-sessions --no-legend`；`loginctl show-session 2 -p Type -p Class -p Seat -p State -p Active` | `2 1000 wings seat0 1186 user tty1 no -`；`Type=wayland Class=user Seat=seat0 State=active Active=yes` |
| XWayland / X11 | `ls /tmp/.X11-unix`；`ps -o args= -p $(pgrep -f "Xwayland :0")` | `X0 X1`；`Xwayland :0 -auth /run/user/1000/xauth_dWvzJS ... -rootless` |
| Wayland | `ls /run/user/1000/wayland-*` | `wayland-1`（KWin 6.8.0） |
| XAUTHORITY | `ls -l /run/user/1000/xauth_*` | `xauth_dWvzJS`（**每次登录都会换名**；`verify_float_ui.sh` 从 app 进程 `/proc` 自动发现，实测自动更新成功） |
| 显示器 | `xrandr --listmonitors`；`xdpyinfo \| grep dimensions` | `DP-1 6034x3394+0+0`（primary）、`HDMI-A-1 2160x3840+6036+0`；X 屏幕 `8196x3840` |
| 缩放 | `xrdb -query \| grep Xft.dpi`；journal 里的 `前端日志 js=度量` | `Xft.dpi: 264`；app 自报 `innerW=107 dpr=2.75 物理=296x296 scale=2` |
| GPU / 驱动 | `nvidia-smi --query-gpu=name,driver_version,memory.total --format=csv,noheader` | `NVIDIA GeForce GTX 1660 Ti, 615.78.08, 6144 MiB` |
| 默认输出（无扬声器） | `pactl get-default-sink`；`pactl list sinks short`；`amixer -c 1 cget numid=41` | `alsa_output.pci-0000_00_1f.3.iec958-stereo`（S/PDIF，`SUSPENDED`）；Front Headphone Jack `values=off` → **没有可用扬声器** |
| 默认输入（有麦） | `pactl get-default-source`；`pactl list sources short`；`amixer -c 1 cget numid=36` | `alsa_input.pci-0000_00_1f.3.analog-stereo`（ALC1220）；Front Mic Jack `values=on` |
| 凭据 | `sed "s/=.*//" ~/.diemeng/credentials` | 只有 `STEPFUN_KEY`（值不打印、不落文档） |
| 工具链 | `command -v ...` / `--version` | python-xlib `(0, 33)`、`xprop`、`xdotool`、`magick`、`chromium`、`node v26.11.1`、`cargo 1.98.1`、`jq` 都有；**没有** `xwininfo` / `wmctrl` |
| DSH | `dsh --version` | `0.2.1-rc.2` |
| 磁盘 | `df -h /home` | 444G 中已用 156G，剩 285G（36%） |
| 会话自启 | `loginctl show-user wings -p Linger` | `Linger=yes` |
| 旧二进制 | `ls -la ~/Code_Project/morphofly/target/release/morphofly-app` | 部署前是 10-24 18:44 的旧版，本次已被 CI 产物覆盖 |

CI 产物核对：

```
$ gh run view 40127175191 --json databaseId,headSha,conclusion,workflowName
{"conclusion":"success","databaseId":40127175191,"headSha":"6687a120fdad9d28f50caeb51a063fe7a8e9ef63",...}

$ tar -xzf morphofly-app-linux-x64.tar.gz -C /tmp/pc-ci-extract && cat build.txt   # 本机解包看
sha=6687a120fdad9d28f50caeb51a063fe7a8e9ef63
built=2026-10-24T19:53:53Z
```

---

## 二、部署步骤（可复制）

前置：pc 上必须已有**用户**图形会话（`loginctl` 里 `seat0 tty1` 的 user session）。app 必须走
XWayland（包装脚本已设 `GDK_BACKEND=x11`），否则置顶/贴边查不到（Wayland 下 `set_position` 无效）。

### 1. 本机下载 CI 产物

```bash
CI_RUN=40127175191
ART=morphofly-app-linux-x64
rm -rf /tmp/pc-ci
gh run download "$CI_RUN" -n "$ART" -D /tmp/pc-ci
tar -tzf /tmp/pc-ci/$ART.tar.gz        # ./build.txt ./morphofly-app ./resources/
```

实测：tar.gz 24,408,258 字节，内含 `./morphofly-app`（59MB）、`./resources/`、`./build.txt`。

### 2. 送到 pc 并解压到 `target/release/`

```bash
PC=wings@100.110.222.0
scp -o BatchMode=yes /tmp/pc-ci/morphofly-app-linux-x64.tar.gz "$PC:/tmp/morphofly-ci.tar.gz"
ssh -o BatchMode=yes "$PC" 'set -e
  mkdir -p ~/Code_Project/morphofly/target/release
  tar -xzf /tmp/morphofly-ci.tar.gz -C ~/Code_Project/morphofly/target/release
  ls -l ~/Code_Project/morphofly/target/release/morphofly-app
  cat ~/Code_Project/morphofly/target/release/build.txt
  ldd ~/Code_Project/morphofly/target/release/morphofly-app | grep -c "not found" || true'
```

实测输出：`morphofly-app 61,853,920 字节`；`build.txt` 是 `sha=6687a12… / built=2026-10-24T19:53:53Z`；
`ldd 缺库 = 0`；`resources/` 下 `keywords.txt` + `kws-model/` + `vad-model/` 齐
（`MORPHOFLY_BUNDLE_DIR` 指的就是这个 `resources`）。

### 3. 装启动包装脚本（启动时现找 XAUTHORITY）

```bash
scp -o BatchMode=yes tools/gpd_run_app.sh "$PC:~/Code_Project/morphofly/tools/gpd_run_app.sh"
ssh -o BatchMode=yes "$PC" 'chmod +x ~/Code_Project/morphofly/tools/gpd_run_app.sh'
```

为什么不能省：X11 的 XAUTHORITY 文件名每次登录都换（这台现在是 `xauth_dWvzJS`），
写死在 service 里重登后会 `Authorization required...` → `Failed to initialize GTK`。
脚本会从 plasmashell/kwin 等进程的 `/proc/<pid>/environ` 里抄，退路取最新的 `xauth_*`。

### 4. 建 user service（照抄 gpd 的单元文件思路）

```bash
cat > /tmp/morphofly-test.service <<'EOF'
[Unit]
Description=MorphoFly voice agent (test)

[Service]
Type=simple
Environment=XDG_RUNTIME_DIR=/run/user/1000
Environment=DISPLAY=:0
Environment=GDK_BACKEND=x11
Environment=RUST_LOG=info,morphofly_app=debug
Environment=MORPHOFLY_BUNDLE_DIR=/home/wings/Code_Project/morphofly/target/release/resources
ExecStart=/usr/bin/bash /home/wings/Code_Project/morphofly/tools/gpd_run_app.sh
Restart=no
EOF
ssh -o BatchMode=yes "$PC" 'mkdir -p ~/.config/systemd/user'
scp -o BatchMode=yes /tmp/morphofly-test.service "$PC:~/.config/systemd/user/morphofly-test.service"
ssh -o BatchMode=yes "$PC" 'export XDG_RUNTIME_DIR=/run/user/1000
  systemctl --user daemon-reload
  systemctl --user start morphofly-test
  sleep 8
  systemctl --user is-active morphofly-test'
```

说明：

- 单元文件是照 gpd 现成的 `morphofly-test.service` 抄的。为了抄它，只对 gpd 做了一次**只读**
  `systemctl --user cat morphofly-test`；没有 stop/restart/写任何东西。
- 与 gpd 的差异：`RUST_LOG=info,morphofly_app=debug`（pc 车道只做几何检查，不需要 dsh 调试）；
  **不写死 XAUTHORITY**（交给包装脚本）。
- **故意不 `enable`**：unit 没有 `[Install]`，状态是 `static`，不会随登录自启 —— 防止一登录就自动拉
  app、连环开 StepFun 会话（见 §五.1）。
- 没动 sddm、没配自动登录、没改任何系统配置；只写了 `~/.config/systemd/user/morphofly-test.service`。

实测：`systemctl --user is-active` → `active`，`Main PID: 3005 (morphofly-app)`，
`Memory: 277.2M`。

### 5. 验证 journal 关键日志

```bash
ssh -o BatchMode=yes "$PC" 'export XDG_RUNTIME_DIR=/run/user/1000
  journalctl --user -u morphofly-test --since "-2min" --no-pager -o cat |
  grep -E "小球已贴边|小球已置顶|前端日志 js=启动自检|前端日志 js=度量"'
```

实测：

```
INFO morphofly_app::js: 前端日志 js=启动自检 tauri=true core=true win=true getCurrentWindow=true dpr=2.75
INFO morphofly_app::js: 前端日志 js=度量 innerW=107 dpr=2.75 物理=296x296 scale=2 ratio=2.7663551401869158
INFO morphofly_app: 小球已贴边 x=5706 y=2970 right=true
INFO morphofly_app: 小球已置顶（always on top）
```

判定：`tauri=true` + `小球已贴边` 都有 → 前端和 Rust 侧都活着，窗口确实贴到右下角。
验证完执行 `ssh pc 'export XDG_RUNTIME_DIR=/run/user/1000; systemctl --user stop morphofly-test'`，
现在 pc 上是 `inactive`、无 `morphofly-app` 进程。

---

## 三、三项真机检查：原始输出

### 3.1 默认阈值（gpd 量纲）—— pos / size 会 FAIL

```
$ GPD=wings@100.110.222.0 bash tools/verify_float_ui.sh pos
pos: 实测 window=0x120000d 位置=(5706,2970) 尺寸=296x296 显示器=DP-1 6034x3394+0+0 → 右距=32px 底距=128px
[pos][raw] 显示器几何：DP-1 6034x3394+0+0（显示器数=2，窗口中心=(5854,3118)）
pos: FAIL 右距 32px / 底距 128px（阈值 右≤24 底≤80）
rc=1

$ GPD=wings@100.110.222.0 bash tools/verify_float_ui.sh size
size: 实测 296x296（window=0x120000d 位置=(5706,2970)）
[size][raw] 窗口 296x296（window=0x120000d）> 上限 176x176
size: FAIL 宽 296 > 176; 高 296 > 176（线上还是旧 360 窗口）
rc=1
```

（`top` 在默认阈值下就 PASS，见 3.3。）

### 3.2 为什么 FAIL：不是几何错，是阈值量纲没换算

`app/src-tauri/src/main.rs` 的 `snap_to_edge()`：`margin = 16*scale`，底部再留 `48*scale`，
收起尺寸 `ORB_SIZE*scale`（`ORB_SIZE=148`）。pc 的 `scale=2`，所以：

| 量 | gpd（scale=1） | pc（scale=2） | 实测 pc |
|---|---|---|---|
| 窗口收起尺寸 | 148 | 148×2 = **296** | 296 ✓ |
| 贴边右 margin | 16 | 16×2 = **32** | 右距 32 ✓ |
| 贴底 margin | 16+48 = 64 | (16+48)×2 = **128** | 底距 128 ✓ |

也就是说 **几何完全正确、就是设计值的 2 倍**；`verify_float_ui.sh` 的阈值（右≤24 / 底≤80 /
尺寸≤176）是用 gpd 的物理像素定的，在 pc 上等于拿 32 去比一个按 16 定出来的上限 → 必 FAIL。
脚本本身支持环境变量覆盖（`POS_TOL` / `POS_TOL_BOTTOM` / `SIZE_MAX`），把阈值乘 scale 即可：

- `POS_TOL=24×2=48`
- `POS_TOL_BOTTOM=80×2=160`
- `SIZE_MAX=176×2=352`

### 3.3 按 scale 折算后 —— 三项全 PASS

```
$ GPD=wings@100.110.222.0 POS_TOL=48 POS_TOL_BOTTOM=160 bash tools/verify_float_ui.sh pos
pos: 实测 window=0x120000d 位置=(5706,2970) 尺寸=296x296 显示器=DP-1 6034x3394+0+0 → 右距=32px 底距=128px
pos: PASS 右距 32px / 底距 128px，均在阈值内
rc=0

$ GPD=wings@100.110.222.0 bash tools/verify_float_ui.sh top
top: 实测 _NET_WM_STATE(ATOM) = _NET_WM_STATE_FOCUSED, _NET_WM_STATE_ABOVE, _NET_WM_STATE_STAYS_ON_TOP, _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER
top: PASS 含 _NET_WM_STATE_ABOVE（X11 后端下置顶生效）
rc=0

$ GPD=wings@100.110.222.0 SIZE_MAX=352 bash tools/verify_float_ui.sh size
size: 实测 296x296（window=0x120000d 位置=(5706,2970)）
size: PASS 宽高 296x296 ≤ 352
rc=0
```

旁证：`pos` 判出来的位置 `(5706,2970)` 和 app 自己 journal 里的 `小球已贴边 x=5706 y=2970 right=true`
一致；296=148×2，且窗口在 primary 显示器 `DP-1` 内、没有越界。
`verify_float_ui.sh` 的 XAUTHORITY 自动发现也生效了：
`[verify_float_ui] XAUTHORITY 自动更新：/run/user/1000/xauth_YRAalP → /run/user/1000/xauth_dWvzJS`。

### 3.4 `e2e_matrix.sh` 的 pc 车道实跑

（先 `systemctl --user start morphofly-test` 再跑；跑完 stop）

```
$ bash tools/e2e_matrix.sh --with-pc --only ui-pos-pc,ui-top-pc,ui-size-pc
输出目录：/tmp/morphofly-e2e/20261025-041336
=== preflight（只读，不改任何东西）===
HOST     CAP         OK? DETAIL
（gpd 的 preflight 行略，只贴 pc）
pc       ssh         OK  wings@100.110.222.0
pc       session     OK  wayland=2 x11=2
pc       pipewire    OK  sources=5 default_source=alsa_input.pci-0000_00_1f.3.analog-stereo
pc       capture     OK  alsa_capture_cards=4
pc       app-bin     OK  target/release/morphofly-app
pc       resources   OK  keywords.txt,kws-model,vad-model,
pc       stepfun-key OK  ~/.diemeng/credentials
pc       dsh         OK  0.2.1-rc.2
pc       service     OK  morphofly-test active

  [PASS   ] ui-pos-pc  rc=0   5s
  [PASS   ] ui-top-pc  rc=0   7s
  [PASS   ] ui-size-pc rc=0   5s

=== 汇总 ===（NOTE 列是日志路径，这里裁掉）
TASK       LANE   STATUS   RC
ui-pos-pc  pc     PASS     0
ui-top-pc  pc     PASS     0
ui-size-pc pc     PASS     0

日志：/tmp/morphofly-e2e/20261025-041336    FAIL/TIMEOUT=0  SKIP=0
matrix rc=0
```

服务没起时（也就是现在 pc 的常态）不会误报 FAIL，而是记 SKIP、退出码 2，提示里带目标机：

```
$ bash tools/e2e_matrix.sh --with-pc --only ui-pos-pc,ui-top-pc,ui-size-pc   # morphofly-test 已 stop
  [SKIP   ] ui-pos-pc  服务没在跑，跳过
  [SKIP   ] ui-top-pc  服务没在跑，跳过
  [SKIP   ] ui-size-pc 服务没在跑，跳过
...
日志：/tmp/morphofly-e2e/20261025-041639    FAIL/TIMEOUT=0  SKIP=3
rc=2
```

---

## 四、`tools/e2e_matrix.sh` 改了什么

1. 新增车道 `pc` + 3 个任务，复用 `verify_float_ui.sh`（靠 `GPD=` 覆盖目标机）：
   - `ui-pos-pc`：`GPD=$PC POS_TOL=$((24*PC_SCALE)) POS_TOL_BOTTOM=$((80*PC_SCALE)) tools/verify_float_ui.sh pos`
   - `ui-top-pc`：`GPD=$PC tools/verify_float_ui.sh top`
   - `ui-size-pc`：`GPD=$PC SIZE_MAX=$((176*PC_SCALE)) tools/verify_float_ui.sh size`
2. 新增开关 `--with-pc`：**默认不启用 pc 车道**（默认跑测不依赖 pc 在线）。
   `--only` 里显式点名 pc 任务时临时启用，并打一行说明（防止误以为默认跑了 pc）。
3. 新环境变量：
   - `PC=${MORPHOFLY_PC_SSH:-wings@100.110.222.0}`
   - `PC_SCALE=${MORPHOFLY_PC_SCALE:-2}`（阈值乘它；改 KDE 缩放/换显示器时要同步改）
4. 服务检查通用化：gpd / pc 两条真桌面车道都会先看目标机 `morphofly-test` 是否 `active`；
   没起就把该车道需要窗口的任务记 `SKIP`（提示里带目标机，脚本仍然**不替人起服务**）。
5. pc 车道**不加 flock**（三项都是只读几何，不点桌面）；gpd 车道保持原 flock 独占逻辑。
6. `usage()` 从写死行号（`sed -n '2,26p'`）改成按 `set -uo pipefail` 标记截取，以后 header
   再加参数不会漏打印。
7. 默认行为不变：用 `git show HEAD:tools/e2e_matrix.sh` 跑旧版 dry-run 和新版默认 dry-run 逐行比，
   任务行完全一致（只多了「pc 车道未启用」那行提示）。

自检：

```
$ bash -n tools/e2e_matrix.sh && echo OK
OK
```

默认 dry-run（no pc lane，和改前一致）：

```
$ bash tools/e2e_matrix.sh --dry-run
pc 车道未启用（ui-pos-pc / ui-top-pc / ui-size-pc）；要跑加 --with-pc。
=== dry-run：只打印，不执行 ===
REPO_ROOT=/tmp/wt-pc-lane
GPD=wings@100.107.9.102  PC=wings@100.110.222.0  PC_SCALE=2  lock=/tmp/morphofly-e2e-gpd.lock
with-cost=0  with-pc=0  allow-restart=0
LANE     TASK       TIMEOUT GATE    COMMAND
local    unit       1800s  -       . tools/dev-env.sh && cargo test -q -p morphofly-core
local    voice-core 900s   cost    . tools/dev-env.sh && python3 tools/e2e/run.py --mode real --case wake-01
gpd      ui-pos     180s   -       tools/verify_float_ui.sh pos
gpd      ui-top     120s   -       tools/verify_float_ui.sh top
gpd      ui-size    120s   -       tools/verify_float_ui.sh size
gpd      ui-shots   900s   -       tools/verify_float_ui.sh shots
gpd      ui-click   240s   -       tools/verify_float_ui.sh click
gpd      voice-app  360s   restart tools/gpd_voice_e2e.sh --wake-only

执行顺序：local 与真桌面车道（gpd/pc）同时启动（&）；各车道内串行；gpd 车道持 flock，pc 车道只读不加锁。
```

`--with-pc` dry-run（多出 3 个 pc 任务 + 计费提醒）：

```
$ bash tools/e2e_matrix.sh --with-pc --dry-run
=== dry-run：只打印，不执行 ===
REPO_ROOT=/tmp/wt-pc-lane
GPD=wings@100.107.9.102  PC=wings@100.110.222.0  PC_SCALE=2  lock=/tmp/morphofly-e2e-gpd.lock
with-cost=0  with-pc=1  allow-restart=0
LANE     TASK       TIMEOUT GATE    COMMAND
...
pc       ui-pos-pc  180s   -       GPD=wings@100.110.222.0 POS_TOL=48 POS_TOL_BOTTOM=160 tools/verify_float_ui.sh pos
pc       ui-top-pc  120s   -       GPD=wings@100.110.222.0 tools/verify_float_ui.sh top
pc       ui-size-pc 120s   -       GPD=wings@100.110.222.0 SIZE_MAX=352 tools/verify_float_ui.sh size

pc 车道注意：只跑只读几何检查、不点球。但 pc 上 morphofly-test 若在跑，
            有未念完的任务时 app 会每 ~30s 重开一次 StepFun 会话（计费）；跑完建议 stop。

执行顺序：local 与真桌面车道（gpd/pc）同时启动（&）；各车道内串行；gpd 车道持 flock，pc 车道只读不加锁。
```

---

## 五、已知限制 / 坑

### 1. pc 没有扬声器 → 服务不能常驻，否则连续开 StepFun 会话（计费）

- 默认 sink 是 S/PDIF（`alsa_output.pci-0000_00_1f.3.iec958-stereo`，`SUSPENDED`），前面板 Headphone / Line Out jack
  都是 `off` → **没有可用的扬声器**。
- journal 里反复出现「有任务结果还没念，主动开一轮汇报 pending=1」，session guard 于是反复
  「主动开一轮汇报」→ 每轮重开一条 StepFun 实时 WS 会话（`~/.diemeng/tasks.json` 里只有一个
  已完成任务，怀疑是「念完才算 reported」的判定在无扬声器机器上不成立，待查）。journal 实测时间线：

```
20:08:50 有任务结果还没念，主动开一轮汇报 pending=1
20:08:50 建立语音会话… → 20:08:52 会话已建立 state="S2"
20:09:38 会话结束 stats= played_frames: 265440, first_audio_ms: None
20:09:38 会话结束，状态机已复位到待机
20:09:53 有任务结果还没念，主动开一轮汇报 pending=1
20:09:53 建立语音会话… → 20:09:55 会话已建立 state="S2"
20:10:25 有任务结果还没念，主动开一轮汇报 pending=1
20:10:26 建立语音会话… → 20:10:28 会话已建立 state="S2"
```

- 结论：**pc 的 `morphofly-test` 只适合「要用时 start、用完 stop」**，别 enable、别常驻。
  本次验证完已停：`systemctl --user is-active` → `inactive`，`pgrep -x morphofly-app` → 无进程。
- 副作用：这次验证本身打开了 5 次左右短会话（每次几秒到 ~40s）。这和「不跑语音 e2e」不冲突
  （没有跑任何唤醒/对话用例），但**后续任何人跑 pc 车道前都要知道它会这样花钱**。

### 2. 分数缩放的 DPI 不一致（275%）

GTK 侧 `scale=2`，WebView `dpr=2.75` → 前端 CSS 画布只有 `innerW=107`（不是 148）。
几何（物理 296）是对的，但小球的视觉比例和 gpd 不同、外发光可能被裁。
本次没拿到截图证据：透明窗 `import -window 0x120000d` 只抓出 149 字节的空图（KWin 合成下抓不到），
所以 pc 车道**暂时只做几何判定**，视觉验收仍然在 gpd。这条列为待验。

### 3. 阈值量纲写死在 `verify_float_ui.sh` 里

阈值假设 scale=1。pc 上必须 `POS_TOL` / `POS_TOL_BOTTOM` / `SIZE_MAX` 乘 `PC_SCALE`。
**如果以后改 KDE 缩放、或换显示器**，要同步改 `MORPHOFLY_PC_SCALE`（`--with-pc --dry-run`
会把算好的阈值和 `PC_SCALE` 打出来，方便核对）。根治办法是让脚本自己探测 scale，但本任务
明确禁止改 `verify_float_ui.sh`，所以没动。

### 4. 语音 e2e 不在这台跑

没扬声器（§五.1）→ AEC / 声学回环不成立。麦克风有（ALC1220 前面板 `Front Mic Jack=on`），
但拾音质量未验证。本任务**没有**跑任何打 StepFun 的语音用例，也没占 key。

### 5. pc 的仓库目录是 `pcsync` 副本（不是 git 仓库）

本次只覆盖了 `target/release/{morphofly-app, resources/, build.txt}` 和 `tools/gpd_run_app.sh`；
pc 上 `tools/` 其余脚本是旧版（没有 `e2e_matrix.sh` / `verify_float_ui.sh`），
车道检查是**从本机 ssh 过去跑**的，别在 pc 上跑 e2e_matrix。

### 6. pc 车道是「只读车道」

不点球、不截图、不起停服务；`ui-click` / `ui-shots` 仍只在 gpd。多人同时跑 pc 车道理论上没问题
（都是只读），但启停 `morphofly-test` 还是要人协调。

---

## 六、下一步建议

1. **让 `verify_float_ui.sh` 自动探测远端 scale**（app 自报的 `scale=`、或窗口物理宽 ÷ 148），
   阈值 = 设计值 × scale。这样 pc 车道不用再手动传 `POS_TOL`，改缩放也不会失效。
2. **解决「没扬声器 → 重开会话」**，二选一：
   - a) 用系统已有的 `snd_aloop` 建个虚拟 sink 当 pc 的默认输出，让播放不失败（不改代码）；
   - b) 给 app 加一个 `MORPHOFLY_DISABLE_PROACTIVE=1` 之类的开关，UI-only 车道启动时关掉主动汇报。
   否则 pc 常驻就是持续计费。
3. 把部署做成 `tools/pc_deploy_ci.sh`（照 `gpd_deploy_ci.sh` 的思路、复用 `tools/pc_ssh.sh`，
   host 参数化）——本次受写作用域限制，只手工跑通，没新增脚本。
4. 补 pc 的视觉证据：用 KDE 自带截图（`spectacle -b -o`）或 `grim`，别用 `import -window`；
   顺便确认 §五.2 的 DPI 差异有没有真的裁掉发光。
5. 若要让 pc 参与语音 e2e：先接扬声器 / 虚拟回环，再验证 ALC1220 前面板拾音。
