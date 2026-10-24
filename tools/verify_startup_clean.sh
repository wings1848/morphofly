#!/usr/bin/env bash
# tools/verify_startup_clean.sh — 「启动瞬间干净」回归验收
#
# ## 这个验收在守什么
#
# 用户反馈（2026-10-25）：morphofly 在 Linux KDE 桌面启动时，屏幕会弹出一个黑框 UI，
# 字跟悬浮球叠一会儿才恢复正常。
#
# 拆开是三件事，本脚本逐条判：
#   T1 启动时不该弹字幕气泡（老版本启动会显示「待机」黑色气泡 4.2 秒）
#   T2 真机连拍：启动中球左侧的「气泡区域」不应出现黑框/文字
#      （直接对真机截图做差分；没有气泡时这块区域只有静态壁纸/桌面）
#   T3 位置 / 尺寸 / 置顶不回归（复用 tools/verify_float_ui.sh 的 pos/top/size）
#   T4 前端单测全绿（字幕清洗 + 旧测试不回归）
#
# ## 怎么判
#
# T1 从 systemd journal 读「启动状态 caption=无」（重启服务可控，日志时间戳可靠）。
# T2 用 KWin ScreenShot2 D-Bus 连拍（见 tools/gpd_startup_frames.py），
#    比 spectacle/portal 快两个数量级：一次 40–90ms，8fps 能拍到 4.2 秒的气泡。
# T3/T4 复用现成的真机几何检查和本地 node 单测。
#
# ## 用法
#
#   tools/verify_startup_clean.sh                 # 默认 run 目录见下
#   tools/verify_startup_clean.sh --no-frames     # 只跑日志判据（调试用，快）
#   tools/verify_startup_clean.sh --no-restart    # 不重启服务（服务必须已经在跑；只做静态判据）
#
# 环境变量：GPD / GPD_SERVICE / RUN_DIR / EVIDENCE_DIR
#
# 退出码：0 全通过 / 1 有失败 / 2 跳过（设备不在线或服务没跑）
set -uo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd -- "$SCRIPT_DIR/.." && pwd)

GPD=${GPD:-}
GPD_SERVICE=${GPD_SERVICE:-morphofly-test}
RUN_DIR=${RUN_DIR:-$REPO_ROOT/runs/20261025-0520-startup-clean}
EVIDENCE_DIR=${EVIDENCE_DIR:-$RUN_DIR/evidence}
FRAMES_LOCAL="$EVIDENCE_DIR/startup-frames"
REC_DURATION=${REC_DURATION:-13}
REC_FPS=${REC_FPS:-8}
LATE_T=${LATE_T:-12.0}

NO_FRAMES=0
NO_RESTART=0
while [ $# -gt 0 ]; do
  case "$1" in
    --no-frames) NO_FRAMES=1; shift ;;
    --no-restart) NO_RESTART=1; shift ;;
    -h|--help) sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

mkdir -p "$EVIDENCE_DIR"

PASS=0
FAIL=0
SKIP=0
declare -a RESULTS=()

ok()   { PASS=$((PASS + 1)); RESULTS+=("PASS  $1"); printf 'PASS  %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); RESULTS+=("FAIL  $1"); printf 'FAIL  %s\n' "$1"; }
skip() { SKIP=$((SKIP + 1)); RESULTS+=("SKIP  $1"); printf 'SKIP  %s\n' "$1"; }

# ── 1) 找设备 ──────────────────────────────────────────────────────────────
if [ -z "${GPD:-}" ]; then
  GPD=$(bash "$REPO_ROOT/tools/gpd_ssh.sh" 2>/dev/null) || {
    echo "SKIP  wings-gpd 不在线（先开机/连网）"
    exit 2
  }
fi

SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=12)
run() { timeout 120 ssh "${SSH_OPTS[@]}" "$GPD" "$@"; }

# gpd 上的桌面环境变量（ssh 不继承桌面会话）：XAUTHORITY 每次登录会换，现找。
DISCOVER=$(cat <<'EOS'
set -u
X=""
for pid in $(pgrep -f 'plasmashell|kwin_wayland|kwin_x11' 2>/dev/null | head -3); do
  v=$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null | sed -n 's/^XAUTHORITY=//p' | head -1)
  if [ -n "${v:-}" ] && [ -r "$v" ]; then X=$v; break; fi
done
if [ -z "$X" ]; then X=$(ls -t /run/user/1000/xauth_* 2>/dev/null | head -1); fi
echo "$X"
EOS
)
GPD_XAUTHORITY=$(run "bash -s" <<<"$DISCOVER" 2>/dev/null | tail -1)
[ -n "${GPD_XAUTHORITY:-}" ] || { echo "SKIP  gpd 上找不到 XAUTHORITY（桌面没起？）"; exit 2; }

