//! 音频设备：枚举 + 按名字选择。
//!
//! ## 为什么需要这一层
//!
//! 2026-10-25 真机排查出的教训：**「系统默认设备」不是我们的设备**。
//!
//! - 默认录音源被测试脚本 / WirePlumber 记住的历史记录反复改回虚拟声卡
//!   `mfsink.monitor` → App 录到的全是空气，用户说话云端一个字都收不到；
//! - 默认输出被改成板载喇叭（不出声）→ 模型明明回了 17 秒语音，用户什么都听不到。
//!
//! 只要 App 跟着系统默认走，这两类问题就会**莫名其妙地复现**，而且日志上看
//! 一切正常（后端是 PulseAudio、流也建起来了）。
//!
//! 所以这里提供「按名字挑设备」的能力，配套两个环境变量（配置见 [`crate::config::AudioConfig`]）：
//!
//! - `MORPHOFLY_INPUT_DEVICE`：麦克风
//! - `MORPHOFLY_OUTPUT_DEVICE`：扬声器/耳机
//!
//! 名字匹配规则见 [`name_matches`]：**不区分大小写的子串**。可以写 PipeWire 全名
//! （`alsa_input.usb-KTMicro_...`），也可以只写 `USB` / `KTMicro` / `录音1`。
//! 找不到就 `warn` 并回退默认 —— 绝不因为配置写错就打不开声音。

use cpal::traits::{DeviceTrait, HostTrait};

/// 一个可选的音频设备（给人看 + 给程序用的两种名字）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    /// 描述名（PipeWire 下是「录音1」「播放1」这种，用户一眼能认）
    pub label: String,
    /// 底层设备名（ALSA / PulseAudio 的真实标识，写进配置最稳）
    pub name: String,
}

impl DeviceEntry {
    /// 日志/菜单统一用这个名字。
    #[must_use]
    pub fn display(&self) -> &str {
        if self.label.is_empty() {
            &self.name
        } else {
            &self.label
        }
    }
}

/// 选音频 host：Linux 优先 PulseAudio（原因见 `capture::pick_host` 的注释：
/// ALSA→PipeWire 桥接在 wings-gpd 的 SOF 声卡上开录音流必现 `No such entity`）。
#[must_use]
pub fn pick_host() -> cpal::Host {
    #[cfg(target_os = "linux")]
    if let Ok(h) = cpal::host_from_id(cpal::HostId::PulseAudio) {
        return h;
    }
    cpal::default_host()
}

/// 名字匹配：不区分大小写的子串匹配。
///
/// 例：`name_matches("alsa_input.usb-KTMicro_CDSP...", "ktmicro") == true`；
/// `name_matches("录音1", "录音") == true`。
#[must_use]
pub fn name_matches(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    haystack.trim().to_lowercase().contains(&needle)
}

fn collect(devices: impl Iterator<Item = cpal::Device>) -> Vec<(cpal::Device, DeviceEntry)> {
    devices
        .filter_map(|d| {
            // cpal 0.18 没有 `Device::name()`：人话名在 description().name()，
            // 底层全名（PipeWire 的 `alsa_input.usb-KTMicro...`）在 `DeviceId::id()`。
            // 两个都要：菜单显示人话名，配置里写全名最稳，用户也可以只写片段。
            let label = d
                .description()
                .map(|x| x.name().to_string())
                .unwrap_or_default();
            let name = d.id().map(|id| id.id().to_string()).unwrap_or_default();
            if label.is_empty() && name.is_empty() {
                return None;
            }
            Some((
                d,
                DeviceEntry {
                    label: if label.is_empty() {
                        name.clone()
                    } else {
                        label
                    },
                    name,
                },
            ))
        })
        .collect()
}

/// 所有输入设备（麦克风）。顺序由后端决定（PipeWire 一般是优先级顺序）。
#[must_use]
pub fn inputs() -> Vec<(cpal::Device, DeviceEntry)> {
    let host = pick_host();
    match host.input_devices() {
        Ok(it) => collect(it),
        Err(e) => {
            tracing::warn!(error = %e, "枚举输入设备失败");
            Vec::new()
        }
    }
}

