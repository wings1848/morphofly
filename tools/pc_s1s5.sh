#!/usr/bin/env bash
# 补 S1 / S5 两态的截图（审计指出六态证据只覆盖 4 态）。
# 用 MORPHOFLY_DEMO_STATES 的定时巡检，精确等到 S1 和 S5 时截图。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
OUTDIR="${1:-/tmp/diemeng-s1s5}"
ssh -o BatchMode=yes "$SSH_T" bash -s -- "$OUTDIR" <<'REMOTE'
set -e
OUTDIR="$1"
export XDG_RUNTIME_DIR=/run/user/1000 XAUTHORITY=/run/user/1000/xauth_bqxIvn DISPLAY=:0
export GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
cd /home/wings/Code_Project/morphofly
pkill -x morphofly-app 2>/dev/null || true
sleep 1
rm -rf "$OUTDIR" && mkdir -p "$OUTDIR"
MORPHOFLY_DEMO_STATES=1 setsid ./target/debug/morphofly-app > "$OUTDIR/app.log" 2>&1 < /dev/null &
sleep 6
WID=$(for id in $(xdotool search --onlyvisible --name '.*' 2>/dev/null); do
        n=$(xdotool getwindowname $id 2>/dev/null)
        [ "$n" = '蝶梦' ] && { echo $id; break; }
      done)
echo "wid=$WID"
# 每 0.4s 对一次日志里的当前 state，命中 S1/S5 就截图
declare -A shot
for _ in $(seq 1 120); do
  cur=$(grep -o 'state="S[0-9]"' "$OUTDIR/app.log" 2>/dev/null | tail -1 | tr -d 'state="')
  if [ -n "$cur" ] && [ -z "${shot[$cur]:-}" ]; then
    shot[$cur]=1
    import -window "$WID" "$OUTDIR/state-${cur}.png" 2>/dev/null && echo "captured $cur"
  fi
  sleep 0.4
done
echo "--- 抓到的状态: ${!shot[@]}"
ls -la "$OUTDIR"/*.png 2>/dev/null | wc -l
pkill -x morphofly-app 2>/dev/null || true
REMOTE
