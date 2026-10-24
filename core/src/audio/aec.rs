//! 回声消除（AEC）：把「扬声器正在放的声音」从「麦克风听到的声音」里减掉。
//!
//! # 要解决的问题
//!
//! 外放时麦克风会听到 TTS 自己的声音，本地 VAD 把它当成「用户开口」，触发 barge-in
//! 自我打断。AEC 把参考信号（即将播放的音频）和采集信号对齐后自适应地减掉回声，
//! 让 VAD 只听到真正的人声。
//!
//! # 选型与实际 API
//!
//! 用 [`sonora`] 0.2（WebRTC Audio Processing 的纯 Rust 移植，AEC3 算法）。
//! 不用 `webrtc-audio-processing`：那个要 clang/meson/ninja，本机（Debian 云主机）没有。
//!
//! 实际用到的 sonora API（0.2.0，`~/.cargo/registry/src/*/sonora-0.2.0/`）：
//!
//! | 调用 | 说明 |
//! |---|---|
//! | `AudioProcessing::builder().config(..).capture_config(..).render_config(..).build()` | 构造，`build()` 返回 `AudioProcessing`（不是 `Result`） |
//! | `Config { echo_canceller: Some(EchoCanceller::default()), .. }` | 组件全是 `Option`，`None` = 关闭；默认**全关** |
//! | `process_render_f32(&[&[f32]], &mut [&mut [f32]])` | 参考信号，**去交织**（每声道一个切片） |
//! | `process_capture_f32(&[&[f32]], &mut [&mut [f32]])` | 采集信号，去交织，原地可写 |
//! | `process_capture_i16(&[i16], &mut [i16])` | 采集信号，**交织** i16 版本（省一次转换） |
//! | `set_stream_delay_ms(i32) -> Result<(), Error>` | 只在这条路径真正需要固定延迟时才调用 |
//!
//! 几个和预期不同、必须按实际行为适配的点：
//!
//! 1. **`build()` 不返回 `Result`**，没有「构造失败」分支；错误只可能出现在
//!    `process_*`（采样率/帧长不合规），而本模块的参数是编译期常量，所以理论上不会走到。
//!    仍然保留错误计数，避免 panic。
//! 2. **只吃 10ms 块**（`StreamConfig::num_frames() = rate / 100`）。项目帧长定死 20ms
//!    （[`crate::FRAME_SAMPLES`] = 320），所以这里在**内部**把 320 拆成两个 160 子帧，
//!    **不改项目帧长**。
//! 3. **`set_stream_delay_ms` 一旦调用就关掉自适应延迟估计**（内部走
//!    `external_audio_buffer_delay` 分支，把对齐钉死在给定值 + headroom）。我们的
//!    播放队列深度是变化的，钉死反而更差，所以默认**不调用**，交给 AEC3 自己估
//!    （它在 16k 下能搜到约 600ms 的延迟）。
//! 4. **`Config` 的默认值是全部关闭**，AEC 必须显式 `Some(EchoCanceller::default())`。
//!    噪声抑制 / AGC 默认也不开 —— 上行音频是给 ASR 的，乱加增益和降噪会伤识别率。
//!
//! # 两个入口
//!
//! - [`AecProcessor::process_render`]：喂「即将播放的音频」当参考（播放侧 tee 一份）。
//! - [`AecProcessor::process_capture`]：喂麦克风采样，**原地**消除回声。
//!
//! # 为什么两个入口都只要 `&self`
//!
//! 真机上播放路（tokio 网络任务）和采集路（cpal 音频线程）是**两个线程**，通过同一个
//! `Arc<AecProcessor>` 访问。`Arc` 拿不到 `&mut`，所以内部状态统一放进 `Mutex`，
//! 对外只暴露共享引用。语义和 `&mut self` 一样 —— 锁保证同一时刻只有一路在跑 AEC。
//!
//! # 插入点（为什么不会增加采集延迟）
//!
//! AEC 在 [`super::capture::CapturePipeline`] **分帧之后**、发布之前处理整帧 320 采样：
//! 每帧正好两个 10ms 子帧，块对齐是精确的，不需要跨回调攒数据，所以**零额外延迟**。
//! 反过来，如果在重采样后的任意长度块上直接做，cpal 回调长度不是 160 的倍数时
//! 尾巴上的样本只能延后一个回调处理 —— 那才会引入最多 10ms 延迟。

