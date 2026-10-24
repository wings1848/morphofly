//! 播放：rodio 0.23（DeviceSinkBuilder / MixerDeviceSink / Player）+ 自有环形缓冲。
//!
//! # 为什么不用 `Player::append(每块 PCM)`
//!
//! 打断（barge-in，`docs/开发方案.md` 4.1）要求**立即静音**（不变量 I4）。
//! rodio 的 `Player::clear()` 会把已入队的 source 逐个跳过，`sleep_until_end()` 会阻塞到
//! 队尾结束 —— 不是"立即"，而且在极端情况下可能等很久。
//!
//! 这里改成**一条常驻的流式 [`RingSource`]**：`open()` 时只 `append` 一次，之后所有 PCM
//! 都写进 [`SampleRing`]（`Arc<Mutex<..>>`）。音频线程每个样本都检查一个 generation 计数，
//! `stop_now()` 只需 `clear()` + generation+1，音频线程在**下一个样本**就静音 —— 真正即时，
//! 且完全不阻塞调用方。
//!
//! # 采样率
//!
//! `RingSource` 以固定的 `nominal_rate`（默认 24kHz，Qwen / StepFun pcm16 的常见输出率）
//! 向 rodio 输出，rodio 的 mixer 再转到设备实际采样率/声道数。上游给的 PCM 若是别的采样率
//! （如 16k），先用 [`super::capture::LinearResampler`] 转到 `nominal_rate` 再入环。

use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::source::Source;
use rodio::{ChannelCount, DeviceSinkBuilder, MixerDeviceSink, Player, SampleRate};

use super::aec::AecProcessor;
use super::capture::LinearResampler;
use super::ring::SampleRing;
use super::{f32_to_i16, pcm16_to_f32, AudioError, DOWNLINK_SAMPLE_RATE, JITTER_MAX_MS};

/// 环内一次批量取样的默认块大小（减少锁竞争）。
const REFILL_BLOCK: usize = 512;

/// 播放侧环形缓冲的共享状态：环 + 打断代数 + 统计。
///
/// 这是播放的**纯逻辑核心**，不依赖任何音频设备，因此可以完整单测。
#[derive(Debug)]
pub struct PlaybackQueue {
    ring: Mutex<SampleRing>,
    /// 打断代数：`stop_now()` 时 +1；[`RingSource`] 见到变化就丢弃本地预取的样本。
    generation: AtomicU64,
    /// 累计写入样本数（nominal rate 下）。
    pushed: AtomicU64,
    /// 累计被音频线程取走的样本数。
    pulled: AtomicU64,
    /// nominal 采样率。
    rate: u32,
}

impl PlaybackQueue {
    /// 按「采样率 + 最大抖动余量」建队列。`rate == 0` 时退化为零容量（不 panic）。
    pub fn new(rate: u32, max_jitter_ms: u32) -> Self {
        let cap = if rate == 0 {
            0
        } else {
            ((u64::from(rate) * u64::from(max_jitter_ms)) / 1000) as usize
        };
        Self {
            ring: Mutex::new(SampleRing::new(cap)),
            generation: AtomicU64::new(0),
            pushed: AtomicU64::new(0),
            pulled: AtomicU64::new(0),
            rate,
        }
    }

    /// nominal 采样率。
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// 当前排队的样本（帧）数。
    pub fn queued_frames(&self) -> u64 {
        lock(&self.ring).len() as u64
    }

    /// 队列容量（样本数）。
    pub fn capacity(&self) -> usize {
        lock(&self.ring).capacity()
    }

    /// 当前填充对应的毫秒数（抗抖动余量的直观读数）。
    pub fn jitter_ms(&self) -> f64 {
        if self.rate == 0 {
            return 0.0;
        }
        self.queued_frames() as f64 * 1000.0 / f64::from(self.rate)
    }

    /// 累计写入样本数。
    pub fn frames_pushed(&self) -> u64 {
        self.pushed.load(Ordering::Relaxed)
    }

    /// 累计被音频线程取走的样本数。
    pub fn frames_pulled(&self) -> u64 {
        self.pulled.load(Ordering::Relaxed)
    }

    /// 是否已被丢弃过旧样本（写满环）。
    pub fn overwritten(&self) -> u64 {
        lock(&self.ring).overwritten()
    }

