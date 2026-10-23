#!/usr/bin/env bash
# 查看/切换 wings-pc 的音频路由。用于确认「声音到底播到哪」。
set -euo pipefail
LOCAL="$(cd "$(dirname "$0")/.." && pwd)"
SSH_T="$(bash "$LOCAL/tools/pc_ssh.sh")"
mode="${1:-list}"
ssh -o BatchMode=yes "$SSH_T" bash -s -- "$mode" <<'REMOTE'
mode="$1"
echo '=== 当前默认 sink ==='
pactl get-default-sink
echo
echo '=== 所有 sinks ==='
pactl list short sinks | while read -r id name rest; do
  case "$name" in
    *aloop*|*diemeng_test*|*null*) echo "$id  $name   ⚠️ 虚拟（不会真出声）";;
    *) echo "$id  $name   ✅ 物理";;
  esac
done
if [ "$mode" = "--physical" ]; then
  target=$(pactl list short sinks | awk '{print $2}' | grep -v -E 'aloop|diemeng_test|null' | head -1)
  echo
  echo "=== 切换默认 sink → $target ==="
  pactl set-default-sink "$target"
  pactl get-default-sink
fi
REMOTE