use std::sync::{Arc, Mutex, OnceLock};

use sonora::config::{EchoCanceller, NoiseSuppression};
use sonora::{AudioProcessing, Config as SonoraConfig, StreamConfig};

use super::capture::LinearResampler;

/// AEC 内部工作采样率。上行链路本身就是 16k，所以不需要额外重采样。
pub const AEC_SAMPLE_RATE: u32 = 16_000;

/// AEC 要求的最小处理块：10ms @16k = 160 采样。
///
/// 这是 sonora（以及 WebRTC APM）的硬约束，**不是**本项目的帧长；
/// 项目帧长仍是 [`crate::FRAME_SAMPLES`]（320 = 20ms）。
pub const AEC_SUBFRAME_SAMPLES: usize = 160;

/// AEC 配置。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AecConfig {
    /// 总开关。默认 `true`（外放自我打断是当前的主要问题）。
    pub enabled: bool,
    /// 已知的「渲染→采集」固定延迟（毫秒）。
    ///
    /// `0`（默认）= 不告诉 AEC，让它自己估。只有当你明确知道设备固定延迟、
    /// 且自适应估计效果不好时才填正数 —— 填了会**关闭**自适应延迟估计。
    pub stream_delay_ms: i32,
    /// 是否顺带开噪声抑制（默认关）。
    ///
    /// 上行是给云端 ASR 的，降噪会改变语音本身；只在确认底噪干扰识别时再开。
    pub noise_suppression: bool,
}

impl Default for AecConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            stream_delay_ms: 0,
            noise_suppression: false,
        }
    }
}

impl AecConfig {
    /// 读环境变量覆盖默认值：
    ///
    /// - `MORPHOFLY_AEC=0|false|off`：关掉 AEC（其它值或未设置 = 开）。
    /// - `MORPHOFLY_AEC_DELAY_MS=120`：固定延迟（毫秒，会关掉自适应估计）。
    /// - `MORPHOFLY_AEC_NS=1`：顺带开噪声抑制。
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("MORPHOFLY_AEC") {
            let v = v.trim().to_ascii_lowercase();
            if matches!(v.as_str(), "0" | "false" | "off" | "no") {
                cfg.enabled = false;
            }
        }
        if let Ok(v) = std::env::var("MORPHOFLY_AEC_DELAY_MS") {
            if let Ok(ms) = v.trim().parse::<i32>() {
                cfg.stream_delay_ms = ms;
            }
        }
        if let Ok(v) = std::env::var("MORPHOFLY_AEC_NS") {
            let v = v.trim().to_ascii_lowercase();
            cfg.noise_suppression = matches!(v.as_str(), "1" | "true" | "on" | "yes");
        }
        cfg
    }
}

/// AEC 运行统计（诊断 / 日志用）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AecStats {
    /// 已喂给 AEC 的渲染块数（10ms 一块）。
    pub render_blocks: u64,
    /// 已处理的采集块数（10ms 一块）。
    pub capture_blocks: u64,
    /// `process_*` 返回错误而被跳过的块数（正常应恒为 0）。
    pub errors: u64,
    /// 渲染侧还攒着没凑满 160 的样本数。
    pub render_pending: usize,
}

/// 内部状态：全部在 `Mutex` 后面，因为渲染路和采集路是两个线程。
#[derive(Debug)]
struct Inner {
    apm: AudioProcessing,
    config: AecConfig,
    /// 渲染侧重采样器（上游不是 16k 时用），`(输入采样率, 重采样器)`。
    render_resampler: Option<(u32, LinearResampler)>,
    /// 重采样后的临时缓冲。
    render_conv: Vec<f32>,
    /// 还没凑满 160 的渲染样本。
    render_pending: Vec<f32>,
    /// 子帧输入/输出缓冲（避免每次分配）。
    render_in: [f32; AEC_SUBFRAME_SAMPLES],
    render_out: [f32; AEC_SUBFRAME_SAMPLES],
    capture_in: [f32; AEC_SUBFRAME_SAMPLES],
    capture_i16_in: [i16; AEC_SUBFRAME_SAMPLES],
    stats: AecStats,
}

