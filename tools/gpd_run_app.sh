#!/usr/bin/env bash
# tools/gpd_run_app.sh — wings-gpd 上的启动包装（systemd 服务的 ExecStart）
#
# 为什么需要它：X11 的 XAUTHORITY 文件每次登录都会换名字
# （/run/user/1000/xauth_XXXXXX）。服务单元里写死过一次，gpd 重新登录后
# 启动直接崩：`Authorization required, but no authorization protocol specified`
# → `Failed to initialize GTK` → status=101。
#
# 这个脚本启动时现找当前会话的 XAUTHORITY，找不到再退回「最新的 xauth_*」。
# 注意 app 必须走 XWayland（GDK_BACKEND=x11），否则置顶/贴边/拖动都没有全局坐标。
set -uo pipefail

APP="${MORPHOFLY_APP:-/home/wings/Code_Project/morphofly/target/release/morphofly-app}"
export DISPLAY="${DISPLAY:-:0}"
export GDK_BACKEND=x11
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
export RUST_LOG="${RUST_LOG:-info,morphofly_app=debug,dsh=debug,morphofly_core::dsh=debug}"
export MORPHOFLY_BUNDLE_DIR="${MORPHOFLY_BUNDLE_DIR:-/home/wings/Code_Project/morphofly/target/release/resources}"

# ── 找 XAUTHORITY ──
if [ -z "${XAUTHORITY:-}" ] || [ ! -r "${XAUTHORITY:-/nonexistent}" ]; then
  XAUTHORITY=""
  # 1) 从桌面进程的 environ 里抄（plasmashell / kwin / 任何有 DISPLAY 的用户进程）
  for name in plasmashell kwin_wayland karma kded6 xdg-desktop-portal-kde; do
    for pid in $(pgrep -f "$name" 2>/dev/null | head -3); do
      val=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null | sed -n 's/^XAUTHORITY=//p' | head -1)
      if [ -n "$val" ] && [ -r "$val" ]; then
        XAUTHORITY=$val
        break 2
      fi
    done
  done
  # 2) 退路：取最新创建的那个 xauth_*
  if [ -z "$XAUTHORITY" ]; then
    XAUTHORITY=$(ls -t "$XDG_RUNTIME_DIR"/xauth_* 2>/dev/null | head -1)
  fi
fi
if [ -n "${XAUTHORITY:-}" ] && [ -r "$XAUTHORITY" ]; then
  export XAUTHORITY
  echo "[gpd_run_app] DISPLAY=$DISPLAY XAUTHORITY=$XAUTHORITY" >&2
else
  echo "[gpd_run_app] ⚠️ 找不到可读的 XAUTHORITY，窗口大概率起不来" >&2
  unset XAUTHORITY
fi

exec "$APP" "$@"
