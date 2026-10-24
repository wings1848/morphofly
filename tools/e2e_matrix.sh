#!/usr/bin/env bash
# tools/e2e_matrix.sh — 蝶梦 e2e 并行编排（本机无头车道 ∥ gpd / pc 真桌面车道）
#
# 目的：把「能并行的」和「必须独占的」分开跑，减少串行等待。
# 结论依据：docs/v2/parallel-e2e.md（两台设备实测事实）；pc 车道见 docs/v2/pc-lane.md。
#
# 车道：
#   local  本机 wings-tx-us1（无显示器/无音频）：单元测试、core 级语音 e2e（打 StepFun，默认关）
#   gpd    wings-gpd（真桌面 + 真麦）：悬浮球几何 / 截图 / 点击 / 整机语音（重启服务，默认关）
#   pc     wings-pc（真桌面 KDE Wayland + NVIDIA，App 走 XWayland）：悬浮球几何三项，
#          复用 gpd 那套 verify_float_ui.sh（GPD=wings@100.110.222.0）。
#          **默认不启用**：加 --with-pc 才跑，免得默认跑测依赖 pc 在线。
#
# 安全默认：
#   - 不加 --with-cost 不跑任何打 StepFun 的用例（花钱）
#   - 不加 --allow-restart 不跑需要重启 gpd 上 morphofly-test 的用例（默认 SKIP）
#   - pc 车道默认关（--with-pc）；脚本不替任何设备起/停 morphofly-test
#   - gpd 车道用 flock 独占：两个人同时驱动同一块桌面会互相点乱（pc 车道只读几何，不加锁）
#   - 每项都有 timeout；超时记 TIMEOUT，不重试、不 sleep 轮询
#
# pc 的阈值：wings-pc 的 X11 缩放系数是 2（Xft.dpi=264，GTK 取整为 2；窗口 296=148×2），
#   而 verify_float_ui.sh 的阈值是按 gpd 的物理像素（scale=1）定的。所以 pc 任务把
#   POS_TOL / POS_TOL_BOTTOM / SIZE_MAX 乘 PC_SCALE 再传进去
#   （同一验收标准换算量纲，不是放松）。改了 KDE 缩放要同步改 MORPHOFLY_PC_SCALE。
#   详见 docs/v2/pc-lane.md。
#
# 用法：
#   tools/e2e_matrix.sh --preflight           # 只探测设备能力（只读；gpd 与 pc 都探）
#   tools/e2e_matrix.sh --dry-run             # 只打印计划，不执行
#   tools/e2e_matrix.sh                        # 默认：单元测试 ∥ gpd 几何三项（不含 pc）
#   tools/e2e_matrix.sh --with-pc              # 加上 pc 车道（pc 上 morphofly-test 需先手动起）
#   tools/e2e_matrix.sh --with-pc --only ui-pos-pc,ui-size-pc
#   tools/e2e_matrix.sh --only ui-pos,ui-top
#   tools/e2e_matrix.sh --with-cost --allow-restart --out /tmp/matrix
#
# 退出码：0 全 PASS（允许 SKIP）/ 1 有 FAIL 或 TIMEOUT / 2 无 FAIL 但有 SKIP / 3 参数错误
set -uo pipefail

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
GPD=${MORPHOFLY_GPD_SSH:-wings@100.107.9.102}
PC=${MORPHOFLY_PC_SSH:-wings@100.110.222.0}
# wings-pc 的 X11 缩放系数。实测依据（2026-10-25，见 docs/v2/pc-lane.md）：
#   Xft.dpi=264；app 自己的 orb_metrics 报 scale=2（GTK 侧）；窗口物理 296=148×2。
# verify_float_ui.sh 的阈值是 scale=1 的物理像素，pc 上要乘这个系数才等价。
PC_SCALE=${MORPHOFLY_PC_SCALE:-2}
GPD_LOCK=${E2E_GPD_LOCK:-/tmp/morphofly-e2e-gpd.lock}
DRY=0
WITH_COST=0
WITH_PC=0
ALLOW_RESTART=0
ONLY=""
OUT=""
PREFLIGHT_ONLY=0