/// 所有输出设备（扬声器/耳机）。
#[must_use]
pub fn outputs() -> Vec<(cpal::Device, DeviceEntry)> {
    let host = pick_host();
    match host.output_devices() {
        Ok(it) => collect(it),
        Err(e) => {
            tracing::warn!(error = %e, "枚举输出设备失败");
            Vec::new()
        }
    }
}

/// 在设备表里找目标（精确命中优先，再退子串）。抽出来是为了能单测。
fn match_index(entries: &[DeviceEntry], wanted: &str) -> Option<usize> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    // 先精确命中（描述名或底层名完全相同），再退到子串 —— 避免「USB」这种
    // 短词先撞上别的设备。
    if let Some(pos) = entries
        .iter()
        .position(|e| e.label.eq_ignore_ascii_case(wanted) || e.name.eq_ignore_ascii_case(wanted))
    {
        return Some(pos);
    }
    entries
        .iter()
        .position(|e| name_matches(&e.label, wanted) || name_matches(&e.name, wanted))
}

fn pick(
    devices: Vec<(cpal::Device, DeviceEntry)>,
    wanted: &str,
) -> Option<(cpal::Device, DeviceEntry)> {
    let entries: Vec<DeviceEntry> = devices.iter().map(|(_, e)| e.clone()).collect();
    let idx = match_index(&entries, wanted)?;
    devices.into_iter().nth(idx)
}

/// 按名字找输入设备。
#[must_use]
pub fn find_input(wanted: &str) -> Option<(cpal::Device, DeviceEntry)> {
    pick(inputs(), wanted)
}

/// 按名字找输出设备。
#[must_use]
pub fn find_output(wanted: &str) -> Option<(cpal::Device, DeviceEntry)> {
    pick(outputs(), wanted)
}

/// 输入设备名字列表（日志/报错时告诉用户「能选的有哪些」）。
#[must_use]
pub fn input_labels() -> Vec<String> {
    inputs()
        .into_iter()
        .map(|(_, e)| e.display().to_string())
        .collect()
}

/// 输出设备名字列表。
#[must_use]
pub fn output_labels() -> Vec<String> {
    outputs()
        .into_iter()
        .map(|(_, e)| e.display().to_string())
        .collect()
}

/// 读环境变量里的设备配置（空串视为未设置）。
#[must_use]
pub fn wanted_input() -> Option<String> {
    wanted_from(|k| std::env::var(k).ok(), "MORPHOFLY_INPUT_DEVICE")
}

/// 读环境变量里的输出设备配置（空串视为未设置）。
#[must_use]
pub fn wanted_output() -> Option<String> {
    wanted_from(|k| std::env::var(k).ok(), "MORPHOFLY_OUTPUT_DEVICE")
}

/// 抽出来是为了能测（不用改进程环境）。
fn wanted_from(get: impl Fn(&str) -> Option<String>, key: &str) -> Option<String> {
    get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_match_is_case_insensitive_substring() {
        assert!(name_matches("alsa_input.usb-KTMicro_CDSP", "ktmicro"));
        assert!(name_matches("录音1", "录音"));
        assert!(name_matches("USB Audio", " usb "));
        assert!(!name_matches("录音1", "播放"));
        assert!(!name_matches("录音1", ""));
    }

    #[test]
    fn wanted_ignores_blank() {
        let none = wanted_from(|_| None, "X");
        assert_eq!(none, None);
        let blank = wanted_from(|_| Some("   ".into()), "X");
        assert_eq!(blank, None);
        let some = wanted_from(|_| Some(" USB ".into()), "X");
        assert_eq!(some.as_deref(), Some("USB"));
    }

    /// 精确命中优先于子串：写「USB 耳机」不该被更靠前的「USB」抢走。
    #[test]
    fn match_index_prefers_exact() {
        let entries = vec![
            DeviceEntry {
                label: "USB".into(),
                name: "alsa_output.usb-generic".into(),
            },
            DeviceEntry {
                label: "USB 耳机".into(),
                name: "alsa_output.usb-ktmicro".into(),
            },
        ];
        assert_eq!(match_index(&entries, "USB"), Some(0));
        assert_eq!(match_index(&entries, "usb 耳机"), Some(1));
        assert_eq!(match_index(&entries, "ktmicro"), Some(1));
        assert_eq!(match_index(&entries, "蓝牙"), None);
        assert_eq!(match_index(&entries, "   "), None);
    }
}
