#!/usr/bin/env bash
# tools/verify_float_ui.sh — 蝶梦「悬浮小球」验收脚本
#
# 对应 runs/20261024-2208-ui-ux-360/acceptance.md：
#   A1 pos    启动后贴在右下角（到所在显示器右下角距离 ≤ 24px）
#   A2 top    默认置顶（_NET_WM_STATE 含 _NET_WM_STATE_ABOVE）
#   A3 size   窗口紧凑（宽高都 ≤ 176px）
#   A4 shots  6 个状态各有可区分视觉（无头 chromium 渲染 + magick RMSE 两两比较）
#   A6 click  点一下离开 S0（computer-use-linux MCP 注入点击 + journal 状态日志；做不到就 SKIP）
#
# 这个脚本在开发机跑，通过 ssh 到真机（默认 wings@100.107.9.102）做检查。
# gpd 上 App 由 systemd user 服务 morphofly-test 运行，且必须是 X11 后端
# （Environment=GDK_BACKEND=x11），否则位置/置顶都查不到 —— 见 facts.md F1~F4。
#
# 用法：
#   tools/verify_float_ui.sh <pos|top|size|shots|click|all>
#
# 退出码：0 通过 / 1 失败 / 2 跳过（跳过 = 客观做不到，需人工核对，不算失败）
#
# 可覆盖的环境变量：
#   GPD                  ssh 目标（默认 wings@100.107.9.102）
#   GPD_DISPLAY          gpd 上的 DISPLAY（默认 :0）
#   GPD_XAUTHORITY       gpd 上的 XAUTHORITY（默认 /run/user/1000/xauth_YRAalP）
#   GPD_XDG_RUNTIME_DIR  gpd 上的 XDG_RUNTIME_DIR（journalctl --user 需要）
#   GPD_WAYLAND_DISPLAY  gpd 上的 WAYLAND_DISPLAY（computer-use-linux MCP 需要，默认 wayland-1）
#   GPD_SERVICE          systemd user 服务名（默认 morphofly-test）
#   RUN_DIR              验收 run 目录（默认 <repo>/runs/20261024-2208-ui-ux-360）
#   EVIDENCE_DIR         截图证据目录（默认 $RUN_DIR/evidence）
#   POS_TOL              右下角距离阈值，右/底共用（默认 24）
#   POS_TOL_BOTTOM       底边距离阈值（默认 = POS_TOL；实现里底部预留了 16+48=64px，见 main.rs）
#   SIZE_MAX             窗口宽高上限（默认 176）
#   MIN_RMSE             两状态判定「可区分」的 RMSE 归一化下限（默认 0.005）
#   SHOT_SIZE            截图视口边长（默认自动取线上窗口宽度，拿不到用 360）
#   SHOT_BUDGET          chromium --virtual-time-budget（默认 2000ms）
#   SHOT_PORT            静态服务器端口（默认 8901；8899/8902 被占用）
#   CLICK_TIMEOUT        点击后等状态变化的秒数（默认 8）
#
# 关键事实（都来自 facts.md，别再踩一遍）：
#   F6  ffmpeg x11grab 抓不到被合成的窗口 —— 视觉证据一律用无头 chromium（F8）
#   F7  MCP/CLI 截图回不到本机 —— 所以截图在 gpd 上渲染、比较，再 tar 回本地
#   gpd 没有 xdotool/xwininfo/wmctrl/rsync，但有 python-xlib + magick + chromium
#   窗口几何用 python-xlib 读；置顶用 xprop 读；比较用 magick compare
#   点击用 computer-use-linux MCP 的 click（kwin 后端 uinput 绝对指针）—— 实测 XTEST
#   在这台 KWin/Xwayland 上注入 MotionNotify 不生效（query_pointer 位置不变），别再用它
#
# 前端契约（A4 依赖）：index.html 需要支持 ?state=S0..S5 直接渲染该状态。
#   前端还没实现时，6 张截图会渲染成同一态 → 两两 RMSE 低于阈值 → shots 判 FAIL（这是对的，
#   不是在报假警）。契约说明见 runs/20261024-2208-ui-ux-360/acceptance.md A4。

set -uo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)

GPD=${GPD:-wings@100.107.9.102}
GPD_DISPLAY=${GPD_DISPLAY:-:0}
GPD_XAUTHORITY=${GPD_XAUTHORITY:-/run/user/1000/xauth_YRAalP}
GPD_XDG_RUNTIME_DIR=${GPD_XDG_RUNTIME_DIR:-/run/user/1000}
GPD_WAYLAND_DISPLAY=${GPD_WAYLAND_DISPLAY:-wayland-1}
GPD_SERVICE=${GPD_SERVICE:-morphofly-test}

RUN_DIR=${RUN_DIR:-$REPO_ROOT/runs/20261024-2208-ui-ux-360}
EVIDENCE_DIR=${EVIDENCE_DIR:-$RUN_DIR/evidence}

