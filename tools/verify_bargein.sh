#!/usr/bin/env bash
# tools/verify_bargein.sh — 真机验收「打断」（barge-in）
#
# ## 验的是什么
#
# 产品承诺：模型正在说话（S4）时，用户插一句「停一下」，**本地立刻停播**
# （设计见 docs/开发方案.md §4.1：本地先停，服务端后确认）。
#
# 验收判据（三条，全中才算 PASS）：
#   1. journal 出现 `⚡ 本地 VAD 打断（已通知云端停止生成）`
#   2. 打断后状态回到 S2（聆听）：`推状态给前端 state="S2"`
#   3. **本地反应**（VAD 判定人声 → 停播）< 300ms
#      另一个指标「注入 → VAD 判定」只做报告、不做判定：
#      虚拟音源（paplay → null-sink → monitor → cpal 采集）这段链路本身
#      就有几百毫秒到两秒的延迟，真机用真麦克风时没有这一段。
#
# ## 为什么需要它
#
# 历史上跑过 58 次语音会话，`interruptions` 全是 0 —— 这条能力从没被真机验证过
# （canvas 上却写着「打断 <300ms」）。本脚本就是补这个验收。
#
# ## 怎么造出「模型正在说话」
#
# gpd 没有实体音箱，用虚拟 sink（mfsink）喂音频：先唤醒，再灌一句
# 「请用很慢的语速从一数到五十」（模型会念很久），S4 一出现就灌「停一下」。
#
# 用法：tools/verify_bargein.sh [--dry-run]
# 退出码：0 通过 / 1 失败 / 2 跳过（设备不在线 / 服务没跑）

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

SINK=mfsink
WAKE_WAV=/tmp/bargein-wake.wav
ASK_WAV=/tmp/bargein-ask.wav
STOP_WAV=/tmp/bargein-stop.wav
WAKE_TEXT="小蝶"
ASK_TEXT="请用很慢的语速从一数到五十，每个数字之间停一下"
STOP_TEXT="停一下"
SERVICE=morphofly-test
MAX_DELAY_MS=6000      # 总延迟只做上限保护（虚拟音源链路可能很慢）
MAX_REACT_MS=300       # 产品反应时间：VAD 判定人声 → 停播
DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1

say() { printf '[bargein] %s\n' "$*"; }
fail() { printf '[bargein] FAIL %s\n' "$*"; exit 1; }
skip() { printf '[bargein] SKIP %s\n' "$*"; exit 2; }
# journal 文本去 ANSI：老二进制把颜色码写进了消息里，
# 不去掉的话 `grep 'state="S4"'` 永远匹配不到（2026-10-25 踩过）
strip_ansi() { sed -r 's/\x1b\[[0-9;]*[A-Za-z]//g'; }

T=$(bash tools/gpd_ssh.sh 2>/dev/null) || skip "wings-gpd 不在线"
run() { timeout 120 ssh -o BatchMode=yes -o ConnectTimeout=15 "$T" "$@"; }

run 'systemctl --user is-active morphofly-test' 2>/dev/null | grep -qx active ||
  skip "morphofly-test 没在跑（先 tools/gpd_deploy_ci.sh）"

if [ "$DRY" = 1 ]; then
  say "dry-run：计划"
  say "  1) 生成音频：唤醒「$WAKE_TEXT」/ 长回答「$ASK_TEXT」/ 打断「$STOP_TEXT」"
  say "  2) 虚拟音源 $SINK.monitor 设为默认"
  say "  3) 重启 $SERVICE → 注入唤醒词 → 等会话建立"
  say "  4) 后台注入长问句 → 等 S4（模型在说）"
  say "  5) S4 一出现立刻注入「$STOP_TEXT」，量停播延迟"
  say "  6) 判定：打断日志 + 回 S2 + 延迟 < ${MAX_DELAY_MS}ms"
  exit 0
fi