    /// 最近一次打断的代数。
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// 写入一批样本（nominal rate）并返回实际写入数。
    pub fn push(&self, samples: &[f32]) -> usize {
        if samples.is_empty() {
            return 0;
        }
        let mut ring = lock(&self.ring);
        let n = ring.extend_from_slice(samples);
        drop(ring);
        self.pushed.fetch_add(n as u64, Ordering::Relaxed);
        n
    }

    /// **立即打断**：清空排队样本，并让音频线程丢掉本地预取。
    ///
    /// 不阻塞、不调用任何 rodio API；调用返回时音频线程最多再放出一个样本就会静音。
    pub fn stop_now(&self) {
        {
            let mut ring = lock(&self.ring);
            ring.clear();
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// 取一批样本给音频线程（[`RingSource`] 用）。
    fn pull_into(&self, out: &mut [f32]) -> usize {
        let n = {
            let mut ring = lock(&self.ring);
            ring.pop_into(out)
        };
        if n > 0 {
            self.pulled.fetch_add(n as u64, Ordering::Relaxed);
        }
        n
    }
}

/// rodio 的流式 source：从 [`PlaybackQueue`] 拉样本，空则输出静音（不结束）。
///
/// 公开是为了让单元测试能直接驱动 [`Iterator::next`]，无需真实设备。
#[derive(Debug)]
pub struct RingSource {
    queue: Arc<PlaybackQueue>,
    nominal_rate: SampleRate,
    local: Vec<f32>,
    pos: usize,
    local_gen: u64,
}

impl RingSource {
    /// 建一个流式 source（`rate` 为 0 时用默认值兜底，避免 NonZero panic）。
    pub fn new(queue: Arc<PlaybackQueue>, rate: u32) -> Self {
        let nominal_rate = NonZero::new(rate)
            .unwrap_or_else(|| NonZero::new(DOWNLINK_SAMPLE_RATE).expect("非零常量"));
        Self {
            local_gen: queue.generation(),
            queue,
            nominal_rate,
            local: Vec::with_capacity(REFILL_BLOCK),
            pos: 0,
        }
    }

    /// 从环里补充本地预取缓冲。
    fn refill(&mut self) {
        self.local.clear();
        self.local.resize(REFILL_BLOCK, 0.0);
        let n = self.queue.pull_into(&mut self.local);
        self.local.truncate(n);
        self.pos = 0;
    }
}

impl Iterator for RingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        // 每一拍都检查打断代数：这是"立即静音"的关键（原子读，代价可忽略）
        let gen = self.queue.generation();
        if gen != self.local_gen {
            self.local.clear();
            self.pos = 0;
            self.local_gen = gen;
        }
        if self.pos >= self.local.len() {
            self.refill();
        }
        if self.pos < self.local.len() {
            let v = self.local[self.pos];
            self.pos += 1;
            Some(v)
        } else {
            // 队列空：输出静音而不是结束，保证打断后还能继续说话
            Some(0.0)
        }
    }
}

impl Source for RingSource {
    fn current_span_len(&self) -> Option<usize> {
        // None = 无限长，不重配采样率/声道
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(1).expect("mono 非零")
    }