POS_TOL=${POS_TOL:-24}
POS_TOL_BOTTOM=${POS_TOL_BOTTOM:-$POS_TOL}
SIZE_MAX=${SIZE_MAX:-176}
MIN_RMSE=${MIN_RMSE:-0.005}
SHOT_SIZE=${SHOT_SIZE:-0}
SHOT_BUDGET=${SHOT_BUDGET:-2000}
SHOT_PORT=${SHOT_PORT:-8901}
CLICK_TIMEOUT=${CLICK_TIMEOUT:-8}

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)

# gpd 侧所有远程命令都要显式带这三个环境变量（ssh 不继承桌面会话的环境）
remote_env() {
  printf "export DISPLAY='%s' XAUTHORITY='%s' XDG_RUNTIME_DIR='%s' WAYLAND_DISPLAY='%s';" \
    "$GPD_DISPLAY" "$GPD_XAUTHORITY" "$GPD_XDG_RUNTIME_DIR" "$GPD_WAYLAND_DISPLAY"
}

CHECK_NAME=""
CHECK_STATUS=""
SUMMARY=""
RC=0

say() { printf '%s: %s\n' "$CHECK_NAME" "$*"; }
note() { printf '[%s] %s\n' "${CHECK_NAME:-verify_float_ui}" "$*" >&2; }
raw() { printf '[%s][raw] %s\n' "${CHECK_NAME:-verify_float_ui}" "$*" >&2; }

# 统一判定出口：第 3 行「判定」+ 设置返回码（0 通过 / 1 失败 / 2 跳过）
verdict() { # $1=PASS|FAIL|SKIP $2=说明
  CHECK_STATUS=$1
  SUMMARY=$2
  say "$1 $2"
  case $1 in
    PASS) RC=0 ;;
    SKIP) RC=2 ;;
    *) RC=1 ;;
  esac
  return "$RC"
}

ssh_run() { ssh "${SSH_OPTS[@]}" "$GPD" "$@"; }

usage() {
  sed -n '2,60p' "${BASH_SOURCE[0]}" | sed -n '/^# /p' | sed 's/^# \{0,1\}//'
  exit 2
}

# ---------------------------------------------------------------- 窗口探测
# 一次 ssh 拿全：窗口 id / pid / 绝对坐标 / 尺寸 / 所在显示器几何。
# 用 python-xlib（gpd 已装）而不是 xwininfo/xdotool（gpd 没有）。
PROBE_OK=0
PROBE_ERR=""
WIN_ID=""; WIN_PID=""; WIN_MAINPID=""; WIN_CANDS=""
WIN_X=""; WIN_Y=""; WIN_W=""; WIN_H=""
MON_NAME=""; MON_X=""; MON_Y=""; MON_W=""; MON_H=""; MON_COUNT=""; MON_FALLBACK=""

