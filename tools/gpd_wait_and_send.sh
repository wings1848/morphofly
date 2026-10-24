#!/usr/bin/env bash
# 等 wings-gpd 上线，然后自动把 AppImage 传过去。
#
# 为什么单独一个脚本：wings-gpd 是掌机，经常几小时不联网。
# 不该让用户或 agent 干等 —— 挂后台，上线就传，传完通知。
#
# 用法：
#   tools/gpd_wait_and_send.sh                      # 默认等 12 小时
#   tools/gpd_wait_and_send.sh --hours 24           # 等更久
#   tools/gpd_wait_and_send.sh --file <路径>         # 传别的文件
#
# 退出码：0 = 传成功；2 = 超时；1 = 其他错误
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

FILE="dist/蝶梦-0.1.0-x86_64.AppImage"
HOURS=12
INTERVAL=60
REMOTE_DIR="~/Downloads"

while [ $# -gt 0 ]; do
  case "$1" in
    --file) FILE="$2"; shift 2 ;;
    --hours) HOURS="$2"; shift 2 ;;
    --interval) INTERVAL="$2"; shift 2 ;;
    --remote-dir) REMOTE_DIR="$2"; shift 2 ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 1 ;;
  esac
done

[ -f "$FILE" ] || { echo "文件不存在: $FILE" >&2; exit 1; }
NAME="$(basename "$FILE")"
SIZE_MB=$(( $(stat -c%s "$FILE") / 1048576 ))
SHA="$(sha256sum "$FILE" | cut -d' ' -f1)"

echo "[gpd] 等待 wings-gpd 上线（最多 ${HOURS} 小时，每 ${INTERVAL}s 探测一次）"
echo "[gpd] 要传: $NAME（${SIZE_MB}MB，sha256=${SHA:0:16}…）"

DEADLINE=$(( $(date +%s) + HOURS * 3600 ))
TRIES=0

while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  TRIES=$((TRIES + 1))
  TARGET="$(tools/gpd_ssh.sh 2>/dev/null || true)"
  if [ -n "$TARGET" ]; then
    echo "[gpd] 第 ${TRIES} 次探测：SSH 可达（$TARGET），开始传输"

    # 确认远端能收
    if ! ssh -o BatchMode=yes -o ConnectTimeout=15 "$TARGET" "mkdir -p $REMOTE_DIR" 2>/dev/null; then
      echo "[gpd] 建远端目录失败，10 分钟后再试"
      sleep 600
      continue
    fi

    SCP_ARGS=(-o BatchMode=yes -o ConnectTimeout=20)
    SCP_ARGS+=("$FILE" "$TARGET:$REMOTE_DIR/")
    if scp "${SCP_ARGS[@]}" 2>&1 | sed 's/^/[gpd] scp: /'; then
      # 校验大小和 sha（掌机可能没 sha256sum，退化成比大小）
      REMOTE_SHA=$(ssh -o BatchMode=yes "$TARGET" "sha256sum $REMOTE_DIR/$NAME 2>/dev/null | cut -d' ' -f1" 2>/dev/null)
      if [ -n "$REMOTE_SHA" ] && [ "$REMOTE_SHA" = "$SHA" ]; then
        echo "[gpd] ✅ 传输完成，sha256 一致"
        ssh -o BatchMode=yes "$TARGET" "chmod +x $REMOTE_DIR/$NAME" 2>/dev/null
        ~/.agents/scripts/notify "【蝶梦】AppImage 已传到 wings-gpd
位置：$REMOTE_DIR/$NAME（${SIZE_MB}MB）
sha256 已校验一致。
已经 chmod +x，直接跑：
  cd $REMOTE_DIR && ./$NAME
第一次要建密钥：见 dist/使用说明.md" 2>/dev/null
        exit 0
      elif [ -z "$REMOTE_SHA" ]; then
        # 远端没 sha256sum，比大小
        REMOTE_SIZE=$(ssh -o BatchMode=yes "$TARGET" "stat -c%s $REMOTE_DIR/$NAME 2>/dev/null" 2>/dev/null)
        LOCAL_SIZE=$(stat -c%s "$FILE")
        if [ "$REMOTE_SIZE" = "$LOCAL_SIZE" ]; then
          echo "[gpd] ✅ 传输完成（远端无 sha256sum，大小一致 ${REMOTE_SIZE}B）"
          ssh -o BatchMode=yes "$TARGET" "chmod +x $REMOTE_DIR/$NAME" 2>/dev/null
          ~/.agents/scripts/notify "【蝶梦】AppImage 已传到 wings-gpd
位置：$REMOTE_DIR/$NAME（${SIZE_MB}MB）
大小校验一致（远端没 sha256sum），已 chmod +x" 2>/dev/null
          exit 0
        fi
        echo "[gpd] 大小不一致（本地 $LOCAL_SIZE，远端 $REMOTE_SIZE），10 分钟重试"
      else
        echo "[gpd] sha256 不一致（本地 ${SHA:0:16}，远端 ${REMOTE_SHA:0:16}），10 分钟重试"
      fi
    else
      echo "[gpd] scp 失败，10 分钟重试"
    fi
    sleep 600
  fi
  sleep "$INTERVAL"
done

echo "[gpd] ⏱️ 超时（${HOURS} 小时）wings-gpd 一直没上线"
~/.agents/scripts/notify "【蝶梦】等 wings-gpd 超时了（${HOURS} 小时没上线）。
文件还在：$(realpath "$FILE")
你想现在要的话，可以从 http://100.99.51.5:5050/ 手动下" 2>/dev/null
exit 2