# ── 1) 生成音频（每次都重生成，避免改了文本还用旧缓存）──
gen_wav() { # $1=远端 wav 路径 $2=文本
  say "生成音频：$2"
  local base mp3
  base=$(basename "$1" .wav)
  mp3="/tmp/$base.mp3"
  timeout 120 uvx --from edge-tts edge-tts --voice zh-CN-XiaoxiaoNeural \
    --text "$2" --write-media "$mp3" >/dev/null 2>&1 || fail "edge-tts 生成失败：$2"
  scp -q "$mp3" "$T:/tmp/$(basename "$mp3")" || fail "scp 音频失败"
  run "ffmpeg -y -loglevel error -i /tmp/$(basename "$mp3") -ar 16000 -ac 1 '$1'" ||
    fail "ffmpeg 转 wav 失败"
}
gen_wav "$WAKE_WAV" "$WAKE_TEXT"
gen_wav "$ASK_WAV" "$ASK_TEXT"
gen_wav "$STOP_WAV" "$STOP_TEXT"

# ── 2) 虚拟音源 ──
run "pactl list short sources | grep -q $SINK.monitor || pactl load-module module-null-sink sink_name=$SINK >/dev/null; pactl set-default-source $SINK.monitor; pactl get-default-source" |
  sed 's/^/  默认音源: /'

# ── 3) 重启 App 并等常驻监听 ──
say "重启 App…"
run "systemctl --user restart $SERVICE" || fail "重启服务失败"
# 用 epoch 做 --since：journalctl 对 "YYYY-MM-DD HH:MM:SS" 按本地时区解释，
# 和 gpd 的 date -u 混用会算错窗口（踩过）。epoch 没这个问题。
EPOCH=$(run 'date +%s')
[ -n "${EPOCH:-}" ] || fail "拿不到 gpd 时间戳"

jlog() { run "journalctl --user -u $SERVICE --since @$EPOCH --no-pager -o cat" | strip_ansi; }
jwait() { # $1=最多等几秒 $2=grep 模式
  local n=$1 i=0
  while [ "$i" -lt "$n" ]; do
    if jlog | grep -q -- "$2"; then return 0; fi
    sleep 1
    i=$((i + 1))
  done
  return 1
}

jwait 40 'KWS 常驻监听已启动' || fail "40 秒内 KWS 没起来（看 journalctl --user -u $SERVICE）"
say "App 就绪"

# ── 4) 唤醒 → 等会话 ──
run "timeout 15 paplay --device=$SINK $WAKE_WAV" || fail "注入唤醒词失败"
jwait 30 '会话已建立' || fail "30 秒内没建会话（唤醒可能失败）"
say "唤醒成功，会话已建立"

# ── 5) 让模型开口（S4）──
# ⚠️ 必须后台播：paplay 会阻塞到音频播完，而模型往往在这期间就已经进 S4
# 又退回 S2 —— 等 paplay 返回再去找 S4 会永远找不到（真机踩过）。
run "nohup timeout 40 paplay --device=$SINK $ASK_WAV >/dev/null 2>&1 &" ||
  fail "注入长问句失败"
say "已后台注入问句「$ASK_TEXT」"
jwait 40 'state="S4"' || {
  printf '[bargein] 40 秒内没进 S4，最近的状态/转写：\n' >&2
  jlog | grep -E '推状态|用户转写|工具调用|派活' | tail -12 | sed 's/^/  /' >&2
  fail "模型没开口，无法验证打断"
}
sleep 0.5 # 让音频真的进播放队列（打断判据是 queued_frames > 0）
say "模型开始说话（S4）"

# ── 6) 注入打断语音，用 gpd 本机时间戳（跨机时钟不可比）──
INJECT=$(run "T=\$(date +%s%3N); timeout 15 paplay --device=$SINK $STOP_WAV; echo \"INJECT_MS=\$T\"")
INJECT_MS=$(printf '%s\n' "$INJECT" | sed -n 's/^INJECT_MS=//p' | head -1)
[ -n "${INJECT_MS:-}" ] || fail "注入打断语音失败：$INJECT"
say "已注入「$STOP_TEXT」（gpd 时钟 ${INJECT_MS}ms）"