probe_x11() {
  [ "$PROBE_OK" = 1 ] && return 0
  local out
  out=$(ssh_run "$(remote_env) python3 - '$GPD_SERVICE'" <<'PYPROBE'
import re
import subprocess
import sys

def die(msg):
    print("ERR=" + str(msg).replace("\n", " "))
    sys.exit(3)

try:
    from Xlib import display, X
except ImportError:
    die("gpd 的 python3 缺 python-xlib（pip install python-xlib）")

try:
    d = display.Display()
except Exception as e:  # noqa: BLE001
    die("连不上 X11（检查 DISPLAY/XAUTHORITY）: %r" % (e,))

root = d.screen().root
service = sys.argv[1] if len(sys.argv) > 1 else "morphofly-test"

atom = d.intern_atom("_NET_CLIENT_LIST")
prop = root.get_full_property(atom, X.AnyPropertyType)
ids = list(prop.value) if prop else []
if not ids:
    die("_NET_CLIENT_LIST 为空（没有受管窗口；App 没起来？）")

main_pid = 0
try:
    out = subprocess.check_output(
        ["systemctl", "--user", "show", "-p", "MainPID", "--value", service],
        text=True, stderr=subprocess.DEVNULL,
    ).strip()
    if out.isdigit():
        main_pid = int(out)
except Exception:  # noqa: BLE001
    pass

cands = []
for wid in ids:
    try:
        w = d.create_resource_object("window", wid)
        cls = w.get_wm_class() or ()
        if "morphofly" not in " ".join(cls).lower():
            continue
        g = w.get_geometry()
        pos = root.translate_coords(w, 0, 0)
        wpid = 0
        try:
            p = w.get_full_property(d.intern_atom("_NET_WM_PID"), X.AnyPropertyType)
            if p and len(p.value):
                wpid = int(p.value[0])
        except Exception:  # noqa: BLE001
            pass
        cands.append({
            "id": int(wid), "pid": wpid, "x": int(pos.x), "y": int(pos.y),
            "w": int(g.width), "h": int(g.height),
        })
    except Exception:  # noqa: BLE001
        continue

if not cands:
    die("窗口里没有 morphofly（共 %d 个窗口: %s）"
        % (len(ids), ",".join("0x%x" % i for i in ids)))

chosen = None
if main_pid:
    for c in cands:
        if c["pid"] == main_pid:
            chosen = c
            break
if chosen is None:
    chosen = cands[0]

# 显示器几何：xrandr --listactivemonitors（Xwayland 下也能用）；解析失败退回整个 root
mons = []
try:
    xr = subprocess.check_output(
        ["xrandr", "--listactivemonitors"], text=True, stderr=subprocess.DEVNULL
    )
    for line in xr.splitlines():
        m = re.match(
            r"^\s*\d+:\s+[+*]*(?P<name>[\w.-]+)\s+"
            r"(?P<w>\d+)/\d+x(?P<h>\d+)/\d+\+(?P<x>\d+)\+(?P<y>\d+)",
            line,
        )
        if m:
            mons.append((m.group("name"), int(m.group("x")), int(m.group("y")),
                         int(m.group("w")), int(m.group("h"))))
except Exception:  # noqa: BLE001
    pass
if not mons:
    scr = d.screen()
    mons = [("ROOT", 0, 0, int(scr.width_in_pixels), int(scr.height_in_pixels))]

cx = chosen["x"] + chosen["w"] // 2
cy = chosen["y"] + chosen["h"] // 2
mon = None
for m in mons:
    if m[1] <= cx < m[1] + m[3] and m[2] <= cy < m[2] + m[4]:
        mon = m
        break
fallback = 0
if mon is None:  # 窗口跨屏/不在任何显示器内：退回 root，避免脚本崩
    scr = d.screen()
    mon = ("ROOT", 0, 0, int(scr.width_in_pixels), int(scr.height_in_pixels))
    fallback = 1

print("WINDOW=0x%x" % chosen["id"])
print("PID=%d" % chosen["pid"])
print("MAINPID=%d" % main_pid)
print("CANDIDATES=%d" % len(cands))
print("X=%d" % chosen["x"])
print("Y=%d" % chosen["y"])
print("W=%d" % chosen["w"])
print("H=%d" % chosen["h"])
print("MON_NAME=%s" % mon[0])
print("MON_X=%d" % mon[1])
print("MON_Y=%d" % mon[2])
print("MON_W=%d" % mon[3])
print("MON_H=%d" % mon[4])
print("MON_COUNT=%d" % len(mons))
print("MON_FALLBACK=%d" % fallback)
PYPROBE
  )
  local rc=$?
  if [ "$rc" -ne 0 ] || ! printf '%s\n' "$out" | grep -q '^WINDOW=0x'; then
    PROBE_ERR=$(printf '%s' "$out" | grep -m1 '^ERR=' | cut -c5-)
    [ -n "$PROBE_ERR" ] || PROBE_ERR="ssh/python 探测失败（rc=$rc）"
    raw "探测输出：$(printf '%s' "$out" | tr '\n' ' ')"
    return 1
  fi
  local k v
  while IFS='=' read -r k v; do
    case "$k" in
      WINDOW) WIN_ID=$v ;;
      PID) WIN_PID=$v ;;
      MAINPID) WIN_MAINPID=$v ;;
      CANDIDATES) WIN_CANDS=$v ;;
      X) WIN_X=$v ;;
      Y) WIN_Y=$v ;;
      W) WIN_W=$v ;;
      H) WIN_H=$v ;;
      MON_NAME) MON_NAME=$v ;;
      MON_X) MON_X=$v ;;
      MON_Y) MON_Y=$v ;;
      MON_W) MON_W=$v ;;
      MON_H) MON_H=$v ;;
      MON_COUNT) MON_COUNT=$v ;;
      MON_FALLBACK) MON_FALLBACK=$v ;;
    esac
  done <<< "$out"
  if [ "$WIN_CANDS" -gt 1 ] 2>/dev/null; then
    note "有 $WIN_CANDS 个 morphofly 窗口，取 PID 匹配 $GPD_SERVICE 的那个（pid=$WIN_PID mainpid=$WIN_MAINPID）"
  fi
  PROBE_OK=1
  return 0
}

probe_failed() {
  say "实测 拿不到窗口探测结果：$PROBE_ERR"
  verdict FAIL "无法测量（gpd 上 App 未运行 / 不是 X11 后端 / SSH 不通？见 stderr）"
}

# ---------------------------------------------------------------- A1 pos
check_pos() {
  CHECK_NAME=pos
  say "查 窗口到所在显示器右下角的距离（阈值：右≤${POS_TOL}px 底≤${POS_TOL_BOTTOM}px）"
  probe_x11 || { probe_failed; return; }

  local right=$(( MON_X + MON_W - (WIN_X + WIN_W) ))
  local bottom=$(( MON_Y + MON_H - (WIN_Y + WIN_H) ))
  say "实测 window=$WIN_ID 位置=(${WIN_X},${WIN_Y}) 尺寸=${WIN_W}x${WIN_H} 显示器=$MON_NAME ${MON_W}x${MON_H}+${MON_X}+${MON_Y} → 右距=${right}px 底距=${bottom}px"

  local bad=""
  if [ "$right" -lt 0 ]; then bad="右距 ${right}px 已出屏; "; elif [ "$right" -gt "$POS_TOL" ]; then bad="右距 ${right}px > ${POS_TOL}px; "; fi
  if [ "$bottom" -lt 0 ]; then bad="${bad}底距 ${bottom}px 已出屏; "; elif [ "$bottom" -gt "$POS_TOL_BOTTOM" ]; then bad="${bad}底距 ${bottom}px > ${POS_TOL_BOTTOM}px; "; fi

  [ "$MON_FALLBACK" = 1 ] && note "窗口中心不在任何 xrandr 显示器矩形内，退回整个 X11 root 计算"
  if [ -n "$bad" ]; then
    raw "显示器几何：$MON_NAME ${MON_W}x${MON_H}+${MON_X}+${MON_Y}（显示器数=$MON_COUNT，窗口中心=($(( WIN_X + WIN_W / 2 )),$(( WIN_Y + WIN_H / 2 )))）"
    if [ "$bottom" -ge "$POS_TOL_BOTTOM" ] && [ "$bottom" -le 72 ]; then
      raw "提示：底距 ${bottom}px 看起来是 main.rs snap_to_edge() 的 16+48=64px 任务栏预留；若 A1 只约束右边缘，用 POS_TOL_BOTTOM=72 复跑（或改验收表）"
    fi
    verdict FAIL "右距 ${right}px / 底距 ${bottom}px（阈值 右≤${POS_TOL} 底≤${POS_TOL_BOTTOM}）"
  else
    verdict PASS "右距 ${right}px / 底距 ${bottom}px，均在阈值内"
  fi
}

