#!/usr/bin/env python3
"""用免费 TTS 合成测试音频。

用 edge-tts（微软免费 TTS，无需 key），中文女声。
生成 16k mono PCM，格式与蝶梦上行一致（320 采样/帧）。

用法：
    python3 tools/e2e/tts.py "小蝶" -o /tmp/a.pcm
    python3 tools/e2e/tts.py --batch cases.yaml -o /tmp/audio/
"""
import argparse
import asyncio
import subprocess
import sys
import tempfile
from pathlib import Path

# 中文女声，接近用户期待的小蝶音色
VOICE = "zh-CN-XiaoxiaoNeural"
SAMPLE_RATE = 16000


async def synth(text: str, out_path: Path, voice: str = VOICE) -> None:
    """合成一句话到 16k mono s16le PCM。"""
    with tempfile.NamedTemporaryFile(suffix=".mp3", delete=False) as tf:
        mp3 = Path(tf.name)
    try:
        # edge-tts 走命令行，避免依赖导入问题
        proc = await asyncio.create_subprocess_exec(
            "edge-tts", "--voice", voice, "--text", text,
            "--write-media", str(mp3),
            stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
        )
        _, err = await proc.communicate()
        if proc.returncode != 0:
            raise RuntimeError(f"edge-tts 失败: {err.decode()[:200]}")
        if not mp3.exists() or mp3.stat().st_size == 0:
            raise RuntimeError("edge-tts 没有产出文件")

        # 转成 16k mono s16le PCM
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-i", str(mp3),
             "-ar", str(SAMPLE_RATE), "-ac", "1", "-f", "s16le", str(out_path)],
            check=True,
        )
    finally:
        mp3.unlink(missing_ok=True)


async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("text", nargs="?", help="要合成的文本")
    ap.add_argument("-o", "--out", default="/tmp/tts.pcm", help="输出 PCM 路径")
    ap.add_argument("--voice", default=VOICE)
    args = ap.parse_args()

    if not args.text:
        ap.print_help()
        return 1

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    await synth(args.text, out, args.voice)
    n = out.stat().st_size // 2
    print(f"✅ {out}  {n} 采样  {n / SAMPLE_RATE:.2f}s")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
