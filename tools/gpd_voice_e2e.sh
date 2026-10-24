#!/usr/bin/env bash
# 在 wings-gpd 上做「真机语音全链路」测试：唤醒 → 建会话 → 说任务 → 派活 → 主动汇报。
#
# ## 为什么需要「数字注入」而不是对着麦克风喊
#
# gpd 是本项目**唯一有真实麦克风**的机器（wings-pc 只有输出声卡）。
# 但它没有接实体音箱 —— 实测：放音时 monitor 源 RMS≈1400，
# 而麦克风里 RMS≈266（就是底噪），也就是**声音出不来、进不去**。
# 所以声学回环做不了，改成把音频直接灌进一个虚拟音源，让 App 从它录音。
#
# 这不算作弊：除了「空气」这一段，KWS、AEC、云端会话、工具调用、
# DSH 派活、主动汇报全都是真的跑。
#
# ## 用法
#   tools/gpd_voice_e2e.sh                 # 唤醒 + 默认任务
#   tools/gpd_voice_e2e.sh --task 文本      # 现场用 edge-tts 合成任务语音
#   tools/gpd_voice_e2e.sh --wake-only     # 只测唤醒
#   tools/gpd_voice_e2e.sh --speaker       # 用真麦克风（需要接音箱）
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

SINK=mfsink
WAKE_WAV=/tmp/xiaodie.wav
TASK_TEXT="帮我看看 morphofly 项目的 README 第一行是什么"
TASK_WAV=/tmp/task1.wav
MODE=inject
WAKE_ONLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --task)      TASK_TEXT="$2"; shift 2 ;;
    --wake-only) WAKE_ONLY=1; shift ;;
    --speaker)   MODE=speaker; shift ;;
    *) echo "未知参数: $1"; exit 2 ;;
  esac
done

T="$(tools/gpd_ssh.sh 2>/dev/null)" || { echo "❌ wings-gpd 不在线"; exit 2; }
S='ssh -o BatchMode=yes -o ConnectTimeout=15'
run() { timeout 120 $S "$T" "$@"; }

echo "=== 1) 准备音频 ==="
if [ "$MODE" = inject ]; then
  # 虚拟音源：往里放什么，App 就录到什么
  run "pactl list short sources | grep -q $SINK.monitor || pactl load-module module-null-sink sink_name=$SINK >/dev/null; pactl set-default-source $SINK.monitor; echo \"默认音源: \$(pactl get-default-source)\""
fi

if ! run "[ -f $WAKE_WAV ]" ; then
  echo "  生成唤醒词音频…"
  timeout 120 uvx --from edge-tts edge-tts --voice zh-CN-XiaoxiaoNeural \
    --text "小蝶" --write-media /tmp/xiaodie.mp3 >/dev/null 2>&1
  scp -q /tmp/xiaodie.mp3 "$T:/tmp/xiaodie.mp3"
  run "ffmpeg -y -loglevel error -i /tmp/xiaodie.mp3 -ar 16000 -ac 1 $WAKE_WAV"
fi
[ "$WAKE_ONLY" = 1 ] || {
  echo "  生成任务语音: $TASK_TEXT"
  timeout 120 uvx --from edge-tts edge-tts --voice zh-CN-XiaoxiaoNeural \
    --text "$TASK_TEXT" --write-media /tmp/task.mp3 >/dev/null 2>&1
  scp -q /tmp/task.mp3 "$T:/tmp/task.mp3"
  run "ffmpeg -y -loglevel error -i /tmp/task.mp3 -ar 16000 -ac 1 $TASK_WAV"
}

echo "=== 2) 重启 App ==="
run "systemctl --user restart morphofly-test; sleep 12; systemctl --user is-active morphofly-test"
run "journalctl --user -u morphofly-test --since '-30 sec' --no-pager | grep -E '音频后端|KWS 常驻' | tail -3"

MARK="$(date '+%Y-%m-%d %H:%M:%S')"
DEV="--device=$SINK"
[ "$MODE" = speaker ] && DEV=""

