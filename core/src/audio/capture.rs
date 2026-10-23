//! 采集：cpal 默认输入设备 → 下混 mono → 重采样到 16kHz → i16 → 20ms(320 采样) 分帧。
//!
//! 结构：
//! - [`ToF32`] / [`downmix_to_mono`]：设备样本（任意位深/声道）→ mono f32，纯函数。
//! - [`LinearResampler`]：设备采样率 → 目标采样率。降采样时先过一阶低通抑制混叠。
//!   纯函数 + 显式状态（跨 chunk 连续，无逐块重算）。
//! - [`FrameAssembler`]：把不定长样本流切成固定 320 采样的帧，不足留到下次。
//! - [`CapturePipeline`]：上面三步串起来，**不碰设备**，所以单元测试可以完整覆盖。
//! - [`AudioCapture`]：真正开 cpal 流；没有设备时 `start()` 返回 [`AudioError`]，不 panic。
//!
//! 帧通过 `tokio::sync::broadcast` 发布（可多消费者，发送非阻塞，适合音频回调线程）。

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use super::{AudioError, rms_i16};
use crate::{FRAME_SAMPLES, UPLINK_SAMPLE_RATE};

/// 采集参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    /// 目标采样率（上行固定 16k）。
    pub sample_rate: u32,
    /// 每帧样本数（20ms @ 16k = 320）。
    pub frame_samples: usize,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            sample_rate: UPLINK_SAMPLE_RATE,
            frame_samples: FRAME_SAMPLES,
        }
    }
}

impl CaptureConfig {
    /// 每帧时长（毫秒），用于日志与断言。
    pub fn frame_ms(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frame_samples as f64 * 1000.0 / self.sample_rate as f64
    }
}

/// 实际打开的设备参数（排查"为什么采样率不对"用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureFormat {
    /// 设备名（cpal 的 human-readable name）。
    pub device_name: String,
    /// 设备原生采样率（Hz）。
    pub device_sample_rate: u32,
    /// 设备原生声道数。
    pub channels: u16,
    /// 设备样本格式（`i16` / `f32` ...）。
    pub sample_format: String,
}

/// 设备样本 → `[-1.0, 1.0]` 的 f32。
///
/// 覆盖 cpal 在 Linux/Windows/macOS 上会返回的全部非 DSD 格式。
pub trait ToF32: Copy {
    /// 单个样本转换。
    fn to_f32_sample(self) -> f32;
}

impl ToF32 for i8 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        f32::from(self) / 128.0
    }
}

impl ToF32 for i16 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        f32::from(self) / 32768.0
    }
}

impl ToF32 for i32 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        self as f32 / 2_147_483_648.0
    }
}

impl ToF32 for i64 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        self as f32 / 9_223_372_036_854_775_808.0
    }
}

impl ToF32 for u8 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        (f32::from(self) - 128.0) / 128.0
    }
}

impl ToF32 for u16 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        (f32::from(self) - 32768.0) / 32768.0
    }
}

impl ToF32 for u32 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        (self as f64 - 2_147_483_648.0) as f32 / 2_147_483_648.0
    }
}

impl ToF32 for u64 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        ((self as f64 - 9_223_372_036_854_775_808.0) / 9_223_372_036_854_775_808.0) as f32
    }
}

impl ToF32 for f32 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        self
    }
}

impl ToF32 for f64 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        self as f32
    }
}

impl ToF32 for cpal::I24 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        // 24 bit 有符号，值域 ±(1<<23)
        self.inner() as f32 / 8_388_608.0
    }
}

impl ToF32 for cpal::U24 {
    #[inline]
    fn to_f32_sample(self) -> f32 {
        (self.inner() as f32 - 8_388_608.0) / 8_388_608.0
    }
}

/// 交织多声道 f32 → mono f32（各声道取算术平均）。
///
/// 末尾不足一个整帧的样本直接丢弃（cpal 只会给整帧，正常不会出现）。
pub fn downmix_to_mono(interleaved: &[f32], channels: u16, out: &mut Vec<f32>) {
    out.clear();
    let ch = channels.max(1) as usize;
    if ch == 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let frames = interleaved.len() / ch;
    out.reserve(frames);
    for f in 0..frames {
        let base = f * ch;
        let sum: f32 = interleaved[base..base + ch].iter().sum();
        out.push(sum / ch as f32);
    }
}

/// 采样率转换：线性插值；降采样时先过一阶低通（截止 ≈ 目标奈奎斯特 × 0.9）抑制混叠。
///
/// 状态跨 chunk 连续：把流分成任意大小的块喂进来，输出等价于一次性处理整段。
#[derive(Debug, Clone)]
pub struct LinearResampler {
    from: u32,
    to: u32,
    /// 每个输出样本在输入坐标上的步进 = from / to。
    step: f64,
    /// 下一个输出样本在**当前块**内的输入坐标（可为负，表示要借上一块的末尾样本）。
    pos: f64,
    /// 上一块（滤波后）的最后一个样本，用于跨块插值。
    prev: f32,
    /// 降采样低通系数；`None` = 不滤波（升采样或同采样率）。
    lp_alpha: Option<f32>,
    /// 低通状态。
    lp_y: f32,
    /// 滤波后的临时缓冲（复用，避免每块分配）。
    scratch: Vec<f32>,
}

impl LinearResampler {
    /// 建一个 `from → to` 的重采样器。参数为 0 时退化成直通（由 [`CapturePipeline`] 拒绝）。
    pub fn new(from: u32, to: u32) -> Self {
        let step = if to == 0 {
            1.0
        } else {
            f64::from(from) / f64::from(to)
        };
        // 只在降采样时开低通：fc = 0.9 * to/2，一阶 IIR 系数 alpha = 1/(1 + from/(2π fc))
        let lp_alpha = if from > to && to > 0 {
            let fc = 0.9 * f64::from(to) / 2.0;
            let alpha = 1.0 / (1.0 + f64::from(from) / (std::f64::consts::TAU * fc));
            Some(alpha as f32)
        } else {
            None
        };
        Self {
            from,
            to,
            step,
            pos: 0.0,
            prev: 0.0,
            lp_alpha,
            lp_y: 0.0,
            scratch: Vec::new(),
        }
    }

