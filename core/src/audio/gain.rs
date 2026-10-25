//! 上行自动增益（AGC）：把麦克风音量拉到服务端认得的水平。
//!
//! ## 为什么需要它（2026-10-25 真机 + 探针实测）
//!
//! 阶跃 realtime 的**服务端 VAD 是能量门控**的（`energy_awakeness_threshold`，
//! 范围 0–5000，默认 2500；我们的模板用 500）。实测（`core/examples/vad_probe.rs`）：
//!
//! | 送进去的音频 | 服务端 VAD |
//! |---|---|
//! | 测试用 TTS mp3（响） | ✅ 触发 speech_started，并**自动**开一轮回复 |
//! | 真实耳机麦录音（峰值 ~0.09，RMS ~0.013） | ❌ 一声不响 |
//! | 同一段录音 ×2（+6dB） | ✅ 触发 |
//! | 同一段录音 ×4（+12dB） | ✅ 触发，且 ASR 更准（"小蝶，你好，你能听见吗？"） |
//!
//! 也就是说：**服务端 VAD 不是不工作，是我们送上去的声音太小。**
//! 浏览器（官方控制台）默认开 WebRTC 的 AGC，所以它那边一直正常。
//!
//! ## 策略
//!
//! 峰值跟踪 + 平滑增益，限制在 `[1.0, max_gain]`：
//! - 安静时慢慢往上加（说话前几帧就能爬到合适增益）
//! - 突然变响时快速回退（防削波）
//! - 底噪（峰值低于 `min_peak`）不放大，否则纯噪声会被抬成"人声"
//!
//! 只动上行（送去识别的那一路）；本地 VAD 仍用未放大的原始帧
//! （silero 在原始音量下已经能稳定判定，放大反而容易误触发）。

/// 目标峰值（约 -9 dBFS）。取 0.35 而不是 1.0：给突发留余量，避免削波。
pub const TARGET_PEAK: f32 = 0.35;

/// 最大放大倍数（约 +18 dB）。再大就会把底噪一起抬起来。
pub const MAX_GAIN: f32 = 8.0;

/// 低于这个峰值（约 -36 dBFS）就认为是底噪，不放大。
pub const MIN_PEAK: f32 = 0.015;

/// 增益上升的平滑系数（每帧）。0.08 ≈ 20 帧（400ms）爬到目标值。
const ATTACK: f32 = 0.08;

/// 增益回落的平滑系数（每帧）。0.4 ≈ 3 帧，防止突然大声削波。
const RELEASE: f32 = 0.4;

/// 上行自动增益器（单声道 i16）。
#[derive(Debug, Clone)]
pub struct AutoGain {
    gain: f32,
    target_peak: f32,
    max_gain: f32,
    min_peak: f32,
    enabled: bool,
}

impl Default for AutoGain {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoGain {
    /// 用默认参数建一个（初始增益 1.0，逐帧爬升）。
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            target_peak: TARGET_PEAK,
            max_gain: MAX_GAIN,
            min_peak: MIN_PEAK,
            enabled: true,
        }
    }

    /// 关闭（等价于原样输出），用于排查「是不是增益搞坏的」。
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::new()
        }
    }

    /// 当前增益（测试/日志用）。
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// 就地放大一帧 i16。
    ///
    /// 返回这一帧的输出峰值（0–1），方便上层打点。
    pub fn process_i16(&mut self, frame: &mut [i16]) -> f32 {
        if frame.is_empty() {
            return 0.0;
        }
        // 1) 量这一帧的峰值
        let mut peak_in: f32 = 0.0;
        for &s in frame.iter() {
            let a = (s as f32 / 32768.0).abs();
            if a > peak_in {
                peak_in = a;
            }
        }
        if !self.enabled {
            return peak_in;
        }
        // 2) 算目标增益：底噪不动，正常语音按目标峰值放大
        if peak_in >= self.min_peak {
            let desired = (self.target_peak / peak_in).clamp(1.0, self.max_gain);
            let alpha = if desired > self.gain { ATTACK } else { RELEASE };
            self.gain += (desired - self.gain) * alpha;
            self.gain = self.gain.clamp(1.0, self.max_gain);
        }
        // 3) 应用（饱和到 i16 范围内，绝不 panic）
        let g = self.gain;
        let mut peak_out: f32 = 0.0;
        for s in frame.iter_mut() {
            let v = *s as f32 * g;
            let clamped = v.clamp(-32768.0, 32767.0);
            *s = clamped.round() as i16;
            let a = (clamped / 32768.0).abs();
            if a > peak_out {
                peak_out = a;
            }
        }
        peak_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiet_frame(peak: f32) -> Vec<i16> {
        // 用方波近似（峰值可控），320 采样 = 20ms
        (0..320)
            .map(|i| {
                if i % 2 == 0 {
                    (peak * 32767.0) as i16
                } else {
                    0
                }
            })
            .collect()
    }

    #[test]
    fn 安静语音会被放大到目标峰值附近() {
        let mut agc = AutoGain::new();
        let mut peak = 0.0f32;
        // 真实语音是连续多帧：跑 30 帧让增益爬到目标
        for _ in 0..30 {
            let mut f = quiet_frame(0.02); // 约 -34 dBFS，真实耳机麦量级
            peak = agc.process_i16(&mut f);
        }
        assert!(agc.gain() > 3.0, "增益应明显上升，实际 {}", agc.gain());
        // 输入 0.02 哪怕给到最大增益 8 也只有 0.16 —— 这里只要求「明显抬高」
        assert!(peak > 0.1, "输出峰值应被抬起来，实际 {peak}");
        assert!(peak <= 1.0);
    }

    #[test]
    fn 本来就很响的语音不会继续放大也不会削波() {
        let mut agc = AutoGain::new();
        for _ in 0..30 {
            let mut f = quiet_frame(0.8);
            let peak = agc.process_i16(&mut f);
            assert!(peak <= 1.0, "不能超过满量程");
        }
        assert!(agc.gain() <= 1.05, "响的信号不该放大，实际 {}", agc.gain());
    }

    #[test]
    fn 底噪不放大() {
        let mut agc = AutoGain::new();
        for _ in 0..50 {
            let mut f = quiet_frame(0.005); // -46 dBFS，低于 min_peak
            agc.process_i16(&mut f);
        }
        assert!((agc.gain() - 1.0).abs() < 1e-6, "底噪不该被放大");
    }

    #[test]
    fn 关闭时原样输出() {
        let mut agc = AutoGain::disabled();
        let mut f = quiet_frame(0.02);
        let before = f.clone();
        agc.process_i16(&mut f);
        assert_eq!(f, before);
    }

    #[test]
    fn 增益上限() {
        let mut agc = AutoGain::new();
        for _ in 0..200 {
            let mut f = quiet_frame(0.021);
            agc.process_i16(&mut f);
        }
        assert!(agc.gain() <= MAX_GAIN + 1e-3, "增益不得超过上限");
    }
}