# ---------------------------------------------------------------- A2 top
check_top() {
  CHECK_NAME=top
  say "查 xprop -id <窗口> _NET_WM_STATE 是否含 _NET_WM_STATE_ABOVE"
  probe_x11 || { probe_failed; return; }

  local out
  out=$(ssh_run "$(remote_env) xprop -id '$WIN_ID' _NET_WM_STATE 2>&1")
  say "实测 $(printf '%s' "$out" | tr '\n' ' ' || true)"

  case "$out" in
    *_NET_WM_STATE_ABOVE*)
      verdict PASS "含 _NET_WM_STATE_ABOVE（X11 后端下置顶生效）" ;;
    *)
      raw "xprop 原始输出：$out"
      verdict FAIL "不含 _NET_WM_STATE_ABOVE（置顶没生效；Wayland 后端下属预期，需 GDK_BACKEND=x11）" ;;
  esac
}

# ---------------------------------------------------------------- A3 size
check_size() {
  CHECK_NAME=size
  say "查 窗口逻辑宽高（阈值：宽高都 ≤ ${SIZE_MAX}px）"
  probe_x11 || { probe_failed; return; }

  say "实测 ${WIN_W}x${WIN_H}（window=$WIN_ID 位置=(${WIN_X},${WIN_Y})）"
  local bad=""
  [ "$WIN_W" -gt "$SIZE_MAX" ] && bad="宽 ${WIN_W} > ${SIZE_MAX}; "
  [ "$WIN_H" -gt "$SIZE_MAX" ] && bad="${bad}高 ${WIN_H} > ${SIZE_MAX}; "
  if [ -n "$bad" ]; then
    raw "窗口 ${WIN_W}x${WIN_H}（window=$WIN_ID）> 上限 ${SIZE_MAX}x${SIZE_MAX}"
    verdict FAIL "${bad%; }（线上还是旧 360 窗口）"
  else
    verdict PASS "宽高 ${WIN_W}x${WIN_H} ≤ ${SIZE_MAX}"
  fi
}

# ---------------------------------------------------------------- A4 shots
# 流程：把本地 app/frontend 同步到 gpd 临时目录 → 起静态服务器(8901) →
#       无头 chromium 按 ?state=S0..S5 各渲一张（S0 再渲一张做噪声底）→
#       magick compare -metric RMSE 两两比较 → 把 6 张证据 tar 回本地 evidence 目录。
# 为什么不抓桌面：facts F6，x11grab / MCP 都拿不到被合成的窗口（F7）。
check_shots() {
  CHECK_NAME=shots
  local shot_size=$SHOT_SIZE
  local probe_note=""
  if [ "$shot_size" = 0 ]; then
    if probe_x11; then
      case "$WIN_W" in
        ''|*[!0-9]*) shot_size=360 ;;
        *) if [ "$WIN_W" -ge 100 ] && [ "$WIN_W" -le 1024 ]; then shot_size=$WIN_W; else shot_size=360; fi ;;
      esac
      probe_note="视口取线上窗口宽度 ${shot_size}px"
    else
      shot_size=360
      probe_note="拿不到窗口尺寸，视口退回 360px"
    fi
  fi
  say "查 无头 chromium 渲染 S0~S5 六态，magick compare -metric RMSE 两两比较（判不可区分阈值：RMSE<max(${MIN_RMSE},2×噪声底)）"

  command -v tar >/dev/null 2>&1 || { say "实测 本机缺 tar"; verdict FAIL "无法同步前端到 gpd"; return; }
  local fe_dir="$REPO_ROOT/app/frontend"
  [ -f "$fe_dir/index.html" ] || { say "实测 找不到 $fe_dir/index.html"; verdict FAIL "前端目录不存在"; return; }

  # gpd 上的临时工作目录：带本地 PID，避免和别的 agent 撞
  local base="/tmp/mf-verify-float-$$"
  local tmp; tmp=$(mktemp "${TMPDIR:-/tmp}/mf-shots.XXXXXX") || { say "实测 mktemp 失败"; verdict FAIL "本机临时文件失败"; return; }

  note "$probe_note；同步 $fe_dir → gpd:$base/fe"
  if ! tar -C "$fe_dir" -czf - . 2>/dev/null \
      | ssh "${SSH_OPTS[@]}" "$GPD" "rm -rf '$base'; mkdir -p '$base/fe'; tar -xzf - -C '$base/fe'"; then
    say "实测 前端同步到 gpd 失败"
    raw "tar | ssh 同步失败（base=$base）"
    rm -f "$tmp"
    verdict FAIL "无法把前端同步到 gpd 做无头渲染"
    return
  fi

  note "在 gpd 上渲染 7 张（6 态 + 1 张噪声底），每张约 20s，预计 2~3 分钟，请等..."
  ssh_run "$(remote_env) timeout 2400 bash -s -- '$base' '$SHOT_PORT' '$SHOT_BUDGET' '$shot_size'" <<'RSHOT' > "$tmp" 2>"$tmp.err"
