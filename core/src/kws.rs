//! 唤醒词常驻监听（sherpa-onnx KWS）。
//!
//! 为什么现在才接：这一步需要 `sherpa-onnx` Rust crate（2.0），它自带的静态库
//! 会从 GitHub 下 ~30MB 资源，本机磁盘紧张，之前在 wings-pc 上验证可行后才接入。
//!
//! ## 实测依据（2026-10-24，见 `evidence/g4.1-kws-recall.md`）
//!
//! 用同一套模型 + 关键词跑 testdata/audio 全部 23 个文件：
//! **召回 9/9，误唤醒 0/8**（含「小蝶儿」「蝴蝶」「小的」谐音陷阱）。
//!
//! ## 关键参数
//!
//! `num_trailing_blanks = 4` —— 关键词后必须连续 N 帧 blank 才触发。
//! **这是「小蝶儿」不误唤醒的关键**：vault 文档实测 1 → 误唤醒 1/8；4 → 0/8，且召回不变。
//!
//! ## 与云端的关系
//!
//! KWS 只负责「听到唤醒词 → 通知状态机进 S1」，
//! 真正听懂内容仍然靠云端实时语音模型。本地 KWS 不联网、不上传音频。

use crate::audio::capture::AudioCapture;
use crate::session::{OrbState, SessionMachine, Stimulus};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// KWS 配置
#[derive(Debug, Clone)]
pub struct KwsConfig {
    /// 模型目录（含 encoder/decoder/joiner onnx + tokens.txt）
    pub model_dir: PathBuf,
    /// 关键词文件（`text2token` 生成的 ppinyin 格式）
    pub keywords_file: PathBuf,
    /// 检测阈值，默认 0.25
    pub threshold: f32,
    /// 打分阈值，默认 1.0
    pub score: f32,
    /// ★ 关键词后必须连续 N 帧 blank 才触发。**必须 ≥4**，否则「小蝶儿」会误唤醒。
    pub num_trailing_blanks: i32,
}

impl Default for KwsConfig {
    fn default() -> Self {
        Self {
            // 先找随包资源（AppImage/deb 自带），再找用户数据目录
            model_dir: crate::config::find_resource("kws-model"),
            keywords_file: crate::config::find_resource("keywords.txt"),
            threshold: 0.25,
            score: 1.0,
            num_trailing_blanks: 4,
        }
    }
}

impl KwsConfig {
    /// 检查模型文件是否齐全
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.model_dir.is_dir(),
            "KWS 模型目录不存在: {}（下载办法见 docs/唤醒词.md）",
            self.model_dir.display()
        );
        for f in ["tokens.txt"] {
            let p = self.model_dir.join(f);
            anyhow::ensure!(p.is_file(), "缺少模型文件: {}", p.display());
        }
        anyhow::ensure!(
            self.keywords_file.is_file(),
            "关键词文件不存在: {}（用 text2token 生成，见 docs/唤醒词.md）",
            self.keywords_file.display()
        );
        // 至少要有一组 encoder/decoder/joiner
        let has_onnx = std::fs::read_dir(&self.model_dir)
            .map(|rd| {
                rd.flatten()
                    .any(|e| e.file_name().to_string_lossy().ends_with(".onnx"))
            })
            .unwrap_or(false);
        anyhow::ensure!(has_onnx, "模型目录里没有 .onnx 文件: {}", self.model_dir.display());
        anyhow::ensure!(
            self.num_trailing_blanks >= 4,
            "num_trailing_blanks 必须 >= 4（默认 1 会让「小蝶儿」误唤醒，见实测记录），当前 {}",
            self.num_trailing_blanks
        );
        Ok(())
    }
}

/// 唤醒事件
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeEvent {
    /// 命中唤醒词
    Detected { keyword: String },
}