# 打印脚本头部注释（不写死行号，改动 header 不会再漏新参数）
usage() { sed -n '2,/^set -uo pipefail/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run)         DRY=1; shift ;;
    --preflight)       PREFLIGHT_ONLY=1; shift ;;
    --with-cost)       WITH_COST=1; shift ;;
    --with-pc)         WITH_PC=1; shift ;;
    --allow-restart)   ALLOW_RESTART=1; shift ;;
    --only)            ONLY="${2:?--only 需要参数}"; shift 2 ;;
    --only=*)          ONLY="${1#*=}"; shift ;;
    --out)             OUT="${2:?--out 需要参数}"; shift 2 ;;
    --out=*)           OUT="${1#*=}"; shift ;;
    -h|--help)         usage; exit 0 ;;
    *)                 echo "未知参数: $1（--help 看用法）" >&2; exit 3 ;;
  esac
done

# ── 任务表 ────────────────────────────────────────────────────────────
# 字段：name | lane | timeout(s) | gate(cost/restart/空) | 命令
T_NAME=(); T_LANE=(); T_TO=(); T_GATE=(); T_CMD=()
add_task() {
  local name=$1 lane=$2 to=$3 gate=$4; shift 4
  T_NAME+=("$name"); T_LANE+=("$lane"); T_TO+=("$to"); T_GATE+=("$gate"); T_CMD+=("$*")
}

add_task unit        local  1800 ""      '. tools/dev-env.sh && cargo test -q -p morphofly-core'
add_task voice-core  local   900 "cost"  '. tools/dev-env.sh && python3 tools/e2e/run.py --mode real --case wake-01'
add_task ui-pos      gpd     180 ""      'tools/verify_float_ui.sh pos'
add_task ui-top      gpd     120 ""      'tools/verify_float_ui.sh top'
add_task ui-size     gpd     120 ""      'tools/verify_float_ui.sh size'
add_task ui-shots    gpd     900 ""      'tools/verify_float_ui.sh shots'
add_task ui-click    gpd     240 ""      'tools/verify_float_ui.sh click'
add_task voice-app   gpd     360 "restart" 'tools/gpd_voice_e2e.sh --wake-only'

# pc 车道（默认关，--with-pc 打开）：同一套 verify_float_ui.sh，用 GPD= 覆盖目标机。
# 阈值乘 PC_SCALE：pc 的 X11 缩放是 2，这是同一个验收标准的物理像素换算，不是放水。
# 只跑只读的三项（pos/top/size）；不在 pc 上点球、不截图、不重启服务。
add_task ui-pos-pc   pc      180 ""      "GPD=$PC POS_TOL=$((24*PC_SCALE)) POS_TOL_BOTTOM=$((80*PC_SCALE)) tools/verify_float_ui.sh pos"
add_task ui-top-pc   pc      120 ""      "GPD=$PC tools/verify_float_ui.sh top"
add_task ui-size-pc  pc      120 ""      "GPD=$PC SIZE_MAX=$((176*PC_SCALE)) tools/verify_float_ui.sh size"

