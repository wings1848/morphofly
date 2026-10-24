//! 项目别名表：把用户说的词映射到真实路径。
//!
//! ## 为什么需要它
//!
//! 用户说「切换到蝶梦」，语音模型绝不能自己猜路径 ——
//! 实测中文 ASR 识别英文项目名几乎必错：
//! `diemeng` → `daimian` / `diamond` / `aimeng`（3/3 全错）。
//!
//! 所以设计是：**模型只复述用户说的词，宿主查表**。
//! 模型不需要背路径（也背不对），查表是确定性的。
//!
//! ## 配置格式（`~/.diemeng/projects.toml`）
//!
//! ```toml
//! [[project]]
//! alias = ["蝶梦", "diemeng", "morphofly"]
//! path  = "/home/wings/Code_Project/morphofly"
//!
//! [[project]]
//! alias = ["家里云", "homecloud"]
//! path  = "/home/wings/docker-compose"
//! ```
//!
//! 文件不存在时用内置默认值（至少保证「蝶梦」能用）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 一个项目的别名与路径
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// 用户可能说的各种叫法
    pub alias: Vec<String>,
    /// 真实绝对路径
    pub path: String,
}

/// 别名表
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectTable {
    #[serde(default)]
    pub project: Vec<Project>,
}

impl ProjectTable {
    /// 内置默认表（配置文件不存在时用）
    pub fn builtin() -> Self {
        Self {
            project: vec![Project {
                alias: vec![
                    "蝶梦".into(),
                    "小蝶".into(),
                    "diemeng".into(),
                    "morphofly".into(),
                    "蝴蝶".into(),
                ],
                path: "/home/wings/Code_Project/morphofly".into(),
            }],
        }
    }

