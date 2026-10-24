#!/usr/bin/env bash
# 内存/CPU 基线测量（release 二进制，真机）
set -uo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
ssh -o BatchMode=yes "$SSH_T" bash -s <<'REMOTE'
export XDG_RUNTIME_DIR=/run/user/1000 XAUTHORITY=/run/user/1000/xauth_bqxIvn DISPLAY=:0
# 注意：这台机器 WebKit 在 GPU 下无法创建 GBM buffer（Nvidia+Wayland），
# 必须软件渲染才能起窗口。所以内存基线只能在软件渲染下测，这是环境限制不是产品缺陷。
export GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1
cd /home/wings/Code_Project/morphofly || exit 1
pkill -9 -f morphofly-app 2>/dev/null; sleep 2
set -a; . "$HOME/.diemeng/credentials" 2>/dev/null; set +a
MORPHOFLY_KWS=0 RUST_LOG=warn setsid ./target/release/morphofly-app > /tmp/mem.log 2>&1 < /dev/null &
PID=$!
for i in $(seq 1 30); do sleep 0.5; kill -0 $PID 2>/dev/null || break; done
sleep 12
if ! kill -0 $PID 2>/dev/null; then echo "启动失败"; tail -3 /tmp/mem.log; exit 1; fi
echo "PID=$PID"
echo "--- 空闲 12s 后 ---"
grep -E "VmRSS|VmPeak" /proc/$PID/status
# CPU：采 5 秒
CPU=$(top -b -n 2 -d 3 -p $PID 2>/dev/null | tail -1 | awk '{print $9}')
echo "CPU%: ${CPU:-N/A}"
# WebKit 子进程也统计进去
echo "--- 全部相关进程 RSS 合计 ---"
TOTAL=0
for p in $(pgrep -f "morphofly-app|WebKitWebProcess|WebKitNetworkProcess" 2>/dev/null); do
  R=$(grep VmRSS /proc/$p/status 2>/dev/null | awk '{print $2}')
  [ -n "$R" ] && TOTAL=$((TOTAL + R))
done
echo "合计: $((TOTAL / 1024)) MB"
pkill -9 -f morphofly-app 2>/dev/null
REMOTE