    /// 源采样率。
    pub fn from_rate(&self) -> u32 {
        self.from
    }

    /// 目标采样率。
    pub fn to_rate(&self) -> u32 {
        self.to
    }

    /// 输入 `n` 个样本大约会输出多少（估算，用于预分配）。
    pub fn estimate_out_len(&self, n: usize) -> usize {
        if self.to == 0 {
            return 0;
        }
        (n as f64 * f64::from(self.to) / f64::from(self.from)).ceil() as usize + 2
    }

    /// 处理一块 mono f32，追加结果到 `out`。
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if input.is_empty() {
            return;
        }

        // 同一采样率：直通（但保持 prev 更新，避免之后改比率时跳变）
        if self.from == self.to || self.to == 0 || self.from == 0 {
            self.prev = *input.last().unwrap();
            out.extend_from_slice(input);
            return;
        }

        // 1) 可选低通（降采样抗混叠）
        let filtered: &[f32] = match self.lp_alpha {
            Some(a) => {
                self.scratch.clear();
                self.scratch.reserve(input.len());
                let mut y = self.lp_y;
                for &x in input {
                    y += a * (x - y);
                    self.scratch.push(y);
                }
                self.lp_y = y;
                &self.scratch
            }
            None => input,
        };

        let n = filtered.len();
        let nf = n as f64;
        if n == 0 {
            return;
        }
        out.reserve(self.estimate_out_len(n));

        // 2) 线性插值。
        //
        // **关键**：只有"左右两个样本都在本块内"时才产出一个输出。
        // 需要借下一块第一个样本的那个输出**推迟到下一块**再算 —— 这样分块处理
        // 与一次性处理的结果逐样本完全相等（实时采集本来就是分块喂进来的）。
        // 代价是流结束时最多丢 `step` 个样本（实时场景无所谓）。
        if self.step > 0.0 {
            loop {
                let i = self.pos.floor();
                let next = i + 1.0;
                // next 必须 <= n-1；i 允许为 -1（借上一块的最后一个样本 prev）
                if next > (n - 1) as f64 {
                    break;
                }
                let t = (self.pos - i) as f32;
                let s0 = if i < 0.0 {
                    self.prev
                } else {
                    filtered[i as usize]
                };
                let s1 = if next < 0.0 {
                    self.prev
                } else {
                    filtered[next as usize]
                };
                out.push(s0 + (s1 - s0) * t);
                self.pos += self.step;
            }
        }

        // 3) 坐标系平移到下一块（pos 现在可能为负 = 下一个输出点落在上一块末尾之后）
        self.pos -= nf;
        self.prev = filtered[n - 1];
    }

    /// 重置内部状态（换设备 / 重新开麦时调用）。
    pub fn reset(&mut self) {
        self.pos = 0.0;
        self.prev = 0.0;
        self.lp_y = 0.0;
        self.scratch.clear();
    }
}

/// 把不定长的 i16 样本流切成固定长度帧。
#[derive(Debug, Clone)]
pub struct FrameAssembler {
    frame_samples: usize,
    pending: Vec<i16>,
}

impl FrameAssembler {
    /// `frame_samples` 为 0 时不会产出任何帧（不 panic）。
    pub fn new(frame_samples: usize) -> Self {
        Self {
            frame_samples,
            pending: Vec::new(),
        }
    }

    /// 每帧样本数。
    pub fn frame_samples(&self) -> usize {
        self.frame_samples
    }

    /// 尚未凑满一帧的样本数。
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 追加样本，把凑满的整帧推进 `out`。
    pub fn push_into(&mut self, samples: &[i16], out: &mut Vec<Vec<i16>>) {
        if self.frame_samples == 0 {
            return;
        }
        self.pending.extend_from_slice(samples);
        let full = self.pending.len() / self.frame_samples;
        if full == 0 {
            return;
        }
        out.reserve(full);
        let mut start = 0;
        for _ in 0..full {
            out.push(self.pending[start..start + self.frame_samples].to_vec());
            start += self.frame_samples;
        }
        self.pending.drain(..start);
    }

    /// [`Self::push_into`] 的便捷版本。
    pub fn push(&mut self, samples: &[i16]) -> Vec<Vec<i16>> {
        let mut out = Vec::new();
        self.push_into(samples, &mut out);
        out
    }

    /// 丢掉未凑满的尾巴（停止采集时用）。
    pub fn reset(&mut self) {
        self.pending.clear();
    }
}

/// 采集纯逻辑流水线：交织设备样本 → mono → 重采样 → i16 → 定长帧。
///
/// 不持有任何设备句柄，因此可以在没有声卡的机器上完整单测。
#[derive(Debug, Clone)]
pub struct CapturePipeline {
    config: CaptureConfig,
    device_rate: u32,
    channels: u16,
    resampler: LinearResampler,
    mono: Vec<f32>,
    resampled: Vec<f32>,
    i16_buf: Vec<i16>,
    assembler: FrameAssembler,
    /// 最近一帧的 RMS（[0,1]）。
    last_rms: f32,
    /// 累计产出的帧数。
    frames_out: u64,
}

impl CapturePipeline {
    /// 建一条流水线。
    ///
    /// # Errors
    /// 设备采样率、声道数、目标采样率、帧长任一为 0 时返回 [`AudioError::InvalidConfig`]。
    pub fn new(
        device_rate: u32,
        channels: u16,
        config: CaptureConfig,
    ) -> Result<Self, AudioError> {
        if device_rate == 0 {
            return Err(AudioError::InvalidConfig("设备采样率为 0".into()));
        }
        if channels == 0 {
            return Err(AudioError::InvalidConfig("设备声道数为 0".into()));
        }
        if config.sample_rate == 0 {
            return Err(AudioError::InvalidConfig("目标采样率为 0".into()));
        }
        if config.frame_samples == 0 {
            return Err(AudioError::InvalidConfig("帧样本数为 0".into()));
        }
        Ok(Self {
            config,
            device_rate,
            channels,
            resampler: LinearResampler::new(device_rate, config.sample_rate),
            mono: Vec::new(),
            resampled: Vec::new(),
            i16_buf: Vec::new(),
            assembler: FrameAssembler::new(config.frame_samples),
            last_rms: 0.0,
            frames_out: 0,
        })
    }

