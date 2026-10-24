#!/usr/bin/env bash
# 六态视觉巡检：起 App（自动走状态），每 3.2s 截一张图。
# 验收 U2「五个状态可区分」的自动化判定手段。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
OUTDIR="${1:-/tmp/diemeng-states}"
WIDFILE=/tmp/diemeng-wid
ssh -o BatchMode=yes "$SSH_T" "
  set -e
  export XDG_RUNTIME_DIR=/run/user/1000 XAUTHORITY=/run/user/1000/xauth_bqxIvn DISPLAY=:0
  export GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
  cd /home/wings/Code_Project/morphofly
  pkill -x morphofly-app 2>/dev/null || true
  sleep 1
  rm -rf $OUTDIR && mkdir -p $OUTDIR
  MORPHOFLY_DEMO_STATES=1 setsid ./target/debug/morphofly-app > $OUTDIR/app.log 2>&1 < /dev/null &
  sleep 7
  WID=\$(for id in \$(xdotool search --onlyvisible --name '.*' 2>/dev/null); do
          n=\$(xdotool getwindowname \$id 2>/dev/null)
          if [ \"\$n\" = '蝶梦' ]; then echo \$id; break; fi
        done)
  echo \"wid=\$WID\" | tee $WIDFILE
  [ -z \"\$WID\" ] && { echo '找不到窗口'; exit 1; }
  # 每 3.2s 截一张，覆盖两个完整状态周期
  for i in \$(seq 1 10); do
    import -window \$WID $OUTDIR/st-\$i.png 2>/dev/null || true
    sleep 3.2
  done
  echo '--- 状态序列'
  grep -o 'state=\"S[0-9]\"' $OUTDIR/app.log | tr '\n' ' '
  echo
  echo \"--- 截图数: \$(ls $OUTDIR/*.png 2>/dev/null | wc -l)\"
"
