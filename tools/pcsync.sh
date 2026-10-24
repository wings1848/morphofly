#!/usr/bin/env bash
# 把本地仓库同步到 wings-pc（不传 target/、不传密钥）。本机没有 rsync，用 tar over ssh。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
REMOTE="${MORPHOFLY_REMOTE:-/home/wings/Code_Project/morphofly}"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
# 排除清单（同步只传源码，不传产物）：
#   target/     编译产物，几十 GB
#   dist/       打包好的 AppImage/deb，127MB+，远端自己打
#   .codegraph/ 本机索引，3.7MB，远端建自己的
#   .git/       历史，远端不需要（用 tar 而非 git push）
#   node_modules/ .venv/ 依赖目录
#   凭据类：credentials .env *.key
tar -C "$LOCAL" -czf - \
  --exclude='./target' --exclude='./dist' --exclude='./.git' \
  --exclude='./.codegraph' --exclude='./.venv' --exclude='./node_modules' \
  --exclude='./.env' --exclude='./*.key' --exclude='./credentials' \
  --exclude='./runs/*/evidence/*.bin' --exclude='*.log' --exclude='*.pcm' \
  . | ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new "$SSH_T" \
  "mkdir -p $REMOTE && tar -C $REMOTE -xzf - && echo synced-$REMOTE"
