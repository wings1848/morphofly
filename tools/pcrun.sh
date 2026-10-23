#!/usr/bin/env bash
# 在 wings-pc 上执行命令（开发机：有真实麦克风/扬声器/显示器）
# 用法: tools/pcrun.sh '<shell 命令>'
set -euo pipefail
target="$(tools/pc_ssh.sh)"
REMOTE_DIR="${DIEMENG_REMOTE:-/home/wings/Code_Project/diemeng}"
cmd="${1:?需要命令}"
ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 "$target" \
  "export PATH=\$HOME/.cargo/bin:\$HOME/.bun/bin:/usr/bin:/bin; cd $REMOTE_DIR && $cmd"
