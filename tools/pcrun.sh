#!/usr/bin/env bash
# 在 wings-pc 上执行命令（开发机：有真实麦克风/扬声器/显示器）
# 用法: tools/pcrun.sh '<shell 命令>'
set -euo pipefail
REMOTE_DIR="${DIEMENG_REMOTE:-/home/wings/Code_Project/diemeng}"
cmd="${1:?需要命令}"
ssh -o BatchMode=yes wings-pc "export PATH=\$HOME/.cargo/bin:\$HOME/.bun/bin:/usr/bin:/bin; export DIEMENG_DEBUG=1; cd $REMOTE_DIR && $cmd"
