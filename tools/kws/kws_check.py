#!/usr/bin/env python3
"""蝶梦 · 唤醒词 KWS 验收脚本

用法：
    python3 kws_check.py <音频文件或目录> [...]

输出：每个文件是否唤醒、唤醒时刻、应唤醒/误唤醒统计。

依赖：
    uv pip install sherpa-onnx soundfile numpy
"""
import glob
import os
import sys

import numpy as np
import soundfile as sf
import sherpa_onnx

# ---- 配置（按需改）----
MODEL_DIR = os.environ.get(
    "KWS_MODEL",
    "sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20",
)
KEYWORDS = os.environ.get("KWS_KEYWORDS", "keywords.txt")
THRESHOLD = float(os.environ.get("KWS_THRESHOLD", "0.25"))
SCORE = float(os.environ.get("KWS_SCORE", "1.0"))
# ★ 关键参数：关键词后必须连续 N 帧 blank 才触发。
# 2026-10-23 实测：1 → 「小蝶儿」误唤醒；4 → 误唤醒归零且召回不变。
# 安全区间 4–10；≥12 会漏检连续语音。
BLANKS = int(os.environ.get("KWS_BLANKS", "4"))

# 文件名前缀 → 期望行为
# 以这些开头的文件「应该唤醒」，其余「不应唤醒」
# 16/17 是 2026-10-24 新增的「带完整唤醒词」正样本（派活/中文项目名）
POSITIVE_PREFIXES = ("01_", "02_", "03_", "06_", "07_", "08_", "09_", "16_", "17_", "ZZ_")


def make_spotter() -> "sherpa_onnx.KeywordSpotter":
    return sherpa_onnx.KeywordSpotter(
        encoder=os.path.join(MODEL_DIR, "encoder-epoch-13-avg-2-chunk-16-left-64.int8.onnx"),
        decoder=os.path.join(MODEL_DIR, "decoder-epoch-13-avg-2-chunk-16-left-64.onnx"),
        joiner=os.path.join(MODEL_DIR, "joiner-epoch-13-avg-2-chunk-16-left-64.int8.onnx"),
        tokens=os.path.join(MODEL_DIR, "tokens.txt"),
        keywords_file=KEYWORDS,
        num_threads=1,
        keywords_score=SCORE,
        keywords_threshold=THRESHOLD,
        num_trailing_blanks=BLANKS,
    )


def to_16k(path: str) -> np.ndarray:
    samples, sr = sf.read(path, dtype="float32")
    if samples.ndim > 1:
        samples = samples[:, 0]
    if sr != 16000:
        n = int(len(samples) * 16000 / sr)
        idx = np.minimum((np.arange(n) * (sr / 16000.0)).astype(int), len(samples) - 1)
        samples = samples[idx]
    return samples


def run(spotter, path: str):
    """逐 100ms 喂音频，返回 [(时刻秒, 关键词)]。"""
    samples = to_16k(path)
    stream = spotter.create_stream()
    hits = []
    step = 1600  # 100ms @ 16kHz
    for i in range(0, len(samples), step):
        stream.accept_waveform(16000, samples[i : i + step])
        while spotter.is_ready(stream):
            spotter.decode_stream(stream)
            result = spotter.get_result(stream)
            if result:
                hits.append((round(i / 16000, 2), result))
                spotter.reset_stream(stream)  # 必须 reset，否则会重复触发
    return len(samples) / 16000, hits


def main(argv):
    if not argv:
        print(__doc__)
        return 1

    paths = []
    for arg in argv:
        if os.path.isdir(arg):
            paths += sorted(glob.glob(os.path.join(arg, "*.mp3")))
            paths += sorted(glob.glob(os.path.join(arg, "*.wav")))
        else:
            paths.append(arg)

    spotter = make_spotter()
    pos_total = pos_hit = neg_total = neg_fp = 0

    for path in paths:
        name = os.path.basename(path)
        dur, hits = run(spotter, path)
        mark = "✅" if hits else "⬜"
        detail = " ".join(f"[{t}s {kw}]" for t, kw in hits)
        print(f"{mark} {name:28s} {dur:5.2f}s  {detail}")

        if name.startswith(POSITIVE_PREFIXES):
            pos_total += 1
            pos_hit += 1 if hits else 0
        else:
            neg_total += 1
            neg_fp += 1 if hits else 0

    print()
    print(f"配置：threshold={THRESHOLD} score={SCORE} num_trailing_blanks={BLANKS}")
    if pos_total:
        print(f"召回：{pos_hit}/{pos_total}")
    if neg_total:
        print(f"误唤醒：{neg_fp}/{neg_total}")

    # 基线对照提醒
    if BLANKS < 4:
        print()
        print("⚠️  num_trailing_blanks < 4 时「小蝶儿」会误唤醒（实测）。")
        print("    推荐 KWS_BLANKS=4（默认）。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
