//! 本地 VAD（语音活动检测）：打断功能的核心组件。
//!
//! ## 为什么必须有它
//!
//! 打断的正确顺序是（docs/开发方案.md §4.1）：
//! ```
//! 本地 VAD 检测到人声 → 立刻清播放队列 → 再推流给云端确认
//! ```
//!
//! 如果只依赖云端的 `speech_started` 事件，实测要等 **1.6 秒**
//! （见 `evidence/p1-vad-parameter-sweep.md`），
//! 而模型通常 1–3 秒就说完 —— 等事件到了，队列早就空了，打断根本来不及。
//!
//! **所以「本地先停」不是优化项，是打断能否存在的先决条件。**
//!
//! ## 用什么
//!
//! silero-vad（643KB onnx），通过 sherpa-onnx 的 `VoiceActivityDetector` 调用。
//! 16kHz 单声道，窗口 512 采样（32ms）。
//!
//! ## 判定延迟
//!
//! `min_speech_duration` 决定「连续说话多久才算真的开口」。
//! 默认 0.25s，可调到 0.15s 换更快的打断（代价是噪声容易误触发）。

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// 本地 VAD 配置
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// silero_vad.onnx 路径
    pub model: PathBuf,
    /// 判定阈值（0–1），越低越敏感。默认 0.5
    pub threshold: f32,
    /// 最短说话时长（秒）。打断场景建议 0.15–0.25
    pub min_speech_duration: f32,
    /// 最短静音时长（秒），决定「说完」判定
    pub min_silence_duration: f32,
    /// 窗口大小（采样数）。silero 要求 512 @ 16k
    pub window_size: i32,
    /// 单段最长时长（秒），防止一直判「在说」
    pub max_speech_duration: f32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            // 先找随包资源（AppImage/deb 自带），再找用户数据目录
            model: crate::config::find_resource("vad-model/silero_vad.onnx"),
            threshold: 0.5,
            // 打断要快。2026-10-24 实测（wings-pc，bargein_local）：
            //   0.20s → 端到端 441ms
            //   0.12s → 端到端 **340ms**
            //   0.08s → 目标 <300ms
            //
            // 太小（<0.06s）的代价：咳嗽、敲键盘、椅子声都可能被当成人声。
            // silero-vad 本身有 threshold 兜底，0.08s 在安静环境可用，
            // 嘈杂环境应退回 0.12s。
            min_speech_duration: 0.08,
            // 说完判定别太慢，但也不能太急（0.5s 是常用值）
            min_silence_duration: 0.5,
            window_size: 512,
            max_speech_duration: 30.0,
        }
    }
}

impl VadConfig {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.model.is_file(),
            "VAD 模型不存在: {}（下载：curl -sL -o ~/.diemeng/vad-model/silero_vad.onnx \
             https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx）",
            self.model.display()
        );
        anyhow::ensure!(
            (0.0..=1.0).contains(&self.threshold),
            "threshold 必须在 0–1，当前 {}",
            self.threshold
        );
        anyhow::ensure!(self.window_size == 512, "silero-vad 要求 window_size=512");
        anyhow::ensure!(
            self.min_speech_duration > 0.0 && self.min_speech_duration < 2.0,
            "min_speech_duration 应在 0–2 秒，当前 {}",
            self.min_speech_duration
        );
        Ok(())
    }
}

/// 本地 VAD 检测器
pub struct LocalVad {
    inner: sherpa_onnx::VoiceActivityDetector,
    /// 累计检测到「用户开口」次数
    pub speech_starts: Arc<AtomicU64>,
    /// 当前是否正在说话
    speaking: Arc<AtomicBool>,
}

