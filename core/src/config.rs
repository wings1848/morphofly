//! 配置：`~/.diemeng/config.toml`，环境变量覆盖。
//!
//! 硬要求（验收 A5）：provider / endpoint / key / voice 全部从配置读，换 provider 不改代码。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 支持的 provider
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Stepfun,
    Qwen,
    Mock,
}

impl Default for Provider {
    fn default() -> Self {
        Provider::Stepfun
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Provider::Stepfun => write!(f, "stepfun"),
            Provider::Qwen => write!(f, "qwen"),
            Provider::Mock => write!(f, "mock"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RealtimeConfig {
    pub provider: Provider,
    /// 完整 WS 端点（含 model 查询参数）
    pub endpoint: String,
    pub model: String,
    pub voice: String,
    /// 环境变量名（不直接存密钥）
    pub api_key_env: String,
    /// 采样率（下行音频）
    pub output_sample_rate: u32,
    /// 系统提示词
    pub instructions: String,
}

impl Default for RealtimeConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Stepfun,
            endpoint: "wss://api.stepfun.com/v1/realtime?model=stepaudio-3-realtime-preview"
                .to_string(),
            model: "stepaudio-3-realtime-preview".to_string(),
            voice: "jingdiannvsheng".to_string(),
            api_key_env: "STEPFUN_KEY".to_string(),
            output_sample_rate: 24_000,
            instructions: DEFAULT_INSTRUCTIONS.to_string(),
        }
    }
}

pub const DEFAULT_INSTRUCTIONS: &str = "你是「小蝶」，蝶梦语音智能体的语音前台。\
你只负责嘴和耳朵：听懂用户说的话、必要时澄清、把任务整理成自包含的文字、交给 delegate_task 执行、再把结果用一两句人话讲回来。\
你没有读写文件、运行命令的能力，也不要假装有。\
说话简短口语化，不要念代码块、路径、URL 或 markdown 符号。";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DshConfig {
    /// 启动命令（第一个是程序名）
    pub command: Vec<String>,
    /// 默认工作目录
    pub default_cwd: String,
}

impl Default for DshConfig {
    fn default() -> Self {
        Self {
            command: vec!["dsh".into(), "--profile".into(), "acp".into()],
            default_cwd: std::env::var("HOME").unwrap_or_else(|_| ".".into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub realtime: RealtimeConfig,
    pub dsh: DshConfig,
    /// 唤醒词（阶段 4 用）
    pub wake_word: String,
    /// 静音多少毫秒后回到待机
    pub idle_timeout_ms: u64,
    /// 数据目录
    pub data_dir: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            realtime: RealtimeConfig::default(),
            dsh: DshConfig::default(),
            wake_word: "小蝶".into(),
            idle_timeout_ms: 30_000,
            data_dir: default_data_dir(),
        }
    }
}

pub fn default_data_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".diemeng")
}

impl Config {
    /// 从默认位置加载；文件不存在则用默认值（并尝试写出模板）。
    pub fn load() -> Result<Self> {
        let path = config_path();
        let mut cfg = if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("读配置失败: {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("解析 TOML 失败: {}", path.display()))?
        } else {
            Config::default()
        };
        cfg.apply_env();
        Ok(cfg)
    }

    pub fn from_toml_str(s: &str) -> Result<Self> {
        let mut cfg: Config = toml::from_str(s)?;
        cfg.apply_env();
        Ok(cfg)
    }

    /// 环境变量覆盖：DIEMENG_PROVIDER / DIEMENG_ENDPOINT / DIEMENG_MODEL / DIEMENG_VOICE
    pub fn apply_env(&mut self) {
        if let Ok(v) = std::env::var("DIEMENG_PROVIDER") {
            match v.to_ascii_lowercase().as_str() {
                "stepfun" | "step" => self.realtime.provider = Provider::Stepfun,
                "qwen" | "dashscope" | "bailian" => self.realtime.provider = Provider::Qwen,
                "mock" => self.realtime.provider = Provider::Mock,
                _ => {}
            }
        }
        if let Ok(v) = std::env::var("DIEMENG_ENDPOINT") {
            self.realtime.endpoint = v;
        }
        if let Ok(v) = std::env::var("DIEMENG_MODEL") {
            self.realtime.model = v;
        }
        if let Ok(v) = std::env::var("DIEMENG_VOICE") {
            self.realtime.voice = v;
        }
        if let Ok(v) = std::env::var("DIEMENG_API_KEY_ENV") {
            self.realtime.api_key_env = v;
        }
    }

