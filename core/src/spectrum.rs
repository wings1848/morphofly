//! 频谱：把 PCM 帧变成 64 个 float 给 WebView 画球。
//!
//! 铁律：WebView 只收这些数字，**永不碰麦克风**。
//! 带宽：64 bin × 4 byte × 30fps ≈ 7.7KB/s。

use crate::SPECTRUM_BINS;
use realfft::RealFftPlanner;
use std::sync::Arc;

/// 频谱分析器
pub struct Spectrum {
    /// 每次分析的窗口采样数（16kHz 下 512 = 32ms）
    window: usize,
    planner: Arc<dyn realfft::RealToComplex<f32>>,
    window_fn: Vec<f32>,
    buf: Vec<f32>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
}

impl Spectrum {
    pub fn new(window: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(window);
        // Hann 窗，减少频谱泄漏
        let window_fn = (0..window)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / (window as f32 - 1.0);
                0.5 * (1.0 - x.cos())
            })
            .collect();
        let scratch = fft.make_output_vec();
        Self {
            window,
            planner: fft,
            window_fn,
            buf: vec![0.0; window],
            scratch,
        }
    }

    pub fn with_default_window() -> Self {
        Self::new(512)
    }

    /// 从 PCM16 计算 64 bin 幅度谱，值域 0.0..=1.0（已做对数压缩，肉眼好看）
    pub fn analyze_i16(&mut self, pcm: &[i16]) -> Vec<f32> {
        let n = self.window.min(pcm.len());
        // 取最后 window 个样本（最新声音）
        let start = pcm.len() - n;
        for (i, slot) in self.buf.iter_mut().enumerate() {
            if i < n {
                *slot = pcm[start + i] as f32 / 32768.0;
            } else {
                *slot = 0.0;
            }
        }
        self.analyze_f32_inplace()
    }

    /// 从 f32 样本计算
    pub fn analyze_f32(&mut self, samples: &[f32]) -> Vec<f32> {
        let n = self.window.min(samples.len());
        let start = samples.len() - n;
        self.buf[..n].copy_from_slice(&samples[start..start + n]);
        for v in self.buf[n..].iter_mut() {
            *v = 0.0;
        }
        self.analyze_f32_inplace()
    }

    fn analyze_f32_inplace(&mut self) -> Vec<f32> {
        let mut input = self.buf.clone();
        for (v, w) in input.iter_mut().zip(self.window_fn.iter()) {
            *v *= *w;
        }
        if self.planner.process(&mut input, &mut self.scratch).is_err() {
            return vec![0.0; SPECTRUM_BINS];
        }
        // 只取正频率一半
        let half = self.scratch.len();
        let mut bins = vec![0.0f32; SPECTRUM_BINS];
        if half == 0 {
            return bins;
        }
        #[allow(clippy::needless_range_loop)]
        // 这里 i 同时用于算 lo/hi 和写入 bins[i]，用 enumerate 反而更绕
        for i in 0..SPECTRUM_BINS {
            // 对数分布：低频给更多 bin（人声集中在低频）
            let lo = ((i as f32 / SPECTRUM_BINS as f32).powi(2) * half as f32) as usize;
            let hi = (((i + 1) as f32 / SPECTRUM_BINS as f32).powi(2) * half as f32) as usize;
            let hi = hi.max(lo + 1).min(half);
            let mut peak = 0.0f32;
            for c in &self.scratch[lo..hi] {
                peak = peak.max(c.norm());
            }
            // 归一化成「振幅」：Hann 窗相干增益 0.5，FFT 峰值 = A*N/4
            let amp = peak / (self.window as f32 / 4.0);
            let db = 20.0 * (amp + 1e-9).log10();
            // -65dB..0dB → 0..1（对数压缩，肉眼好看且不轻易饱和）
            bins[i] = ((db + 65.0) / 65.0).clamp(0.0, 1.0);
        }
        bins
    }
}