# ── preflight（只读探测）──────────────────────────────────────────────
PROBE=$(cat <<'PROBE_EOF'
export XDG_RUNTIME_DIR=/run/user/1000
uid=$(id -u)
n_x=$(ls /tmp/.X11-unix 2>/dev/null | wc -l)
n_w=$(ls /run/user/$uid/wayland-* 2>/dev/null | wc -l)
if [ "$n_w" -gt 0 ] || [ "$n_x" -gt 0 ]; then
  echo "session|OK|wayland=$n_w x11=$n_x"
else
  echo "session|NO|wayland=0 x11=0（只有 greeter，无用户桌面）"
fi
if pactl info >/dev/null 2>&1; then
  s=$(pactl list sources short 2>/dev/null | wc -l)
  echo "pipewire|OK|sources=$s default_source=$(pactl get-default-source 2>/dev/null)"
else
  echo "pipewire|NO|pactl 连不上"
fi
cap=$(arecord -l 2>/dev/null | grep -c '^card')
echo "capture|$( [ "$cap" -gt 0 ] && echo OK || echo NO )|alsa_capture_cards=$cap"
if command -v cargo >/dev/null 2>&1; then echo "cargo|OK|$(cargo --version 2>&1 | head -1)"; else echo "cargo|NO|未安装"; fi
if command -v pkg-config >/dev/null 2>&1; then
  ok=1
  for p in gtk+-3.0 webkit2gtk-4.1; do pkg-config --exists "$p" || ok=0; done
  echo "gtk-deps|$( [ "$ok" = 1 ] && echo OK || echo NO )|gtk3+webkit2gtk-4.1"
else
  echo "gtk-deps|NO|无 pkg-config"
fi
repo=/home/wings/Code_Project/morphofly
[ -x "$repo/target/release/morphofly-app" ] && echo "app-bin|OK|target/release/morphofly-app" || echo "app-bin|NO|没有 release 二进制"
[ -d "$repo/target/release/resources" ] && echo "resources|OK|$(ls "$repo/target/release/resources" 2>/dev/null | tr '\n' ',' )" || echo "resources|NO|缺 KWS/VAD 资源"
grep -qx STEPFUN_KEY <(sed 's/=.*//' "$HOME/.diemeng/credentials" 2>/dev/null) && echo "stepfun-key|OK|~/.diemeng/credentials" || echo "stepfun-key|NO|缺 key"
command -v dsh >/dev/null 2>&1 && echo "dsh|OK|$(dsh --version 2>&1 | head -1)" || echo "dsh|NO|未安装"
systemctl --user is-active morphofly-test >/dev/null 2>&1 && echo "service|OK|morphofly-test active" || echo "service|NO|没有 morphofly-test"
PROBE_EOF
)

preflight_host() { # $1=标签 $2=ssh 目标
  local label=$1 target=$2 line
  if ! timeout 8 ssh -o BatchMode=yes -o ConnectTimeout=6 "$target" true 2>/dev/null; then
    printf '%-8s %-11s %-3s %s\n' "$label" ssh NO "连不上 $target"
    return
  fi
  printf '%-8s %-11s %-3s %s\n' "$label" ssh OK "$target"
  while IFS= read -r line; do
    printf '%-8s %-11s %-3s %s\n' "$label" "${line%%|*}" "$(echo "$line" | cut -d'|' -f2)" "$(echo "$line" | cut -d'|' -f3)"
  done < <(timeout 25 ssh -o BatchMode=yes -o ConnectTimeout=6 "$target" "bash -c $(printf '%q' "$PROBE")" 2>/dev/null)
}

do_preflight() {
  echo "=== preflight（只读，不改任何东西）==="
  printf '%-8s %-11s %-3s %s\n' HOST CAP OK? DETAIL
  preflight_host gpd "$GPD"
  preflight_host pc "$PC"
  echo
}

if [ "$PREFLIGHT_ONLY" = 1 ]; then do_preflight; exit 0; fi

# ── 选择任务 ──────────────────────────────────────────────────────────
# pc 车道默认不启用（要 --with-pc）。例外：--only 里显式点名了 pc 任务，
# 就视为用户明确要它，临时启用并打一行说明（免得以为默认跑了 pc）。
PC_IMPLICIT=0
SEL=()
for i in "${!T_NAME[@]}"; do
  if [ -n "$ONLY" ]; then
    case ",$ONLY," in
      *",${T_NAME[$i]},"*)
        [ "${T_LANE[$i]}" = pc ] && [ "$WITH_PC" != 1 ] && PC_IMPLICIT=1
        SEL+=("$i") ;;
    esac
  else
    [ "${T_LANE[$i]}" = pc ] && [ "$WITH_PC" != 1 ] && continue
    SEL+=("$i")
  fi