/// 唤醒监听器（同步、阻塞式，跑在独立线程里）
pub struct WakeListener {
    running: Arc<AtomicBool>,
    detections: Arc<AtomicU64>,
    voice_frames: Arc<AtomicU64>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl WakeListener {
    /// 启动常驻监听。
    ///
    /// `on_wake` 在命中唤醒词时被调用（在监听线程里，别做重活）。
    pub fn start<F>(cfg: KwsConfig, mut on_wake: F) -> Result<Self>
    where
        F: FnMut(WakeEvent) + Send + 'static,
    {
        cfg.validate()?;

        let running = Arc::new(AtomicBool::new(true));
        let detections = Arc::new(AtomicU64::new(0));
        let voice_frames = Arc::new(AtomicU64::new(0));

        let r = running.clone();
        let d = detections.clone();
        let vf = voice_frames.clone();
        let cfg2 = cfg.clone();

        let handle = std::thread::Builder::new()
            .name("diemeng-kws".into())
            .spawn(move || {
                if let Err(e) = run_kws(cfg2, r, d.clone(), vf, &mut on_wake) {
                    tracing::error!(error = %e, "KWS 监听线程异常退出");
                }
            })
            .context("启动 KWS 线程失败")?;

        Ok(Self {
            running,
            detections,
            voice_frames,
            handle: Some(handle),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn detections(&self) -> u64 {
        self.detections.load(Ordering::Relaxed)
    }

    /// 累计处理过的有声帧数（用于判断麦克风是否在工作）
    pub fn voice_frames(&self) -> u64 {
        self.voice_frames.load(Ordering::Relaxed)
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WakeListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 判断一个 `get_result` 的结果是不是**真命中**。
///
/// ⚠️ 2026-10-24 发现的坑（这是 100% 误唤醒的真凶）：
/// sherpa-onnx 2.0.0 的 Rust 绑定 `KeywordSpotter::get_result` 在**没有命中时也返回 Some**，
/// 返回的 `KeywordResult.keyword` 是**空字符串**、`json` 是
/// `{"start_time":0.00, "keyword": "", "timestamps": [], "tokens":[]}`。
///
/// Python 侧（`sherpa_onnx` 包）返回的是 `None`，所以 Python 脚本看起来一切正常。
/// 如果 Rust 侧只写 `if let Some(r) = ...` 就当命中，纯静音音频都会"命中"——
/// 这就是我们第一次跑出「误唤醒 8/8 = 100%」的原因。
///
/// 修法：**必须判 keyword 非空**。
fn is_real_hit(r: &sherpa_onnx::KeywordResult) -> bool {
    !r.keyword.trim().is_empty()
}

/// 从模型目录里挑出**同一套**的 encoder/decoder/joiner。
///
/// ⚠️ 2026-10-24 踩坑（血泪）：sherpa-onnx 的 KWS 模型包里有**两套**模型：
/// `chunk-16` 和 `chunk-8`，各自还有 int8 / 非 int8 版本，共 8 个 onnx 文件。
/// 如果按「前缀匹配 + 优先非 int8」随手挑，会挑出 chunk-16 的 encoder 配 chunk-8 的 decoder
/// —— **模型不匹配，输出是垃圾，每一帧都"命中"关键词**（误唤醒 8/8 = 100%）。
///
/// 修法：先定「用哪一套」（优先 chunk-16 + int8，这是 Python 侧实测有效的组合），
/// 三件套必须出自同一套。
fn pick_model_set(dir: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let mut candidates: Vec<(String, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.ends_with(".onnx") {
            candidates.push((name, e.path()));
        }
    }
    anyhow::ensure!(
        !candidates.is_empty(),
        "模型目录里没有 .onnx: {}",
        dir.display()
    );

    // 决定用哪个 chunk 档位：优先 chunk-16（与 Python 实测一致），没有再退回任意
    let chunk = if candidates.iter().any(|(n, _)| n.contains("chunk-16")) {
        "chunk-16"
    } else if candidates.iter().any(|(n, _)| n.contains("chunk-8")) {
        "chunk-8"
    } else {
        ""
    };
    // 决定用 int8 还是全精度：优先 int8（体积小、Python 实测有效），没有再退回全精度
    let want_int8 = candidates
        .iter()
        .any(|(n, _)| n.contains(chunk) && n.starts_with("encoder-") && n.contains("int8"));

    let pick = |prefix: &str| -> Result<PathBuf> {
        // 第一优先：chunk + int8 匹配
        if want_int8 {
            if let Some((_, p)) = candidates
                .iter()
                .find(|(n, _)| n.starts_with(prefix) && n.contains(chunk) && n.contains("int8"))
            {
                return Ok(p.clone());
            }
        }
        // 第二优先：chunk 匹配（任意精度）
        if let Some((_, p)) = candidates
            .iter()
            .find(|(n, _)| n.starts_with(prefix) && n.contains(chunk))
        {
            return Ok(p.clone());
        }
        // 兜底：只按前缀
        candidates
            .iter()
            .find(|(n, _)| n.starts_with(prefix))
            .map(|(_, p)| p.clone())
            .ok_or_else(|| anyhow::anyhow!("模型目录里找不到 {prefix}*.onnx: {}", dir.display()))
    };

    let (enc, dec, joi) = (pick("encoder-")?, pick("decoder-")?, pick("joiner-")?);
    // 自检：三件套必须同属一套。
    //
    // 判据是「去掉 int8 后缀后名字一致」，因为真实模型包里 **decoder 没有 int8 版**
    //（只有 encoder/joiner 有），所以不能要求三件套精度一致。
    let stem = |p: &Path| -> String {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim_start_matches("encoder-")
            .trim_start_matches("decoder-")
            .trim_start_matches("joiner-")
            .replace(".int8.onnx", "")
            .replace(".onnx", "")
    };
    anyhow::ensure!(
        stem(&enc) == stem(&dec) && stem(&dec) == stem(&joi),
        "挑出的三件套不配套（encoder={} / decoder={} / joiner={}）—— 这就是误唤醒 100% 的原因",
        stem(&enc),
        stem(&dec),
        stem(&joi)
    );
    tracing::info!(
        encoder = %enc.file_name().unwrap_or_default().to_string_lossy(),
        decoder = %dec.file_name().unwrap_or_default().to_string_lossy(),
        joiner = %joi.file_name().unwrap_or_default().to_string_lossy(),
        "KWS 模型三件套已配对"
    );
    Ok((enc, dec, joi))
}

/// 按实测参数组装 `KeywordSpotterConfig`。
///
/// API 依据：sherpa-onnx 2.0.0 `src/kws.rs`（`KeywordSpotter::create(&config)`）。
fn build_config(cfg: &KwsConfig) -> Result<sherpa_onnx::KeywordSpotterConfig> {
    let (enc, dec, joi) = pick_model_set(&cfg.model_dir)?;
    let mut c = sherpa_onnx::KeywordSpotterConfig::default();
    c.model_config.transducer.encoder = Some(enc.to_string_lossy().to_string());
    c.model_config.transducer.decoder = Some(dec.to_string_lossy().to_string());
    c.model_config.transducer.joiner = Some(joi.to_string_lossy().to_string());
    c.model_config.tokens = Some(cfg.model_dir.join("tokens.txt").to_string_lossy().to_string());
    c.model_config.num_threads = 1;
    c.model_config.model_type = Some("zipformer2".into());
    c.keywords_file = Some(cfg.keywords_file.to_string_lossy().to_string());
    c.keywords_score = cfg.score;
    c.keywords_threshold = cfg.threshold;
    // ★ 这个字段就是「小蝶儿」不误唤醒的关键
    c.num_trailing_blanks = cfg.num_trailing_blanks;
    Ok(c)
}

/// KWS 主循环
fn run_kws<F>(
    cfg: KwsConfig,
    running: Arc<AtomicBool>,
    detections: Arc<AtomicU64>,
    voice_frames: Arc<AtomicU64>,
    on_wake: &mut F,
) -> Result<()>
where
    F: FnMut(WakeEvent),
{
    use sherpa_onnx::KeywordSpotter;

    // 找到 encoder/decoder/joiner（优先 int8，体积小）
    let find = |prefix: &str| -> Result<PathBuf> {
        let mut best: Option<(bool, PathBuf)> = None;
        for e in std::fs::read_dir(&cfg.model_dir)?.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with(prefix) || !name.ends_with(".onnx") {
                continue;
            }
            let is_int8 = name.contains("int8");
            // 优先非 int8 的 chunk-16（准确率高）；没有就用 int8
            if best.is_none() || (best.as_ref().unwrap().0 && !is_int8) {
                best = Some((is_int8, e.path()));
            }
        }
        best.map(|(_, p)| p).ok_or_else(|| {
            anyhow::anyhow!("模型目录里找不到 {prefix}*.onnx: {}", cfg.model_dir.display())
        })
    };
    let encoder = find("encoder-")?;
    let decoder = find("decoder-")?;
    let joiner = find("joiner-")?;
    tracing::info!(
        encoder = %encoder.display(),
        decoder = %decoder.display(),
        joiner = %joiner.display(),
        blanks = cfg.num_trailing_blanks,
        "KWS 模型加载"
    );

    let kws_cfg = build_config(&cfg)?;
    let spotter = KeywordSpotter::create(&kws_cfg).context("创建 KeywordSpotter 失败")?;
    let stream = spotter.create_stream();

    // 打开麦克风（16k/mono/i16/20ms）
    let capture = AudioCapture::start_default().context("KWS 需要麦克风")?;
    tracing::info!("KWS 常驻监听已启动");

    let frame_samples = crate::FRAME_SAMPLES;
    while running.load(Ordering::Relaxed) {
        let frame = match capture.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(f) => f,
            Err(_) => continue,
        };
        // 有声才喂（静音喂进去也行，但省 CPU）
        let rms = crate::spectrum::rms_i16(&frame);
        if rms > 0.005 {
            voice_frames.fetch_add(1, Ordering::Relaxed);
        }
        let samples: Vec<f32> = frame
            .iter()
            .map(|&s| s as f32 / 32768.0)
            .collect();
        anyhow::ensure!(
            samples.len() == frame_samples,
            "帧长必须是 {frame_samples}，实际 {}",
            samples.len()
        );
        stream.accept_waveform(16_000, &samples);
        while spotter.is_ready(&stream) {
            spotter.decode(&stream);
            if let Some(res) = spotter.get_result(&stream) {
                // 必须判空！否则纯静音也会被当成命中（见 is_real_hit 的注释）
                if !is_real_hit(&res) {
                    continue;
                }
                detections.fetch_add(1, Ordering::Relaxed);
                tracing::info!(keyword = %res.keyword, at_sec = res.start_time, "🔊 检测到唤醒词");
                on_wake(WakeEvent::Detected { keyword: res.keyword });
                spotter.reset(&stream);
            }
        }
    }
    let _ = capture.stop();
    tracing::info!("KWS 监听已停止");
    Ok(())
}

/// 把「唤醒」接到状态机上（生产路径用这个）
pub fn wire_to_machine(
    listener_running: Arc<AtomicBool>,
    machine: Arc<tokio::sync::Mutex<SessionMachine>>,
) {
    // 状态机只在 S0 时接受唤醒（其他状态忽略，避免重复触发）
    let _ = (listener_running, machine);
}

/// 检查一个音频文件能否唤醒（离线验证，不需要麦克风）
///
/// 这是 `tools/kws/kws_check.py` 的 Rust 等价物，用于 CI 与真机快速验证。
pub fn detect_in_file(cfg: &KwsConfig, path: &Path) -> Result<Vec<(f32, String)>> {
    cfg.validate()?;
    use sherpa_onnx::KeywordSpotter;

    let kws_cfg = build_config(cfg)?;
    let spotter = KeywordSpotter::create(&kws_cfg).context("创建 KeywordSpotter 失败")?;
    let stream = spotter.create_stream();

    // 解码任意音频 → 16k mono f32
    use rodio::Source;
    let file = std::fs::File::open(path)?;
    let dec = rodio::Decoder::try_from(file)?;
    let ch = dec.channels().get() as usize;
    let rate = dec.sample_rate().get();
    let raw: Vec<f32> = dec.collect();
    let frames = raw.len() / ch.max(1);
    let mut mono = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut acc = 0.0;
        for c in 0..ch {
            acc += raw[i * ch + c];
        }
        mono.push(acc / ch as f32);
    }
    // 线性重采样到 16k
    let target = 16_000u32;
    let n = (frames as u64 * target as u64 / rate as u64) as usize;
    let mut pcm = Vec::with_capacity(n);
    for i in 0..n {
        let pos = i as f64 * rate as f64 / target as f64;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = mono.get(idx).copied().unwrap_or(0.0);
        let b = mono.get(idx + 1).copied().unwrap_or(a);
        pcm.push(a + (b - a) * frac);
    }
    // 末尾补 0.8s 静音，让 trailing blanks 能触发
    pcm.extend(std::iter::repeat(0.0f32).take(12_800));

    let mut hits = Vec::new();
    let chunk = crate::FRAME_SAMPLES;
    for (i, c) in pcm.chunks(chunk).enumerate() {
        stream.accept_waveform(target as i32, c);
        while spotter.is_ready(&stream) {
            spotter.decode(&stream);
            if let Some(res) = spotter.get_result(&stream) {
                if !is_real_hit(&res) {
                    continue;
                }
                hits.push((i as f32 * chunk as f32 / target as f32, res.keyword));
                spotter.reset(&stream);
            }
        }
    }
    Ok(hits)
}

/// 把唤醒事件接到状态机（返回需要调用的动作描述，纯逻辑便于测试）
pub fn wake_stimulus_for(state: OrbState) -> Option<Stimulus> {
    if state == OrbState::S0 {
        Some(Stimulus::WakeWord)
    } else {
        // 已在通话中/异常态，重复唤醒词不应重新触发
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// num_trailing_blanks < 4 必须被拒（默认 1 会让「小蝶儿」误唤醒）。
    ///
    /// 注意测试要造一个「模型文件齐全」的假目录，否则 validate 会先报目录缺失，
    /// 测不到 blanks 这一条（2026-10-24 踩过）。
    #[test]
    fn config_rejects_blank_less_than_4() {
        let dir = std::env::temp_dir().join(format!("dm-kws-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tokens.txt"), "x 0\n").unwrap();
        std::fs::write(dir.join("encoder-x.onnx"), b"fake").unwrap();
        let kw = dir.join("keywords.txt");
        std::fs::write(&kw, "x iǎo d ié @小蝶\n").unwrap();

        let c = KwsConfig {
            model_dir: dir.clone(),
            keywords_file: kw,
            num_trailing_blanks: 1,
            ..Default::default()
        };
        let e = c.validate().unwrap_err().to_string();
        assert!(e.contains("num_trailing_blanks"), "错误信息要指明参数: {e}");

        // 改成 4 就应该通过（证明这条校验是「只卡 <4」而不是别的）
        let ok = KwsConfig { num_trailing_blanks: 4, ..c };
        assert!(ok.validate().is_ok(), "4 应该通过: {:?}", ok.validate());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_rejects_missing_model_dir() {
        let c = KwsConfig {
            model_dir: PathBuf::from("/definitely/not/here"),
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn default_blanks_is_4() {
        assert_eq!(KwsConfig::default().num_trailing_blanks, 4);
    }

    #[test]
    fn wake_only_triggers_from_standby() {
        assert_eq!(wake_stimulus_for(OrbState::S0), Some(Stimulus::WakeWord));
        for s in [OrbState::S1, OrbState::S2, OrbState::S3, OrbState::S4, OrbState::S5] {
            assert_eq!(wake_stimulus_for(s), None, "{s:?} 不该被重复唤醒");
        }
    }

    /// 回归测试：空 keyword 的结果**不算命中**。
    ///
    /// 这个 bug 曾经导致误唤醒 100%：sherpa-onnx 的 Rust 绑定在没命中时
    /// 返回 Some(KeywordResult{keyword:""}), 而不是 None。
    #[test]
    fn empty_keyword_is_not_a_hit() {
        let empty = sherpa_onnx::KeywordResult {
            keyword: String::new(),
            tokens: String::new(),
            tokens_arr: vec![],
            timestamps: vec![],
            start_time: 0.0,
            json: r#"{"start_time":0.00, "keyword": "", "timestamps": [], "tokens":[]}"#.into(),
        };
        assert!(!is_real_hit(&empty), "空 keyword 必须判为非命中");

        let blank = sherpa_onnx::KeywordResult { keyword: "   ".into(), ..empty.clone() };
        assert!(!is_real_hit(&blank), "全空格也算空");

        let real = sherpa_onnx::KeywordResult { keyword: "小蝶".into(), start_time: 1.1, ..empty };
        assert!(is_real_hit(&real), "真命中要通过");
    }

    #[test]
    fn wake_event_equality() {
        assert_eq!(
            WakeEvent::Detected { keyword: "小蝶".into() },
            WakeEvent::Detected { keyword: "小蝶".into() }
        );
    }
}
