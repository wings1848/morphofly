#!/usr/bin/env bash
# tools/verify_orb_drag.sh — 真机验收「悬浮球能拖动」
#
# 背景：用户反馈「悬浮球无法正常拖动」。真机实测原生 startDragging() 在
# gpd 的 KWin/XWayland 上完全无效（采样 8 秒窗口零移动），现在改成
# Rust 侧每 16ms 读全局指针搬窗口。证据与修法见 docs/v2/orb-drag.md。
#
# 判定（两条用例）：
#   A 拖到左半屏 → 松手贴左边缘（x ≈ 屏左 + margin），垂直保留（不弹回底部）
#   B 再拖回右半屏 → 松手贴右边缘（x ≈ 屏右 − 球宽 − margin）
# 容差：横向 12px（贴边位置精确），纵向 48px（抓取偏移 + 采样延迟 + 夹取）
#
# 用法：
#   tools/verify_orb_drag.sh              # 跑 A、B 两条
#   tools/verify_orb_drag.sh --dry-run    # 只打印计划
#
# 退出码：0 通过 / 1 失败 / 2 跳过（gpd 不在线 / 服务没跑 / 没有桌面）

set -uo pipefail
ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT" || exit 1

TOL_X=12
TOL_Y=48
DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

notice() { printf '[drag] %s\n' "$*"; }
fail() { printf '[drag] FAIL %s\n' "$*"; exit 1; }
skip() { printf '[drag] SKIP %s\n' "$*"; exit 2; }

# ── 0) 设备与服务 ──
T=$(bash tools/gpd_ssh.sh 2>/dev/null) || skip "wings-gpd 不在线"
ssh -o BatchMode=yes -o ConnectTimeout=10 "$T" 'systemctl --user is-active morphofly-test' 2>/dev/null |
  grep -qx active || skip "morphofly-test 没在跑（先 tools/gpd_deploy_ci.sh）"