done
if [ ${#SEL[@]} -eq 0 ]; then echo "没有匹配的任务（--only $ONLY）" >&2; exit 3; fi
if [ "$PC_IMPLICIT" = 1 ]; then
  echo "pc 车道：--only 显式点名了 pc 任务，本次临时启用（平时要加 --with-pc）。"
elif [ "$WITH_PC" != 1 ] && [ -z "$ONLY" ]; then
  echo "pc 车道未启用（ui-pos-pc / ui-top-pc / ui-size-pc）；要跑加 --with-pc。"
fi

if [ "$DRY" = 1 ]; then
  echo "=== dry-run：只打印，不执行 ==="
  echo "REPO_ROOT=$REPO_ROOT"
  echo "GPD=$GPD  PC=$PC  PC_SCALE=$PC_SCALE  lock=$GPD_LOCK"
  echo "with-cost=$WITH_COST  with-pc=$WITH_PC  allow-restart=$ALLOW_RESTART"
  printf '%-8s %-10s %-6s %-7s %s\n' LANE TASK TIMEOUT GATE COMMAND
  for i in "${SEL[@]}"; do
    printf '%-8s %-10s %-6s %-7s %s\n' "${T_LANE[$i]}" "${T_NAME[$i]}" "${T_TO[$i]}s" "${T_GATE[$i]:--}" "${T_CMD[$i]}"
  done
  echo
  if [ "$WITH_PC" = 1 ] || [ "$PC_IMPLICIT" = 1 ]; then
    echo "pc 车道注意：只跑只读几何检查、不点球。但 pc 上 morphofly-test 若在跑，"
    echo "            有未念完的任务时 app 会每 ~30s 重开一次 StepFun 会话（计费）；跑完建议 stop。"
    echo
  fi
  echo "执行顺序：local 与真桌面车道（gpd/pc）同时启动（&）；各车道内串行；gpd 车道持 flock，pc 车道只读不加锁。"
  exit 0
fi

OUT=${OUT:-/tmp/morphofly-e2e/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT/results" || { echo "建不了输出目录 $OUT" >&2; exit 3; }
echo "输出目录：$OUT"

record() { # name lane rc status secs note
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" "$5" "$6" > "$OUT/results/$1.tsv"
}

run_task() { # $1=index
  local i=$1 name=${T_NAME[$1]} lane=${T_LANE[$1]} to=${T_TO[$1]} gate=${T_GATE[$1]} cmd=${T_CMD[$1]}
  local log="$OUT/$name.log" t0 t1 rc status
  if [ "$gate" = cost ] && [ "$WITH_COST" != 1 ]; then
    record "$name" "$lane" 0 SKIP 0 "需要 --with-cost（会打 StepFun 计费）"
    printf '  [SKIP   ] %-10s 跳过：需要 --with-cost\n' "$name"; return
  fi
  if [ "$gate" = restart ] && [ "$ALLOW_RESTART" != 1 ]; then
    record "$name" "$lane" 0 SKIP 0 "需要 --allow-restart（会重启 gpd 的 morphofly-test）"
    printf '  [SKIP   ] %-10s 跳过：需要 --allow-restart\n' "$name"; return
  fi
  t0=$(date +%s)
  ( cd "$REPO_ROOT" && timeout --kill-after=10 "$to" bash -c "$cmd" ) >"$log" 2>&1
  rc=$?
  t1=$(date +%s)
  case "$rc" in
    0) status=PASS ;;
    2) status=SKIP ;;
    124|137) status=TIMEOUT ;;
    *) status=FAIL ;;
  esac
  record "$name" "$lane" "$rc" "$status" "$((t1-t0))" "$log"
  printf '  [%-7s] %-10s rc=%-3s %ss log=%s\n' "$status" "$name" "$rc" "$((t1-t0))" "$log"
}