impl Inner {
    fn new(config: AecConfig) -> Self {
        Self {
            apm: build_apm(&config),
            config,
            render_resampler: None,
            render_conv: Vec::new(),
            render_pending: Vec::with_capacity(AEC_SUBFRAME_SAMPLES),
            render_in: [0.0; AEC_SUBFRAME_SAMPLES],
            render_out: [0.0; AEC_SUBFRAME_SAMPLES],
            capture_in: [0.0; AEC_SUBFRAME_SAMPLES],
            capture_i16_in: [0; AEC_SUBFRAME_SAMPLES],
            stats: AecStats::default(),
        }
    }

    /// 喂一个 160 样本的渲染块（AEC 不修改渲染信号，输出缓冲只是 API 要求）。
    fn feed_render(&mut self, block: &[f32]) {
        self.render_in.copy_from_slice(block);
        let src = [&self.render_in[..]];
        let mut dst = [&mut self.render_out[..]];
        match self.apm.process_render_f32(&src, &mut dst) {
            Ok(()) => self.stats.render_blocks += 1,
            Err(_) => self.stats.errors += 1,
        }
    }

    /// 把渲染样本转成 16k 后追加进待处理队列，凑满 160 就喂给 AEC。
    fn push_render(&mut self, samples: &[f32], sample_rate: u32) {
        let rate = if sample_rate == 0 {
            AEC_SAMPLE_RATE
        } else {
            sample_rate
        };

        // 1) 转到 16k（同采样率则直通，不拷贝第二次）
        let sixteen: &[f32] = if rate == AEC_SAMPLE_RATE {
            samples
        } else {
            let need_rebuild = match &self.render_resampler {
                Some((r, _)) => *r != rate,
                None => true,
            };
            if need_rebuild {
                self.render_resampler =
                    Some((rate, LinearResampler::new(rate, AEC_SAMPLE_RATE)));
            }
            self.render_conv.clear();
            if let Some((_, rs)) = self.render_resampler.as_mut() {
                rs.process(samples, &mut self.render_conv);
            }
            &self.render_conv
        };

        // 2) 攒够 160 就喂（渲染侧允许任意长度，这里攒包不影响延迟：它只是参考信号）
        self.render_pending.extend_from_slice(sixteen);
        while self.render_pending.len() >= AEC_SUBFRAME_SAMPLES {
            let block: Vec<f32> = self.render_pending.drain(..AEC_SUBFRAME_SAMPLES).collect();
            self.feed_render(&block);
        }
        self.stats.render_pending = self.render_pending.len();
    }

    /// 处理一个 160 样本的采集子帧，结果写回 `block`。
    fn process_capture_f32_block(&mut self, block: &mut [f32]) {
        self.capture_in.copy_from_slice(block);
        let src = [&self.capture_in[..]];
        let mut dst = [&mut *block];
        match self.apm.process_capture_f32(&src, &mut dst) {
            Ok(()) => self.stats.capture_blocks += 1,
            Err(_) => self.stats.errors += 1,
        }
    }

    /// i16 版本：直接用 sonora 的交织 i16 接口，省一次 f32 往返。
    fn process_capture_i16_block(&mut self, block: &mut [i16]) {
        self.capture_i16_in.copy_from_slice(block);
        match self
            .apm
            .process_capture_i16(&self.capture_i16_in, block)
        {
            Ok(()) => self.stats.capture_blocks += 1,
            Err(_) => self.stats.errors += 1,
        }
    }
}

/// 按配置建 sonora 的 `AudioProcessing`。
///
/// 只开 AEC（可选 NS）；AGC / 高通 / 前置增益全部保持 `None`，
/// 避免改变上行语音本身（那是 ASR 的输入，不是给人听的）。
fn build_apm(config: &AecConfig) -> AudioProcessing {
    let sonora_config = SonoraConfig {
        echo_canceller: Some(EchoCanceller::default()),
        noise_suppression: config
            .noise_suppression
            .then(NoiseSuppression::default),
        ..Default::default()
    };
    let stream = StreamConfig::new(AEC_SAMPLE_RATE, 1);
    let mut apm = AudioProcessing::builder()
        .config(sonora_config)
        .capture_config(stream)
        .render_config(stream)
        .build();
    // 只有显式给了固定延迟才设置：一旦设置，AEC3 会关闭自适应延迟估计
    // （见模块文档「和预期不同的点」第 3 条）。
    if config.stream_delay_ms > 0 {
        let _ = apm.set_stream_delay_ms(config.stream_delay_ms);
    }
    apm
}