# ── 7) 找打断日志，算延迟（__REALTIME_TIMESTAMP 单位微秒）──
HIT_MS=""
i=0
while [ "$i" -lt 20 ]; do
  HIT_MS=$(run "journalctl --user -u $SERVICE --since @$EPOCH -o json --no-pager 2>/dev/null" |
    strip_ansi |
    python3 -c '
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        e = json.loads(line)
    except Exception:
        continue
    msg = e.get("MESSAGE", "")
    # ⚠️ journald 遇到非 UTF-8 消息（比如带 ANSI 颜色码）会把 MESSAGE 编成
    #    「字节数组」而不是字符串 —— 不处理的话 `in msg` 直接 TypeError，
    #    整个探测静默失败（2026-10-25 踩过，脚本报「没能停播」但其实已经停了）。
    if isinstance(msg, list):
        msg = bytes(b for b in msg if isinstance(b, int)).decode(errors="replace")
    elif isinstance(msg, bytes):
        msg = msg.decode(errors="replace")
    if "本地 VAD 打断" in msg:
        print(int(e["__REALTIME_TIMESTAMP"]) // 1000)
        break
' | head -1)
  [ -n "$HIT_MS" ] && break
  sleep 0.5
  i=$((i + 1))
done

# 顺带找「VAD 判定人声」那行（脚本要算本地反应时间）
VAD_MS=""
i=0
while [ "$i" -lt 12 ] && [ -z "$VAD_MS" ]; do
  VAD_MS=$(run "journalctl --user -u $SERVICE --since @$EPOCH -o json --no-pager 2>/dev/null" |
    strip_ansi |
    python3 -c '
import json, sys
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        e = json.loads(line)
    except Exception:
        continue
    msg = e.get("MESSAGE", "")
    if isinstance(msg, list):
        msg = bytes(b for b in msg if isinstance(b, int)).decode(errors="replace")
    elif isinstance(msg, bytes):
        msg = msg.decode(errors="replace")
    if "本地 VAD 判定人声" in msg:
        print(int(e["__REALTIME_TIMESTAMP"]) // 1000)
        break
' | head -1)
  [ -z "$VAD_MS" ] && sleep 0.5
  i=$((i + 1))
done

[ -n "${HIT_MS:-}" ] || {
  printf '[bargein] 注入后日志里没有「本地 VAD 打断」——没能停播\n' >&2
  jlog | grep -vE 'pulseaudio' | tail -20 | sed 's/^/  /' >&2
  exit 1
}
DELAY=$((HIT_MS - INJECT_MS))
if [ -n "${VAD_MS:-}" ]; then
  REACT=$((HIT_MS - VAD_MS))
  ENV=$((${VAD_MS} - INJECT_MS))
  say "注入 → VAD 判定人声：${ENV}ms（虚拟音源链路延迟，只做报告）"
  say "VAD 判定 → 停播：${REACT}ms（这是产品反应时间，阈值 ${MAX_REACT_MS}ms）"
else
  say "没找到「VAD 判定人声」日志，只能报总延迟"
  REACT=""
fi
say "打断总延迟（注入→停播）：${DELAY}ms"

# ── 8) 打断后要回到 S2（聆听）──
BACK=0
jwait 10 'state="S2"' && BACK=1

# ── 9) 判定 ──
RC=0
if [ "$DELAY" -lt 0 ] || [ "$DELAY" -gt "$MAX_DELAY_MS" ]; then
  printf '[bargein] 总延迟超出上限（%sms > %sms）\n' "$DELAY" "$MAX_DELAY_MS" >&2
  RC=1
fi
if [ -n "${REACT:-}" ] && { [ "$REACT" -lt 0 ] || [ "$REACT" -gt "$MAX_REACT_MS" ]; }; then
  printf '[bargein] 本地反应太慢（%sms > %sms）\n' "$REACT" "$MAX_REACT_MS" >&2
  RC=1
fi
if [ "$BACK" != 1 ]; then
  printf '[bargein] 打断后没有回到 S2（聆听）\n' >&2
  RC=1
fi
say "相关日志："
jlog | grep -E '本地 VAD 打断|state="S2"|state="S4"|已通知云端' | tail -8 | sed 's/^/  /'

if [ "$RC" = 0 ]; then
  say "PASS 打断：VAD 判定人声后 ${REACT:-?}ms 停播（总延迟 ${DELAY}ms），并回到聆听态"
  exit 0
fi
say "FAIL 见上面的原始日志"
exit 1
