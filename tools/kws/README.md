# 唤醒词验收工具（kws-harness）

一套可以直接跑的 sherpa-onnx KWS 验收脚本，配合 `../test-audio/` 的 15 个音频使用。

## 一次性准备

```bash
# 1. 装依赖（本机 python3 -m venv 是坏的，用 uv）
uv venv .venv && source .venv/bin/activate
uv pip install sherpa-onnx soundfile numpy

# 2. 下模型（8.4MB）
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2
tar xf sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2

# 3. 生成「小蝶」关键词文件
cd sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20
printf '小蝶 @小蝶\n' > keywords_raw.txt
sherpa-onnx-cli text2token --tokens tokens.txt --tokens-type ppinyin keywords_raw.txt keywords.txt
# 输出：x iǎo d ié @小蝶
cd ..
```

> `sherpa-onnx-cli text2token` 需要额外装 `click sentencepiece pypinyin`，否则会一个个报 ModuleNotFoundError。

## 跑验收

```bash
python3 kws_check.py ../test-audio
```

**预期输出**（2026-10-23 实测）：

```
✅ 01_wake_short.mp3             1.87s  [1.1s 小蝶]
✅ 02_wake_slow.mp3              1.51s  [1.1s 小蝶]
✅ 03_wake_fast_long.mp3         1.94s  [0.7s 小蝶]
⬜ 04_ack_zai.mp3                1.87s
⬜ 05_ack_zai_long.mp3           2.64s
✅ 06_cmd_progress.mp3           2.90s  [1.1s 小蝶]
✅ 07_cmd_fix_bug.mp3            3.17s  [1.1s 小蝶]
✅ 08_cmd_stop.mp3               2.04s  [1.1s 小蝶]
✅ 09_multiturn.mp3              6.17s  [1.1s 小蝶]
⬜ 10_qa_status.mp3              1.87s
⬜ 11_neg_chat.mp3               8.26s
⬜ 12_neg_near_miss.mp3          3.17s
✅ 13_fp_xiaodieer.mp3           1.87s  [1.1s 小蝶]   ← 已知误唤醒
⬜ 14_neg_hudie.mp3              2.93s
⬜ 15_neg_xiaode.mp3             2.40s

召回：7/7
误唤醒：1/8
```

## 可调参数（环境变量）

```bash
KWS_MODEL=/path/to/model        # 模型目录
KWS_KEYWORDS=keywords.txt       # 关键词文件
KWS_THRESHOLD=0.25              # 实测不敏感，保持默认
KWS_SCORE=1.0                   # 调高会破坏效果，保持默认
KWS_BLANKS=4                    # ★ 关键！默认 1 会误唤醒「小蝶儿」
```

**★ `KWS_BLANKS`（`num_trailing_blanks`）是最重要的参数**：

| 值 | 召回 | 误唤醒 |
|---|---|---|
| 1（sherpa 默认） | 8/8 | **1/8**（小蝶儿） |
| **4（本工具默认）** | **8/8** | **0/8** ✅ |

延迟零代价。安全区间 4–10，≥12 会漏检连续语音。
详见 `../唤醒词优化实测.md`。

## 附带的诊断脚本

| 脚本 | 作用 |
|---|---|
| `test_xiaodie.py` | 单文件快速测试（最简版，~20 行） |
| `realtime_sim.py` | 模拟实时喂音频，测 CPU 占用率 |
| `noise_test.py` | SNR 噪声鲁棒性扫描（阈值 × 信噪比矩阵） |
| `kw_variants.py` | 对比「小蝶」vs「小蝶小蝶」两种唤醒词 |
| `latency.py` | 对比 chunk-8 与 chunk-16 的唤醒延迟 |
| `mic_demo.py` | 真麦克风实时唤醒（需 `pip install sounddevice` + 声卡） |

## 关键词文件格式

`kw_a.txt` / `kw_b.txt` / `kw_c.txt` 是三种配置，直接复制成 `keywords.txt` 用：

| 文件 | 内容 |
|---|---|
| `kw_a.txt` | 只有 `小蝶`（推荐） |
| `kw_b.txt` | 只有 `小蝶小蝶`（更稳但要说两遍） |
| `kw_c.txt` | 两个都要 |

格式说明：`拼音 @显示名`，多个词用换行分隔；同一行放多个词用 `/` 分隔。

## 已知结论（实测）

- **`keywords_threshold` 调 0.05–0.50 对误唤醒完全无影响**（实测八档，纹丝不动）
- **`keywords_score` 调到 2 或 3 会让快语速「小蝶」漏检**，且 1.5 以上会重新引入误唤醒，保持 1.0
- **`num_trailing_blanks=4` 能彻底解决「小蝶儿」误唤醒**，且零延迟代价
- **chunk-8 比 chunk-16 快 0.2s**（0.9s vs 1.1s），但准确率略低
- **8kHz 音源升采样到 16k 后识别失败**，麦克风必须直采 16k
