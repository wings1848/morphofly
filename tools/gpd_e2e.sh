#!/usr/bin/env bash
# 在 wings-gpd 上做端到端真机测试。
#
# 为什么用 wings-gpd 而不是 wings-pc：
#   - gpd 是掌机，有真实桌面、真实麦克风、真实扬声器
#   - 它跑的是「用户实际使用场景」（Intel N6000，无 AVX，低功耗）
#   - 没有 wings-pc 的 NVIDIA + KDE Wayland 那堆环境坑
#
# 用法：
#   tools/gpd_e2e.sh                # 全套（同步+编译+跑）
#   tools/gpd_e2e.sh --check        # 只看环境，不编译
#   tools/gpd_e2e.sh --audio        # 重点测音频/AEC
#   tools/gpd_e2e.sh --wait         # 等到上线再跑（最多 6 小时）
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

MODE=full
for a in "$@"; do
  case "$a" in
    --check) MODE=check ;;
    --audio) MODE=audio ;;
    --wait)  MODE=wait  ;;
  esac
done

REPO_REMOTE="${MORPHOFLY_GPD_REPO:-$HOME/Code_Project/morphofly}"

# ── 等上线 ──
if [ "$MODE" = wait ]; then
  echo "等 wings-gpd 上线（最多 6 小时，每 60s 探一次）..."
  for i in $(seq 1 360); do
    if tools/gpd_ssh.sh >/dev/null 2>&1; then
      echo "✅ 上线了（等了 $((i)) 分钟）"
      MODE=full
      break
    fi
    sleep 60
  done
  if [ "$MODE" = wait ]; then
    echo "❌ 6 小时内没上线"
    ~/.agents/scripts/notify "wings-gpd 6 小时没上线，蝶梦真机测试做不了" 2>/dev/null
    exit 2
  fi
fi

T="$(tools/gpd_ssh.sh 2>/dev/null)" || {
  echo "❌ wings-gpd 不在线（主机名和两个 IP 都不通）"
  echo "   掌机需要开机并连上网络。"
  exit 2
}
echo "目标: $T"
ssh_run() { timeout 900 ssh -o BatchMode=yes -o ConnectTimeout=15 "$T" "$@"; }

# ── 1) 环境检查 ──
echo "=== 环境检查 ==="
ssh_run 'echo "主机: $(hostname)"
echo "架构: $(uname -m)"
echo "CPU: $(grep -m1 "model name" /proc/cpuinfo | cut -d: -f2 | xargs)"
echo "核心: $(nproc)"
echo "内存: $(free -m | awk "/Mem:/{print \$2}")MB"
echo "会话: $(loginctl list-sessions --no-legend 2>/dev/null | wc -l)"
ls /run/user/1000/wayland-* 2>/dev/null | head -1 && echo "wayland: 有" || echo "wayland: 无"
echo "音频服务: $(pactl info 2>/dev/null | grep -m1 "Server Name" | cut -d: -f2 | xargs || echo 无)"
echo "麦克风: $(pactl list short sources 2>/dev/null | grep -v monitor | head -1 | cut -f2 || echo 无)"
echo "已装蝶梦: $(ls -d ~/.local/opt/morphofly 2>/dev/null || echo 未安装)"
echo "磁盘: $(df -h ~ | tail -1 | awk "{print \$4}") 可用"' 2>&1

[ "$MODE" = check ] && exit 0

# ── 2) 同步代码并编译 ──
echo "=== 同步代码 ==="
for f in Cargo.toml Cargo.lock .cargo/config.toml; do
  timeout 60 scp -q -o BatchMode=yes "$f" "$T:$REPO_REMOTE/$f" 2>/dev/null && echo "  ✓ $f"
done
for d in core/src app/src-tauri/src app/frontend core/examples; do
  timeout 300 tar czf - "$d" 2>/dev/null | \
    timeout 300 ssh -o BatchMode=yes "$T" "cd $REPO_REMOTE && tar xzf -" 2>/dev/null && echo "  ✓ $d"
done
timeout 60 scp -q -o BatchMode=yes core/Cargo.toml "$T:$REPO_REMOTE/core/Cargo.toml" 2>/dev/null
timeout 60 scp -q -o BatchMode=yes app/src-tauri/tauri.conf.json "$T:$REPO_REMOTE/app/src-tauri/tauri.conf.json" 2>/dev/null

echo "=== 编译（release，可能要 10-20 分钟）==="
ssh_run "cd $REPO_REMOTE && cargo build --release -p morphofly-app 2>&1 | tail -5"

if [ "$MODE" = audio ]; then
  echo "=== 音频设备详情 ==="
  ssh_run 'pactl list short sources 2>/dev/null; echo "---"; pactl list short sinks 2>/dev/null'
  echo "=== 跑 AEC 单测 ==="
  ssh_run "cd $REPO_REMOTE && cargo test -q -p morphofly-core --lib aec 2>&1 | tail -5"
  exit 0
fi

# ── 3) 跑单元测试 ──
echo "=== 单元测试 ==="
ssh_run "cd $REPO_REMOTE && cargo test -q -p morphofly-core --lib 2>&1 | grep -E 'test result|FAILED' | head -3"

# ── 4) 跑真机 e2e（需要桌面+音频）──
echo "=== 真机 e2e ==="
HAS_DESKTOP=$(ssh_run 'ls /run/user/1000/wayland-* 2>/dev/null | head -1' 2>/dev/null)
if [ -z "$HAS_DESKTOP" ]; then
  echo "⚠️ 没有图形会话，跑不了 GUI 相关的 e2e。"
  echo "   但语音链路（e2e_drive）不依赖 GUI，可以跑："
  ssh_run "cd $REPO_REMOTE && ls ~/.diemeng/credentials 2>/dev/null && \
    cargo run -q --release -p morphofly-core --example e2e_drive -- \
      --file /tmp/diemeng-e2e-audio/wake-01.pcm 2>&1 | tail -3"
else
  echo "有图形会话，可以跑完整 GUI 测试"
fi

echo "=== 完成 ==="