set -u
base=$1; port=$2; budget=$3; size=$4
fe="$base/fe"; shots="$base/shots"
rm -rf "$shots"; mkdir -p "$shots"

timeout 900 python3 -m http.server "$port" --bind 127.0.0.1 --directory "$fe" >"$base/http.log" 2>&1 &
srv=$!
trap 'kill "$srv" 2>/dev/null' EXIT

ready=0
for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 \
        21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40; do
  if python3 -c "import socket,sys; s=socket.socket(); s.settimeout(0.3); sys.exit(0 if s.connect_ex(('127.0.0.1',$port))==0 else 1)" 2>/dev/null; then
    ready=1; break
  fi
  sleep 0.25
done
if [ "$ready" != 1 ]; then
  echo "ERR_SERVER|静态服务器没起来（端口 $port 被占用？）"
  echo "ERR_SERVER_LOG|$(tail -2 "$base/http.log" 2>/dev/null | tr '\n' ' ')"
  exit 4
fi

shot() { # $1=名字 $2=query；超时/失败重试一次（gpd 上可能被别的任务挤）
  local name=$1 query=$2 out="$shots/ui-$1.png"
  local attempt=1 rc=1
  while [ "$attempt" -le 2 ]; do
    timeout 150 chromium --headless=new --disable-gpu --no-sandbox --hide-scrollbars \
      --force-device-scale-factor=1 --no-first-run --no-default-browser-check \
      --disable-background-networking --disable-component-update --disable-sync \
      --disable-extensions --disable-breakpad --disable-crash-reporter \
      --disable-dev-shm-usage --mute-audio --log-level=3 \
      --user-data-dir="$base/cprof" --window-size="$size,$size" \
      --virtual-time-budget="$budget" --screenshot="$out" \
      "http://127.0.0.1:$port/index.html$query" >"$base/chromium-$name.log" 2>&1
    rc=$?
    [ -s "$out" ] && break
    echo "RETRY|$name|attempt=$attempt|rc=$rc"
    attempt=$(( attempt + 1 ))
  done
  if [ -s "$out" ]; then
    echo "SHOT|$name|$(stat -c%s "$out")|$(magick identify -format '%wx%h' "$out" 2>/dev/null)"
  else
    echo "SHOTFAIL|$name|rc=$rc|$(tail -3 "$base/chromium-$name.log" 2>/dev/null | tr '\n' ' ')"
  fi
}

shot S0  "?state=S0"
shot S0b "?state=S0"
shot S1  "?state=S1"
shot S2  "?state=S2"
shot S3  "?state=S3"
shot S4  "?state=S4"
shot S5  "?state=S5"

cmp_pair() { # $1 $2 = 状态名
  local a=$1 b=$2 rawout norm
  rawout=$(magick compare -metric RMSE "$shots/ui-$a.png" "$shots/ui-$b.png" null: 2>&1 || true)
  norm=$(printf '%s' "$rawout" | sed -n 's/.*(\([0-9.]*\)).*/\1/p')
  printf 'PAIR|%s|%s|%s|%s\n' "$a" "$b" "$rawout" "${norm:-ERR}"
}

rawout=$(magick compare -metric RMSE "$shots/ui-S0.png" "$shots/ui-S0b.png" null: 2>&1 || true)
norm=$(printf '%s' "$rawout" | sed -n 's/.*(\([0-9.]*\)).*/\1/p')
printf 'NOISE|S0|S0b|%s|%s\n' "$rawout" "${norm:-ERR}"

