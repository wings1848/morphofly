#!/usr/bin/env bash
# 把本地仓库同步到 wings-pc（不传 target/、不传密钥）。本机没有 rsync，用 tar over ssh。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
REMOTE="${DIEMENG_REMOTE:-/home/wings/Code_Project/diemeng}"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
tar -C "$LOCAL" -czf - \
  --exclude='./target' --exclude='./.git' --exclude='./.venv' \
  --exclude='./.env' --exclude='./*.key' --exclude='./credentials' \
  --exclude='./node_modules' --exclude='./runs/*/evidence/*.bin' \
  . | ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$SSH_T" \
  "mkdir -p $REMOTE && tar -C $REMOTE -xzf - && echo synced-$REMOTE"
