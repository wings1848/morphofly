#!/usr/bin/env bash
# tools/gpd_deploy_ci.sh — 把 GitHub Actions 编好的 Linux 应用装到 wings-gpd
#
# 为什么这么做：用户明确「编译走 GitHub Actions」。gpd 是掌机（Pentium N6000），
# 本地编一次要 5–13 分钟，编译期间 app 和 Chrome 都会卡；所以改成
#   push → CI 编 → 本脚本下载产物 → 装到 gpd → 重启服务。
#
# 用法：
#   tools/gpd_deploy_ci.sh                  # 取 main 上最近一次成功的 CI
#   tools/gpd_deploy_ci.sh --sha <commit>   # 取指定提交的 CI 产物
#   tools/gpd_deploy_ci.sh --run <run_id>   # 取指定 CI run
#   tools/gpd_deploy_ci.sh --no-restart     # 只装，不重启服务
#   tools/gpd_deploy_ci.sh --dry-run        # 只打印要做什么
#
# 退出码：0 成功 / 1 找不到产物或部署失败 / 2 参数错误
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

ART=morphofly-app-linux-x64
REMOTE_DIST='$HOME/Code_Project/morphofly/target/release'
RUN_ID=""
SHA=""
DO_RESTART=1
DRY=0

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; }
while [ $# -gt 0 ]; do
  case "$1" in
    --run) RUN_ID=${2:-}; shift 2 ;;
    --sha) SHA=${2:-}; shift 2 ;;
    --no-restart) DO_RESTART=0; shift ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "未知参数：$1" >&2; usage >&2; exit 2 ;;
  esac
done

command -v gh >/dev/null || { echo "需要 gh CLI（GitHub 认证）" >&2; exit 1; }

# ── 1) 找到要部署的 CI run ──
if [ -z "$RUN_ID" ]; then
  if [ -n "$SHA" ]; then
    RUN_ID=$(gh run list --workflow=ci.yml --limit 30 \
      --json databaseId,headSha,conclusion \
      -q ".[] | select(.headSha==\"$SHA\" and .conclusion==\"success\") | .databaseId" |
      head -1)
  else
    RUN_ID=$(gh run list --workflow=ci.yml --limit 20 \
      --json databaseId,conclusion,headBranch \
      -q '[.[] | select(.conclusion=="success" and .headBranch=="main")][0].databaseId')
  fi
fi
[ -n "${RUN_ID:-}" ] && [ "$RUN_ID" != "null" ] || {
  echo "❌ 找不到成功的 CI 运行（需要 ci.yml 的 build-app-linux 跑完）" >&2
  exit 1
}
echo "CI run: $RUN_ID"

if [ "$DRY" = 1 ]; then
  echo "dry-run：会下载 artifact=$ART，解到 gpd:$REMOTE_DIST，然后$([ "$DO_RESTART" = 1 ] && echo '重启 morphofly-test' || echo '不重启')"
  exit 0
fi

# ── 2) 下载产物 ──
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
gh run download "$RUN_ID" -n "$ART" -D "$TMP"
TARBALL="$TMP/$ART.tar.gz"
[ -f "$TARBALL" ] || TARBALL=$(find "$TMP" -name '*.tar.gz' -maxdepth 2 | head -1)
[ -f "$TARBALL" ] || { echo "❌ artifact 里没找到 tar.gz" >&2; exit 1; }
tar -tzf "$TARBALL" | sed -n '1,6p'

# ── 3) 送到 gpd 并解压 ──
SSH_T=$(bash tools/gpd_ssh.sh) || { echo "❌ wings-gpd 不在线" >&2; exit 1; }
echo "目标：$SSH_T"
if [ "$DO_RESTART" = 1 ]; then
  ssh -o BatchMode=yes "$SSH_T" 'systemctl --user stop morphofly-test' || true
fi
scp -q -o BatchMode=yes "$TARBALL" "$SSH_T:/tmp/morphofly-ci.tar.gz"
ssh -o BatchMode=yes "$SSH_T" "set -e
  D=$REMOTE_DIST
  mkdir -p \"\$D\"
  tar -xzf /tmp/morphofly-ci.tar.gz -C \"\$D\"
  echo '--- 装好了 ---'
  ls -l \"\$D/morphofly-app\"
  cat \"\$D/build.txt\""

# ── 4) 重启并确认活着 ──
if [ "$DO_RESTART" = 1 ]; then
  ssh -o BatchMode=yes "$SSH_T" 'systemctl --user start morphofly-test'
  # 最多等 20s 看服务是否 active（不 sleep 轮询太久，也不无限等）
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if ssh -o BatchMode=yes "$SSH_T" 'systemctl --user is-active morphofly-test' |
      grep -qx active; then
      echo "✅ morphofly-test 已起来"
      ssh -o BatchMode=yes "$SSH_T" \
        'journalctl --user -u morphofly-test --since "-40s" --no-pager -o cat |
         grep -E "小球已贴边|前端日志|拖动|错误|ERROR" | tail -6' || true
      exit 0
    fi
  done
  echo "❌ 服务没起来，看日志：ssh $SSH_T journalctl --user -u morphofly-test -n 50" >&2
  exit 1
fi
echo "✅ 已安装（没重启服务）"