    /// 目标配置。
    pub fn config(&self) -> CaptureConfig {
        self.config
    }

    /// 设备采样率。
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// 设备声道数。
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// 最近一帧的 RMS。
    pub fn last_rms(&self) -> f32 {
        self.last_rms
    }

    /// 累计产出帧数。
    pub fn frames_out(&self) -> u64 {
        self.frames_out
    }

    /// 未凑满一帧的样本数。
    pub fn pending_len(&self) -> usize {
        self.assembler.pending_len()
    }

    /// 处理一块交织的任意类型设备样本，返回本次产出的完整帧。
    pub fn process<T: ToF32>(&mut self, interleaved: &[T]) -> Vec<Vec<i16>> {
        let mut frames = Vec::new();
        self.process_into(interleaved, &mut frames);
        frames
    }

    /// 处理一块交织样本，帧追加到 `out`（音频回调里用这个避免反复分配）。
    pub fn process_into<T: ToF32>(&mut self, interleaved: &[T], out: &mut Vec<Vec<i16>>) {
        if interleaved.is_empty() {
            return;
        }

        // 1) 解交织 + 下混；同采样率时直接写 i16，省一次拷贝
        self.mono.clear();
        if self.channels == 1 {
            self.mono.extend(interleaved.iter().map(|s| s.to_f32_sample()));
        } else {
            let ch = self.channels as usize;
            let frames = interleaved.len() / ch;
            self.mono.reserve(frames);
            for f in 0..frames {
                let base = f * ch;
                let mut sum = 0.0f32;
                for c in 0..ch {
                    sum += interleaved[base + c].to_f32_sample();
                }
                self.mono.push(sum / ch as f32);
            }
        }

        // 2) 重采样
        if self.resampler.from_rate() == self.resampler.to_rate() {
            self.resampled.clear();
            self.resampled.extend_from_slice(&self.mono);
        } else {
            self.resampled.clear();
            self.resampler.process(&self.mono, &mut self.resampled);
        }

        // 3) f32 → i16
        self.i16_buf.clear();
        self.i16_buf.reserve(self.resampled.len());
        self.i16_buf
            .extend(self.resampled.iter().map(|&v| super::f32_to_i16(v)));

        // 4) 分帧 + RMS
        let before = out.len();
        self.assembler.push_into(&self.i16_buf, out);
        if let Some(last) = out[before..].last() {
            self.last_rms = rms_i16(last);
            self.frames_out += (out.len() - before) as u64;
        }
    }

    /// 重置内部状态（保留配置）。
    pub fn reset(&mut self) {
        self.resampler.reset();
        self.assembler.reset();
        self.mono.clear();
        self.resampled.clear();
        self.i16_buf.clear();
        self.last_rms = 0.0;
    }
}

/// 采集统计，供 UI / 诊断读。
#[derive(Debug, Default)]
struct CaptureStats {
    /// 最新一帧的 RMS（f32 bits）。
    rms_bits: AtomicU32,
    /// 成功发出的帧数。
    frames_sent: AtomicU64,
    /// 没有接收者导致丢弃的帧数。
    frames_dropped: AtomicU64,
}

/// cpal 麦克风采集。
///
/// 用法：
/// ```no_run
/// # use diemeng_core::audio::capture::AudioCapture;
/// let cap = AudioCapture::new();
/// let mut rx = cap.subscribe();          // 先订阅，避免丢开头
/// cap.start().expect("打开麦克风");       // 没有设备时这里是 Err，不会 panic
/// // let frame = rx.blocking_recv().unwrap(); // 320 个 i16
/// cap.stop();
/// ```
pub struct AudioCapture {
    config: CaptureConfig,
    tx: tokio::sync::broadcast::Sender<Vec<i16>>,
    stats: Arc<CaptureStats>,
    last_error: Arc<Mutex<Option<String>>>,
    stream: Mutex<Option<cpal::Stream>>,
    format: Mutex<Option<CaptureFormat>>,
    /// 便捷消费端：一个常驻 broadcast 订阅者 + std mpsc 转发器。
    ///
    /// 目的是让调用方不必自己管 `broadcast::Receiver`（它没有 `recv_timeout`），
    /// 直接 `cap.recv_timeout(Duration)` 就能同步阻塞取帧，适合 CLI 示例与测试。
    simple_rx: Mutex<Option<std::sync::mpsc::Receiver<Vec<i16>>>>,
    forwarder: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 转发线程的停车开关（stop 时置 true 让线程退出）。
    forward_stop: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for AudioCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioCapture")
            .field("config", &self.config)
            .field("is_running", &self.is_running())
            .finish_non_exhaustive()
    }
}

impl Default for AudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

/// broadcast 缓冲深度：128 帧 ≈ 2.56s，消费端卡顿时丢旧帧而不是阻塞采集线程。
const CAPTURE_CHANNEL_FRAMES: usize = 128;

