#!/usr/bin/env python3
"""从修复前连拍帧复算「球左侧气泡区」差异（红证据可复核）。

帧包：startup2-pre-fix-frames.tgz（内含 startup2/ 目录，来自 2026-10-25 05:09
重启 morphofly-test 时的 KWin ScreenShot2 连拍，修复前二进制）。
用法（需要 PIL）：
    tar xzf startup2-pre-fix-frames.tgz
    python3 recompute.py startup2
输出：一行 JSON（max_mean_diff / max_at_t / verdict）
"""
import json
import os
import sys

from PIL import Image, ImageChops

d = sys.argv[1] if len(sys.argv) > 1 else "startup2"
metas = [json.loads(l) for l in open(os.path.join(d, "meta.jsonl")) if l.strip()]
metas = [m for m in metas if "br" in m]

# 当时窗口：球窗口 x=2908 y=1516（148x148），展开时向左多 212
# 截图是逻辑像素 2048x1152，屏幕缩放 1.5，br 裁剪框 = (1820, 930)
BOX, SCALE, WIN_X, WIN_Y = (1820, 930), 1.5, 2908, 1516
region = (
    max(0, int((WIN_X - 212) / SCALE) - BOX[0]),
    max(0, int(WIN_Y / SCALE) - BOX[1]),
    int((WIN_X - 4) / SCALE) - BOX[0],
    int((WIN_Y + 148) / SCALE) - BOX[1],
)
late = Image.open(os.path.join(d, metas[-1]["br"])).convert("RGB").crop(region)
worst = (0.0, None, 0.0)
for m in metas:
    im = Image.open(os.path.join(d, m["br"])).convert("RGB").crop(region)
    diff = ImageChops.difference(im, late).convert("L")
    h = diff.histogram()
    tot = sum(h)
    mean = sum(i * c for i, c in enumerate(h)) / tot
    changed = sum(c for i, c in enumerate(h) if i > 12) / tot
    if mean > worst[0]:
        worst = (mean, m["t"], changed)
print(json.dumps({
    "frames": len(metas), "region": region, "baseline_t": metas[-1]["t"],
    "max_mean_diff": round(worst[0], 2), "max_at_t": worst[1],
    "max_changed_ratio": round(worst[2], 4),
    "verdict": "FAIL" if worst[0] >= 1.5 else "PASS",
}, ensure_ascii=False))
