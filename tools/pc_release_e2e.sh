#!/usr/bin/env bash
# release 二进制端到端验收（带 kws：唤醒 + 打断 + 派活）
set -uo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
ssh -o BatchMode=yes "$SSH_T" bash -s <<'REMOTE'
export XDG_RUNTIME_DIR=/run/user/1000 XAUTHORITY=/run/user/1000/xauth_bqxIvn DISPLAY=:0
export GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
cd /home/wings/Code_Project/diemeng || exit 1
pkill -9 -f diemeng-app 2>/dev/null; sleep 2
set -a; . "$HOME/.diemeng/credentials" 2>/dev/null; set +a
echo "=== release 二进制信息 ==="
ls -la target/release/diemeng-app | awk '{print "  大小:", $5}'
echo "=== 启动（带 kws）==="
DIEMENG_KWS=1 DIEMENG_AUTOCLICK=1 RUST_LOG=info setsid ./target/release/diemeng-app > /tmp/rel.log 2>&1 < /dev/null &
sleep 20
echo "=== 关键日志 ==="
sed -e 's/\x1b\[[0-9;]*m//g' /tmp/rel.log | grep -E "蝶梦启动|唤醒词|KWS|会话已建立|WS 已连接|自动点击|本地判定|失败|ERROR" | head -12
echo "=== 状态序列 ==="
sed -e 's/\x1b\[[0-9;]*m//g' /tmp/rel.log | grep -oE 'state="S[0-9]"' | tr '\n' ' '
echo
pkill -9 -f diemeng-app 2>/dev/null
REMOTE
