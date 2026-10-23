# 蝶梦 / 小蝶 · 唤醒词检测 + 免费 TTS 测试音频调研

> 调研日期：2026-10-23
> 原则：**带「实测」字样的都是本机跑出来的数字**；查不到的写「未查证」；不编 API。

---

## 0. 三句话结论

1. **唤醒词**：桌面 / 手机 / Web 用 **sherpa-onnx KWS**（实测「小蝶」能识别，单核 3% CPU）；**ESP32 上它跑不了**，必须换 **乐鑫 esp-sr WakeNet**。所以是**双引擎**，不是一个引擎通吃。
2. **「小蝶」两个字太短**是最大隐患。实测「小蝶儿」会误唤醒。建议主唤醒词保持「小蝶」，但把「小蝶小蝶」做成可选更稳的模式。
3. **测试音频用 edge-tts**，本机已实测跑通（版本 7.3.1），323 个音色里 14 个中文，`zh-CN-XiaoxiaoNeural` 最适合助手人设。

---

## 任务 A：唤醒词检测

### A1. 七个方案横向对比

| 方案 | GitHub | Star | 许可 | 中文自定义词 | Go | Rust | ESP32 | 模型体积 | 维护 |
|---|---|---|---|---|---|---|---|---|---|
| **sherpa-onnx KWS** | [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) | 15,456 | Apache-2.0 | ✅ 改文本即可 | ✅ cgo | ✅ crate | ❌ | 8.4MB(int8) | 2026-10-23 |
| **esp-sr / WakeNet** | [espressif/esp-sr](https://github.com/espressif/esp-sr) | 1,549 | ESPRESSIF MIT* | ⚠️ 找乐鑫定制 | ❌ | ❌ | ✅ **只有它行** | ~几百 KB | 2026-10-13 |
| openWakeWord | [dscripka/openWakeWord](https://github.com/dscripka/openWakeWord) | 2,872 | Apache-2.0 | ⚠️ 要自己训练 | 需 FFI | 需 FFI | ⚠️ 理论可行 | 11MB | 2025-12-30 |
| Porcupine | [Picovoice/porcupine](https://github.com/Picovoice/porcupine) | 4,959 | Apache-2.0（仅代码） | ✅ 但**要钱** | ❌ 无官方 | ❌ 无官方 | ❌ 官方明确不做 | ~1MB | 2026-10-22 |
| snowboy | [Kitt-AI/snowboy](https://github.com/Kitt-AI/snowboy) | 3,361 | Apache-2.0 | ❌ **训练站已关** | ❌ | ❌ | ❌ | — | **2021-10 停更** |
| Vosk 小模型 | [alphacep/vosk-api](https://github.com/alphacep/vosk-api) | 15,197 | Apache-2.0 | ✅ 改词表 | ✅ | ✅ 0.3.1 | ❌ | 44MB | 2026-10-15 |
| whisper.cpp | [ggml-org/whisper.cpp](https://github.com/ggml-org/whisper.cpp) | 54,582 | MIT | ✅ 文本匹配 | ✅ | ✅ | ❌ | 75MB+ | 2026-10-19 |
| FunASR KWS | [modelscope/FunASR](https://github.com/modelscope/FunASR) | 20,798 | MIT | ⚠️ 有限 | ❌ | ❌ | ❌ | 未查证 | 2026-10-22 |
| microWakeWord | [OHF-Voice/micro-wake-word](https://github.com/OHF-Voice/micro-wake-word) | 953 | Apache-2.0 | ❌ 无中文 | ❌ | ❌ | ✅ TFLite Micro | ~50KB | 2026-07-06 |

\* esp-sr 的「ESPRESSIF MIT License」只在 ESP 系列芯片上免费，换芯片就不适用。

---

### A2. sherpa-onnx 实测（重点，这是推荐方案）

**为什么它赢**：改一个文本文件就能换唤醒词，不用训练、不用等、不用花钱。

#### 环境与版本（实测）

```bash
pip install sherpa-onnx        # 实测版本 1.13.8
```

#### 模型下载（实测可用）

```bash
# 中英双语版（推荐，8.4MB int8）
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2
tar xf sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2

# 纯中文版（备选，训练数据是 WenetSpeech 1 万小时）
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01.tar.bz2
```

文件构成（实测 `ls`）：

| 文件 | 大小 |
|---|---|
| encoder-...chunk-16-left-64.int8.onnx | 4.4 MB |
| decoder-...chunk-16-left-64.onnx | 742 KB |
| joiner-...chunk-16-left-64.int8.onnx | 85 KB |
| en.phone（音素表） | 3.2 MB |
| tokens.txt | 1.9 KB |
| **合计** | **约 8.4 MB** |

#### 把「小蝶」变成唤醒词（实测）

```bash
# 1. 写原始关键词（注意格式：文本 @标签）
printf '小蝶 @小蝶\n' > keywords_raw.txt

# 2. 转成拼音 token
sherpa-onnx-cli text2token \
  --tokens tokens.txt \
  --tokens-type ppinyin \
  keywords_raw.txt keywords.txt

# 3. 结果（实测输出）
# x iǎo d ié @小蝶
```

需要装 4 个依赖才跑得起来（我一个个踩出来的）：`click`、`sentencepiece`、`pypinyin`，加上 sherpa-onnx 本身。

#### 实测性能

| 指标 | 实测值 | 条件 |
|---|---|---|
| CPU 占用 | **单核约 3.1%** | 13.1s 音频处理耗时 0.40s，num_threads=1 |
| 内存 | 86 MB RSS | 含 Python + onnxruntime 整个进程 |
| 唤醒延迟 | **1.1s 触发** | chunk-16；音频开头有 0.1s 静音，「小蝶」说完约 0.55s |
| 低延迟模式 | **0.9s 触发** | chunk-8，延迟 160ms 而非 320ms |
| 召回 | **7/7** | 7 条应唤醒的音频全部命中 |
| 误唤醒 | **1/8** | 只有「小蝶儿真可爱」被误触发 |
| 长语音误报 | **0** | 47.6s 连续说话无唤醒词 |
| 噪声鲁棒性 | 6/6 命中 | SNR 5dB 和 15dB 下都全中 |
| 音量鲁棒性 | 7/7 命中 | 增益降到 3% 仍能识别 |

**阈值不是越调越好**：我试了 `keywords_threshold` 从 0.15 到 0.5，结果几乎一样；反而把 `keywords_score` 调到 2 或 3 时，快语速的「小蝶」会漏掉。**建议 `keywords_score=1.0, keywords_threshold=0.25`**（官方默认值就是最优）。

#### 已知短板（实测发现）

- **「小蝶儿」会误唤醒**，这是两个字唤醒词的通病，调阈值解决不了。
- **超快语速的单独「小蝶」会漏**（0.7s 内说完）。但同样的快语速放在长句里（「小蝶，帮我看看项目进度」+50% 语速）又能识别——说明模型依赖上下文，孤立短词最难。
- **8kHz 音源不行**：我用 8kHz 的 aishell3 TTS 生成音频升采样到 16k 后，识别失败。麦克风录音走 16k 就没问题，但如果要做电话场景要注意。

#### 各语言绑定（官方仓库里都有，已核实目录存在）

| 平台 | 路径 | 说明 |
|---|---|---|
| Go | `go-api-examples/keyword-spotting-from-file` | 依赖 `github.com/k2-fsa/sherpa-onnx-go` v1.13.8，**需要 cgo** |
| Rust | `rust-api-examples/run-keyword-spotter.sh` | crates.io `sherpa-onnx` 1.13.8 |
| C | `c-api-examples/kws-c-api.c` | 最底层，任何语言都能 FFI |
| Android | `android/SherpaOnnxKws` | 完整 App 示例 |
| iOS | `swift-api-examples/keyword-spotting-from-file.swift` | Swift |
| Flutter | `flutter/sherpa_onnx/lib/src/keyword_spotter.dart` | 有 KWS 类 |
| Web | `wasm/kws/` | WebAssembly |
| C# | `dotnet-examples/keyword-spotting-from-microphone` | 带麦克风版本 |
| Java | `java-api-examples/KeywordSpotterFromFile.java` | |

Go 的坑：**必须开 cgo**，而且 Windows 要把 `*.dll` 从 [sherpa-onnx-go-windows](https://github.com/k2-fsa/sherpa-onnx-go-windows) 拷到可执行文件旁边。

#### ESP32：**不支持，这条要纠正现有文档**

我查了官方 README 的平台表，只有 x64 / x86 / arm64 / arm32 / riscv64 × Android / iOS / Windows / macOS / Linux / HarmonyOS。**没有 ESP32，也没有任何 MCU / Cortex-M**。全仓库搜 `esp32` 只搜到 1 个和 TTS 示例无关的 issue。ONNX Runtime 本身在 ESP32 的 512KB SRAM 上就跑不动。

> ⚠️ 现有 `技术选型.md` 里写的「sherpa-onnx 是唯一同时支持中文自定义词 + Go/Rust/ESP32」——**ESP32 那一半是错的**，需要改。

---

### A3. 其他方案的关键结论

**Porcupine：好技术，但商业路子已经堵死**

- 支持中文（`lib/common/porcupine_params_zh.pv`），但**内置中文唤醒词只有 4 个**：你好、咖啡、水饺、豪猪。
- 自定义唤醒词要在 Console 训练 `.ppn`，**按平台分别训练**（给树莓派训的不能用在 Android）。
- **免费额度已经取消**：2026-06-30 停止 Free Tier，旧 AccessKey 全部失效。官方 FAQ 原话：「目前没有面向个人或非商业用途的免费或付费方案」，公司定位是 B2B。
- 就算买了，**每次启动都要联网校验 AccessKey**，纯离线部署做不到。
- **没有官方 Go / Rust SDK**（官方只有 Android/.NET/Flutter/iOS/Java/Node/Python/React Native/React/Web + C）。社区 Go 封装 `charithe/porcupine-go` 只有 16 star，最后一次提交 **2018 年**。Rust 的 `pv_porcupine` crate 已 yanked。
- **ESP32 官方明确不做**：issue #321/#106 里团队回复「需要有大客户或乐鑫出钱才考虑移植」，并提到「ESP32 只有 512KB RAM」。

**openWakeWord：预训练模型只有英文**

- README 原话：「Currently, only English models are supported」。
- 能训中文，但要用 TTS 合成几万条样本 + 4GB 负样本，Colab 跑 1 小时起步，而且**冻结的 backbone 是英文语音嵌入**，中文效果没人保证（有商业公司文章说架构层面就做不好中文，但那是卖替代品的营销文，**标注：未独立验证**）。
- 模型确实小（melspectrogram 1MB + embedding 1.3MB + 单个唤醒词 0.5–1.7MB），ONNX 格式任何语言都能调。
- 但既然有 sherpa-onnx 这种「改文本就生效」的，没必要去训练。

**snowboy：彻底死了**

- README 第一行就是 KITT.AI 的告别信：「我们计划在 2020 年 12 月 31 日前关停所有产品」。
- 最后提交 2021-10-13。
- **致命问题**：自定义唤醒词要上传到 snowboy.kitt.ai 训练，**网站已经下线**，所以除了官方自带的几个模型，**无法生成任何新唤醒词**。直接淘汰。
- 现存 fork（`tue-robotics/snowboy_ros` 6 star、`mideind/FlutterSnowboy` 17 star）都是 2023–2024 年的小项目，没人在维护核心。

**Vosk：能做，但太重**

- 中文小模型 `vosk-model-small-cn-0.22.zip` **43.9MB**（实测 Content-Length）。
- 原理是「ASR + 语法约束」，不是专门的 KWS，**常驻后台功耗高**。
- 有 Go 绑定和 Rust crate（`vosk` 0.3.1，2024-10 后没更新）。

**whisper.cpp 持续识别：方向就不对**

- 54,582 star 很唬人，但 whisper tiny 也有 75MB，**常驻跑持续识别 = 笔记本风扇狂转**。
- 它是「听完整句再转文字」，不是「听到两个字就醒」。

**FunASR KWS：能跑但生态偏 Python**

- 有 FSMN KWS，模型 `iic/speech_charctc_kws_phone-xiaoyun`。
- 官方文档明确：**流式接口是「句尾返回结果」，不是逐包唤醒事件**，做唤醒词不合适。
- 部署依赖 PyTorch/ONNX，桌面端还行，手机和嵌入式没戏。

---

### A4. 重点回答：ESP32 + 桌面端，最佳选择是什么？

**答案：没有单一方案，必须双引擎。**

| 端 | 引擎 | 理由 |
|---|---|---|
| Windows / macOS / Linux / Web / Android / iOS | **sherpa-onnx KWS** | 改文本即换词、Go/Rust/C 全有绑定、Apache-2.0、实测 3% CPU |
| ESP32 | **esp-sr WakeNet** | sherpa-onnx 跑不了，Porcupine 不做，只剩它 |

两边**共用一个唤醒词定义**（一个 JSON 里写 `"小蝶"` 和对应的拼音），各自加载各自的模型，上层接口统一成「检测到唤醒 → 回调」。

#### ESP32 这条路怎么走

esp-sr 内置 **94 个唤醒词模型**，其中中文 **35 个**（去重后）：小爱同学、你好小智、嗨小欧、你好小安、Hi小巫、Hi小星、小峰小峰、小珈小珈、小箭小箭、小康同学、小鹿小鹿、小鸭小鸭、小特小特、小滨小滨、小酥肉、璃奈板…… **没有「小蝶」**。

> 清单来自官方 [wakeword_list.md](https://github.com/espressif/esp-sr/blob/master/wakeword_list.md)，用 `grep -P "[\x{4e00}-\x{9fff}]"` 统计得到。

三条路，按代价排序：

1. **先用现成的近似词验证产品**（零成本）
   比如 `wn9_hixiaoxing_tts`（Hi小星）或 `wn9_haixiaowu_tts`（Hi小巫），读音结构接近「小蝶」，先把端到端链路跑通。

2. **去官方 issue 申请免费训练**（免费，但要等）
   [espressif/esp-sr#88](https://github.com/espressif/esp-sr/issues/88) 是官方收集唤醒词需求的 issue（551 条评论，还开着）。官方说 TTS 训练的模型能达到人工录制样本的 95–98% 准确率，且**模型和 esp-sr 同许可，商用免费**。门槛：要有进行中的项目链接，或者被 5 个人点赞。

3. **付费定制**（要花钱，需你拍板）
   官方文档：只收**一次性定制费**，不按设备数量或时长收 License 费。三种方式：纯 TTS 样本 / 乐鑫众包录制（500 人）/ 自带语料（至少 2 万条）。价格要发邮件问 `sales@espressif.com`。**具体多少钱我查不到，标「未查证」，这个得你去谈。**

> 技术前提：WakeNet9 支持 ESP32 / ESP32S3 / ESP32P4；WakeNet9s 支持 ESP32C3 / C5 / C6（无 PSRAM、无 SIMD 的芯片）；WakeNet10 从 esp-sr v2.6.0 起支持中文、英、日、法、德、西、葡、俄。

#### 桌面端集成路径（可以直接抄）

```bash
# 1. 拿模型
wget https://github.com/k2-fsa/sherpa-onnx/releases/download/kws-models/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2
tar xf sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20.tar.bz2

# 2. 生成「小蝶」的 token
cd sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20
printf '小蝶 @小蝶\n' > keywords_raw.txt
sherpa-onnx-cli text2token --tokens tokens.txt --tokens-type ppinyin keywords_raw.txt keywords.txt

# 3. Go 侧（需 cgo）
go get github.com/k2-fsa/sherpa-onnx-go@v1.13.8
```

Go 核心代码就 5 行（来自官方 `go-api-examples/keyword-spotting-from-file/main.go`）：

```go
config := sherpa.KeywordSpotterConfig{}
config.ModelConfig.Transducer.Encoder = "encoder-...int8.onnx"
config.ModelConfig.Transducer.Decoder = "decoder-....onnx"
config.ModelConfig.Transducer.Joiner  = "joiner-...int8.onnx"
config.ModelConfig.Tokens = "tokens.txt"
config.KeywordsFile = "keywords.txt"
spotter := sherpa.NewKeywordSpotter(&config)
```

麦克风循环（我写了个可跑的版本）：

```go
stream := sherpa.NewKeywordStream(spotter)
for {
    samples := readMicChunk(1600)          // 100ms @ 16kHz
    stream.AcceptWaveform(16000, samples)
    for spotter.IsReady(stream) {
        spotter.Decode(stream)
        if r := spotter.GetResult(stream); r.Keyword != "" {
            spotter.Reset(stream)          // ⚠️ 检测到之后必须 Reset
            onWake(r.Keyword)              // → 播「在」，进通话模式
        }
    }
}
```

> `Reset` 那行别漏。不 Reset 会一直重复触发同一个唤醒。

---

### A5. 「小蝶」两个字太短怎么办

实测数据摆在这：

| 唤醒词 | 应唤醒 | 误唤醒 | 说明 |
|---|---|---|---|
| `小蝶` | 7/7 | **1/8**（小蝶儿） | 召回满分，但有误触 |
| `小蝶小蝶` | 0/7（测的是单次「小蝶」，本就该不触发） | 0/8 | 只连说两次才触发，明显更稳 |

**建议**：
- 主唤醒词保持 **「小蝶」**（用户明确要求）
- 代码里留个配置项，可以切成 **「小蝶小蝶」**
- 阈值做成用户可调滑块（`keywords_threshold` 0.15–0.5 都试过，实测对误唤醒影响不大，别指望靠它救）
- **必须有兜底**：双击小球 / 快捷键，误触烦人时能关掉语音唤醒

---

## 任务 B：免费 TTS 生成测试音频

### B1. 推荐：edge-tts（已在本机实测跑通）

**理由**：免费、音色最自然、一行命令出结果、中文音色多。缺点是**非官方接口**，只适合做测试音频，别塞进产品。

#### 本机实测状态（Debian，2026-10-23）

```bash
# 这台机器原来没装，我用 uv 装好了
$ edge-tts --version
edge-tts 7.3.1
```

⚠️ 注意：`python3 -m venv` 在这台机器上**是坏的**（缺 python3-venv 包）。用 `uv venv` 才行：

```bash
uv venv .venv && source .venv/bin/activate
uv pip install edge-tts
```

或者一步到位装成全局命令：

```bash
uv tool install edge-tts
```

#### 能直接复制的命令

```bash
# 最基本：生成「小蝶，帮我看看项目进度」
edge-tts --voice zh-CN-XiaoxiaoNeural \
         --text "小蝶，帮我看看项目进度" \
         --write-media test.mp3

# 带字幕（做对齐测试用）
edge-tts --voice zh-CN-XiaoxiaoNeural \
         --text "小蝶，帮我看看项目进度" \
         --write-media test.mp3 \
         --write-subtitles test.srt

# 调语速（做快/慢语速鲁棒性测试）
edge-tts --voice zh-CN-XiaoxiaoNeural --rate=+50% --text "小蝶，帮我看看项目进度" --write-media fast.mp3
edge-tts --voice zh-CN-XiaoxiaoNeural --rate=-15% --text "小蝶" --write-media slow.mp3

# 转成 16kHz 单声道 wav（喂 KWS 必须这格式）
ffmpeg -i test.mp3 -ar 16000 -ac 1 -c:a pcm_s16le test.wav

# 末尾补 0.5s 静音（防 VAD 卡住）
ffmpeg -i test.mp3 -af "apad=pad_dur=0.5" -ar 16000 -ac 1 test_padded.wav
```

> 本机没装 ffmpeg。我是用 `pip install imageio-ffmpeg` 拿到的静态二进制（`ffmpeg-linux-x86_64-v7.0.2`，实测可用），或者直接 `apt install ffmpeg`。

#### 中文音色清单（实测 `--list-voices` 输出）

全库 323 个音色，中文 14 个。**zh-CN 只有 8 个**：

| 音色 ID | 性别 | 官方风格标签 | 推荐度 |
|---|---|---|---|
| **zh-CN-XiaoxiaoNeural** | 女 | News, Novel / Warm | ⭐⭐⭐⭐⭐ **助手人设首选** |
| zh-CN-XiaoyiNeural | 女 | Cartoon, Novel / Lively | ⭐⭐⭐⭐ 更年轻活泼 |
| zh-CN-YunxiNeural | 男 | Novel / Lively, Sunshine | ⭐⭐⭐ 男声助手 |
| zh-CN-YunjianNeural | 男 | Sports, Novel / Passion | ⭐⭐ 太有激情，不像助手 |
| zh-CN-YunxiaNeural | 男 | Cartoon / Cute | ⭐⭐ 童声感 |
| zh-CN-YunyangNeural | 男 | News / Professional | ⭐⭐ 播音腔，不适合对话 |
| zh-CN-liaoning-XiaobeiNeural | 女 | Dialect / Humorous | ⭐ 东北话，测方言鲁棒性可用 |
| zh-CN-shaanxi-XiaoniNeural | 女 | Dialect / Bright | ⭐ 陕西话 |

另外还有 zh-HK（粤语，HiuGaai / HiuMaan / WanLung）和 zh-TW（台湾腔，HsiaoChen / HsiaoYu / YunJhe）各 3 个。

**建议**：主力用 `XiaoxiaoNeural`，需要区分说话人时加 `XiaoyiNeural`（女）和 `YunxiNeural`（男）。

#### edge-tts 的坑（都实测过）

| 坑 | 实测情况 | 应对 |
|---|---|---|
| **只出 mp3** | `--write-media out.wav` 出来的还是 MP3（`MPEG ADTS, layer III, 24kHz`） | 必须过一遍 ffmpeg |
| **尾部自动补静音** | 「小蝶」总长 1.87s，有声部分只 0.47s，**尾部 1.4s 静音** | 反而对 VAD 友好，不用手动补 |
| **`--rate` 有时不生效** | 同样 `+30%` 参数，6 次里 2 次输出还是默认时长 | 批量生成后要**检查时长**，异常的重跑 |
| **重跑不是逐字节相同** | 同文本同音色两次 md5 不同（`3287fa4a…` vs `10f32cb8…`），时长一致 | mp3 要进 git，别现场重生成做基线 |
| **403 风险** | 2026-10 微软改鉴权，issue #490 一堆 403；7.3.1 已修复 | 固定版本 `edge-tts==7.3.1`，别乱升级 |
| **无官方额度** | 没有公开限额，但也不是给你量产的 | 只用于测试，产品里换真 TTS |
| **需要联网** | 每次合成都要连微软服务器 | 离线场景换 Piper / sherpa-onnx TTS |

> 许可提醒：rany2/edge-tts 是 **LGPLv3**（只有 `srt_composer.py` 是 MIT）。只当命令行工具用没问题，链接进产品要注意。

#### Go / Rust 封装

| 语言 | 包 | 状态 |
|---|---|---|
| Go | [difyz9/edge-tts-go](https://github.com/difyz9/edge-tts-go) | 12 star，LGPL-3.0，最后更新 2025-09 |
| Go | [bytectlgo/edge-tts](https://pkg.go.dev/github.com/bytectlgo/edge-tts) | 有 CLI + 库两种用法 |
| Rust | [msedge-tts](https://crates.io/crates/msedge-tts) 0.4.1 | 31,501 次下载，2026-10-14 更新 ✅ |
| Rust | [edge-tts-rust](https://crates.io/crates/edge-tts-rust) 0.1.7 | 8,192 次下载，2026-10-16 更新 |

**但建议别用封装**——edge-tts 是测试工具，直接 `subprocess` 调命令行最省事，省得跟着第三方库的 bug 跑。

---

### B2. 其他免费 TTS 对比

| 方案 | 中文 | 质量 | 体积/成本 | 许可 | 结论 |
|---|---|---|---|---|---|
| **edge-tts** | ✅ 8 个 zh-CN | ⭐⭐⭐⭐⭐ 最自然 | 免费，需联网 | LGPLv3 | **测试首选** |
| **Piper** | ⚠️ 3 个音色，1 个能用 | ⭐⭐ 一般 | 60MB/音色，纯本地 | **GPL-3.0** | 离线备选 |
| **Kokoro** | ✅ zf_001–zf_099 等 | ⭐⭐⭐⭐ 好 | 147MB(int8) / 327MB(pth) | Apache-2.0 | 质量好但重 |
| sherpa-onnx VITS | ✅ aishell3 174 说话人 | ⭐⭐ 8kHz 很闷 | 329MB | Apache-2.0 | 不推荐做测试音频 |
| 腾讯云 TTS | ✅ | ⭐⭐⭐⭐ | 免费额度 800 万字符 | 商业 | 要实名，未实测 |
| 阿里云百炼 qwen3-tts | ✅ | ⭐⭐⭐⭐⭐ | 0.1 元/千字符，90 天 100 万字符免费 | 商业 | 要实名，未实测 |
| GPT-SoVITS | ✅ | ⭐⭐⭐⭐⭐ 可克隆 | 需 GPU | MIT | 杀鸡用牛刀 |
| CosyVoice | ✅ | ⭐⭐⭐⭐⭐ | 需 GPU | Apache-2.0 | 同上 |

#### Piper 实测细节（踩坑记录）

```bash
# 安装（实测 1.8.0）
uv pip install piper-tts

# 下载中文模型（3 个音色，实测都能下）
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/zh/zh_CN/huayan/medium/zh_CN-huayan-medium.onnx
curl -LO https://huggingface.co/rhasspy/piper-voices/resolve/main/zh/zh_CN/huayan/medium/zh_CN-huayan-medium.onnx.json

# 生成（⚠️ 输入走 stdin，不是 --text）
echo "小蝶，帮我看看项目进度" | piper -m zh_CN-huayan-medium.onnx -f out.wav
```

| 音色 | 状态 | 原因 |
|---|---|---|
| **huayan** | ✅ 可用 | espeak `cmn` 音素，输出 22050Hz，2.53s 音频合成耗时 4s |
| chaowen | ❌ 报错 | `phoneme_type: pinyin`，缺 `unicode_rbnf` 模块 |
| xiao_ya | ❌ 未测 | 需要 `g2pW`，且训练数据是 **非商用许可**（BZNSYP） |

**Piper 的两个大坑**：
1. `rhasspy/piper` **已归档**（archived），新家在 [OHF-Voice/piper1-gpl](https://github.com/OHF-Voice/piper1-gpl)，**许可证从 MIT 变成 GPL-3.0**。如果产品要闭源，不能用。
2. 中文质量一般——huayan 是从英文 lessac 音色 finetune 出来的。

#### Kokoro 实测细节

- 官方模型 [hexgrad/Kokoro-82M-v1.1-zh](https://huggingface.co/hexgrad/Kokoro-82M-v1.1-zh)：`kokoro-v1_1-zh.pth` **327MB**，Apache-2.0
- sherpa-onnx 转换版：`kokoro-int8-multi-lang-v1_1.tar.bz2` **147MB**
- 中文音色：女声 `zf_001`–`zf_099`，男声 `zm_009`–`zm_100`，**共 191 个说话人**
- 中英混读是强项（「用 Timeline Studio 做 AI video editing」这种句子不会破音）
- **结论**：质量比 Piper 好，但 147MB 对「生成几个测试音频」来说太重了，除非你本来就要做本地 TTS

#### 云厂商免费额度

| 厂商 | 免费额度 | 来源可信度 |
|---|---|---|
| 腾讯云 | 基础/精品音色 **800 万字符**；大模型音色 10 万字符；超自然大模型 2 万字符 | 腾讯云文档（页面 JS 渲染没抓到正文，来自搜索摘要，**未直接核实**） |
| 阿里云百炼 | qwen3-tts-flash 约 0.1 元/千字符，开通 90 天内 100 万字符 | 第三方 GitHub 仓库整理，**未官方核实** |

> 云厂商都要实名认证 + 开通服务，而且额度政策随时变。**做测试音频没必要折腾**，edge-tts 一行命令就够。

---

### B3. 已经生成好的测试音频

放在 `Project/蝶梦智能/test-audio/`，15 个文件，全部 `zh-CN-XiaoxiaoNeural`（11 号是 YunxiNeural）：

| 文件 | 内容 | 时长 | 用途 |
|---|---|---|---|
| 01_wake_short.mp3 | 小蝶 | 1.87s | 标准唤醒 |
| 02_wake_slow.mp3 | 小蝶（-15% 语速） | 1.51s | 慢语速唤醒 |
| 03_wake_fast_long.mp3 | 小蝶，帮我看看项目进度（+50%） | 1.94s | 快语速唤醒 |
| 04_ack_zai.mp3 | 在 | 1.87s | 应答词（单字） |
| 05_ack_zai_long.mp3 | 在，我在听，你说吧 | 2.64s | 应答词（长） |
| 06_cmd_progress.mp3 | 小蝶，帮我看看项目进度 | 2.90s | 完整指令 |
| 07_cmd_fix_bug.mp3 | 小蝶，把登录页的报错修一下 | 3.17s | 长指令 |
| 08_cmd_stop.mp3 | 小蝶，停一下 | 2.04s | 打断测试 |
| 09_multiturn.mp3 | 小蝶，帮我看看项目进度。还有，把昨天的日志也整理一下。 | 6.17s | 多轮/多意图 |
| 10_qa_status.mp3 | 还有一个任务吗 | 1.87s | 无唤醒词的追问 |
| 11_neg_chat.mp3 | 今天下午我们开会讨论了项目的进度和预算…… | 8.26s | 负样本（不该唤醒） |
| 12_neg_near_miss.mp3 | 小弟，帮我把这个箱子搬过去 | 3.17s | 近音词负样本 |
| 13_fp_xiaodieer.mp3 | 小蝶儿真可爱 | 1.87s | **已知会误唤醒** |
| 14_neg_hudie.mp3 | 小小的蝴蝶停在花朵上 | 2.93s | 含「蝶」字负样本 |
| 15_neg_xiaode.mp3 | 晓得这个事情怎么办吗 | 2.40s | 近音词负样本 |

**用 KWS 跑这套音频的实测结果**：

```
01_wake_short.mp3          ✅ [1.1s]
02_wake_slow.mp3           ✅ [1.1s]
03_wake_fast_long.mp3      ✅ [0.7s]
04_ack_zai.mp3             ⬜ 无唤醒（正确）
05_ack_zai_long.mp3        ⬜ 无唤醒（正确）
06_cmd_progress.mp3        ✅ [1.1s]
07_cmd_fix_bug.mp3         ✅ [1.1s]
08_cmd_stop.mp3            ✅ [1.1s]
09_multiturn.mp3           ✅ [1.1s]
10_qa_status.mp3           ⬜ 无唤醒（正确）
11_neg_chat.mp3            ⬜ 无唤醒（正确）
12_neg_near_miss.mp3       ⬜ 无唤醒（正确）
13_fp_xiaodieer.mp3        ❌ 误唤醒 [1.1s]
14_neg_hudie.mp3           ⬜ 无唤醒（正确）
15_neg_xiaode.mp3          ⬜ 无唤醒（正确）
```

召回 **7/7**，误唤醒 **1/8**。这 1 个误唤醒就是「小蝶」两个字的代价。

> 一条命令重跑这套验收：`python3 kws-harness/kws_check.py test-audio`
> 详见 [kws-harness/README.md](kws-harness/README.md)。

---

## 5. 需要你拍板 / 未查证的事

| 事项 | 说明 |
|---|---|
| **ESP32 唤醒词走哪条路** | ①先用近似词验证（免费）②issue #88 申请（免费但看运气）③付费定制（**价格未知，要发邮件谈，要你决定**） |
| **Porcupine 值不值得买** | 官方已转 B2B、无公开报价。如果只是个人项目，**建议直接放弃** |
| **云 TTS 免费额度** | 腾讯/阿里的具体数字来自搜索摘要，**没在官方页面直接核实到**，真要用人肉确认 |
| **esp-sr 定制价格** | 官方只说「一次性定制费，不按量收 License」，**具体金额未查证** |
| **sherpa-onnx 在 ESP32 上的替代方案** | 除了 esp-sr，我**没找到**第二个能在 ESP32 跑中文自定义唤醒词的方案 |

---

## 6. 附：与现有文档的差异

`技术选型.md` 原先写的是「sherpa-onnx 是唯一同时支持中文自定义词 + Go/Rust/ESP32」，ESP32 那格打了 ✅。
**那个 ✅ 是错的**：实测查证 sherpa-onnx 不支持 ESP32（官方平台表里没有 MCU，全仓库搜 `esp32` 只搜到一个无关 issue）。

`技术选型.md` 已在 2026-10-23 18:47 改成双引擎表述（sherpa-onnx 管桌面/移动，esp-sr 管 ESP32），与本报告一致。
模型体积也从 ~15MB 修正为实测 **8.4MB**（int8）。

如果其他文档（`开发方案.md`、`调研记录.md`、`验收标准.md`）里还有「sherpa-onnx 上 ESP32」的说法，按同样口径改。