cmp_pair S0 S1; cmp_pair S0 S2; cmp_pair S0 S3; cmp_pair S0 S4; cmp_pair S0 S5
cmp_pair S1 S2; cmp_pair S1 S3; cmp_pair S1 S4; cmp_pair S1 S5
cmp_pair S2 S3; cmp_pair S2 S4; cmp_pair S2 S5
cmp_pair S3 S4; cmp_pair S3 S5
cmp_pair S4 S5
echo "DONE|shots"
RSHOT
  local render_rc=$?

  if ! grep -q '^DONE|shots' "$tmp"; then
    if grep -q '^ERR_SERVER|' "$tmp"; then
      say "实测 $(grep -m1 '^ERR_SERVER|' "$tmp" | cut -d'|' -f2-)"
      raw "http.server 日志：$(grep -m1 '^ERR_SERVER_LOG|' "$tmp" | cut -d'|' -f2-)"
      ssh_run "rm -rf '$base'" >/dev/null 2>&1 || true
      rm -f "$tmp" "$tmp.err"
      verdict SKIP "需人工核对：端口 ${SHOT_PORT} 被占用，静态服务器起不来（换 SHOT_PORT 或先清端口）"
      return
    fi
    say "实测 gpd 侧渲染/比较没有跑完（rc=$render_rc）"
    raw "stdout 尾部：$(tail -5 "$tmp" | tr '\n' ' ')"
    [ -s "$tmp.err" ] && raw "stderr 尾部：$(tail -8 "$tmp.err" | tr '\n' ' ')"
    ssh_run "rm -rf '$base'" >/dev/null 2>&1 || true
    rm -f "$tmp" "$tmp.err"
    verdict FAIL "无头渲染流程中断（见 stderr 原始输出）"
    return
  fi

  # ---- 解析实测结果 ----
  local tag a b c d missing=0 fail_shot=""
  local noise="ERR" noise_raw=""
  local -a pair_entries=()
  while IFS= read -r line; do
    tag=${line%%|*}
    case "$tag" in
      SHOT)
        IFS='|' read -r _ a b c <<< "$line"
        if [ "$c" != "${shot_size}x${shot_size}" ]; then
          missing=$(( missing + 1 ))
          fail_shot="${fail_shot}${a} 尺寸 ${c}≠${shot_size}x${shot_size}; "
        fi
        ;;
      SHOTFAIL)
        IFS='|' read -r _ a b c <<< "$line"
        missing=$(( missing + 1 ))
        fail_shot="${fail_shot}${a} 渲染失败($b $c); "
        ;;
      NOISE)
        IFS='|' read -r _ a b noise_raw noise <<< "$line"
        ;;
      PAIR)
        IFS='|' read -r _ a b c d <<< "$line"
        pair_entries+=("$a|$b|$c|$d")
        ;;
    esac
  done < "$tmp"

  local threshold
  threshold=$(awk -v m="$MIN_RMSE" -v n="$noise" 'BEGIN{
    if (n !~ /^[0-9.]+$/) n = 0;
    t = 2 * n; if (m > t) t = m;
    printf "%.6f", t
  }')

  local similar=0 min_norm="" min_pair="" detail=""
  local entry
  for entry in "${pair_entries[@]}"; do
    IFS='|' read -r a b c d <<< "$entry"
    if [ "$d" = "ERR" ] || [ -z "$d" ]; then
      missing=$(( missing + 1 ))
      fail_shot="${fail_shot}${a}-${b} 比较失败; "
      continue
    fi
    if awk -v v="$d" -v t="$threshold" 'BEGIN{exit !(v+0 < t+0)}'; then
      similar=$(( similar + 1 ))
      detail="${detail}${a}-${b}=${d} "
      if [ -z "$min_norm" ] || awk -v v="$d" -v m="$min_norm" 'BEGIN{exit !(v+0 < m+0)}'; then
        min_norm=$d; min_pair="$a-$b"
      fi
    fi
  done
  local npairs=${#pair_entries[@]}

  # ---- 证据回传：6 张 ui-S0..S5.png 落到本地 evidence 目录（S0b 噪声底图不落地） ----
  mkdir -p "$EVIDENCE_DIR"
  rm -f "$EVIDENCE_DIR"/ui-S[0-9].png
  local ev_tar_ok=1
  ssh_run "cd '$base/shots' && tar -czf - ui-S[0-9].png" 2>/dev/null \
    | tar -xzf - -C "$EVIDENCE_DIR" 2>/dev/null || ev_tar_ok=0
  local got=0 lost=0 s
  for s in S0 S1 S2 S3 S4 S5; do
    if [ -s "$EVIDENCE_DIR/ui-$s.png" ]; then got=$(( got + 1 )); else lost=$(( lost + 1 )); fi
  done
  missing=$(( missing + lost ))
  ssh_run "rm -rf '$base'" >/dev/null 2>&1 || true
  rm -f "$tmp" "$tmp.err"

  say "实测 证据 ${got}/6 张 ${shot_size}x${shot_size} @ $EVIDENCE_DIR（回传=$([ "$ev_tar_ok" = 1 ] && echo ok || echo 失败)）；噪声底 RMSE=$noise；${npairs} 对中 ${similar} 对判为不可区分（阈值 $threshold，最小 ${min_pair}=${min_norm:-无}）"

  if [ "$missing" -gt 0 ] || [ "$similar" -gt 0 ]; then
    [ -n "$fail_shot" ] && raw "渲染/回传异常：$fail_shot"
    [ "$similar" -gt 0 ] && raw "判为视觉不可区分的对：$detail"
    if [ "$similar" = "$npairs" ] && [ "$npairs" -gt 0 ]; then
      raw "6 张全部互相不可区分 —— 多半是前端还没支持 ?state= 参数（A4 依赖前端改造，见 acceptance.md）"
    fi
    local why=""
    [ "$missing" -gt 0 ] && why="渲染/回传缺 ${missing} 项; "
    [ "$similar" -gt 0 ] && why="${why}${similar}/${npairs} 对视觉不可区分（RMSE<$threshold）; "
    verdict FAIL "${why%; }"
  else
    verdict PASS "${npairs} 对两两可区分（RMSE 全部 ≥$threshold）"
  fi
}