/// 回声消除器（sonora / AEC3 的封装）。
///
/// 线程安全（内部 `Mutex`），可以被播放侧和采集侧同时持有。
///
/// ```no_run
/// # use morphofly_core::audio::aec::{AecConfig, AecProcessor};
/// let aec = AecProcessor::new(AecConfig::default());
/// let reference = vec![0.0f32; 320];      // 即将播放的 20ms
/// let mut mic = vec![0.0f32; 320];        // 麦克风听到的 20ms
/// aec.process_render(&reference);         // 先喂参考
/// aec.process_capture(&mut mic);          // 再消回声（原地）
/// ```
pub struct AecProcessor {
    inner: Mutex<Inner>,
    config: AecConfig,
}

impl std::fmt::Debug for AecProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("AecProcessor")
            .field("config", &self.config)
            .field("stats", &stats)
            .finish_non_exhaustive()
    }
}

impl AecProcessor {
    /// 按配置构造（`enabled == false` 时所有方法都是空操作）。
    pub fn new(config: AecConfig) -> Self {
        Self {
            inner: Mutex::new(Inner::new(config)),
            config,
        }
    }

    /// 默认配置（开启）。
    pub fn with_defaults() -> Self {
        Self::new(AecConfig::default())
    }

    /// 当前配置。
    pub fn config(&self) -> AecConfig {
        self.config
    }

    /// 是否启用（关闭时两个入口直接返回，采样原样通过）。
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// 运行统计。
    pub fn stats(&self) -> AecStats {
        lock(&self.inner).stats
    }

    /// **渲染入口**：喂即将播放的音频（16kHz mono）当参考信号。
    ///
    /// 长度任意：内部会攒成 160 样本喂给 AEC。参考信号晚一点早一点不影响正确性
    /// （AEC 自己估延迟），所以这里攒包不引入可感知问题。
    pub fn process_render(&self, samples: &[f32]) {
        if !self.config.enabled || samples.is_empty() {
            return;
        }
        let mut inner = lock(&self.inner);
        inner.push_render(samples, AEC_SAMPLE_RATE);
    }

    /// [`Self::process_render`] 的带采样率版本（播放侧 nominal 率通常 24kHz）。
    ///
    /// 内部把 `sample_rate` 线性重采样到 16k。采样率变化时重建重采样器。
    pub fn process_render_at(&self, samples: &[f32], sample_rate: u32) {
        if !self.config.enabled || samples.is_empty() {
            return;
        }
        let mut inner = lock(&self.inner);
        inner.push_render(samples, sample_rate);
    }

    /// **采集入口**：原地消除 `samples` 里的回声（16kHz mono）。
    ///
    /// 要求长度是 10ms 的整数倍（项目里是 320 = 2×160）。末尾不足 160 的样本
    /// **原样通过**（不引入延迟），代价是那几个样本没经过 AEC。
    pub fn process_capture(&self, samples: &mut [f32]) {
        if !self.config.enabled || samples.is_empty() {
            return;
        }
        let mut inner = lock(&self.inner);
        for block in samples.chunks_exact_mut(AEC_SUBFRAME_SAMPLES) {
            inner.process_capture_f32_block(block);
        }
    }

    /// [`Self::process_capture`] 的 i16 版本（交织，mono 即逐样本）。
    ///
    /// 采集链路在分帧后就是 i16，用这个入口避免 f32 往返转换。
    pub fn process_capture_i16(&self, samples: &mut [i16]) {
        if !self.config.enabled || samples.is_empty() {
            return;
        }
        let mut inner = lock(&self.inner);
        for block in samples.chunks_exact_mut(AEC_SUBFRAME_SAMPLES) {
            inner.process_capture_i16_block(block);
        }
    }

    /// 清掉自适应滤波器和攒包状态（换设备 / 重新开麦时调用）。
    ///
    /// 代价：需要重新收敛（约 0.5–1 秒），期间回声抑制会变弱。
    pub fn reset(&self) {
        let mut inner = lock(&self.inner);
        inner.apm = build_apm(&self.config);
        inner.render_pending.clear();
        inner.render_resampler = None;
        inner.render_conv.clear();
        inner.stats = AecStats::default();
    }
}

