#!/usr/bin/env bash
# 在 wings-pc 上控制蝶梦桌面 App：start | stop | status | shot
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
action="${1:-status}"
case "$action" in
  start)
    bash "$LOCAL/tools/pc_gui.sh" 'pkill -x morphofly-app 2>/dev/null || true; sleep 1; setsid ./target/debug/morphofly-app > /tmp/morphofly-app.log 2>&1 < /dev/null & sleep 6; echo started'
    ;;
  stop)
    bash "$LOCAL/tools/pc_gui.sh" 'pkill -x morphofly-app 2>/dev/null || true; sleep 1; echo stopped'
    ;;
  status)
    bash "$LOCAL/tools/pc_gui.sh" 'echo "--- alive"; pgrep -x morphofly-app || echo "(not running)"; echo "--- log"; tail -12 /tmp/morphofly-app.log 2>/dev/null || true; echo "--- wins"; xdotool search --name "蝶梦" 2>/dev/null | head -3'
    ;;
  shot)
    bash "$LOCAL/tools/pc_gui.sh" 'WID=$(xdotool search --name "蝶梦" 2>/dev/null | head -1); if [ -n "$WID" ]; then import -window "$WID" /tmp/diemeng-orb.png && echo "window shot -> /tmp/diemeng-orb.png"; else echo "no orb window; full screen"; fi' || true
    bash "$LOCAL/tools/pcscreenshot.sh" /tmp/diemeng-full.png >/dev/null 2>&1 || true
    ;;
  *)
    echo "用法: tools/pc_app.sh {start|stop|status|shot}" >&2; exit 2
    ;;
esac