# ---------------------------------------------------------------- A6 click
check_click() {
  CHECK_NAME=click
  say "查 用 computer-use-linux(kwin/uinput) 点一下窗口中心后，journal 出现状态离开 S0（$GPD_SERVICE）"
  probe_x11 || { probe_failed; return; }

  local cx=$(( WIN_X + WIN_W / 2 ))
  local cy=$(( WIN_Y + WIN_H / 2 ))

  # 1) 读 journal 可用性 + 点击前状态 + 远程时间戳
  local st_out
  st_out=$(ssh_run "$(remote_env) bash -s -- '$GPD_SERVICE'" <<'RSTATE' 2>&1
set -u
service=$1
if ! journalctl --user -u "$service" -n 1 -o cat --no-pager >/dev/null 2>&1; then
  echo "JOURNAL=bad"; exit 0
fi
echo "JOURNAL=ok"
echo "EPOCH=$(date +%s)"
st=$(journalctl --user -u "$service" -o cat --no-pager -n 400 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' | grep -o 'state="S[0-9]"' | tail -1 | grep -o 'S[0-9]' || true)
echo "STATE=${st:-NONE}"
RSTATE
  )
  local journal_ok="" since="" baseline=""
  while IFS='=' read -r a b; do
    case "$a" in
      JOURNAL) journal_ok=$b ;;
      EPOCH) since=$b ;;
      STATE) baseline=$b ;;
    esac
  done <<< "$st_out"
  if [ "$journal_ok" != ok ]; then
    say "实测 journalctl --user -u $GPD_SERVICE 不可用"
    verdict SKIP "需人工核对：读不到 $GPD_SERVICE 的状态日志，无法自动判定点击效果"
    return
  fi

  # 2) 点一下：走 computer-use-linux MCP 的 click（kwin 后端，uinput 绝对指针）。
  #    为什么不用 XTest：这台 KWin/Xwayland 上 XTEST 注入的 MotionNotify 不生效
  #    （实测 query_pointer 位置不变），而 MCP click 实测能把状态从 S0 推到 S1。
  click_once() {
    ssh_run "$(remote_env) python3 - '$1' '$2'" <<'PYCLICK' 2>&1
import json
import subprocess
import sys
import time

x, y = int(sys.argv[1]), int(sys.argv[2])  # X11 root 设备像素（= screenshot 元数据坐标）

try:
    p = subprocess.Popen(
        ["computer-use-linux", "mcp"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )
except FileNotFoundError:
    print("ERR=gpd 上没有 computer-use-linux 命令")
    sys.exit(3)


def send(msg):
    p.stdin.write(json.dumps(msg) + "\n")
    p.stdin.flush()


send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                 "clientInfo": {"name": "verify_float_ui", "version": "1"}}})
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
send({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
      "params": {"name": "click",
                 "arguments": {"x": x, "y": y,
                               "screenshot_after": False,
                               "include_tree": False}}})

deadline = time.time() + 25
while time.time() < deadline:
    line = p.stdout.readline()
    if not line:
        print("ERR=MCP 进程提前退出（kwin 后端不可用？）")
        sys.exit(3)
    try:
        obj = json.loads(line)
    except Exception:  # noqa: BLE001
        continue
    if obj.get("id") != 2:
        continue
    res = obj.get("result") or {}
    if res.get("isError"):
        print("ERR=MCP click 返回错误: " + json.dumps(res, ensure_ascii=False)[:300])
        sys.exit(3)
    text = ""
    for item in res.get("content") or []:
        if item.get("type") == "text":
            text = item.get("text", "")
            break
    p.kill()
    print("CLICKED|%d|%d|%s" % (x, y, text[:200].replace("\n", " ")))
    sys.exit(0)
print("ERR=等 MCP click 回包超时")
sys.exit(3)
PYCLICK
  }

  wait_state() { # $1=baseline $2=mode(changed|s0) $3=timeout
    ssh_run "$(remote_env) bash -s -- '$since' '$1' '$2' '$3' '$GPD_SERVICE'" <<'RWAIT' 2>&1
set -u
since=$1; baseline=$2; mode=$3; timeout=$4; service=$5
deadline=$(( $(date +%s) + timeout ))
last=""
while [ "$(date +%s)" -lt "$deadline" ]; do
  states=$(journalctl --user -u "$service" -o cat --no-pager --since "@$since" 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' | grep -o 'state="S[0-9]"' || true)
  last=$(printf '%s\n' "$states" | grep -o 'S[0-9]' | tail -1 || true)
  case "$mode" in
    changed)
      if [ -n "$last" ] && [ "$last" != "$baseline" ]; then echo "CHANGED|$last"; exit 0; fi ;;
    s0)
      if [ "$last" = S0 ]; then echo "CHANGED|S0"; exit 0; fi ;;
  esac
  sleep 0.4