echo "=== 3) 注入唤醒词 ==="
run "timeout 15 paplay $DEV $WAKE_WAV"
sleep 5
WOKE=$(run "journalctl --user -u morphofly-test --since '$MARK' --no-pager | grep -c '检测到唤醒词'")
if [ "$WOKE" -lt 1 ]; then
  echo "❌ 没检测到唤醒词"
  run "journalctl --user -u morphofly-test --since '$MARK' --no-pager | tail -10"
  exit 1
fi
echo "✅ 唤醒成功（$WOKE 次）"

[ "$WAKE_ONLY" = 1 ] && { echo "=== 只测唤醒，结束 ==="; exit 0; }

echo "=== 4) 等会话真的建立再说话 ==="
# ⚠️ 2026-10-24 踩坑：原来固定 sleep 2 就灌任务语音，但建会话有时要 10+ 秒
# （云端 WS 握手 + 机器忙），语音灌进去时还没开始听 → delegate_task 为 0，
# 看起来像「模型没干活」，其实是我们说太早。
READY=0
for _ in $(seq 1 30); do
  if run "journalctl --user -u morphofly-test --since '$MARK' --no-pager | grep -c '会话已建立'" | grep -qv '^0$'; then
    READY=1
    break
  fi
  sleep 1
done
if [ "$READY" = 1 ]; then echo "会话已建立"; else echo "⚠️ 30 秒内没建会话，仍然尝试说话"; fi
sleep 1
run "timeout 20 paplay $DEV $TASK_WAV"
# 派活之后 DSH 要真干活，汇报通常 15–40 秒才回来。轮询等它，别用固定 sleep 蒙。
for _ in $(seq 1 40); do
  if run "journalctl --user -u morphofly-test --since '$MARK' --no-pager | grep -q '主动汇报'" ; then break; fi
  sleep 1
done
sleep 2

echo "=== 5) 结果 ==="
LOG=$(run "journalctl --user -u morphofly-test --since '$MARK' --no-pager")
echo "$LOG" | grep -E "工具调用|派活|主动汇报|会话已建立|纠正" | sed 's/.*morphofly-app\[[0-9]*\]: //' | head -30

# ★ 派活有两条路径：
#   1. 模型自己调 delegate_task（工具调用）
#   2. 模型只说不动 → 本地拿用户原话代派（日志是「派活 agent=」）
# 两条都算「派出去了」，否则本地代派这种正常路径会被判失败。
# 只数「真的派出去了」这一行。工具调用日志和派活日志会同时出现，
# 一起数会数成 2 → 脚本误判「重复派活」（2026-10-25 04:28 踩过）。
DELEGATED=$(echo "$LOG" | grep -c "派活 agent=")
TOOLCALLS=$(echo "$LOG" | grep -c "工具调用 name=delegate_task")
SESSIONS=$(echo "$LOG" | grep -c "DSH 会话已建")
REPORTS=$(echo "$LOG" | grep -c "主动汇报")
CORRECTIONS=$(echo "$LOG" | grep -c "已注入纠正提醒")

echo
echo "── 计数 ──"
echo "派活次数:          $DELEGATED   （期望 1；模型自己调工具和本地代派都走这一行）"
echo "模型工具调用:      $TOOLCALLS   （仅供参考：>0 表示模型自己干活，0 表示走了本地代派）"
echo "DSH 会话新建:      $SESSIONS    （期望 1）"
echo "主动汇报:          $REPORTS     （期望 1）"
echo "兜底纠正注入:      $CORRECTIONS （期望 0；本地代派路径下也是 0）"

if [ "$DELEGATED" -eq 1 ] && [ "$SESSIONS" -le 1 ] && [ "$CORRECTIONS" -le 1 ] && [ "$REPORTS" -ge 1 ]; then
  echo "✅ 全链路通过：唤醒 1 次、派活 1 次、汇报 $REPORTS 次"
  exit 0
fi
if [ "$DELEGATED" -eq 1 ] && [ "$SESSIONS" -le 1 ] && [ "$CORRECTIONS" -le 1 ]; then
  echo "⚠️ 派活成功但没等到主动汇报（DSH 可能还在干，或会话已超时）"
  exit 1
fi
echo "❌ 有重复派活或多余纠正，看上面日志"
exit 1