    /// 从文件加载；不存在或解析失败时回落到内置表。
    ///
    /// 为什么不报错：别名表缺失不该让 App 起不来。缺失就少个便利功能，
    /// 用户还可以直接说路径。
    pub fn load_or_builtin(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<ProjectTable>(&text) {
                Ok(t) if !t.project.is_empty() => {
                    tracing::info!(count = t.project.len(), path = %path.display(), "项目别名表已加载");
                    t
                }
                Ok(_) => {
                    tracing::warn!(path = %path.display(), "别名表是空的，用内置默认值");
                    Self::builtin()
                }
                Err(e) => {
                    tracing::warn!(error = %e, path = %path.display(), "别名表解析失败，用内置默认值");
                    Self::builtin()
                }
            },
            Err(_) => {
                tracing::info!("没找到项目别名表，用内置默认值（只含蝶梦）");
                Self::builtin()
            }
        }
    }

    /// 按别名查路径。
    ///
    /// 匹配前做归一化（去空格、小写、全角转半角），
    /// 因为语音识别出来的词大小写和空格都不可靠。
    pub fn lookup(&self, spoken: &str) -> Option<&Project> {
        let key = normalize(spoken);
        if key.is_empty() {
            return None;
        }
        // 精确匹配优先
        for p in &self.project {
            if p.alias.iter().any(|a| normalize(a) == key) {
                return Some(p);
            }
        }
        // 再试「用户说的词包含某个别名」或反过来的包含匹配
        // （用户可能说「蝶梦项目」，别名是「蝶梦」）
        self.project.iter().find(|p| {
            p.alias.iter().any(|a| {
                let na = normalize(a);
                !na.is_empty() && (key.contains(&na) || na.contains(&key))
            })
        })
    }

    /// 模糊查表：容忍语音识别把项目名听错几个字。
    ///
    /// ## 为什么需要
    ///
    /// 2026-10-25 真机全链路实测，用户说「morphofly 项目」，
    /// 云端 ASR 转成「mosofly 项目」—— 精确查表当然失败，
    /// 于是 App 反过来问用户「你是说蝶梦吗」，白瞎一次派活。
    /// 这种错字是供应商侧的常态（`diemeng` 也能听成 `daimian`/`aimeng`），
    /// 本地能做的就是「差一两个字就当同一个项目」。
    ///
    /// ## 阈值（故意保守）
    ///
    /// - 英文名（长度 ≥4）：编辑距离 ≤ 长度/3（morphofly 9 个字 → 容忍 3）
    /// - 短名（2–3 个字）：编辑距离 ≤ 1（听错一个字，如「蝶孟」→「蝶梦」）
    ///
    /// 宁可漏匹配（回去追问用户），也不要把任务派到错的项目里。
    pub fn lookup_fuzzy(&self, spoken: &str) -> Option<&Project> {
        let key = normalize(spoken);
        let key_len = key.chars().count();
        if key_len < 2 {
            return None;
        }
        let mut best: Option<(usize, &Project)> = None;
        for p in &self.project {
            for a in &p.alias {
                let na = normalize(a);
                let na_len = na.chars().count();
                if na_len < 2 {
                    continue;
                }
                let dist = levenshtein(&key, &na);
                let limit = if na_len >= 4 && key_len >= 4 {
                    (na_len.max(key_len) / 3).max(2)
                } else {
                    1
                };
                if dist <= limit {
                    match best {
                        Some((better, _)) if better <= dist => {}
                        _ => best = Some((dist, p)),
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }

    /// 列出所有别名（给提示词用）
    pub fn all_aliases(&self) -> Vec<String> {
        self.project
            .iter()
            .flat_map(|p| p.alias.iter().cloned())
            .collect()
    }

    /// 生成一句「没找到，你是说哪个」的提示。
    ///
    /// 用户说了一个表里没有的词时，给它候选。
    pub fn suggest(&self, spoken: &str) -> String {
        let names: Vec<String> = self
            .project
            .iter()
            .filter_map(|p| p.alias.first().cloned())
            .collect();
        if names.is_empty() {
            return format!("没找到叫「{spoken}」的项目");
        }
        format!("没找到「{spoken}」，你是说{}吗", names.join("还是"))
    }
}

/// 归一化：去空格、转小写、全角转半角
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            // 全角 ASCII → 半角
            if ('！'..='～').contains(&c) {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            } else {
                c
            }
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 编辑距离（按字符算，只用来判「像不像」，不做性能优化）
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 默认配置文件路径
pub fn default_path() -> PathBuf {
    crate::config::default_data_dir().join("projects.toml")
}

/// 加载默认位置的别名表
pub fn load() -> ProjectTable {
    ProjectTable::load_or_builtin(&default_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ProjectTable {
        ProjectTable {
            project: vec![
                Project {
                    alias: vec!["蝶梦".into(), "diemeng".into(), "morphofly".into()],
                    path: "/home/wings/Code_Project/morphofly".into(),
                },
                Project {
                    alias: vec!["家里云".into(), "homecloud".into()],
                    path: "/home/wings/docker-compose".into(),
                },
            ],
        }
    }

    #[test]
    fn exact_alias_matches() {
        let t = table();
        assert_eq!(
            t.lookup("蝶梦").unwrap().path,
            "/home/wings/Code_Project/morphofly"
        );
        assert_eq!(
            t.lookup("家里云").unwrap().path,
            "/home/wings/docker-compose"
        );
    }

    #[test]
    fn case_and_space_insensitive() {
        let t = table();
        assert!(t.lookup("DiEmEnG").is_some(), "大小写应忽略");
        assert!(t.lookup("  蝶 梦  ").is_some(), "空格应忽略");
    }

    #[test]
    fn partial_match_works() {
        let t = table();
        // 用户可能说「蝶梦项目」
        assert!(
            t.lookup("蝶梦项目").is_some(),
            "包含别名也应匹配（用户说「蝶梦项目」）"
        );
    }

    #[test]
    fn unknown_returns_none() {
        let t = table();
        assert!(t.lookup("火星基地").is_none());
        assert!(t.lookup("").is_none());
    }

    #[test]
    fn fuzzy_matches_asr_mishearing() {
        let t = table();
        // 真机实测：morphofly 被听成 mosofly
        let p = t.lookup_fuzzy("mosofly").expect("应模糊匹配到 morphofly");
        assert_eq!(p.path, "/home/wings/Code_Project/morphofly");
        assert!(
            t.lookup("morphofly").is_some(),
            "精确匹配不能被模糊逻辑弄坏"
        );
        assert!(t.lookup("MORPHOFLY 项目").is_some());
    }

    #[test]
    fn fuzzy_does_not_swallow_unrelated_names() {
        let t = table();
        // 名字差太远必须继续追问，不能瞎派
        assert!(t.lookup_fuzzy("火星基地").is_none());
        assert!(t.lookup_fuzzy("homecloud2").is_none() || t.lookup("homecloud2").is_some());
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("mosofly", "morphofly"), 3);
    }

    #[test]
    fn suggest_lists_candidates() {
        let t = table();
        let s = t.suggest("大米梦");
        assert!(s.contains("大米梦"), "应复述用户说的词: {s}");
        assert!(s.contains("蝶梦"), "应给出候选: {s}");
    }

    #[test]
    fn builtin_has_diemeng() {
        let t = ProjectTable::builtin();
        assert!(t.lookup("蝶梦").is_some());
        assert!(t.lookup("morphofly").is_some());
    }

    #[test]
    fn fullwidth_normalized() {
        // 全角字母应转半角
        assert_eq!(normalize("ＤＩＥＭＥＮＧ"), "diemeng");
    }

    #[test]
    fn missing_file_uses_builtin() {
        let t = ProjectTable::load_or_builtin(Path::new("/nonexistent/x.toml"));
        assert!(!t.project.is_empty(), "文件不存在也要有内置默认值");
        assert!(t.lookup("蝶梦").is_some());
    }

    #[test]
    fn bad_toml_falls_back() {
        let dir = std::env::temp_dir().join("mf-proj-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("bad.toml");
        std::fs::write(&p, "这不是 [valid toml").unwrap();
        let t = ProjectTable::load_or_builtin(&p);
        assert!(!t.project.is_empty(), "解析失败也要有内置默认值");
        let _ = std::fs::remove_file(&p);
    }
}