    fn sample_rate(&self) -> SampleRate {
        self.nominal_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// 把任意采样率的 i16 PCM 转成 nominal rate 的 f32 mono（requests 侧的重采样）。
///
/// 纯函数 + 显式状态：`state` 里的重采样器在采样率变化时重建。
#[derive(Debug, Default)]
pub struct AppendConverter {
    state: Option<(u32, LinearResampler)>,
    scratch: Vec<f32>,
}

impl AppendConverter {
    /// 新建。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前跟踪的输入采样率。
    pub fn input_rate(&self) -> Option<u32> {
        self.state.as_ref().map(|(r, _)| *r)
    }

    /// 转换一段 i16 PCM。返回 `nominal_rate` 下的 f32 mono 样本。
    ///
    /// 采样率相同则直通（只做 i16→f32）；不同则先线性重采样。
    pub fn convert(&mut self, pcm_i16: &[i16], sample_rate: u32, nominal_rate: u32) -> Vec<f32> {
        if pcm_i16.is_empty() {
            return Vec::new();
        }
        // 采样率变化时重建重采样器，避免把上一段的状态带进来
        let need_rebuild = match &self.state {
            Some((rate, _)) => *rate != sample_rate,
            None => true,
        };
        if need_rebuild {
            self.state = Some((sample_rate, LinearResampler::new(sample_rate, nominal_rate)));
        }

        if sample_rate == nominal_rate || sample_rate == 0 || nominal_rate == 0 {
            return pcm16_to_f32(pcm_i16);
        }

        self.scratch.clear();
        self.scratch.reserve(pcm_i16.len());
        self.scratch
            .extend(pcm_i16.iter().map(|&s| super::i16_to_f32(s)));

        let mut out = Vec::new();
        if let Some((_, resampler)) = self.state.as_mut() {
            resampler.process(&self.scratch, &mut out);
        }
        out
    }

    /// 重置状态（新一段语音开始时调用）。
    pub fn reset(&mut self) {
        self.state = None;
        self.scratch.clear();
    }
}

/// 播放设备信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFormat {
    /// 设备名（rodio 不直接暴露，这里留空或由调用方填）。
    pub device_sample_rate: u32,
    /// 设备声道数。
    pub channels: u16,
    /// 本层向 rodio 输出的 nominal 采样率。
    pub nominal_rate: u32,
}

/// rodio 扬声器播放器：可追加任意采样率 PCM、可立即打断、可报告队列深度。
///
/// 用法：
/// ```no_run
/// # use morphofly_core::audio::playback::AudioPlayer;
/// let p = AudioPlayer::open().expect("打开扬声器"); // 无设备时 Err，不 panic
/// p.append_pcm16(&vec![0i16; 480], 24_000);         // 20ms @24k
/// assert!(p.queued_frames() > 0);
/// p.stop_now();                                     // 打断：下一个样本就静音
/// assert_eq!(p.queued_frames(), 0);
/// ```
pub struct AudioPlayer {
    // 字段顺序即 drop 顺序：先 drop player 再 drop sink（sink 关掉流后 player 无意义）
    player: Player,
    _sink: MixerDeviceSink,
    queue: Arc<PlaybackQueue>,
    nominal_rate: u32,
    format: OutputFormat,
    converter: Mutex<AppendConverter>,
    volume: Mutex<f32>,
    /// 回声消除的参考信号入口（`None` = 关闭）。
    ///
    /// 必须和采集侧是**同一个** AEC 实例，否则参考信号对不上，等于没做。
    /// 默认取 [`super::aec::shared()`]，可用 [`AudioPlayer::set_aec`] 运行中替换。
    aec: Mutex<Option<Arc<AecProcessor>>>,
}

impl std::fmt::Debug for AudioPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioPlayer")
            .field("format", &self.format)
            .field("queued_frames", &self.queued_frames())
            .finish_non_exhaustive()
    }
}

impl AudioPlayer {
    /// 用默认 nominal 采样率（[`DOWNLINK_SAMPLE_RATE`] = 24kHz）打开默认输出设备。
    ///
    /// # Errors
    /// - [`AudioError::NoOutputDevice`]：没有扬声器。
    /// - [`AudioError::Device`]：后端起不来（设备被占用等）。
    pub fn open() -> Result<Self, AudioError> {
        Self::open_with_rate(DOWNLINK_SAMPLE_RATE)
    }