impl AudioCapture {
    /// **测试/自动化专用**：用音频文件代替物理麦克风。
    ///
    /// 不打开任何设备（在无声卡机器上也能跑），把文件解码成 16k mono i16，
    /// 按 20ms 帧灌进与真采集相同的 `broadcast` 通道，下游代码完全无感。
    /// 只有 `duplex --input-file` 这类探针会用它；生产路径永远走 `start_default()`。
    pub fn file_source(path: &str) -> Result<Self, AudioError> {
        use rodio::Source;
        let file = std::fs::File::open(path)
            .map_err(|e| AudioError::Device(format!("打开音频文件失败 {path}: {e}")))?;
        let decoder = rodio::Decoder::try_from(file)
            .map_err(|e| AudioError::Device(format!("解码失败 {path}: {e}")))?;
        let channels = decoder.channels().get() as usize;
        let rate = decoder.sample_rate().get();
        let samples: Vec<f32> = decoder.collect();
        if channels == 0 || rate == 0 || samples.is_empty() {
            return Err(AudioError::Device(format!("音频文件是空的: {path}")));
        }
        // 下混 mono
        let frames = samples.len() / channels;
        let mut mono = Vec::with_capacity(frames);
        for i in 0..frames {
            let mut acc = 0.0f32;
            for c in 0..channels {
                acc += samples[i * channels + c];
            }
            mono.push(acc / channels as f32);
        }
        // 重采样到 16k
        let target = UPLINK_SAMPLE_RATE;
        let out_len = (frames as u64 * target as u64 / rate as u64) as usize;
        let mut out: Vec<i16> = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 * rate as f64 / target as f64;
            let idx = pos as usize;
            let frac = (pos - idx as f64) as f32;
            let a = mono.get(idx).copied().unwrap_or(0.0);
            let b = mono.get(idx + 1).copied().unwrap_or(a);
            let v = a + (b - a) * frac;
            out.push((v.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
        }

        let cfg = CaptureConfig::default();
        let (tx, _) = tokio::sync::broadcast::channel::<Vec<i16>>(256);
        let tx2 = tx.clone();
        let frame_samples = cfg.frame_samples;
        let total = out.len();
        std::thread::spawn(move || {
            let mut buf = out;
            // 末尾补静音让服务端 VAD 判定「说完了」。
            //
            // ⚠️ 2026-10-24 审计教训：原来写死补 2 秒静音，导致延迟测量被自己污染——
            // 测「最后有声帧 → speech_stopped」时，那 2 秒静音也要按实时推完才算数，
            // 于是量出 3.2s 的假延迟。现在改成可配置，且默认只补 0.6s（刚好够 VAD
            // 的 silence_duration_ms=300 判定两次）。
            let pad_ms: u32 = std::env::var("DIEMENG_FILE_PAD_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000);
            buf.extend(std::iter::repeat(0i16).take(UPLINK_SAMPLE_RATE as usize * pad_ms as usize / 1000));
            let mut acc: Vec<i16> = Vec::with_capacity(frame_samples);
            let started = std::time::Instant::now();
            let mut sent = 0usize;
            for s in buf {
                acc.push(s);
                if acc.len() >= frame_samples {
                    let frame = std::mem::take(&mut acc);
                    let _ = tx2.send(frame);
                    sent += 1;
                    // 按 20ms 节奏发；落后于真实时间就追上（文件比实时快时不会有节奏问题）
                    let due = started + std::time::Duration::from_millis(20 * sent as u64);
                    let now = std::time::Instant::now();
                    if due > now {
                        std::thread::sleep(due - now);
                    }
                }
            }
            if !acc.is_empty() {
                let _ = tx2.send(acc);
            }
            tracing::info!(total, "file_source 推完");
        });

        Ok(Self {
            config: cfg,
            tx,
            stats: Arc::new(CaptureStats::default()),
            last_error: Arc::new(Mutex::new(None)),
            stream: Mutex::new(None),
            format: Mutex::new(None),
            simple_rx: Mutex::new(None),
            forwarder: Mutex::new(None),
            forward_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// 用默认配置（16kHz / 320 采样 / 20ms）。
    pub fn new() -> Self {
        Self::with_config(CaptureConfig::default())
    }

    /// 指定配置。
    pub fn with_config(config: CaptureConfig) -> Self {
        let (tx, _rx) = tokio::sync::broadcast::channel(CAPTURE_CHANNEL_FRAMES);
        Self {
            config,
            tx,
            stats: Arc::new(CaptureStats::default()),
            last_error: Arc::new(Mutex::new(None)),
            stream: Mutex::new(None),
            format: Mutex::new(None),
            simple_rx: Mutex::new(None),
            forwarder: Mutex::new(None),
            forward_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 便捷构造 + 启动：[`Self::new`] 后立刻 [`Self::start`]。
    ///
    /// # Errors
    /// 同 [`Self::start`]（无麦克风时返回 [`AudioError::NoInputDevice`]，**不 panic**）。
    ///
    /// ```no_run
    /// # use diemeng_core::audio::capture::AudioCapture;
    /// let cap = AudioCapture::start_default().expect("需要麦克风");
    /// let frame = cap.recv_timeout(std::time::Duration::from_millis(500)).unwrap();
    /// assert_eq!(frame.len(), 320);
    /// cap.stop();
    /// ```
    pub fn start_default() -> Result<Self, AudioError> {
        let cap = Self::new();
        cap.start_with_forwarder()?;
        Ok(cap)
    }

    /// 启动采集，并启动 std mpsc 转发线程（[`Self::recv_timeout`] 依赖它）。
    ///
    /// [`Self::start`] 只开 cpal 流 + broadcast；本方法额外挂一个转发器。
    pub fn start_with_forwarder(&self) -> Result<(), AudioError> {
        self.start()?;

        let need_forwarder = lock_unpoisoned(&self.forwarder).is_none();
        if need_forwarder {
            let mut rx = self.tx.subscribe();
            let (tx, std_rx) = std::sync::mpsc::channel();
            let stop = self.forward_stop.clone();
            stop.store(false, Ordering::Release);
            let handle = std::thread::Builder::new()
                .name("diemeng-capture-forward".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        match rx.blocking_recv() {
                            Ok(frame) => {
                                // 消费端不读也算丢弃，继续转发不阻塞采集线程
                                if tx.send(frame).is_err() {
                                    break;
                                }
                            }
                            // broadcast 落后（消费太慢）或通道关闭：继续尝试
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                })
                .map_err(|e| AudioError::Device(format!("启动采集转发线程失败: {e}")))?;
            *lock_unpoisoned(&self.simple_rx) = Some(std_rx);
            *lock_unpoisoned(&self.forwarder) = Some(handle);
        }
        Ok(())
    }

    /// 同步阻塞取一帧（最多等 `timeout`）。
    ///
    /// 需要先调用 [`Self::start_default`] 或 [`Self::start_with_forwarder`]；
    /// 未启动转发器时直接返回超时错误（不会 panic）。
    ///
    /// # Errors
    /// - `RecvTimeoutError::Timeout`：超时。
    /// - `RecvTimeoutError::Disconnected`：采集已停止。
    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Vec<i16>, std::sync::mpsc::RecvTimeoutError> {
        let guard = lock_unpoisoned(&self.simple_rx);
        match guard.as_ref() {
            Some(rx) => rx.recv_timeout(timeout),
            // 未启动转发器：等价于超时
            None => Err(std::sync::mpsc::RecvTimeoutError::Timeout),
        }
    }

    /// 订阅 20ms 帧（`Vec<i16>`，长度恒为 `config.frame_samples`）。
    ///
    /// 可以多次订阅（多消费者）；消费太慢时 broadcast 会丢弃旧帧，
    /// 丢弃量见 [`Self::frames_dropped`]。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Vec<i16>> {
        self.tx.subscribe()
    }

    /// 配置。
    pub fn config(&self) -> CaptureConfig {
        self.config
    }

    /// 是否正在采集。
    pub fn is_running(&self) -> bool {
        lock_unpoisoned(&self.stream).is_some()
    }

    /// 最近一帧的 RMS（[0,1]）；未采集时为 0。
    pub fn rms(&self) -> f32 {
        f32::from_bits(self.stats.rms_bits.load(Ordering::Relaxed))
    }

    /// 累计成功发出的帧数。
    pub fn frames_sent(&self) -> u64 {
        self.stats.frames_sent.load(Ordering::Relaxed)
    }

    /// 因没有接收者而丢弃的帧数。
    pub fn frames_dropped(&self) -> u64 {
        self.stats.frames_dropped.load(Ordering::Relaxed)
    }

    /// 实际打开的设备参数；未启动时为 `None`。
    pub fn format(&self) -> Option<CaptureFormat> {
        lock_unpoisoned(&self.format).clone()
    }

    /// 最近一次流错误（设备掉线等）；没有则 `None`。
    pub fn last_error(&self) -> Option<String> {
        lock_unpoisoned(&self.last_error).clone()
    }

    /// 打开默认输入设备并开始推送帧。重复调用是幂等的（已在跑就直接返回 `Ok`）。
    ///
    /// # Errors
    /// - [`AudioError::NoInputDevice`]：没有麦克风。
    /// - [`AudioError::UnsupportedSampleFormat`]：设备格式本层不支持（DSD）。
    /// - [`AudioError::Device`]：cpal 报错（设备被占用、权限不足等）。
    pub fn start(&self) -> Result<(), AudioError> {
        {
            let mut guard = lock_unpoisoned(&self.stream);
            if guard.is_some() {
                return Ok(());
            }
            // 占位防止并发 start 重复开流：先拿锁，出了作用域再真正构建会释放锁；
            // 这里用局部标志 + 后续赋值的方式，保持简单。
            *guard = None;
        }

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or(AudioError::NoInputDevice)?;

        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::Device(format!("读取默认输入配置失败: {e}")))?;

        let format = CaptureFormat {
            device_name,
            device_sample_rate: supported.sample_rate(),
            channels: supported.channels(),
            sample_format: supported.sample_format().to_string(),
        };

        let pipeline = CapturePipeline::new(
            format.device_sample_rate,
            format.channels,
            self.config,
        )?;

        let stream_config: cpal::StreamConfig = supported.config();
        let sample_format = supported.sample_format();

        let stream = build_input_stream(
            &device,
            &stream_config,
            sample_format,
            pipeline,
            self.tx.clone(),
            self.stats.clone(),
            self.last_error.clone(),
        )?;

        stream
            .play()
            .map_err(|e| AudioError::Device(format!("启动输入流失败: {e}")))?;

        *lock_unpoisoned(&self.stream) = Some(stream);
        *lock_unpoisoned(&self.format) = Some(format);
        *lock_unpoisoned(&self.last_error) = None;
        Ok(())
    }

    /// 停止采集（丢弃底层流）。未启动时是空操作。
    pub fn stop(&self) -> Result<(), AudioError> {
        let taken = lock_unpoisoned(&self.stream).take();
        *lock_unpoisoned(&self.format) = None;
        drop(taken); // 释放 cpal 流
        Ok(())
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(stream) = lock_unpoisoned(&self.stream).take() {
            drop(stream);
        }
    }
}

/// 用具体的样本类型构建 cpal 输入流。
fn build_stream_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: SampleFormat,
    mut pipeline: CapturePipeline,
    tx: tokio::sync::broadcast::Sender<Vec<i16>>,
    stats: Arc<CaptureStats>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, AudioError>
where
    T: cpal::SizedSample + Send + 'static + ToF32,
{
    let _ = sample_format; // 由 T::FORMAT 决定，保留参数便于日志
    let mut frames: Vec<Vec<i16>> = Vec::with_capacity(8);
    let err_stats = stats.clone();
    let err_cb = move |err: cpal::Error| {
        *lock_unpoisoned(&last_error) = Some(err.to_string());
        err_stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
    };

    device
        .build_input_stream::<T, _, _>(
            config.clone(),
            move |data: &[T], _info: &cpal::InputCallbackInfo| {
                frames.clear();
                pipeline.process_into(data, &mut frames);
                for frame in frames.drain(..) {
                    let rms = rms_i16(&frame);
                    stats.rms_bits.store(rms.to_bits(), Ordering::Relaxed);
                    match tx.send(frame) {
                        Ok(_) => {
                            stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            stats.frames_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            },
            err_cb,
            None, // timeout：不等待，由 cpal 自己阻塞
        )
        .map_err(|e| AudioError::Device(format!("构建输入流失败: {e}")))
}

/// 按设备上报的样本格式分派到 [`build_input_stream`]。
fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: SampleFormat,
    pipeline: CapturePipeline,
    tx: tokio::sync::broadcast::Sender<Vec<i16>>,
    stats: Arc<CaptureStats>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<cpal::Stream, AudioError> {
    macro_rules! dispatch {
        ($($fmt:pat => $ty:ty),* $(,)?) => {
            match sample_format {
                $($fmt => build_stream_typed::<$ty>(
                    device, config, sample_format, pipeline, tx, stats, last_error,
                ),)*
                other => Err(AudioError::UnsupportedSampleFormat(other.to_string())),
            }
        };
    }
    dispatch! {
        SampleFormat::I8 => i8,
        SampleFormat::I16 => i16,
        SampleFormat::I24 => cpal::I24,
        SampleFormat::I32 => i32,
        SampleFormat::I64 => i64,
        SampleFormat::U8 => u8,
        SampleFormat::U16 => u16,
        SampleFormat::U24 => cpal::U24,
        SampleFormat::U32 => u32,
        SampleFormat::U64 => u64,
        SampleFormat::F32 => f32,
        SampleFormat::F64 => f64,
    }
}

/// `Mutex::lock` 的未中毒版本：中毒时取回内部数据而不是 panic（音频线程 panic 不该传染）。
pub(crate) fn lock_unpoisoned<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{i16_to_f32, rms_i16};

    // ---------- 下混 ----------

    #[test]
    fn downmix_mono_is_passthrough() {
        let input = [0.1f32, -0.5, 0.25];
        let mut out = Vec::new();
        downmix_to_mono(&input, 1, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn downmix_stereo_averages_channels() {
        // L/R 交织：帧0 = (0.5, -0.5) -> 0；帧1 = (1.0, 0.0) -> 0.5
        let input = [0.5f32, -0.5, 1.0, 0.0];
        let mut out = Vec::new();
        downmix_to_mono(&input, 2, &mut out);
        assert_eq!(out, vec![0.0, 0.5]);
    }

    #[test]
    fn downmix_4ch_and_partial_frame() {
        let input = [0.4f32, 0.2, 0.0, -0.2, 1.0, 1.0, 1.0, 1.0, 9.9];
        let mut out = Vec::new();
        downmix_to_mono(&input, 4, &mut out);
        // 两个整帧；最后 9.9 属于不完整帧，丢弃
        assert_eq!(out.len(), 2);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert_eq!(out[1], 1.0);
    }

    #[test]
    fn downmix_all_zero_channels_does_not_panic() {
        let mut out = Vec::new();
        downmix_to_mono(&[0.1f32, 0.2, 0.3], 0, &mut out);
        // channels 被 max(1) 兜底，等价单声道
        assert_eq!(out.len(), 3);
    }

    // ---------- ToF32 ----------

    #[test]
    fn to_f32_sample_conversions() {
        assert_eq!(0i16.to_f32_sample(), 0.0);
        assert_eq!((-32768i16).to_f32_sample(), -1.0);
        assert!((32767i16.to_f32_sample() - 0.999_969_7).abs() < 1e-6);
        assert_eq!(0i8.to_f32_sample(), 0.0);
        assert_eq!((-128i8).to_f32_sample(), -1.0);
        assert_eq!(128u8.to_f32_sample(), 0.0);
        assert_eq!(0u8.to_f32_sample(), -1.0);
        assert_eq!(32768u16.to_f32_sample(), 0.0);
        assert_eq!(0u16.to_f32_sample(), -1.0);
        assert_eq!(0i32.to_f32_sample(), 0.0);
        assert!((i32::MAX.to_f32_sample() - 1.0).abs() < 1e-6);
        assert_eq!(0.25f32.to_f32_sample(), 0.25);
        assert_eq!(0.25f64.to_f32_sample(), 0.25);
        // 24bit
        assert_eq!(<cpal::I24 as cpal::FromSample<i16>>::from_sample_(0i16).to_f32_sample(), 0.0);
        assert!((<cpal::I24 as cpal::FromSample<i16>>::from_sample_(16384i16).to_f32_sample() - 0.5).abs() < 1e-3);
        assert!((<cpal::U24 as cpal::FromSample<i16>>::from_sample_(16384i16).to_f32_sample() - 0.5).abs() < 1e-3);
    }

    // ---------- 重采样 ----------

    /// 生成正弦波（mono f32）。
    fn sine(freq: f64, rate: u32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| {
                amp * (std::f64::consts::TAU * freq * i as f64 / f64::from(rate)).sin() as f32
            })
            .collect()
    }

    /// 取中段的峰峰值，避开首尾瞬态。
    fn peak_to_peak(samples: &[f32]) -> f32 {
        let skip = samples.len() / 4;
        let body = &samples[skip..samples.len() - skip.max(1)];
        if body.is_empty() {
            return 0.0;
        }
        let max = body.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min = body.iter().cloned().fold(f32::INFINITY, f32::min);
        max - min
    }

    #[test]
    fn resampler_same_rate_is_passthrough() {
        let mut r = LinearResampler::new(16_000, 16_000);
        let input: Vec<f32> = (0..320).map(|i| i as f32 / 320.0).collect();
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out, input);
        assert_eq!(r.from_rate(), 16_000);
        assert_eq!(r.to_rate(), 16_000);
    }

    #[test]
    fn resampler_48k_to_16k_keeps_dc() {
        let mut r = LinearResampler::new(48_000, 16_000);
        let input = vec![0.5f32; 4_800]; // 100ms DC
        let mut out = Vec::new();
        r.process(&input, &mut out);
        assert_eq!(out.len(), 1_600, "48k→16k 应精确 3:1");
        // 低通对 DC 增益为 1（首几个样本有爬升，跳过）
        for &v in &out[10..] {
            assert!((v - 0.5).abs() < 1e-3, "DC 被改变: {v}");
        }
    }

    #[test]
    fn resampler_aligns_to_16k_frame_320() {
        // 48kHz 采 20ms = 960 样本 → 16k 应正好 320 样本
        let mut r = LinearResampler::new(48_000, 16_000);
        let mut out = Vec::new();
        r.process(&vec![0.1f32; 960], &mut out);
        assert_eq!(out.len(), 320);
    }

    #[test]
    fn resampler_44k1_to_16k_no_drift_over_chunks() {
        let mut r = LinearResampler::new(44_100, 16_000);
        let chunk = 441; // 10ms @44.1k
        let mut out = Vec::new();
        let tone = sine(440.0, 44_100, chunk, 0.5);
        for _ in 0..100 {
            r.process(&tone, &mut out);
        }
        // 100 × 10ms → 1s → 16000 样本，允许 ±2 的取整误差
        let expect = 16_000i64;
        let diff = (out.len() as i64 - expect).abs();
        assert!(diff <= 2, "漂移过大: out={} expect≈{}", out.len(), expect);
    }

    #[test]
    fn resampler_16k_to_32k_doubles_length() {
        let mut r = LinearResampler::new(16_000, 32_000);
        let input = sine(1000.0, 16_000, 160, 0.8);
        let mut out = Vec::new();
        r.process(&input, &mut out);
        // 流式语义：末尾需要"下一块第一个样本"的输出会被推迟，最多少 2 个（1/step）
        assert!((out.len() as i64 - 320).abs() <= 2, "len={}", out.len());
        // 峰值幅度应保持（线性插值不改变包络）
        let pp_in = peak_to_peak(&input);
        let pp_out = peak_to_peak(&out);
        assert!((pp_out - pp_in).abs() < 0.05, "in={pp_in} out={pp_out}");
    }

    #[test]
    fn resampler_keeps_speech_band_amplitude() {
        // 1kHz @48k → 16k：远低于 8k 奈奎斯特，幅度应基本不变
        let mut r = LinearResampler::new(48_000, 16_000);
        let input = sine(1000.0, 48_000, 48_000, 0.5); // 1s
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let pp = peak_to_peak(&out);
        assert!((pp - 1.0).abs() < 0.1, "1kHz 峰峰值应≈1.0，实际 {pp}");
    }

    #[test]
    fn resampler_attenuates_aliasing_band() {
        // 10kHz @48k → 16k：会被折到 6kHz，低通应明显压制
        let mut r = LinearResampler::new(48_000, 16_000);
        let input = sine(10_000.0, 48_000, 48_000, 0.5);
        let mut out = Vec::new();
        r.process(&input, &mut out);
        let pp = peak_to_peak(&out);
        assert!(pp < 0.8, "10kHz 未被压制，峰峰值 {pp}（1kHz 时为 1.0）");
    }

    #[test]
    fn resampler_empty_and_tiny_chunks() {
        let mut r = LinearResampler::new(48_000, 16_000);
        let mut out = Vec::new();
        r.process(&[], &mut out);
        assert!(out.is_empty());
        // 1 个样本不该 panic
        r.process(&[0.3], &mut out);
        assert!(out.len() <= 1);
        assert_eq!(r.estimate_out_len(0), 2);
    }

    #[test]
    fn resampler_chunked_equals_one_shot() {
        // 同一段信号，分块喂 vs 一次喂，结果应完全一致（状态跨块连续）
        let input = sine(880.0, 44_100, 10_000, 0.4);
        let mut whole = Vec::new();
        LinearResampler::new(44_100, 16_000).process(&input, &mut whole);

        let mut r = LinearResampler::new(44_100, 16_000);
        let mut chunked = Vec::new();
        for chunk in input.chunks(97) {
            r.process(chunk, &mut chunked);
        }
        assert_eq!(whole.len(), chunked.len());
        for (i, (a, b)) in whole.iter().zip(chunked.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "第 {i} 个样本不一致: {a} vs {b}");
        }
    }

    // ---------- 分帧 ----------

    #[test]
    fn assembler_makes_exact_20ms_frames() {
        let mut a = FrameAssembler::new(FRAME_SAMPLES);
        let frames = a.push(&vec![7i16; 640]);
        assert_eq!(frames.len(), 2);
        assert!(frames.iter().all(|f| f.len() == 320));
        assert_eq!(a.pending_len(), 0);
        // 每帧 640 字节（I5）
        assert_eq!(frames[0].len() * 2, 640);
    }

    #[test]
    fn assembler_carries_partial_remainder() {
        let mut a = FrameAssembler::new(320);
        // 700 = 2 整帧(640) + 60 余量
        assert_eq!(a.push(&vec![1i16; 700]).len(), 2);
        assert_eq!(a.pending_len(), 60, "余量必须带到下一次调用");

        // 再补 60 只有 120，还不够一帧（这正是不丢样本的证明）
        assert!(a.push(&vec![2i16; 60]).is_empty());
        assert_eq!(a.pending_len(), 120);

        // 补满剩下 200 → 凑出第 3 帧，且前 60 个是 1、后 260 个是 2（顺序与边界正确）
        let frames = a.push(&vec![2i16; 200]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        assert!(frames[0][..60].iter().all(|&x| x == 1), "余量顺序错乱");
        assert!(frames[0][60..].iter().all(|&x| x == 2));
        assert_eq!(a.pending_len(), 0);
        assert_eq!(a.frame_samples(), 320);
        a.reset();
        assert_eq!(a.pending_len(), 0);
    }

    #[test]
    fn assembler_zero_frame_size_is_safe() {
        let mut a = FrameAssembler::new(0);
        assert!(a.push(&[1i16; 100]).is_empty());
        assert_eq!(a.pending_len(), 0);
    }

    // ---------- 整条流水线 ----------

    #[test]
    fn pipeline_48k_stereo_produces_16k_mono_frames() {
        // 100ms、48k、双声道正弦；左+右同相 → 下混幅度不变
        let n = 4_800;
        let left = sine(500.0, 48_000, n, 0.5);
        let mut interleaved = Vec::with_capacity(n * 2);
        for v in &left {
            interleaved.push(*v);
            interleaved.push(*v);
        }
        let mut p = CapturePipeline::new(48_000, 2, CaptureConfig::default()).unwrap();
        let frames = p.process(&interleaved);
        // 100ms / 20ms = 5 帧
        assert_eq!(frames.len(), 5);
        for f in &frames {
            assert_eq!(f.len(), FRAME_SAMPLES);
            assert_eq!(f.len() * 2, crate::FRAME_BYTES);
        }
        // RMS 应约为 0.5/√2 ≈ 0.354（正弦）
        let r = rms_i16(&frames[2]);
        assert!((r - 0.354).abs() < 0.05, "rms={r}");
        assert_eq!(p.device_rate(), 48_000);
        assert_eq!(p.channels(), 2);
        assert_eq!(p.frames_out(), 5);
        assert!(p.last_rms() > 0.1);
    }

    #[test]
    fn pipeline_16k_mono_is_direct() {
        let input = sine(1000.0, 16_000, 320, 0.5);
        let mut p = CapturePipeline::new(16_000, 1, CaptureConfig::default()).unwrap();
        let frames = p.process(&input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].len(), 320);
        // 逐样本检查转换（16k→16k 直通，只差量化）
        for (i, (&f32_sample, &i16_sample)) in
            input.iter().zip(frames[0].iter()).enumerate()
        {
            let want = crate::audio::f32_to_i16(f32_sample);
            assert!(
                (i16_sample as i32 - want as i32).abs() <= 1,
                "第 {i} 个样本 {i16_sample} vs {want}"
            );
            // 同时确认 i16_to_f32 往返
            assert!((i16_to_f32(i16_sample) - f32_sample).abs() < 1e-3);
        }
    }

    #[test]
    fn pipeline_rejects_invalid_config() {
        assert!(matches!(
            CapturePipeline::new(0, 1, CaptureConfig::default()),
            Err(AudioError::InvalidConfig(_))
        ));
        assert!(matches!(
            CapturePipeline::new(48_000, 0, CaptureConfig::default()),
            Err(AudioError::InvalidConfig(_))
        ));
        assert!(matches!(
            CapturePipeline::new(
                48_000,
                1,
                CaptureConfig {
                    sample_rate: 0,
                    frame_samples: 320
                }
            ),
            Err(AudioError::InvalidConfig(_))
        ));
        assert!(matches!(
            CapturePipeline::new(
                48_000,
                1,
                CaptureConfig {
                    sample_rate: 16_000,
                    frame_samples: 0
                }
            ),
            Err(AudioError::InvalidConfig(_))
        ));
    }

    #[test]
    fn pipeline_accumulates_across_callbacks() {
        // cpal 回调块大小不固定：把 1s 音频拆成不均匀的块，总帧数应仍为 50
        let mut p = CapturePipeline::new(48_000, 1, CaptureConfig::default()).unwrap();
        let input = sine(300.0, 48_000, 48_000, 0.3);
        let mut total_frames = 0;
        let sizes = [1usize, 7, 960, 13, 2048, 37];
        let mut idx = 0;
        let mut i = 0;
        while i < input.len() {
            let n = sizes[idx % sizes.len()].min(input.len() - i);
            total_frames += p.process(&input[i..i + n]).len();
            i += n;
            idx += 1;
        }
        assert_eq!(total_frames, 50, "1s / 20ms 应为 50 帧");
        assert_eq!(p.frames_out(), 50);
    }

    #[test]
    fn pipeline_reset_clears_state() {
        let mut p = CapturePipeline::new(48_000, 1, CaptureConfig::default()).unwrap();
        p.process(&sine(440.0, 48_000, 500, 0.5));
        p.reset();
        assert_eq!(p.pending_len(), 0);
        assert_eq!(p.last_rms(), 0.0);
        // reset 后帧计数不清零（累计统计），但流水线可以继续工作
        let frames = p.process(&sine(440.0, 48_000, 960, 0.5));
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn pipeline_custom_config_10ms_frames() {
        let cfg = CaptureConfig {
            sample_rate: 16_000,
            frame_samples: 160, // 10ms
        };
        let mut p = CapturePipeline::new(48_000, 1, cfg).unwrap();
        let frames = p.process(&sine(500.0, 48_000, 1_920, 0.5)); // 40ms
        assert_eq!(frames.len(), 4);
        assert!(frames.iter().all(|f| f.len() == 160));
        assert!((cfg.frame_ms() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn config_defaults_match_uplink() {
        let cfg = CaptureConfig::default();
        assert_eq!(cfg.sample_rate, 16_000);
        assert_eq!(cfg.frame_samples, 320);
        assert!((cfg.frame_ms() - 20.0).abs() < 1e-9);
    }

    // ---------- 设备无关性 ----------

    #[test]
    fn capture_new_does_not_open_device() {
        // 构造 + 订阅不能碰设备（本机没有声卡，碰了会在 CI 上失败）
        let cap = AudioCapture::new();
        assert!(!cap.is_running());
        assert_eq!(cap.rms(), 0.0);
        assert_eq!(cap.frames_sent(), 0);
        assert!(cap.format().is_none());
        assert!(cap.last_error().is_none());
        let _rx = cap.subscribe();
        assert_eq!(cap.config().frame_samples, 320);
        // 未启动时 stop 是空操作
        cap.stop().unwrap();
        assert!(!cap.is_running());
    }

    #[test]
    fn broadcast_frames_reach_subscriber() {
        // 校验帧通道语义（不经过设备）：tokio broadcast 可同步 try_recv
        let cap = AudioCapture::new();
        let mut rx = cap.subscribe();
        let frame = vec![0i16; FRAME_SAMPLES];
        // 手工发一帧，确认订阅者能取到同样的数据
        cap.tx.send(frame.clone()).unwrap();
        assert_eq!(rx.try_recv().unwrap(), frame);
    }
}