export_env() {
  echo "export DISPLAY=:0 XAUTHORITY='$GPD_XAUTHORITY' XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus"
}

# ── 2) 服务在跑吗 ──────────────────────────────────────────────────────────
STATE=$(run "systemctl --user is-active $GPD_SERVICE" 2>/dev/null || true)
if [ "${STATE:-}" != "active" ]; then
  echo "SKIP  $GPD_SERVICE 没在跑（先 tools/gpd_deploy_ci.sh）；当前=$STATE"
  exit 2
fi

# ── 3) 上传帧采集器 ────────────────────────────────────────────────────────
FRAMES_PY=/tmp/morphofly-gpd-startup-frames.py
if [ "$NO_FRAMES" = 0 ]; then
  timeout 60 scp -q "${SSH_OPTS[@]}" "$REPO_ROOT/tools/gpd_startup_frames.py" "$GPD:$FRAMES_PY" || {
    echo "SKIP  上传帧采集器失败（scp 不通）"
    exit 2
  }
fi

# ── 4) 录屏 + 重启服务 ─────────────────────────────────────────────────────
REC_DIR=/tmp/morphofly-startup-frames
START_EPOCH=""
if [ "$NO_RESTART" = 0 ]; then
  START_EPOCH=$(date +%s)
  if [ "$NO_FRAMES" = 0 ]; then
    run "$(export_env); rm -rf $REC_DIR; nohup python3 $FRAMES_PY record $REC_DIR $REC_DURATION $REC_FPS >/tmp/mf-frames.log 2>&1 & echo started" >/dev/null 2>&1 || true
    sleep 1
  fi
  echo "[verify_startup_clean] 重启 $GPD_SERVICE（t0=$START_EPOCH）"
  # epoch 用「restart 返回之后」的时间：保证分析只用新实例的帧
  # （重启前旧实例可能正在显示正常的运行期字幕，不能算启动不干净的账）。
  RESTART_EPOCH=$(run "systemctl --user restart $GPD_SERVICE && date +%s" 2>/dev/null | tail -1) \
    || { echo "FAIL  重启服务失败"; exit 1; }
  [[ "${RESTART_EPOCH:-}" =~ ^[0-9]+$ ]] || RESTART_EPOCH=0
  # 等录屏结束（最多 REC_DURATION+8s）
  if [ "$NO_FRAMES" = 0 ]; then
    DEADLINE=$(( $(date +%s) + REC_DURATION + 10 ))
    while [ "$(date +%s)" -lt "$DEADLINE" ]; do
      if run "grep -q 'frames ' /tmp/mf-frames.log 2>/dev/null" >/dev/null 2>&1; then break; fi
      sleep 1
    done
    run "tail -2 /tmp/mf-frames.log" 2>/dev/null || true
  else
    sleep 8
  fi
else
  echo "[verify_startup_clean] --no-restart：只做静态判据（T2/T3/T5 用当前进程日志）"
fi

# ── 5) 拉 journal（从重启时刻起）──────────────────────────────────────────
JOURNAL="$EVIDENCE_DIR/startup-journal.log"
if [ -n "${START_EPOCH:-}" ]; then
  run "journalctl --user -u $GPD_SERVICE --since '@$START_EPOCH' --no-pager -o cat" >"$JOURNAL" 2>/dev/null
else
  run "journalctl --user -u $GPD_SERVICE --no-pager -o cat -n 500" >"$JOURNAL" 2>/dev/null
fi
[ -s "$JOURNAL" ] || { echo "FAIL  拿不到 journal"; exit 1; }
JOURNAL_RAW="$EVIDENCE_DIR/startup-journal-full.log"
mv "$JOURNAL" "$JOURNAL_RAW"
# 同一个 unit 重启前后的日志在 --since 里会混在一起；只从**本次重启**的
# 「Started ...」标记之后算日志，并且只截启动后 60 行（约 3–5 秒）：
#   1. 重启前旧实例的字幕是正常运行行为，不能算账；
#   2. 启动 20 秒后又可能被真实语音会话触发运行期 S2 字幕（环境声误唤醒），
#      不截窗口的话 T1 会假失败（2026-10-25 独立验收时实测到过一次）。
# 取第一个（而不是最后一个）Started：验收期间若别的进程又重启服务，
# 也不能把别人的启动当成本次验收对象。
awk 'BEGIN{keep=0;n=0}
     /Started MorphoFly voice agent/{ if(!keep){keep=1}; next }
     keep && n<60 { print; n++ }' "$JOURNAL_RAW" >"$JOURNAL"