/// RMS（0.0..=1.0），用于音量指示与小球呼吸
pub fn rms_i16(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm
        .iter()
        .map(|&s| {
            let v = s as f64 / 32768.0;
            v * v
        })
        .sum();
    (sum / pcm.len() as f64).sqrt() as f32
}

/// RMS → 视觉强度（对数，避免小声音看不出来）
pub fn rms_to_visual(rms: f32) -> f32 {
    if rms <= 1e-6 {
        return 0.0;
    }
    let db = (20.0 * rms.log10()).max(-65.0);
    ((db + 65.0) / 65.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, n: usize, amp: f32) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                (amp * (2.0 * std::f32::consts::PI * freq * t).sin() * 32767.0) as i16
            })
            .collect()
    }

    #[test]
    fn silence_gives_zero_bins() {
        let mut s = Spectrum::with_default_window();
        let bins = s.analyze_i16(&vec![0i16; 512]);
        assert_eq!(bins.len(), SPECTRUM_BINS);
        assert!(bins.iter().all(|&b| b == 0.0), "静音必须是全 0");
    }

    #[test]
    fn loud_signal_is_not_flat_zero() {
        let mut s = Spectrum::with_default_window();
        let pcm = sine(440.0, 16_000, 512, 0.5);
        let bins = s.analyze_i16(&pcm);
        let max = bins.iter().cloned().fold(0.0f32, f32::max);
        assert!(max > 0.3, "440Hz 正弦要有明显能量，实际 max={max}");
        // 不是所有 bin 都一样（频谱必须随音量变化）
        let min = bins.iter().cloned().fold(1.0f32, f32::min);
        assert!(max - min > 0.1, "频谱不能是常数");
    }

    #[test]
    fn louder_gives_higher_peak() {
        let mut s = Spectrum::with_default_window();
        let quiet = s.analyze_i16(&sine(440.0, 16_000, 512, 0.05));
        let loud = s.analyze_i16(&sine(440.0, 16_000, 512, 0.9));
        let qmax = quiet.iter().cloned().fold(0.0f32, f32::max);
        let lmax = loud.iter().cloned().fold(0.0f32, f32::max);
        assert!(lmax > qmax, "响的峰值必须更高: {lmax} vs {qmax}");
    }

    #[test]
    fn high_freq_lands_in_higher_bins() {
        let mut s = Spectrum::with_default_window();
        let low = s.analyze_i16(&sine(200.0, 16_000, 512, 0.8));
        let high = s.analyze_i16(&sine(6000.0, 16_000, 512, 0.8));
        let low_peak = argmax(&low);
        let high_peak = argmax(&high);
        assert!(
            high_peak > low_peak,
            "6kHz 的峰值 bin 应高于 200Hz: {high_peak} vs {low_peak}"
        );
    }

    fn argmax(v: &[f32]) -> usize {
        v.iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    #[test]
    fn rms_matches_known_value() {
        // 满幅方波 RMS = 1.0
        let pcm: Vec<i16> = vec![32767; 100];
        assert!((rms_i16(&pcm) - 1.0).abs() < 0.01);
        // 静音 = 0
        assert_eq!(rms_i16(&[0i16; 100]), 0.0);
        // 空输入不 panic
        assert_eq!(rms_i16(&[]), 0.0);
    }

    #[test]
    fn rms_to_visual_maps_quiet_to_low() {
        assert_eq!(rms_to_visual(0.0), 0.0);
        assert!(rms_to_visual(0.001) < 0.3);
        assert!(rms_to_visual(0.5) > 0.9);
    }

    #[test]
    fn short_input_does_not_panic() {
        let mut s = Spectrum::with_default_window();
        let bins = s.analyze_i16(&[100i16, -100]);
        assert_eq!(bins.len(), SPECTRUM_BINS);
    }

    #[test]
    fn bins_are_in_range() {
        let mut s = Spectrum::with_default_window();
        let bins = s.analyze_i16(&sine(1000.0, 16_000, 512, 1.0));
        assert!(
            bins.iter().all(|&b| (0.0..=1.0).contains(&b)),
            "bin 必须在 0..1"
        );
    }
}
