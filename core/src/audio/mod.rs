//! 原生音频：cpal 采集 / rodio 播放 / 可即时清空的环形缓冲。
//!
//! 架构铁律（`docs/开发方案.md` 4.3）：
//! - 采集固定 **16kHz / mono / i16 / 20ms(320 采样)**，与 [`crate::UPLINK_SAMPLE_RATE`] /
//!   [`crate::FRAME_SAMPLES`] 一致；设备原生采样率在采集回调里重采样到目标采样率。
//! - 下行 PCM 常见 24kHz（Qwen / StepFun），播放侧接受任意采样率，由 rodio 转到设备采样率。
//! - 播放队列必须可**立即清空**（不变量 I4，barge-in 打断的前提）。
//!
//! 分层（每层的真机调用在没有音频设备的机器上都"可编译 + 返回 Err 而不是 panic"）：
//!
//! | 文件 | 职责 | 无设备可测部分 |
//! |---|---|---|
//! | [`capture`] | cpal 采集 → 下混 → 重采样 → 分帧 | 下混 / 重采样 / 分帧（纯函数） |
//! | [`playback`] | rodio 0.23 播放、追加任意采样率 PCM、`stop_now()`、队列深度 | 队列计量 / 流式 source |
//! | [`ring`] | 有界环形缓冲（播放侧 200–500ms 抗抖动） | 全部 |

pub mod capture;
pub mod playback;
pub mod ring;

pub use capture::{AudioCapture, CaptureConfig, CaptureFormat, CapturePipeline, LinearResampler};
pub use playback::{AppendConverter, AudioPlayer, OutputFormat, PlaybackQueue, RingSource};
pub use ring::SampleRing;

/// 下行（TTS）音频的 nominal 采样率：Qwen / StepFun 的 pcm16 输出常见 24kHz。
pub const DOWNLINK_SAMPLE_RATE: u32 = 24_000;

/// 播放侧抗抖动缓冲下限（毫秒）。设计值见 `docs/开发方案.md` 4.3：200–500ms。
pub const JITTER_MIN_MS: u32 = 200;

/// 播放侧抗抖动缓冲上限（毫秒）。超出即丢最旧（用户等不到的音频没有保留价值）。
pub const JITTER_MAX_MS: u32 = 500;

/// 音频层统一错误。
///
/// 约定：所有真机调用（打开设备 / 构建流）返回 `Result`，**绝不 panic** ——
/// 本机（只有 `/dev/snd/timer`、没有声卡）必须走 `Err` 分支。
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// 没有可用的输入设备（麦克风）。
    #[error("没有可用的输入设备（麦克风）")]
    NoInputDevice,
    /// 没有可用的输出设备（扬声器）。
    #[error("没有可用的输出设备（扬声器）")]
    NoOutputDevice,
    /// 设备声明的采样格式本层还不支持（例如 DSD）。
    #[error("设备音频格式不受支持: {0}")]
    UnsupportedSampleFormat(String),
    /// 设备/后端报错，保留原始文本便于排查。
    #[error("音频设备错误: {0}")]
    Device(String),
    /// 参数非法（如采样率为 0、声道数为 0）。
    #[error("音频参数非法: {0}")]
    InvalidConfig(String),
}

/// i16 PCM → f32（`[-1.0, 1.0]`）。
#[inline]
pub fn i16_to_f32(sample: i16) -> f32 {
    f32::from(sample) / 32768.0
}

/// f32 → i16，超出 `[-1.0, 1.0]` 的部分饱和裁剪，NaN → 0。
///
/// 采用**对称满量程**约定：乘 32768 后裁剪到 `[-32768, 32767]`。
/// 这样与 [`i16_to_f32`]（除以 32768）严格互逆 —— 任意 i16 都能往返回到原值，
/// 不产生 ±1 的量化偏差；`-1.0` 映射到 i16 的下限 `-32768`，`1.0` 映射到上限 `32767`。
#[inline]
pub fn f32_to_i16(sample: f32) -> i16 {
    if sample.is_nan() {
        return 0;
    }
    let scaled = (sample.clamp(-1.0, 1.0) * 32768.0).round();
    scaled.clamp(-32768.0, 32767.0) as i16
}