# ── 1) 拿 App 进程的 DISPLAY / XAUTHORITY（每次登录都变，不能写死）──
APPENV=$(ssh -o BatchMode=yes "$T" '
  pid=$(pgrep -f "release/morphofly-app" | head -1)
  [ -n "$pid" ] || { echo "ERR=找不到 morphofly-app 进程"; exit 3; }
  tr "\0" "\n" < "/proc/$pid/environ" |
    grep -E "^(DISPLAY|XAUTHORITY|XDG_RUNTIME_DIR)=" || true') || skip "读不到 App 进程环境"
printf '%s\n' "$APPENV" | grep -q '^DISPLAY=' || skip "App 进程没有 DISPLAY（不在 X11 会话里？）"
DISPLAY_V=$(printf '%s\n' "$APPENV" | sed -n 's/^DISPLAY=//p')
XAUTH_V=$(printf '%s\n' "$APPENV" | sed -n 's/^XAUTHORITY=//p')
notice "设备=$T DISPLAY=$DISPLAY_V"

# ── 2) 探针：读窗口位置 + 所在显示器几何（复用 verify_float_ui.sh 的探测）──
probe() {
  bash tools/verify_float_ui.sh pos 2>/dev/null |
    sed -n 's/.*位置=(\([0-9-]*\),\([0-9-]*\)).*显示器=[^ ]* \([0-9]*\)x\([0-9]*\)+\([0-9-]*\)+\([0-9-]*\).*/\1 \2 \3 \4 \5 \6/p' |
    head -1
}

read -r WX WY MON_W MON_H MON_X MON_Y <<<"$(probe)"
[ -n "${WX:-}" ] || skip "探测不到窗口位置（App 没起来？）"
notice "初始位置=($WX,$WY) 显示器=${MON_W}x${MON_H}+${MON_X}+${MON_Y}"

BALL=148
MARGIN=16
ox=$((BALL / 2))
oy=$((BALL / 2))

# ── 3) 拖动：走 computer-use MCP 的 drag（uinput 绝对指针，绕过 XTest 限制）──
drag() { # $1..$4 = 起终点（设备像素）
  ssh -o BatchMode=yes "$T" "python3 - '$1' '$2' '$3' '$4'" <<'PYDRAG'
import json, subprocess, sys
x0, y0, x1, y1 = (int(a) for a in sys.argv[1:5])
p = subprocess.Popen(["computer-use-linux", "mcp"], stdin=subprocess.PIPE,
                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
def send(m):
    p.stdin.write(json.dumps(m) + "\n"); p.stdin.flush()
def recv():
    while True:
        line = p.stdout.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except Exception:  # noqa: BLE001
            continue
send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
      "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                 "clientInfo": {"name": "verify-orb-drag", "version": "1"}}})
recv()
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
send({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
      "params": {"name": "drag",
                 "arguments": {"start_x": x0, "start_y": y0, "end_x": x1, "end_y": y1}}})
while True:
    msg = recv()
    if msg is None:
        print("ERR=EOF"); sys.exit(3)
    if msg.get("id") == 2:
        res = msg.get("result") or {}
        if res.get("isError"):
            print("ERR=" + "".join(c.get("text", "") for c in (res.get("content") or [])))
            sys.exit(4)
        print("OK")
        break
p.terminate()
PYDRAG
}

# 松手后位置可能再被贴边改一次，等它稳定（最多 3 秒，不无限等）
settle() {
  local prev="" cur="" n=0
  while [ "$n" -lt 15 ]; do
    cur=$(probe)
    [ "$cur" = "$prev" ] && [ -n "$cur" ] && break
    prev=$cur
    n=$((n + 1))
    sleep 0.2
  done
  printf '%s' "$cur"
}

expect() { # $1=用例名 $2=期望x $3=期望y $4=实得 "x y ..."
  local name=$1 ex=$2 ey=$3 got=$4
  local gx gy
  read -r gx gy _ <<<"$got"
  [ -n "${gx:-}" ] || fail "$name 读不到位置"
  local dx=$((gx - ex)) dy=$((gy - ey))
  [ "$dx" -lt 0 ] && dx=$((-dx))
  [ "$dy" -lt 0 ] && dy=$((-dy))
  if [ "$dx" -le "$TOL_X" ] && [ "$dy" -le "$TOL_Y" ]; then
    notice "$name PASS 期望≈($ex,$ey) 实得=($gx,$gy) 误差=($dx,$dy)"
    return 0
  fi
  printf '[drag] %s 窗口位置不符合预期\n' "$name" >&2
  printf '[drag]   期望≈(%d,%d)  实得=(%d,%d)  误差=(%d,%d) 容差=(%d,%d)\n' \
    "$ex" "$ey" "$gx" "$gy" "$dx" "$dy" "$TOL_X" "$TOL_Y" >&2
  printf '[drag]   排查：ssh %s journalctl --user -u morphofly-test -n 60 | grep -E "拖动|贴边"\n' "$T" >&2
  return 1
}

# ── 用例 A：拖到左半屏 ──
target_x=$((MON_X + MON_W / 4))
target_y=$((MON_Y + MON_H / 3))
start_x=$((WX + ox)); start_y=$((WY + oy))
notice "A 拖到左半屏：($start_x,$start_y) → ($target_x,$target_y)"
if [ "$DRY" = 1 ]; then
  notice "dry-run：只打印计划，不执行"
  exit 0
fi
out=$(drag "$start_x" "$start_y" "$target_x" "$target_y") || fail "A 拖动调用失败：$out"
got=$(settle)
expect "A" $((MON_X + MARGIN)) $((target_y - oy)) "$got" || exit 1
read -r WX WY MON_W MON_H MON_X MON_Y <<<"$got"

# ── 用例 B：再拖回右半屏 ──
target_x2=$((MON_X + MON_W * 3 / 4))
target_y2=$((MON_Y + MON_H / 2))
start_x=$((WX + ox)); start_y=$((WY + oy))
notice "B 拖回右半屏：($start_x,$start_y) → ($target_x2,$target_y2)"
out=$(drag "$start_x" "$start_y" "$target_x2" "$target_y2") || fail "B 拖动调用失败：$out"
got=$(settle)
expect "B" $((MON_X + MON_W - BALL - MARGIN)) $((target_y2 - oy)) "$got" || exit 1

notice "全部通过：悬浮球可拖动，且松手贴最近的水平边、垂直位置保留"