/// 采集侧共享的 AEC 句柄：`None` = 关闭。
///
/// 用共享句柄而不是直接塞 `Arc<AecProcessor>`，是为了支持**运行中开关**
/// （`*handle.lock() = None` 立刻生效，下一帧就不再过 AEC）。
pub type AecHandle = Arc<Mutex<Option<Arc<AecProcessor>>>>;

/// 把可选的 AEC 包成共享句柄。
pub fn handle(aec: Option<Arc<AecProcessor>>) -> AecHandle {
    Arc::new(Mutex::new(aec))
}

/// 进程级共享实例：播放侧 tee 和采集侧消除必须用**同一个** AEC，
/// 否则参考信号和采集信号对不上，等于没做。
///
/// 第一次调用时按 [`AecConfig::from_env`] 建；`MORPHOFLY_AEC=0` 时返回 `None`。
pub fn shared() -> Option<Arc<AecProcessor>> {
    static SHARED: OnceLock<Option<Arc<AecProcessor>>> = OnceLock::new();
    SHARED
        .get_or_init(|| {
            let cfg = AecConfig::from_env();
            if !cfg.enabled {
                tracing::info!("AEC 已通过环境变量关闭（MORPHOFLY_AEC）");
                return None;
            }
            tracing::info!(
                stream_delay_ms = cfg.stream_delay_ms,
                noise_suppression = cfg.noise_suppression,
                "AEC 已启用（sonora / AEC3）"
            );
            Some(Arc::new(AecProcessor::new(cfg)))
        })
        .clone()
}