RESTART_COUNT=$(grep -c 'Started MorphoFly voice agent' "$JOURNAL_RAW" 2>/dev/null || true)
RESTART_COUNT=${RESTART_COUNT:-1}
echo "[verify_startup_clean] journal（本次实例启动窗口）→ $JOURNAL（$(wc -l <"$JOURNAL") 行，原始 $(wc -l <"$JOURNAL_RAW") 行）"
if [ "$RESTART_COUNT" -gt 1 ] 2>/dev/null; then
  echo "⚠ 验收期间服务被重启了 $RESTART_COUNT 次（可能有别的 agent/部署在操作 gpd）。"
  echo "  判据只取本次（第一次）启动窗口；连拍帧可能混入后续启动，重跑可消除噪声。"
fi

# 第一次「运行期状态变化」的时间（唤醒/会话/推状态）。
# 这之后出现的字幕气泡是正常产品行为（真有人/音频在说话），不是启动 bug；
# T2 只分析这之前的帧，避免把别人的语音测试当成启动气泡。
RUNTIME_EPOCH=0
RUNTIME_TS=$(grep -m1 -E '检测到唤醒词|推状态给前端|会话已建立' "$JOURNAL_RAW" 2>/dev/null |
  sed -n 's/^\([0-9T:.-]*Z\).*/\1/p')
if [ -n "${RUNTIME_TS:-}" ]; then
  RUNTIME_EPOCH=$(date -u -d "$RUNTIME_TS" +%s 2>/dev/null || echo 0)
fi
if [ "${RUNTIME_EPOCH:-0}" -gt 0 ] 2>/dev/null; then
  echo "ℹ 本次启动后出现运行期语音状态（$RUNTIME_TS）；T2 只分析此前的帧。"
fi

# ── T1 启动时不该显示任何初始字幕 ─────────────────────────────────────────
# 用户看到的就是这个：启动时 orb_state 把状态名「待机」当字幕返回，前端直接
# showCaption + orb_tip 把窗口撑到 360 宽 → 球旁边一个黑色气泡停 4.2 秒。
# 两个证据缺一不可：
#   a) 前端启动日志 caption=无（不受「窗口来不来得及撑开」的竞态影响）
#   b) 重启后没有「气泡窗口已调整 show=true」
START_LOG=$(grep -o '启动状态 state=[^ ]* caption=[^ ]*' "$JOURNAL" | tail -1)
SHOW_LINE=$(grep -n '气泡窗口已调整 show=true' "$JOURNAL" | head -1 | cut -d: -f1)
STATE_LINE=$(grep -n '推状态给前端' "$JOURNAL" | head -1 | cut -d: -f1)
if [ -z "${START_LOG:-}" ]; then
  bad "T1 没有「启动状态 caption=无」日志（旧二进制没有这条判据；不能证明启动干净）"
elif ! echo "$START_LOG" | grep -q 'caption=无'; then
  bad "T1 启动时显示了初始字幕（${START_LOG}）"
elif [ -n "${SHOW_LINE:-}" ] && { [ -z "${STATE_LINE:-}" ] || [ "$SHOW_LINE" -lt "$STATE_LINE" ]; }; then
  bad "T1 启动时弹了字幕气泡（show=true 出现在第一次状态推送之前，line=$SHOW_LINE）"
else
  if [ -n "${SHOW_LINE:-}" ]; then
    note "T1 启动窗口内出现过运行期字幕（show=true 在状态推送之后，line=$SHOW_LINE），不算启动 bug"
  fi
  ok "T1 启动时没有显示初始字幕（${START_LOG}）"
fi

# ── T2 真机连拍：球左侧「气泡区」不应出现黑框/文字 ────────────────────────
# 连拍帧只保留 restart 之后的（重启前旧实例的正常字幕不算账，见 README/踩坑）。
if [ "$NO_FRAMES" = 0 ]; then
  POS_LINE=$(grep -o '小球已贴边 x=[0-9-]* y=[0-9-]*' "$JOURNAL" | tail -1)
  X=$(echo "$POS_LINE" | sed 's/.*x=\([-0-9]*\).*/\1/')
  Y=$(echo "$POS_LINE" | sed 's/.*y=\([-0-9]*\).*/\1/')
  DPR=$(grep -o 'dpr=[0-9.]*' "$JOURNAL" | head -1 | cut -d= -f2)
  DPR=${DPR:-1.5}
  RES=$(run "$(export_env); python3 $FRAMES_PY analyze $REC_DIR $LATE_T ${X:-0} ${Y:-0} $DPR ${RESTART_EPOCH:-0} ${RUNTIME_EPOCH:-0}" 2>&1)
  echo "$RES" >"$EVIDENCE_DIR/startup-frames-analyze.json"
  echo "$RES"
  # 注意：没设置不透明度/没有「首帧就绪」日志 —— 这条判据只看像素：
  # 用户视觉上到底还有没有那个黑色气泡。
  if echo "$RES" | grep -q '"verdict": "PASS"'; then
    ok "T2 真机连拍：球左侧气泡区无黑框/文字（$(echo "$RES" | grep -o '"max_mean_diff": [0-9.]*')）"
  else
    bad "T2 真机连拍：球左侧仍有黑框/文字（$(echo "$RES" | grep -o '"max_mean_diff": [0-9.]*') / $(echo "$RES" | grep -o '"max_at_t": [0-9.]*')）"
  fi
  # 把帧拉回来留证据（体积很小：只存了右下角 400x300）
  rm -rf "$FRAMES_LOCAL"; mkdir -p "$FRAMES_LOCAL"
  run "cd /tmp && tar czf /tmp/mf-frames.tgz morphofly-startup-frames" >/dev/null 2>&1 || true
  timeout 180 scp -q "${SSH_OPTS[@]}" "$GPD:/tmp/mf-frames.tgz" "$FRAMES_LOCAL/frames.tgz" 2>/dev/null \
    && tar xzf "$FRAMES_LOCAL/frames.tgz" -C "$FRAMES_LOCAL" 2>/dev/null \
    && echo "[verify_startup_clean] 启动帧 → $FRAMES_LOCAL" \
    || skip "帧回传失败（T2 已在设备端判定，不影响结论）"
