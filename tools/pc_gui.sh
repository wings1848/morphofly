#!/usr/bin/env bash
# 在 wings-pc 上跑 GUI 命令，带齐 Xwayland/X11 + 软件渲染环境。
# 用途：Tauri 窗口在 KWin Wayland 下需要 GDK_BACKEND=x11 + 关 dmabuf 渲染器。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
cmd="${1:?需要命令}"
ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$SSH_T" "
  export XDG_RUNTIME_DIR=/run/user/1000
  export XAUTHORITY=/run/user/1000/xauth_bqxIvn
  export DISPLAY=:0
  export GDK_BACKEND=x11
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  export WEBKIT_DISABLE_DMABUF_RENDERER=1
  export LIBGL_ALWAYS_SOFTWARE=1
  cd /home/wings/Code_Project/diemeng
  $cmd
"