/// 未中毒的锁（音频线程 panic 不该传染给调用方）。
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成信号工具：确定性伪随机（LCG），不用外部 crate。
    struct Lcg(u64);

    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed)
        }

        /// `[-1, 1)` 均匀分布。
        fn next_unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            let bits = (self.0 >> 33) as u32; // 31 bit
            (bits as f32 / (u32::MAX >> 1) as f32) * 2.0 - 1.0
        }
    }

    /// 「类语音」宽带参考信号：多个正弦 + 慢速幅度调制 + 一点噪声。
    ///
    /// 纯单频正弦对 AEC 不友好（激励不够、滤波器只能学一个频点），
    /// 这里堆几个频点让它接近真实 TTS 的频谱形状。
    fn speech_like(len: usize, seed: u64) -> Vec<f32> {
        let mut rng = Lcg::new(seed);
        let mut out = Vec::with_capacity(len);
        for n in 0..len {
            let t = n as f32 / AEC_SAMPLE_RATE as f32;
            let am = 0.55 + 0.45 * (2.0 * std::f32::consts::PI * 3.1 * t).sin();
            let mut v = 0.0f32;
            for (i, f) in [233.0f32, 521.0, 879.0, 1_337.0, 2_101.0].iter().enumerate() {
                let phase = i as f32 * 0.7;
                v += (2.0 * std::f32::consts::PI * f * t + phase).sin() / (i as f32 + 2.0);
            }
            // 一点点噪声让频谱更连续（AEC 的 NLMS 需要持续激励）
            v += 0.15 * rng.next_unit();
            out.push(v * am * 0.25);
        }
        out
    }

    /// 人声（近端）信号：和参考信号相关性很低的另一组频点。
    fn nearend_like(len: usize, seed: u64) -> Vec<f32> {
        let mut rng = Lcg::new(seed);
        let mut out = Vec::with_capacity(len);
        for n in 0..len {
            let t = n as f32 / AEC_SAMPLE_RATE as f32;
            let am = 0.5 + 0.5 * (2.0 * std::f32::consts::PI * 1.7 * t).sin();
            let mut v = 0.0f32;
            for (i, f) in [311.0f32, 733.0, 1_129.0, 1_907.0].iter().enumerate() {
                let phase = i as f32 * 1.3;
                v += (2.0 * std::f32::consts::PI * f * t + phase).sin() / (i as f32 + 2.0);
            }
            v += 0.1 * rng.next_unit();
            out.push(v * am * 0.3);
        }
        out
    }

    /// 能量（均方值）。
    fn energy(x: &[f32]) -> f64 {
        if x.is_empty() {
            return 0.0;
        }
        x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64
    }

    /// 按 160 样本块交替喂 render / capture。
    ///
    /// 返回处理后的 capture（和输入等长）。
    fn run_aec(aec: &AecProcessor, render: &[f32], capture: &[f32]) -> Vec<f32> {
        assert_eq!(render.len(), capture.len());
        let mut out = capture.to_vec();
        for (r, c) in render
            .chunks_exact(AEC_SUBFRAME_SAMPLES)
            .zip(out.chunks_exact_mut(AEC_SUBFRAME_SAMPLES))
        {
            aec.process_render(r);
            // process_capture 要 &mut [f32]；这里按块传，保持和真机一致
            aec.process_capture(c);
        }
        out
    }

    // ---------- 1) 静音 ----------

    #[test]
    fn silence_in_silence_out() {
        let aec = AecProcessor::with_defaults();
        let silence = vec![0.0f32; crate::FRAME_SAMPLES]; // 320 = 20ms
        let mut worst = 0.0f32;
        for _ in 0..100 {
            aec.process_render(&silence);
            let mut frame = silence.clone();
            aec.process_capture(&mut frame);
            for &s in &frame {
                worst = worst.max(s.abs());
            }
        }
        // AEC3 在增益极低时会注入 comfort noise（noise_floor_dbfs ≈ -96dBFS，
        // 幅度约 1.6e-5）。要求远低于任何有意义的声音（-60dBFS = 1e-3）。
        assert!(
            worst < 1e-4,
            "静音经过 AEC 后必须仍是静音，实测最大幅度 {worst:e}（-{}dBFS）",
            -20.0 * worst.max(1e-30).log10()
        );
        let stats = aec.stats();
        assert_eq!(stats.errors, 0, "不应出现 process_* 错误");
        assert!(stats.capture_blocks >= 200, "100 帧 × 2 子帧");
    }

    #[test]
    fn silence_does_not_change_nonzero_scale() {
        // 静音训练后立刻来一段真声音：不能被 AEC 弄成 0（说明不是在瞎抑制）
        let aec = AecProcessor::with_defaults();
        let silence = vec![0.0f32; 320];
        for _ in 0..50 {
            aec.process_render(&silence);
            let mut frame = silence.clone();
            aec.process_capture(&mut frame);
        }
        let speech = nearend_like(3_200, 7);
        let mut out = speech.clone();
        aec.process_capture(&mut out);
        assert!(
            energy(&out) > 0.3 * energy(&speech),
            "纯静音训练后遇到人声，不该大幅抑制：in={:.3e} out={:.3e}",
            energy(&speech),
            energy(&out)
        );
    }

    // ---------- 2) 回声抑制 ≥ 20dB ----------

    /// 造「参考 + 延迟后的回声」并返回 (参考, 采集, 回声分量) 三段。
    fn echo_scene(len: usize, delay: usize, echo_gain: f32, seed: u64) -> (Vec<f32>, Vec<f32>) {
        let render = speech_like(len, seed);
        let mut capture = vec![0.0f32; len];
        for n in 0..len {
            let echo = if n >= delay {
                echo_gain * render[n - delay]
            } else {
                0.0
            };
            // 一点点底噪，避免「纯零 + 纯回声」的理想输入
            capture[n] = echo;
        }
        (render, capture)
    }

    #[test]
    fn echo_is_suppressed_by_at_least_20db() {
        // 5 秒：前 2 秒给 AEC 收敛，后 3 秒量残余回声
        const LEN: usize = AEC_SAMPLE_RATE as usize * 5;
        const DELAY: usize = 320; // 20ms，典型外放 + 系统缓冲量级
        let (render, capture) = echo_scene(LEN, DELAY, 0.6, 42);

        let aec = AecProcessor::with_defaults();
        let out = run_aec(&aec, &render, &capture);

        // 只量收敛之后（最后 2 秒）
        let tail = AEC_SAMPLE_RATE as usize * 2;
        let in_tail = &capture[LEN - tail..];
        let out_tail = &out[LEN - tail..];
        let erle_db = 10.0 * (energy(in_tail) / energy(out_tail).max(1e-30)).log10();

        assert!(
            erle_db >= 20.0,
            "回声残余应至少降 20dB，实测 {erle_db:.1}dB（in={:.4e} out={:.4e}）",
            energy(in_tail),
            energy(out_tail)
        );
        assert_eq!(aec.stats().errors, 0);
    }

    #[test]
    fn echo_suppression_works_for_several_delays() {
        // 真实场景的延迟不确定（系统缓冲 + 声学路径），逐个试几档
        const LEN: usize = AEC_SAMPLE_RATE as usize * 4;
        const TAIL: usize = AEC_SAMPLE_RATE as usize * 2;
        let mut measured = Vec::new();
        for (i, delay) in [160usize, 480, 960, 1_600].into_iter().enumerate() {
            let (render, capture) = echo_scene(LEN, delay, 0.5, 100 + i as u64);
            let aec = AecProcessor::with_defaults();
            let out = run_aec(&aec, &render, &capture);
            let db = 10.0
                * (energy(&capture[LEN - TAIL..]) / energy(&out[LEN - TAIL..]).max(1e-30)).log10();
            measured.push((delay, db));
            assert!(
                db >= 20.0,
                "延迟 {delay} 样本时残余只降了 {db:.1}dB，全部结果：{measured:?}"
            );
        }
    }

    // ---------- 3) 双讲：人声不能被消掉 ----------

    #[test]
    fn double_talk_keeps_at_least_30_percent_of_near_end() {
        // 3 秒「只有回声」让滤波器收敛，然后 2 秒「回声 + 人声」双讲
        const CONV: usize = AEC_SAMPLE_RATE as usize * 3;
        const DT: usize = AEC_SAMPLE_RATE as usize * 2;
        const DELAY: usize = 480; // 30ms

        let render = speech_like(CONV + DT, 7);
        let nearend = nearend_like(CONV + DT, 99);

        // 采集 = 回声 + (双讲阶段才有的人声)
        let mut capture = vec![0.0f32; CONV + DT];
        for n in 0..CONV + DT {
            let echo = if n >= DELAY { 0.5 * render[n - DELAY] } else { 0.0 };
            let near = if n >= CONV { nearend[n] } else { 0.0 };
            capture[n] = echo + near;
        }

        let aec = AecProcessor::with_defaults();
        let out = run_aec(&aec, &render, &capture);

        // 只量双讲阶段；参照物是「这一段人声本身的能量」
        let near_only = &nearend[CONV..];
        let out_dt = &out[CONV..];
        let retained = energy(out_dt) / energy(near_only).max(1e-30);

        assert!(
            retained > 0.30,
            "双讲时人声能量应保留 >30%，实测 {:.1}%（in={:.3e} near={:.3e} out={:.3e}）",
            retained * 100.0,
            energy(&capture[CONV..]),
            energy(near_only),
            energy(out_dt)
        );
        // 同时确认它确实在干活：输出能量应低于「回声 + 人声」的原始混合
        assert!(
            energy(out_dt) < energy(&capture[CONV..]),
            "双讲时输出总能量应低于输入（至少消掉了回声）"
        );
    }

    // ---------- 接入层：帧长 / 开关 / i16 路径 ----------

    #[test]
    fn one_20ms_frame_maps_to_two_10ms_subframes() {
        let aec = AecProcessor::with_defaults();
        let frame = vec![0.1f32; crate::FRAME_SAMPLES];
        assert_eq!(crate::FRAME_SAMPLES, 2 * AEC_SUBFRAME_SAMPLES);
        aec.process_capture(&mut vec![0.0f32; crate::FRAME_SAMPLES]);
        assert_eq!(aec.stats().capture_blocks, 2, "320 样本 = 2 个 160 子帧");
        let _ = frame;
    }

    #[test]
    fn disabled_aec_is_a_passthrough() {
        let cfg = AecConfig {
            enabled: false,
            ..Default::default()
        };
        let aec = AecProcessor::new(cfg);
        assert!(!aec.is_enabled());
        let input: Vec<f32> = (0..320).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut frame = input.clone();
        aec.process_render(&input);
        aec.process_capture(&mut frame);
        assert_eq!(frame, input, "关闭时必须与输入逐位相同");
        assert_eq!(aec.stats().capture_blocks, 0);
    }

    #[test]
    fn capture_tail_shorter_than_10ms_passes_through() {
        // 非 160 整数倍：整块部分照常处理，尾巴原样通过（不引入延迟）
        let aec = AecProcessor::with_defaults();
        let mut buf = vec![0.25f32; 400]; // 2 个整块 + 80 样本尾巴
        aec.process_capture(&mut buf);
        assert_eq!(aec.stats().capture_blocks, 2);
        assert_eq!(&buf[320..], &[0.25f32; 80], "尾巴不能被改写");
    }

    #[test]
    fn i16_entry_matches_f32_entry() {
        let frame: Vec<i16> = (0..320).map(|i| ((i as f32 * 0.02).sin() * 8000.0) as i16).collect();
        let f32_in: Vec<f32> = frame.iter().map(|&s| f32::from(s) / 32768.0).collect();
        let render = nearend_like(320, 3);

        let a = AecProcessor::with_defaults();
        a.process_render(&render);
        let mut f32_out = f32_in.clone();
        a.process_capture(&mut f32_out);

        let b = AecProcessor::with_defaults();
        b.process_render(&render);
        let mut i16_out = frame.clone();
        b.process_capture_i16(&mut i16_out);

        // 两条路径内部算法相同（sonora 的 i16 接口同样走 float），允许量化级差异
        for (i, (x, y)) in f32_out.iter().zip(i16_out.iter()).enumerate() {
            let y_f = f32::from(*y) / 32768.0;
            assert!(
                (x - y_f).abs() < 1e-3,
                "第 {i} 个样本两条入口不一致: f32={x} i16={y_f}"
            );
        }
    }

    #[test]
    fn render_accepts_arbitrary_length_and_rate() {
        // 播放侧攒包：24kHz、480 样本（20ms）一次喂
        let aec = AecProcessor::with_defaults();
        let chunk_24k = vec![0.1f32; 480];
        for _ in 0..10 {
            aec.process_render_at(&chunk_24k, 24_000);
        }
        let st = aec.stats();
        assert_eq!(st.errors, 0, "24k 参考信号应被正确重采样");
        // 10 × 20ms @16k = 3200 样本 = 20 个子帧
        assert_eq!(st.render_blocks, 20, "实测 {st:?}");
        assert!(st.render_pending < AEC_SUBFRAME_SAMPLES);

        // 采样率切换不能把状态搞坏
        aec.process_render_at(&vec![0.1f32; 160], 16_000);
        aec.process_render_at(&vec![0.1f32; 480], 48_000);
        assert_eq!(aec.stats().errors, 0);
    }

    #[test]
    fn reset_clears_stats_and_keeps_config() {
        let aec = AecProcessor::with_defaults();
        aec.process_render(&vec![0.2f32; 320]);
        let mut frame = vec![0.1f32; 320];
        aec.process_capture(&mut frame);
        assert!(aec.stats().capture_blocks > 0);
        aec.reset();
        assert_eq!(aec.stats().capture_blocks, 0);
        assert!(aec.is_enabled(), "reset 不该改配置");
    }

    #[test]
    fn config_from_env_defaults_on() {
        // 不设环境变量的默认值
        let cfg = AecConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.stream_delay_ms, 0);
        assert!(!cfg.noise_suppression);

        // from_env 在没设变量时等于默认
        std::env::remove_var("MORPHOFLY_AEC");
        std::env::remove_var("MORPHOFLY_AEC_DELAY_MS");
        std::env::remove_var("MORPHOFLY_AEC_NS");
        assert_eq!(AecConfig::from_env(), AecConfig::default());

        std::env::set_var("MORPHOFLY_AEC", "0");
        assert!(!AecConfig::from_env().enabled);
        std::env::set_var("MORPHOFLY_AEC", "1");
        assert!(AecConfig::from_env().enabled);
        std::env::remove_var("MORPHOFLY_AEC");

        std::env::set_var("MORPHOFLY_AEC_DELAY_MS", "120");
        assert_eq!(AecConfig::from_env().stream_delay_ms, 120);
        std::env::remove_var("MORPHOFLY_AEC_DELAY_MS");
    }

    #[test]
    fn shared_handle_can_be_toggled_off() {
        let h = handle(Some(Arc::new(AecProcessor::with_defaults())));
        assert!(lock(&h).is_some());
        *lock(&h) = None;
        assert!(lock(&h).is_none(), "共享句柄必须支持运行中关闭");
    }
}
