#!/usr/bin/env bash
# tools/verify_audio_topology.sh — 检查真机音频输入/输出流，防「自打断」
#
# ## 为什么要这个
#
# 语音测试靠虚拟音源（mfsink）注入：`paplay --device=mfsink` → mfsink.monitor
# → App 采集。而 App 的**播放**走系统默认 sink。两者如果撞在同一个设备上，
# App 会听到自己的 TTS → 本地 VAD 判定「用户开口」→ 自己打断自己。
# 2026-10-25 用户明确要求排查这个风险。
#
# ## 检查什么
#
#   1. 默认 sink / source 分别是什么
#   2. 默认 source 是不是**默认 sink 的 monitor**（是 → 自打断高危配置）
#   3. App 是否同时持有「播放流」和「录音流」（sink-input / source-output）
#   4. 两者的设备 id 是否冲突（播放写进采集源 = 自打断）
#
# 用法：tools/verify_audio_topology.sh [--live]
#   --live  额外跑一次「模型说话但没人插话」的会话，断言 interruptions=0
# 退出码：0 结构正常 / 1 有高危配置 / 2 跳过（设备不在线/服务没跑）

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

SERVICE=morphofly-test
SINK_NAME=mfsink
LIVE=0
[ "${1:-}" = "--live" ] && LIVE=1

say() { printf '[audio] %s\n' "$*"; }
fail() { printf '[audio] FAIL %s\n' "$*"; exit 1; }
skip() { printf '[audio] SKIP %s\n' "$*"; exit 2; }

T=$(bash tools/gpd_ssh.sh 2>/dev/null) || skip "wings-gpd 不在线"
run() { timeout 60 ssh -o BatchMode=yes -o ConnectTimeout=10 "$T" "$@"; }

run "systemctl --user is-active $SERVICE" 2>/dev/null | grep -qx active ||
  skip "morphofly-test 没在跑（先 tools/gpd_deploy_ci.sh）"

RC=0

# ── 1) 默认设备 ──
DEF_SINK=$(run 'pactl get-default-sink')
DEF_SRC=$(run 'pactl get-default-source')
say "默认 sink   = $DEF_SINK"
say "默认 source = $DEF_SRC"

# ── 2) 自打断高危：默认 source 是默认 sink 的 monitor ──
if [ "$DEF_SRC" = "$DEF_SINK.monitor" ]; then
  printf '[audio] ❌ 高危配置：采集源就是默认输出的 monitor ——\n' >&2
  printf '[audio]    App 会听到自己的 TTS → 自打断。测试时别这么配。\n' >&2
  RC=1
else
  say "✅ 采集源与默认输出不是同一个设备（无扬声器回灌路径）"
fi

# ── 3) App 的流（播放 / 录音）──
say "App 的音频流："
STREAMS=$(run 'pactl list sink-inputs; echo ---- ; pactl list source-outputs' | sed -r 's/\x1b\[[0-9;]*[A-Za-z]//g')
printf '%s\n' "$STREAMS" | grep -E 'Sink Input|Source Output|application.name|Sink:|Source:' |
  sed 's/^/  /' | head -20

PLAY_ID=$(printf '%s\n' "$STREAMS" | awk '
  /Sink Input/ {cur=$0}
  /application.name = "cpal-pulseaudio/ {print cur}
' | head -1)
REC_ID=$(printf '%s\n' "$STREAMS" | awk '
  /Source Output/ {cur=$0}
  /application.name = "cpal-pulseaudio/ {print cur}
' | head -1)

if [ -n "$PLAY_ID" ]; then
  say "✅ App 播放流存在（cpal-pulseaudio → sink）"
else
  say "⚠️ 没看到 App 播放流（可能这一刻没在说话；会话中再看更准）"
fi
if [ -n "$REC_ID" ]; then
  say "✅ App 录音流存在（cpal-pulseaudio ← source）"
else
  say "⚠️ 没看到 App 录音流（采集可能没起）"
fi

# ── 4) 注入通道是否落在 mfsink 上（测试口径）──
if [ "$DEF_SRC" = "$SINK_NAME.monitor" ]; then
  say "✅ 测试口径：默认采集源 = $SINK_NAME.monitor（注入的语音能被 App 录到）"
else
  say "ℹ️ 默认采集源不是 $SINK_NAME.monitor，注入测试前要 pactl set-default-source $SINK_NAME.monitor，并重启 $SERVICE"
fi

# ── 5) 可选：跑一次「只让它说，不插话」的会话，断言 interruptions=0 ──
if [ "$LIVE" = 1 ]; then
  say "live：跑一次「模型说话，没人插话」的会话…"
  WAKE=/tmp/bargein-wake.wav
  ASK=/tmp/bargein-ask.wav
  run "[ -f $WAKE ] && [ -f $ASK ]" || skip "缺 $WAKE / $ASK（先跑一次 tools/verify_bargein.sh 生成）"
  run "systemctl --user restart $SERVICE" >/dev/null || fail "重启服务失败"
  for _ in $(seq 1 40); do
    if run "journalctl --user -u $SERVICE -n 8 --no-pager -o cat" |
      sed -r 's/\x1b\[[0-9;]*[A-Za-z]//g' | grep -q 'KWS 常驻监听已启动'; then break; fi
    sleep 1
  done
  EPOCH=$(run 'date +%s')
  run "timeout 15 paplay --device=$SINK_NAME $WAKE" >/dev/null
  sleep 6
  run "nohup timeout 40 paplay --device=$SINK_NAME $ASK >/dev/null 2>&1 &"
  say "等模型开口后观察 25 秒（不注入任何打断语音）…"
  for _ in $(seq 1 50); do
    if run "journalctl --user -u $SERVICE -n 6 --no-pager -o cat" |
      sed -r 's/\x1b\[[0-9;]*[A-Za-z]//g' | grep -q 'state="S4"'; then break; fi
    sleep 0.5
  done
  sleep 25
  LOG=$(run "journalctl --user -u $SERVICE --since @$EPOCH --no-pager -o cat" |
    sed -r 's/\x1b\[[0-9;]*[A-Za-z]//g')
  HITS=$(printf '%s\n' "$LOG" | grep -c '记一次打断' || true)
  STATS=$(printf '%s\n' "$LOG" | grep -oE 'interruptions: [0-9]+' | tail -1)
  say "期间打断日志条数=${HITS:-0}，会话统计 ${STATS:-（会话还没结束）}"
  if [ "${HITS:-0}" -gt 0 ]; then
    printf '[audio] ❌ 没人插话却记了打断 —— 存在自打断/误打断\n' >&2
    RC=1
  else
    say "✅ 没人插话时没有打断记录（无自打断）"
  fi
fi

[ "$RC" = 0 ] && say "PASS 音频输入输出结构正常" || say "FAIL 见上面的高危项"
exit "$RC"
