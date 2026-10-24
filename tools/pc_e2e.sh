#!/usr/bin/env bash
# 端到端验收：启动 App → 自动唤醒 → 建会话 → 连云端 → 进 S2 聆听
#
# 这是验收 F1/F3/F8 的自动化判定手段。
# 注意：截图必须用 spectacle 截全屏再裁剪（import -window 在 KWin 下拿到过期图层）。
set -uo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
WAIT="${1:-22}"
ssh -o BatchMode=yes "$SSH_T" bash -s -- "$WAIT" <<'REMOTE'
WAIT="$1"
export XDG_RUNTIME_DIR=/run/user/1000 XAUTHORITY=/run/user/1000/xauth_bqxIvn DISPLAY=:0
export GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
cd /home/wings/Code_Project/morphofly || exit 1
pkill -9 -f morphofly-app 2>/dev/null
sleep 2
set -a; . "$HOME/.diemeng/credentials" 2>/dev/null; set +a
MORPHOFLY_KWS="${MORPHOFLY_KWS:-0}" MORPHOFLY_AUTOCLICK=1 RUST_LOG=info \
  setsid ./target/debug/morphofly-app > /tmp/e2e-app.log 2>&1 < /dev/null &
sleep "$WAIT"
echo "=== 状态迁移 ==="
sed -e 's/\x1b\[[0-9;]*m//g' /tmp/e2e-app.log | grep -oE 'state="S[0-9]"' | tr '\n' ' '
echo
echo "=== 关键事件 ==="
sed -e 's/\x1b\[[0-9;]*m//g' /tmp/e2e-app.log | grep -E "建立语音会话|会话已建立|WS 已连接|工具调用|主动汇报|失败|ERROR" | head -10
echo "=== 截图 ==="
spectacle -b -n -f -o /tmp/e2e-screen.png >/dev/null 2>&1 && echo "全屏截图 -> /tmp/e2e-screen.png"
pkill -9 -f morphofly-app 2>/dev/null
REMOTE