done
echo "TIMEOUT|last=${last:-none}|baseline=$baseline"
exit 1
RWAIT
  }

  journal_last_state() {
    ssh_run "$(remote_env) journalctl --user -u '$GPD_SERVICE' -o cat --no-pager -n 200 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' | grep -o 'state=\"S[0-9]\"' | tail -1 | grep -o 'S[0-9]' || true"
  }

  # 3) 点击。A6 要求「离开 S0」：若当前不在 S0，先点一下归位（回 S0），再点第二下离开。
  local ck out
  if [ "$baseline" = "S0" ] || [ "$baseline" = "NONE" ]; then
    ck=$(click_once "$cx" "$cy")
    case "$ck" in
      CLICKED*) : ;;
      *)
        say "实测 点击注入没成功：$(printf '%s' "$ck" | tr '\n' ' ')"
        verdict SKIP "需人工核对：computer-use-linux 点击注入不可用，请手动点一下小球确认"
        return ;;
    esac
  else
    note "点击前不在 S0（$baseline），先点一下归位再验证"
    ck=$(click_once "$cx" "$cy")
    case "$ck" in
      CLICKED*) : ;;
      *) say "实测 点击注入没成功：$(printf '%s' "$ck" | tr '\n' ' ')"
         verdict SKIP "需人工核对：computer-use-linux 点击注入不可用，请手动点一下小球确认"; return ;;
    esac
    out=$(wait_state "$baseline" s0 6)
    case "$out" in
      CHANGED*) baseline=S0 ;;
      *)
        say "实测 点击前 state=$baseline，点击后 6s 内没回到 S0"
        raw "wait 输出：$out"
        verdict SKIP "需人工核对：App 当前不在待机态，自动点击无法归位"
        return ;;
    esac
    ck=$(click_once "$cx" "$cy")
    case "$ck" in
      CLICKED*) : ;;
      *) say "实测 第二次点击注入失败"; verdict SKIP "需人工核对：点击注入失败"; return ;;
    esac
  fi

  out=$(wait_state "$baseline" changed "$CLICK_TIMEOUT")
  case "$out" in
    CHANGED*)
      local got=${out#CHANGED|}
      say "实测 点击前 state=${baseline}，点击后 state=${got}（window=$WIN_ID 点击点=($cx,$cy)）"
      # 收尾：点击会真的建语音会话，尽量点回去取消（best-effort，不影响判定）
      if [ "$got" != "S0" ]; then
        local last
        last=$(journal_last_state)
        if [ -n "$last" ] && [ "$last" != "S0" ]; then
          click_once "$cx" "$cy" >/dev/null 2>&1 || true
          note "已补点一次尝试收回 S0（刚才 state=$got）"
        fi
      fi
      verdict PASS "点击触发状态机离开 ${baseline} → ${got}"
      ;;
    *)
      say "实测 点击后 ${CLICK_TIMEOUT}s 内 journal 无状态变化（baseline=${baseline}）"
      raw "wait 输出：$out"
      raw "journal 最近 5 条状态：$(ssh_run "$(remote_env) journalctl --user -u '$GPD_SERVICE' -o cat --no-pager -n 300 2>/dev/null | sed -e 's/\x1b\[[0-9;]*m//g' | grep '推状态给前端' | tail -5 | tr '\n' ' '")"
      verdict FAIL "点击后状态没变化（IPC/点击链路可能坏了）"
      ;;
  esac
}

# ---------------------------------------------------------------- all
run_all() {
  local -a names=(pos top size shots click)
  local -A st_ sum_ cnt_
  local n rc fails=0 skips=0
  printf '\n===== 逐项检查 =====\n'
  for n in "${names[@]}"; do
    printf '\n--- %s ---\n' "$n"
    "check_$n"
    rc=$?
    st_[$n]=$CHECK_STATUS
    sum_[$n]=$SUMMARY
    cnt_[$n]=$rc
    [ "$rc" -eq 1 ] && fails=$(( fails + 1 ))
    [ "$rc" -eq 2 ] && skips=$(( skips + 1 ))
  done

  printf '\n===== 验收总表 =====\n'
  printf '%-6s\t%-6s\t%s\n' "检查" "结果" "说明"
  printf '%-6s\t%-6s\t%s\n' "------" "------" "------"
  local cn
  for n in "${names[@]}"; do
    case "${st_[$n]}" in
      PASS) cn="通过" ;;
      SKIP) cn="跳过" ;;
      *) cn="失败" ;;
    esac
    printf '%-6s\t%s\t%s\n' "$n" "$cn" "${sum_[$n]}"
  done
  printf '\n总计：通过 %d / 失败 %d / 跳过 %d\n' \
    "$(( ${#names[@]} - fails - skips ))" "$fails" "$skips"
  if [ "$fails" -gt 0 ]; then
    return 1
  fi
  return 0
}

# ---------------------------------------------------------------- main
main() {
  local cmd=${1:-all}
  case "$cmd" in
    pos|top|size|shots|click)
      "check_$cmd"
      exit $?
      ;;
    all)
      run_all
      exit $?
      ;;
    -h|--help|help)
      usage
      ;;
    *)
      printf '未知子命令：%s\n\n' "$cmd" >&2
      usage
      ;;
  esac
}

main "$@"