    /// 指定 nominal 采样率打开输出设备。
    ///
    /// 设备选择顺序：
    /// 1. 环境变量 `MORPHOFLY_OUTPUT_DEVICE`（配置文件的 `[audio] output_device` 会在
    ///    启动时映射到这个变量）→ 按名字精确/子串匹配，见 [`super::device`]；
    /// 2. 都没配或找不到 → 系统默认 sink。
    ///
    /// ⚠️ 为什么不能只信「系统默认」：2026-10-25 真机踩坑 —— 默认 sink 被改回板载
    /// 喇叭（不出声），模型回了 17 秒语音用户一个字都听不到，日志却完全正常。
    /// 也不能只靠外部先改默认 sink（`tools/pc_audio_route.sh` 那套）：别的东西会再改回去。
    pub fn open_with_rate(nominal_rate: u32) -> Result<Self, AudioError> {
        if nominal_rate == 0 {
            return Err(AudioError::InvalidConfig("nominal 采样率为 0".into()));
        }
        let mut sink = match super::device::wanted_output() {
            Some(want) => match super::device::find_output(&want) {
                Some((dev, entry)) => {
                    tracing::info!(
                        wanted = %want,
                        device = %entry.display(),
                        "输出设备：按配置指定（不跟随系统默认）"
                    );
                    DeviceSinkBuilder::from_device(dev)
                        .and_then(|b| b.open_stream())
                        .map_err(|e| map_sink_error(e, true))?
                }
                // 配置写错不该让 App 变哑巴：告警 + 回默认。
                None => {
                    tracing::warn!(
                        wanted = %want,
                        available = ?super::device::output_labels(),
                        "指定的输出设备不存在，回退系统默认"
                    );
                    DeviceSinkBuilder::open_default_sink().map_err(|e| map_sink_error(e, true))?
                }
            },
            None => DeviceSinkBuilder::open_default_sink().map_err(|e| map_sink_error(e, true))?,
        };
        sink.log_on_drop(false); // 别在 stderr 刷 "Dropping DeviceSink"

        let cfg = *sink.config();
        let format = OutputFormat {
            device_sample_rate: cfg.sample_rate().get(),
            channels: cfg.channel_count().get(),
            nominal_rate,
        };

        let player = Player::connect_new(sink.mixer());
        let queue = Arc::new(PlaybackQueue::new(nominal_rate, JITTER_MAX_MS));
        player.append(RingSource::new(queue.clone(), nominal_rate));

        Ok(Self {
            player,
            _sink: sink,
            queue,
            nominal_rate,
            format,
            converter: Mutex::new(AppendConverter::new()),
            volume: Mutex::new(1.0),
            aec: Mutex::new(super::aec::shared()),
        })
    }

    /// 设备实际格式与 nominal 采样率。
    pub fn format(&self) -> &OutputFormat {
        &self.format
    }

    /// 是否已打开设备（总是 true —— 构造成功即有设备）。
    pub fn is_open(&self) -> bool {
        true
    }

    /// 当前排队帧数（nominal rate 下的样本数；空 = 已放完）。
    pub fn queued_frames(&self) -> u64 {
        self.queue.queued_frames()
    }

    /// 队列对应的毫秒数。
    pub fn queued_ms(&self) -> f64 {
        self.queue.jitter_ms()
    }

    /// 队列容量（毫秒，抗抖动上限）。
    pub fn capacity_ms(&self) -> f64 {
        if self.nominal_rate == 0 {
            return 0.0;
        }
        self.queue.capacity() as f64 * 1000.0 / f64::from(self.nominal_rate)
    }

    /// 队列是否为空。
    pub fn is_empty(&self) -> bool {
        self.queued_frames() == 0
    }

    /// 累计写入的帧数。
    pub fn frames_pushed(&self) -> u64 {
        self.queue.frames_pushed()
    }

    /// 累计被音频线程取走的帧数。
    pub fn frames_pulled(&self) -> u64 {
        self.queue.frames_pulled()
    }

    /// 追加 i16 PCM（任意采样率）。
    ///
    /// 内部先把 `sample_rate` 转到 nominal rate（24k），再写入环。返回写入的样本数
    /// （nominal rate 下）。
    pub fn append_pcm16(&self, pcm: &[i16], sample_rate: u32) -> usize {
        if pcm.is_empty() {
            return 0;
        }
        let converted = {
            let mut conv = lock(&self.converter);
            conv.convert(pcm, sample_rate, self.nominal_rate)
        };
        // ★ 回声消除的参考信号：把「即将送扬声器的这一路」tee 一份给 AEC。
        //
        //   为什么在 `append_pcm16` 而不是别处：这是**真正进扬声器**的唯一入口
        //   （`append_pcm_f32` 最终也会走到这里），所以这里 tee 能保证参考信号
        //   和实际播放内容严格一致 —— 在 `RingSource` 里 tee 反而更差，因为
        //   那里取的是「队列里还有什么」，且被 stop_now 清空后参考信号会断。
        //
        //   为什么在 `queue.push` **之前**：AEC3 假设 render 块先于对应的
        //   capture 块到达（它靠这个判断「这块参考信号麦克风还没听到」）。
        //   先推队列再喂 AEC 的话，参考信号相对采集永远晚一拍。
        //
        //   tee 的采样率是 `nominal_rate`（通常 24k），AEC 内部会重采样到 16k
        //   —— 它自带 `LinearResampler`，这里不用再转一次。
        if let Some(aec) = lock(&self.aec).as_ref() {
            aec.process_render_at(&converted, self.nominal_rate);
        }
        self.queue.push(&converted)
    }