/// i16 PCM 切片 → f32 向量（mono，逐样本）。
pub fn pcm16_to_f32(pcm: &[i16]) -> Vec<f32> {
    pcm.iter().map(|&s| i16_to_f32(s)).collect()
}

/// 一段 i16 PCM 的 RMS，归一化到 `[0, 1]`（满量程方波 ≈ 0.99997）。空切片返回 0.0。
///
/// 用于频谱/音量指示与 barge-in 阈值判断。
pub fn rms_i16(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame
        .iter()
        .map(|&s| {
            let x = f64::from(s);
            x * x
        })
        .sum();
    (sum / frame.len() as f64).sqrt() as f32 / 32768.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FRAME_BYTES, FRAME_SAMPLES, UPLINK_SAMPLE_RATE};

    #[test]
    fn uplink_constants_match_design() {
        assert_eq!(UPLINK_SAMPLE_RATE, 16_000);
        assert_eq!(FRAME_SAMPLES, 320);
        assert_eq!(FRAME_BYTES, 640);
        // 一帧 320 采样在 16k 下正好 20ms
        assert_eq!(FRAME_SAMPLES * 1000 / UPLINK_SAMPLE_RATE as usize, 20);
    }

    #[test]
    fn jitter_constants_match_design() {
        // 开发方案 4.3：播放侧 ring buffer 200–500ms 余量
        assert_eq!(JITTER_MIN_MS, 200);
        assert_eq!(JITTER_MAX_MS, 500);
        assert_eq!(DOWNLINK_SAMPLE_RATE, 24_000);
    }

    #[test]
    fn i16_f32_roundtrip() {
        assert_eq!(i16_to_f32(0), 0.0);
        assert_eq!(i16_to_f32(-32768), -1.0);
        assert!((i16_to_f32(32767) - 0.999_969_7).abs() < 1e-6);
        assert_eq!(f32_to_i16(0.0), 0);
        assert_eq!(f32_to_i16(1.0), 32767);
        assert_eq!(f32_to_i16(-1.0), -32768);
        // 全量程逐位往返：i16→f32→i16 必须恒等（对称缩放的不变量）
        for s in i16::MIN..=i16::MAX {
            assert_eq!(f32_to_i16(i16_to_f32(s)), s, "sample={s}");
        }
    }

    #[test]
    fn f32_to_i16_saturates_and_handles_nan() {
        assert_eq!(f32_to_i16(2.0), 32767);
        assert_eq!(f32_to_i16(-2.0), -32768);
        assert_eq!(f32_to_i16(f32::INFINITY), 32767);
        assert_eq!(f32_to_i16(f32::NEG_INFINITY), -32768);
        assert_eq!(f32_to_i16(f32::NAN), 0);
    }

    #[test]
    fn rms_math() {
        // 满量程方波 ≈ 1.0
        let square: Vec<i16> = (0..FRAME_SAMPLES)
            .map(|i| if i % 2 == 0 { 32767 } else { -32768 })
            .collect();
        assert!((rms_i16(&square) - 0.999_97).abs() < 1e-4);

        // 半量程恒值 = 0.5
        assert!((rms_i16(&vec![16384i16; FRAME_SAMPLES]) - 0.5).abs() < 1e-4);

        // 空帧 / 全零安全
        assert_eq!(rms_i16(&[]), 0.0);
        assert_eq!(rms_i16(&[0i16; FRAME_SAMPLES]), 0.0);

        // 固定值手算
        let want = (((3000f64 * 3000.0 + 4000f64 * 4000.0) / 2.0).sqrt() / 32768.0) as f32;
        assert!((rms_i16(&[3000i16, -4000]) - want).abs() < 1e-6);
    }

    #[test]
    fn pcm16_to_f32_batch() {
        let out = pcm16_to_f32(&[0, 16384, -32768]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], 0.0);
        assert!((out[1] - 0.5).abs() < 1e-6);
        assert_eq!(out[2], -1.0);
        assert!(pcm16_to_f32(&[]).is_empty());
    }
}