    /// 读取密钥。密钥只从环境变量/凭据文件读，绝不写进仓库或前端产物。
    pub fn api_key(&self) -> Result<String> {
        if let Ok(k) = std::env::var(&self.realtime.api_key_env) {
            if !k.trim().is_empty() {
                return Ok(k.trim().to_string());
            }
        }
        // 兜底：~/.diemeng/credentials（600）
        let p = self.data_dir.join("credentials");
        if p.exists() {
            let text = std::fs::read_to_string(&p)?;
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    if k.trim() == self.realtime.api_key_env {
                        return Ok(v.trim().trim_matches('"').to_string());
                    }
                }
            }
        }
        anyhow::bail!(
            "找不到 API 密钥：请设置环境变量 {} 或写入 {}",
            self.realtime.api_key_env,
            p.display()
        )
    }

    /// 密钥脱敏显示（日志用）
    pub fn masked_key(&self) -> String {
        match self.api_key() {
            Ok(k) if k.len() > 8 => format!("{}****{}", &k[..4], &k[k.len() - 4..]),
            Ok(_) => "****".into(),
            Err(_) => "<未配置>".into(),
        }
    }

    pub fn task_store_path(&self) -> PathBuf {
        self.data_dir.join("tasks.json")
    }
}

pub fn config_path() -> PathBuf {
    default_data_dir().join("config.toml")
}

/// 写一份配置模板（不覆盖已存在的）
pub fn write_template_if_missing() -> Result<Option<PathBuf>> {
    let p = config_path();
    if p.exists() {
        return Ok(None);
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let cfg = Config::default();
    std::fs::write(&p, toml::to_string_pretty(&cfg)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(Some(p))
}

pub fn ensure_data_dir(cfg: &Config) -> Result<()> {
    std::fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("建数据目录失败: {}", cfg.data_dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_stepfun() {
        let c = Config::default();
        assert_eq!(c.realtime.provider, Provider::Stepfun);
        assert!(c.realtime.endpoint.starts_with("wss://"));
        assert_eq!(c.wake_word, "小蝶");
    }

    #[test]
    fn env_overrides_provider() {
        std::env::set_var("DIEMENG_PROVIDER", "qwen");
        let mut c = Config::default();
        c.apply_env();
        assert_eq!(c.realtime.provider, Provider::Qwen);
        std::env::remove_var("DIEMENG_PROVIDER");
    }

    #[test]
    fn toml_roundtrip_keeps_fields() {
        let c = Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back = Config::from_toml_str(&s).unwrap();
        assert_eq!(back.realtime.model, c.realtime.model);
        assert_eq!(back.idle_timeout_ms, c.idle_timeout_ms);
    }

    #[test]
    fn key_is_masked_in_log() {
        std::env::set_var("DIEMENG_TEST_KEY", "abcdefghijklmnop");
        let mut c = Config::default();
        c.realtime.api_key_env = "DIEMENG_TEST_KEY".into();
        assert_eq!(c.masked_key(), "abcd****mnop");
        std::env::remove_var("DIEMENG_TEST_KEY");
    }

    #[test]
    fn key_never_in_toml_serialization() {
        std::env::set_var("DIEMENG_TEST_KEY2", "supersecretvalue");
        let mut c = Config::default();
        c.realtime.api_key_env = "DIEMENG_TEST_KEY2".into();
        let s = toml::to_string_pretty(&c).unwrap();
        assert!(!s.contains("supersecretvalue"), "密钥不能出现在配置序列化里");
        std::env::remove_var("DIEMENG_TEST_KEY2");
    }
}