else
  skip "T2 本次 --no-frames 没录"
fi

# ── T3 位置 / 尺寸 / 置顶不回归（复用 tools/verify_float_ui.sh）────────────
T3_FAIL=""
T3_SKIP=""
T3_DETAIL=""
for check in pos top size; do
  OUT=$(GPD="$GPD" GPD_XAUTHORITY="$GPD_XAUTHORITY" RUN_DIR="$RUN_DIR" \
    EVIDENCE_DIR="$EVIDENCE_DIR" timeout 90 bash "$REPO_ROOT/tools/verify_float_ui.sh" "$check" 2>&1)
  RC=$?
  echo "$OUT" >"$EVIDENCE_DIR/startup-float-$check.log"
  DETAIL=$(echo "$OUT" | grep -m1 "${check}: \(实测\|PASS\|FAIL\)" | sed 's/^[[:space:]]*//')
  [ -n "${DETAIL:-}" ] || DETAIL=$(echo "$OUT" | tail -1)
  T3_DETAIL="${T3_DETAIL}${DETAIL}; "
  if [ "$RC" -eq 1 ]; then T3_FAIL="${T3_FAIL}${check}(FAIL) "; fi
  if [ "$RC" -eq 2 ]; then T3_SKIP="${T3_SKIP}${check}(SKIP) "; fi
done
if [ -n "$T3_FAIL" ]; then
  bad "T3 悬浮窗几何/置顶回归：${T3_FAIL}（${T3_DETAIL}）"
elif [ -n "$T3_SKIP" ]; then
  skip "T3 部分检查跳过：${T3_SKIP}"
else
  ok "T3 位置/尺寸/置顶均未回归（${T3_DETAIL}）"
fi

# ── T4 前端单测（修正字幕清洗回归）────────────────────────────────────────
if command -v node >/dev/null 2>&1; then
  NODE_OUT=$(cd "$REPO_ROOT/app/frontend" && node --test 'test/*.test.mjs' 2>&1)
  NODE_RC=$?
  echo "$NODE_OUT" >"$EVIDENCE_DIR/startup-frontend-tests.log"
  PASSED=$(echo "$NODE_OUT" | sed -n 's/^ℹ pass \([0-9]*\)$/\1/p' | head -1)
  if [ "$NODE_RC" -eq 0 ]; then
    ok "T4 前端单测全绿（${PASSED:-?} passed）"
  else
    bad "T4 前端单测失败（见 evidence/startup-frontend-tests.log）"
  fi
else
  skip "T4 本机没有 node，跳过前端单测"
fi

# ── 汇总 ──────────────────────────────────────────────────────────────────
echo
echo "── 启动干净度验收 ──"
printf '%s\n' "${RESULTS[@]}"
echo "总计：通过 $PASS / 失败 $FAIL / 跳过 $SKIP"
{
  echo "# 启动干净度验收（$(date '+%Y-%m-%d %H:%M:%S')）"
  echo
  echo "设备：$GPD  服务：$GPD_SERVICE"
  echo "判据：T1 启动无字幕气泡 / T2 启动连拍无黑框 / T3 位置尺寸置顶不回归 / T4 前端单测全绿"
  echo "启动窗口：本次启动后 60 行（约 3–5 秒）；验收期间重启次数：${RESTART_COUNT:-1}"
  echo
  printf '%s\n' "${RESULTS[@]}"
  echo
  echo "总计：通过 $PASS / 失败 $FAIL / 跳过 $SKIP"
} >"$EVIDENCE_DIR/startup-clean-summary.md"

if [ "$FAIL" -gt 0 ]; then exit 1; fi
exit 0
