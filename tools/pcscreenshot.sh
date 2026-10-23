#!/usr/bin/env bash
# 在 wings-pc 上截全屏（KWin 不支持 grim 的 screencapture 协议，用 spectacle 兜底）
set -euo pipefail
OUT="${1:-/tmp/diemeng-shot.png}"
target="$(tools/pc_ssh.sh)"
ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$target" \
  "XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-1 spectacle -b -n -f -o $OUT >/dev/null 2>&1; ls -la $OUT"
