#!/usr/bin/env bash
# AEC 真机活体验证（在有麦克风+扬声器的机器上跑）。
#
# 原理：播放一段真实语音，同时用麦克风录音。
# 录到的就是「外放被麦克风收回来的回声」。
# 然后对比过 AEC / 不过 AEC 的能量。
#
# 这是唯一能真实验证 AEC 的方法 —— 需要真实声学路径。
# monitor 源和 aloop 都不行（数字域回环没有真实卷积关系）。
#
# 用法：
#   tools/aec_live.sh                  # 用默认设备
#   tools/aec_live.sh --list           # 列出设备
#   tools/aec_live.sh --sink X --source Y
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

SINK=""
SOURCE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --list)
      echo "=== 输出设备 ==="; pactl list short sinks 2>/dev/null
      echo "=== 输入设备 ==="; pactl list short sources 2>/dev/null | grep -v monitor
      exit 0 ;;
    --sink) SINK="$2"; shift 2 ;;
    --source) SOURCE="$2"; shift 2 ;;
    *) shift ;;
  esac
done

# 默认设备
[ -z "$SINK" ] && SINK="$(pactl get-default-sink 2>/dev/null || echo '')"
[ -z "$SOURCE" ] && SOURCE="$(pactl get-default-source 2>/dev/null || echo '')"

echo "输出: ${SINK:-默认}"
echo "输入: ${SOURCE:-默认}"

# 生成测试语音（用 edge-tts，没有就退回合成音）
WAV=/tmp/aec_live_speech.wav
if [ ! -f "$WAV" ]; then
  echo "=== 生成测试语音 ==="
  if command -v uv >/dev/null 2>&1; then
    timeout 90 uv run --quiet --with edge-tts edge-tts \
      --voice zh-CN-XiaoxiaoNeural \
      --text "小蝶小蝶，帮我看看这个项目的测试覆盖率怎么样了" \
      --write-media /tmp/aec_live_tts.mp3 2>/dev/null
    if [ -f /tmp/aec_live_tts.mp3 ]; then
      timeout 90 uv run --quiet --with miniaudio python -c "
import miniaudio, wave, array
d = miniaudio.decode_file('/tmp/aec_live_tts.mp3',
    output_format=miniaudio.SampleFormat.SIGNED16,
    nchannels=1, sample_rate=16000)
w = wave.open('/tmp/aec_live_speech.wav','wb')
w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
w.writeframes(bytes(d.samples)); w.close()
print('语音已生成')
" && rm -f /tmp/aec_live_tts.mp3
    fi
  fi
  if [ ! -f "$WAV" ]; then
    echo "edge-tts 不可用，用合成音代替"
    python3 -c "
import math, struct, wave
sr=16000
fr=bytearray()
for n in range(sr*3):
    # 基频滑动，模拟语调（固定频率会让 AEC3 的延迟估计锁错）
    f=200+150*math.sin(2*math.pi*0.8*n/sr)
    v=int(12000*math.sin(2*math.pi*f*n/sr))
    fr+=struct.pack('<h',v)
w=wave.open('/tmp/aec_live_speech.wav','wb')
w.setnchannels(1); w.setsampwidth(2); w.setframerate(sr)
w.writeframes(bytes(fr)); w.close()
"
  fi
fi

echo "=== 开始：先录底噪 2s，再边放边录 ==="
REC=/tmp/aec_live_rec.wav
rm -f "$REC"
SRC_ARG=""; [ -n "$SOURCE" ] && SRC_ARG="--target=$SOURCE"
SNK_ARG=""; [ -n "$SINK" ] && SNK_ARG="--target=$SINK"

# ⚠️ 用 parecord/paplay 而不是 pw-cat。
#
# 2026-10-24 实测：wings-gpd 上 pw-cat 录到纯静音（RMS=0），
# 但设备状态正常（80% 音量、未静音）。
# 设备 SUSPENDED 时 pw-cat 的默认节点解析没正确 resume。
# parecord/paplay 走 PulseAudio 兼容层，显式指定设备就能录到。
SRC_DEV=""; [ -n "$SOURCE" ] && SRC_DEV="--device=$SOURCE"
SNK_DEV=""; [ -n "$SINK" ] && SNK_DEV="--device=$SINK"

# 1) 静音底噪
timeout 4 parecord $SRC_DEV --format=s16le --rate=16000 --channels=1 --file-format=wav /tmp/aec_noise.wav 2>/dev/null

# 2) 播放 + 同时录音
timeout 15 parecord $SRC_DEV --format=s16le --rate=16000 --channels=1 --file-format=wav "$REC" 2>/dev/null &
RECPID=$!
sleep 2
timeout 12 paplay $SNK_DEV "$WAV" 2>/dev/null
sleep 1
kill $RECPID 2>/dev/null
wait $RECPID 2>/dev/null

if [ ! -f "$REC" ]; then
  echo "❌ 录音失败"
  exit 1
fi

echo "=== 分析 ==="
cd ~/Code_Project/morphofly 2>/dev/null
cargo run -q --release -p morphofly-core --example aec_aloop_test 2>/dev/null || true

python3 <<'PYEOF'
import wave, array, math

def load(p):
    try:
        w = wave.open(p)
        d = w.readframes(w.getnframes())
        a = array.array('h'); a.frombytes(d)
        return [x/32768.0 for x in a]
    except Exception as e:
        return None

speech = load('/tmp/aec_live_speech.wav')
noise  = load('/tmp/aec_noise.wav')
rec    = load('/tmp/aec_live_rec.wav')

if not rec:
    print("❌ 读不到录音"); raise SystemExit(1)

rms = lambda v: math.sqrt(sum(x*x for x in v)/max(len(v),1))
print(f"底噪 RMS:  {rms(noise) if noise else 0:.5f}")
print(f"录音 RMS:  {rms(rec):.5f}")
print(f"语音 RMS:  {rms(speech) if speech else 0:.5f}")
print()

if noise and rms(rec) < rms(noise) * 1.5:
    print("❌ 录音里没有明显的音频 —— 扬声器没响或麦克风没收到")
    print("   检查：音量是否静音、麦克风是否被占用")
    raise SystemExit(1)

print("✅ 录音里有音频，说明通路成立")
print()
print("注意：真正的 AEC 效果要跑完整 App 才能测（需要 reference 信号对齐）。")
print("这个脚本只验证「外放能被麦克风采到」这个前提。")
print()
print("下一步：")
print("  1. 启动蝶梦 App")
print("  2. 外放让它说话")
print("  3. 中间喊『停』，看它会不会被打断")
print("  4. 观察日志里 AEC 的 stats")
PYEOF