# 车道 → ssh 目标（local 没有远端）
lane_target() { case "$1" in gpd) echo "$GPD" ;; pc) echo "$PC" ;; *) echo "" ;; esac; }

# 这些任务要求目标机上的 morphofly-test 正在跑（否则窗口都不存在，跑出来必 FAIL）
NEEDS_APP=" ui-pos ui-top ui-size ui-click ui-pos-pc ui-top-pc ui-size-pc "
needs_app() { case "$NEEDS_APP" in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

run_lane() { # $1=lane
  local lane=$1 i svc_up=0 target
  target=$(lane_target "$lane")
  if [ -n "$target" ]; then
    if timeout 10 ssh -o BatchMode=yes -o ConnectTimeout=6 "$target" \
         'export XDG_RUNTIME_DIR=/run/user/1000; systemctl --user is-active morphofly-test' 2>/dev/null \
         | grep -qx active; then
      svc_up=1
    fi
  fi
  for i in "${SEL[@]}"; do
    [ "${T_LANE[$i]}" = "$lane" ] || continue
    if [ -n "$target" ] && [ "$svc_up" = 0 ] && needs_app "${T_NAME[$i]}"; then
      record "${T_NAME[$i]}" "$lane" 0 SKIP 0 "morphofly-test 没在跑（$target）；先 ssh $target systemctl --user start morphofly-test（本脚本不替你起服务）"
      printf '  [SKIP   ] %-10s 服务没在跑，跳过\n' "${T_NAME[$i]}"
      continue
    fi
    run_task "$i"
  done
}

# gpd 车道独占：拿不到锁就整体 SKIP，避免两人同时点桌面
run_lane_gpd_locked() {
  exec 9>"$GPD_LOCK" || { echo "无法打开锁文件 $GPD_LOCK" >&2; return 1; }
  if ! flock -n 9; then
    for i in "${SEL[@]}"; do
      [ "${T_LANE[$i]}" = gpd ] || continue
      record "${T_NAME[$i]}" gpd 0 SKIP 0 "gpd 正被另一个进程占用（$GPD_LOCK）"
      printf '  [SKIP   ] %-10s gpd 车道被占用\n' "${T_NAME[$i]}"
    done
    return 0
  fi
  run_lane gpd
}

do_preflight

LANES=$(for i in "${SEL[@]}"; do echo "${T_LANE[$i]}"; done | sort -u)
pids=()
for lane in $LANES; do
  case "$lane" in
    local) run_lane local & pids+=($!) ;;
    gpd)   run_lane_gpd_locked & pids+=($!) ;;
    pc)    run_lane pc & pids+=($!) ;;
    *)     echo "未知车道 $lane" >&2 ;;
  esac
done
for p in "${pids[@]}"; do wait "$p"; done

# ── 汇总 ──────────────────────────────────────────────────────────────
echo
echo "=== 汇总 ==="
nfail=0; nskip=0
printf '%-10s %-6s %-8s %-6s %s\n' TASK LANE STATUS RC NOTE
for i in "${SEL[@]}"; do
  name=${T_NAME[$i]}
  f="$OUT/results/$name.tsv"
  if [ ! -f "$f" ]; then
    printf '%-10s %-6s %-8s %-6s %s\n' "$name" "${T_LANE[$i]}" MISSING - "没有结果文件"
    nfail=$((nfail+1)); continue
  fi
  IFS=$'\t' read -r _n _l rc status secs note < "$f"
  printf '%-10s %-6s %-8s %-6s %s\n' "$name" "$_l" "$status" "$rc" "$note"
  case "$status" in FAIL|TIMEOUT) nfail=$((nfail+1)) ;; SKIP) nskip=$((nskip+1)) ;; esac
done
echo
echo "日志：$OUT    FAIL/TIMEOUT=$nfail  SKIP=$nskip"
[ "$nfail" -gt 0 ] && exit 1
[ "$nskip" -gt 0 ] && exit 2
exit 0