    /// 追加 f32 PCM（任意采样率，值域 `[-1,1]`）。
    pub fn append_pcm_f32(&self, pcm: &[f32], sample_rate: u32) -> usize {
        if pcm.is_empty() {
            return 0;
        }
        if sample_rate == self.nominal_rate || sample_rate == 0 {
            return self.queue.push(pcm);
        }
        // f32 路径：先量化到 i16 走同一条重采样链，保证行为一致
        let as_i16: Vec<i16> = pcm.iter().map(|&s| f32_to_i16(s)).collect();
        self.append_pcm16(&as_i16, sample_rate)
    }

    /// 直接播放一整段（便捷：内部 append + 等待放完）。
    pub fn play_pcm16_blocking(&self, pcm: &[i16], sample_rate: u32) {
        self.append_pcm16(pcm, sample_rate);
        let total = self.frames_pushed();
        while self.frames_pulled() < total {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// **立即打断**（不变量 I4）：清空播放队列，下一个样本起静音。
    ///
    /// 不阻塞、不调用 rodio 的 clear/stop（那些会等队尾）。
    pub fn stop_now(&self) {
        self.queue.stop_now();
        // 下一段语音的采样率可能不同，重置重采样状态
        lock(&self.converter).reset();
    }

    /// 当前音量（1.0 = 原始）。
    pub fn volume(&self) -> f32 {
        *lock(&self.volume)
    }

    /// 设置音量（0.0 – 1.0 之外会被 rodio 按倍数放大，不做裁剪）。
    pub fn set_volume(&self, v: f32) {
        let clamped = v.clamp(0.0, 4.0);
        *lock(&self.volume) = clamped;
        self.player.set_volume(clamped);
    }

    /// 播放侧环形缓冲的引用（诊断 / 频谱 reference 通道用）。
    pub fn queue(&self) -> Arc<PlaybackQueue> {
        self.queue.clone()
    }
}

/// rodio 的 `DeviceSinkError` → [`AudioError`]。
fn map_sink_error(err: rodio::DeviceSinkError, output: bool) -> AudioError {
    match err {
        rodio::DeviceSinkError::NoDevice => {
            if output {
                AudioError::NoOutputDevice
            } else {
                AudioError::NoInputDevice
            }
        }
        other => AudioError::Device(format!("打开音频设备失败: {other}")),
    }
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
    use crate::audio::JITTER_MIN_MS;

    // ---------- 纯逻辑：PlaybackQueue ----------

    #[test]
    fn queue_push_pop_and_depth() {
        let q = PlaybackQueue::new(24_000, 500);
        assert_eq!(q.queued_frames(), 0);
        assert_eq!(q.capacity(), 12_000);

        let n = q.push(&vec![0.5f32; 480]);
        assert_eq!(n, 480);
        assert_eq!(q.queued_frames(), 480);
        assert!((q.jitter_ms() - 20.0).abs() < 1e-9);
        assert_eq!(q.frames_pushed(), 480);
        assert_eq!(q.frames_pulled(), 0);

        let mut out = [0.0f32; 100];
        assert_eq!(q.pull_into(&mut out), 100);
        assert_eq!(q.queued_frames(), 380);
        assert_eq!(q.frames_pulled(), 100);
        assert_eq!(out[0], 0.5);
    }

    #[test]
    fn queue_stop_now_is_immediate_i4() {
        // 不变量 I4：塞满队列 → stop_now → 深度必须是 0
        let q = PlaybackQueue::new(24_000, 500);
        q.push(&vec![0.1f32; 8_000]);
        assert_eq!(q.queued_frames(), 8_000);
        let gen_before = q.generation();

        q.stop_now();

        assert_eq!(q.queued_frames(), 0, "stop_now 后队列必须为 0");
        assert_eq!(q.generation(), gen_before + 1);
        assert!(q.is_empty_for_test());
        // 幂等：再打一次也不出事
        q.stop_now();
        assert_eq!(q.queued_frames(), 0);
    }

    impl PlaybackQueue {
        fn is_empty_for_test(&self) -> bool {
            self.queued_frames() == 0
        }
    }

    #[test]
    fn queue_bounded_overwrites_oldest() {
        let q = PlaybackQueue::new(1_000, 10); // 10 样本
        assert_eq!(q.capacity(), 10);
        q.push(&[1.0f32; 25]);
        assert_eq!(q.queued_frames(), 10, "有界：最多 10");
        assert_eq!(q.overwritten(), 15);
        // 留下的是最新的 10 个（全是 1.0，这里再验证顺序）
        q.stop_now();
        q.push(&[1.0, 2.0, 3.0]);
        let mut out = [0.0f32; 3];
        assert_eq!(q.pull_into(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn queue_zero_rate_is_safe() {
        let q = PlaybackQueue::new(0, 500);
        assert_eq!(q.capacity(), 0);
        assert_eq!(q.rate(), 0);
        assert_eq!(q.jitter_ms(), 0.0);
        q.push(&[0.5, 0.5]);
        assert_eq!(q.queued_frames(), 0);
        assert_eq!(q.overwritten(), 2);
        q.stop_now();
        assert_eq!(q.queued_frames(), 0);
    }

    // ---------- RingSource（无需设备即可驱动） ----------

    #[test]
    fn ring_source_plays_pushed_samples_then_silence() {
        let q = Arc::new(PlaybackQueue::new(24_000, 500));
        let mut src = RingSource::new(q.clone(), 24_000);

        // 空队列 → 静音，且永不结束
        for _ in 0..10 {
            assert_eq!(src.next(), Some(0.0));
        }

        q.push(&[0.25, 0.5, 0.75]);
        assert_eq!(src.next(), Some(0.25));
        assert_eq!(src.next(), Some(0.5));
        assert_eq!(src.next(), Some(0.75));
        // 放完继续静音
        assert_eq!(src.next(), Some(0.0));
    }

    #[test]
    fn ring_source_drops_prefetched_audio_on_stop_i4() {
        // 关键不变量：本地预取里还有样本时，stop_now 必须让下一个 next() 直接静音
        let q = Arc::new(PlaybackQueue::new(24_000, 500));
        let mut src = RingSource::new(q.clone(), 24_000);

        q.push(&vec![0.9f32; 2_000]); // 远超 REFILL_BLOCK，会预取
                                      // 先取一个，触发预取
        assert_eq!(src.next(), Some(0.9));

        q.stop_now();

        // 下一个样本立即静音（预取被 generation 作废）
        assert_eq!(src.next(), Some(0.0), "打断后必须立刻静音");
        assert_eq!(src.next(), Some(0.0));
    }

    #[test]
    fn ring_source_resumes_after_stop() {
        let q = Arc::new(PlaybackQueue::new(24_000, 500));
        let mut src = RingSource::new(q.clone(), 24_000);
        q.push(&[0.1f32; 100]);
        assert_eq!(src.next(), Some(0.1));
        q.stop_now();
        assert_eq!(src.next(), Some(0.0));
        // 新一轮语音可以继续
        q.push(&[0.7, 0.8]);
        assert_eq!(src.next(), Some(0.7));
        assert_eq!(src.next(), Some(0.8));
    }

    #[test]
    fn ring_source_reports_nominal_format() {
        let q = Arc::new(PlaybackQueue::new(24_000, 500));
        let src = RingSource::new(q.clone(), 24_000);
        assert_eq!(src.channels().get(), 1);
        assert_eq!(src.sample_rate().get(), 24_000);
        assert!(src.current_span_len().is_none());
        assert!(src.total_duration().is_none());

        // 16k 输入也按 16k 报告，交给 rodio 转设备率
        let q16 = Arc::new(PlaybackQueue::new(16_000, 500));
        let src16 = RingSource::new(q16, 16_000);
        assert_eq!(src16.sample_rate().get(), 16_000);
    }

    // ---------- AppendConverter（任意采样率 → nominal） ----------

    #[test]
    fn converter_same_rate_passes_through() {
        let mut c = AppendConverter::new();
        let pcm: Vec<i16> = vec![0, 16384, -32768];
        let out = c.convert(&pcm, 24_000, 24_000);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 0.0);
        assert!((out[1] - 0.5).abs() < 1e-6);
        assert_eq!(out[2], -1.0);
        assert_eq!(c.input_rate(), Some(24_000));
    }

    #[test]
    fn converter_16k_to_24k_upsamples() {
        let mut c = AppendConverter::new();
        // 20ms @16k = 320 样本 → 20ms @24k = 480 样本
        let out = c.convert(&vec![1000i16; 320], 16_000, 24_000);
        assert!(
            (out.len() as i64 - 480).abs() <= 2,
            "16k→24k 长度应≈480，实际 {}",
            out.len()
        );
        assert_eq!(c.input_rate(), Some(16_000));
    }

    #[test]
    fn converter_48k_to_24k_downsamples() {
        let mut c = AppendConverter::new();
        let out = c.convert(&vec![1000i16; 960], 48_000, 24_000);
        assert!(
            (out.len() as i64 - 480).abs() <= 2,
            "48k→24k 长度应≈480，实际 {}",
            out.len()
        );
    }

    #[test]
    fn converter_handles_rate_change_between_utterances() {
        let mut c = AppendConverter::new();
        let a = c.convert(&vec![100i16; 240], 24_000, 24_000);
        assert_eq!(a.len(), 240);
        let b = c.convert(&vec![100i16; 320], 16_000, 24_000);
        assert!((b.len() as i64 - 480).abs() <= 2);
        // 再切回 24k：状态重建，长度精确
        let c2 = c.convert(&vec![100i16; 240], 24_000, 24_000);
        assert_eq!(c2.len(), 240);
        c.reset();
        assert_eq!(c.input_rate(), None);
    }

    #[test]
    fn converter_empty_input() {
        let mut c = AppendConverter::new();
        assert!(c.convert(&[], 24_000, 24_000).is_empty());
        // 空输入不该建立状态
        assert_eq!(c.input_rate(), None);
    }

    #[test]
    fn converter_48k_pcm_through_volume_keeps_length() {
        // 一次典型 StepFun/Qwen 24k delta（20ms = 480 样本）
        let mut c = AppendConverter::new();
        let out = c.convert(&vec![0i16; 480], 24_000, 24_000);
        assert_eq!(out.len(), 480);
    }

    // ---------- 无设备路径 ----------

    #[test]
    fn open_without_device_returns_err_not_panic() {
        // 本机没有声卡：必须是 Err（或真机上 Ok），绝不能 panic
        match AudioPlayer::open() {
            Ok(p) => {
                // 真机（wings-pc）分支：验证基本 API 可用
                assert!(p.is_open());
                assert_eq!(p.queued_frames(), 0);
                assert!(p.capacity_ms() > 0.0);
                assert_eq!(p.format().nominal_rate, DOWNLINK_SAMPLE_RATE);
                p.stop_now();
            }
            Err(AudioError::NoOutputDevice) | Err(AudioError::Device(_)) => {}
            Err(other) => panic!("意外错误类型: {other:?}"),
        }
    }

    #[test]
    fn open_with_zero_rate_is_invalid_config() {
        assert!(matches!(
            AudioPlayer::open_with_rate(0),
            Err(AudioError::InvalidConfig(_))
        ));
    }

    #[test]
    fn jitter_bounds_match_design_doc() {
        // docs/开发方案.md 4.3：播放侧 ring buffer 200–500ms 余量
        assert_eq!(JITTER_MIN_MS, 200);
        assert_eq!(JITTER_MAX_MS, 500);
        let q = PlaybackQueue::new(DOWNLINK_SAMPLE_RATE, JITTER_MAX_MS);
        assert_eq!(q.capacity(), (DOWNLINK_SAMPLE_RATE as usize * 500) / 1000);
        assert_eq!(DOWNLINK_SAMPLE_RATE, 24_000);
    }

    #[test]
    fn samples_buffer_is_available_for_oneshot_playback() {
        // rodio 0.23 的一次性播放路径（提示音等）也走 SamplesBuffer，验证类型可用
        let rate = NonZero::new(24_000u32).unwrap();
        let ch = NonZero::new(1u16).unwrap();
        let buf = rodio::buffer::SamplesBuffer::new(ch, rate, vec![0.0f32; 480]);
        assert_eq!(buf.sample_rate().get(), 24_000);
        assert_eq!(buf.channels().get(), 1);
    }
}