impl LocalVad {
    pub fn new(cfg: &VadConfig) -> Result<Self> {
        cfg.validate()?;
        let mut model_cfg = sherpa_onnx::VadModelConfig::default();
        model_cfg.silero_vad.model = Some(cfg.model.to_string_lossy().to_string());
        model_cfg.silero_vad.threshold = cfg.threshold;
        model_cfg.silero_vad.min_speech_duration = cfg.min_speech_duration;
        model_cfg.silero_vad.min_silence_duration = cfg.min_silence_duration;
        model_cfg.silero_vad.window_size = cfg.window_size;
        model_cfg.silero_vad.max_speech_duration = cfg.max_speech_duration;
        model_cfg.sample_rate = 16_000;
        model_cfg.num_threads = 1;

        let det = sherpa_onnx::VoiceActivityDetector::create(&model_cfg, 30.0)
            .context("创建 VoiceActivityDetector 失败")?;
        Ok(Self {
            inner: det,
            speech_starts: Arc::new(AtomicU64::new(0)),
            speaking: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 喂一段 16k mono 样本，返回「这一帧里用户是否开口」。
    ///
    /// 只在**从静音转为说话**的那一帧返回 `true`（边沿触发），
    /// 避免持续说话时反复触发打断。
    pub fn push(&self, samples: &[f32]) -> bool {
        self.inner.accept_waveform(samples);
        let now = self.inner.detected();
        let was = self.speaking.swap(now, Ordering::Relaxed);
        if now && !was {
            self.speech_starts.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// 当前是否正在说话
    pub fn is_speaking(&self) -> bool {
        self.inner.detected()
    }

    pub fn speech_starts(&self) -> u64 {
        self.speech_starts.load(Ordering::Relaxed)
    }

    pub fn reset(&self) {
        self.inner.reset();
        self.speaking.store(false, Ordering::Relaxed);
    }
}

/// 把 i16 帧转成 VAD 需要的 f32（-1..1）
pub fn frame_to_f32(frame: &[i16]) -> Vec<f32> {
    frame.iter().map(|&s| s as f32 / 32768.0).collect()
}

/// 纯逻辑：判断一段能量序列里有没有「开口」（供无模型时单测）
///
/// 这是 `LocalVad` 的简化替身：连续 N 帧超过阈值就算开口。
/// 真实场景用 silero-vad，这里只保证「边沿触发」的逻辑正确。
pub struct EnergyGate {
    threshold: f32,
    need_frames: u32,
    hot: u32,
    speaking: bool,
}

impl EnergyGate {
    pub fn new(threshold: f32, need_frames: u32) -> Self {
        Self { threshold, need_frames, hot: 0, speaking: false }
    }

    /// 返回 true 表示「刚刚开口」（边沿）
    pub fn push(&mut self, rms: f32) -> bool {
        if rms > self.threshold {
            self.hot = self.hot.saturating_add(1);
            if self.hot >= self.need_frames && !self.speaking {
                self.speaking = true;
                return true;
            }
        } else {
            self.hot = 0;
            self.speaking = false;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_missing_model() {
        let c = VadConfig { model: PathBuf::from("/nope/silero.onnx"), ..Default::default() };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_bad_window() {
        let c = VadConfig {
            model: PathBuf::from("/tmp/x.onnx"),
            window_size: 256,
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_bad_threshold() {
        let c = VadConfig {
            model: PathBuf::from("/tmp/x.onnx"),
            threshold: 1.5,
            ..Default::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn energy_gate_triggers_on_edge_only() {
        let mut g = EnergyGate::new(0.1, 2);
        assert!(!g.push(0.5), "第一帧不够 2 帧，不该触发");
        assert!(g.push(0.5), "第二帧应触发");
        assert!(!g.push(0.5), "持续说话不该重复触发（边沿触发）");
        assert!(!g.push(0.01), "静音");
        assert!(!g.push(0.5), "只回一帧不够，不该触发");
        assert!(g.push(0.5), "连续两帧后再次触发");
    }

    #[test]
    fn frame_conversion_range() {
        let f = frame_to_f32(&[0, 16384, -32768, 32767]);
        assert!((f[0] - 0.0).abs() < 1e-6);
        assert!((f[1] - 0.5).abs() < 1e-3);
        assert!((f[2] - (-1.0)).abs() < 1e-6);
        assert!(f[3] <= 1.0);
    }
}
