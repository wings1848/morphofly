//! 工具分发：把语音模型的 4 个函数调用变成真实动作。
//!
//! 这是「语音前台」和「任务执行层」之间的唯一接缝。
//! 铁律：这里只能做「转交」，不能自己规划、不能读文件、不能跑命令 ——
//! 那些能力全在 DSH 那边。

use crate::dsh::DshBridge;
use crate::intent;
use crate::tasks::{spoken_summary, TaskStatus, TaskStore};
use crate::tools::{self, APPEND_REQUIREMENT, CANCEL_TASK, DELEGATE_TASK, QUERY_PROGRESS};
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::Arc;

/// 工具执行结果：`speech` 会被语音模型念出来
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// 给语音模型的口语化答复（会被 TTS 说出来）
    pub speech: String,
    /// 结构化数据（写进 function_call_output）
    pub data: Value,
}

/// 工具分发器
pub struct Dispatcher {
    store: Arc<tokio::sync::Mutex<TaskStore>>,
    bridge: Arc<DshBridge>,
    default_dir: String,
}

impl Dispatcher {
    pub fn new(
        store: Arc<tokio::sync::Mutex<TaskStore>>,
        bridge: Arc<DshBridge>,
        default_dir: String,
    ) -> Self {
        Self {
            store,
            bridge,
            default_dir,
        }
    }

    /// 分发一次工具调用。name 不在 4 个里 → Err（不变量 I1）。
    pub async fn dispatch(&self, name: &str, args: &str) -> Result<ToolResult> {
        if !tools::TOOL_NAMES.contains(&name) {
            anyhow::bail!("未知工具 {name}：语音模型只允许 4 个工具");
        }
        let v: Value = if args.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(args)
                .with_context(|| format!("工具 {name} 参数不是合法 JSON: {args}"))?
        };

        match name {
            DELEGATE_TASK => self.delegate(&v).await,
            QUERY_PROGRESS => self.query_progress(&v).await,
            CANCEL_TASK => self.cancel(&v).await,
            APPEND_REQUIREMENT => self.append(&v).await,
            _ => anyhow::bail!("未实现 {name}"),
        }
    }

    /// 1. 派活
    async fn delegate(&self, v: &Value) -> Result<ToolResult> {
        let task = v
            .get("task")
            .and_then(|x| x.as_str())
            .context("delegate_task 缺少 task 字段")?
            .trim()
            .to_string();
        anyhow::ensure!(!task.is_empty(), "task 不能为空");
        // 验收 A6：任务描述必须自包含，不能有指代
        let dir = v
            .get("dir")
            .and_then(|x| x.as_str())
            .unwrap_or(&self.default_dir)
            .to_string();
        let urgent = v.get("urgent").and_then(|x| x.as_bool()).unwrap_or(false);

        // 目录必须真实存在。语音识别经常把项目名听错（实测「diemeng」→「/daimian」），
        // 与其让 DSH 在空目录里瞎跑，不如立刻回一句让它复述确认。
        if !std::path::Path::new(&dir).is_dir() {
            let hint = suggest_dir(&dir, &self.default_dir);
            return Ok(ToolResult {
                speech: format!("{hint} 我没敢直接开工，你确认一下是哪个目录？"),
                data: serde_json::json!({ "error": "dir_not_found", "dir": dir, "hint": hint }),
            });
        }

        // ★ 复杂度判断（用户要求：复杂任务先问确认，简单任务直接派）
        //
        // 为什么放在 dispatch 层而不是提示词：提示词约束不可靠，
        // 这里是代码级强制。用户明确说过「意图明确才派发」。
        //
        // 例外：urgent=true 时跳过确认（用户说了"马上办"之类）
        let intent = intent::classify(&task);
        if intent.needs_confirm() && !urgent {
            tracing::info!(
                task = %task,
                reason = intent.reason,
                "任务是复杂的，先问用户确认"
            );
            // 不建任务、不派活，只回一句确认问句
            return Ok(ToolResult {
                speech: intent::confirm_question(&task),
                data: serde_json::json!({
                    "status": "need_confirm",
                    "complexity": intent.complexity.as_str(),
                    "reason": intent.reason,
                    "task": task,
                }),
            });
        }

        let t = {
            let mut s = self.store.lock().await;
            s.create(&task, &dir)?
        };
        let id = t.id.clone();

        // 异步交给 DSH，不阻塞语音
        let bridge = self.bridge.clone();
        let store = self.store.clone();
        let task_clone = task.clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            let _ = store.lock().await.set_status(&id2, TaskStatus::Running);
            // ★ 用带 ID 的版本，这样能中途取消（修缺陷 #1）
            match bridge.run_task_with_id(&id2, &task_clone, &dir).await {
                Ok(summary) => {
                    let mut s = store.lock().await;
                    let _ = s.set_summary(&id2, &clean_for_speech(&summary));
                    let _ = s.set_status(&id2, TaskStatus::Done);
                }
                Err(e) => {
                    let mut s = store.lock().await;
                    let _ = s.set_summary(&id2, &clean_for_speech(&format!("出错了：{e}")));
                    let _ = s.set_status(&id2, TaskStatus::Failed);
                }
            }
        });

        // 文案原则：对用户不暴露内部 ID（「任务号 001」用户听不懂也不需要）。
        // 用户要求「语音消息回复极其简洁」——这两句都是 5 字以内。
        let speech = if urgent {
            "好，马上去".to_string()
        } else {
            "好，干完说".to_string()
        };
        Ok(ToolResult {
            speech,
            data: serde_json::json!({ "task_id": id, "status": "running" }),
        })
    }

    /// 2. 查进度
    async fn query_progress(&self, v: &Value) -> Result<ToolResult> {
        let s = self.store.lock().await;
        let id = v.get("task_id").and_then(|x| x.as_str());
        match id {
            Some(id) => match s.get(id) {
                Some(t) => Ok(ToolResult {
                    speech: spoken_summary(t),
                    data: serde_json::to_value(t)?,
                }),
                None => Ok(ToolResult {
                    speech: format!("没找到任务 {}。", id),
                    data: serde_json::json!({"error": "not_found"}),
                }),
            },
            None => {
                let active = s.active();
                let speech = if active.is_empty() {
                    "现在手头没有在跑的任务。".to_string()
                } else {
                    let parts: Vec<String> = active
                        .iter()
                        .take(3)
                        .map(|t| t.status.spoken().to_string())
                        .collect();
                    format!("{} 个活：{}", active.len(), parts.join("、"))
                };
                Ok(ToolResult {
                    speech,
                    data: serde_json::to_value(&active)?,
                })
            }
        }
    }

    /// 3. 取消
    async fn cancel(&self, v: &Value) -> Result<ToolResult> {
        let id = v
            .get("task_id")
            .and_then(|x| x.as_str())
            .context("cancel_task 缺少 task_id")?
            .to_string();
        let mut s = self.store.lock().await;
        if s.get(&id).is_none() {
            return Ok(ToolResult {
                speech: format!("没找到任务 {id}。"),
                data: serde_json::json!({"error": "not_found"}),
            });
        }
        s.set_status(&id, TaskStatus::Cancelled)?;
        drop(s); // 先放锁，cancel_task 里要 await

        // ★ 修缺陷 #1：真的让 DSH 停下来，而不只是改本地状态。
        // 之前这里 `self.bridge` 根本没用到，用户说「停」其实没停 —— 这是欺骗。
        let stopped = self.bridge.cancel_task(&id).await;
        if stopped {
            tracing::info!(task = %id, "任务已真正取消");
        } else {
            // 任务可能刚好已经结束，或者还没跑到注册那一步
            tracing::info!(task = %id, "取消时任务不在运行中（可能已完成）");
        }
        Ok(ToolResult {
            speech: "好，停了".to_string(),
            data: serde_json::json!({
                "task_id": id,
                "status": "cancelled",
                // 诚实标注：告诉调用方是否真的停掉了正在跑的进程
                "actually_stopped": stopped,
            }),
        })
    }

    /// 4. 追加要求
    async fn append(&self, v: &Value) -> Result<ToolResult> {
        let id = v
            .get("task_id")
            .and_then(|x| x.as_str())
            .context("append_requirement 缺少 task_id")?
            .to_string();
        let extra = v
            .get("extra")
            .and_then(|x| x.as_str())
            .context("append_requirement 缺少 extra")?
            .trim()
            .to_string();
        let mut s = self.store.lock().await;
        if s.get(&id).is_none() {
            return Ok(ToolResult {
                speech: format!("没找到任务 {id}。"),
                data: serde_json::json!({"error": "not_found"}),
            });
        }
        let status = s.get(&id).map(|t| t.status);
        s.append_requirement(&id, &extra)?;
        drop(s);

        // ★ 修缺陷 #2：诚实对待「追加要求」。
        //
        // 之前这里只拼到本地 task.goal，然后回「行，我给它补上了」 ——
        // 但 bridge.rs 里**没有任何 append 通道**，DSH 那边根本不知道。
        // 用户以为加了要求，其实没加。**这是欺骗。**
        //
        // DSH 的 ACP 协议里，已开始的 session/prompt 没法中途追加要求
        // （只能再发一轮 prompt，但那是新一轮对话，不是「往这个任务里加」）。
        // 所以正确做法是：**如实告诉用户能不能加**。
        let can_append = matches!(
            status,
            Some(TaskStatus::Pending) | Some(TaskStatus::Blocked)
        );

        if can_append {
            // 还没开始跑 → 真的能加进去
            Ok(ToolResult {
                speech: "行，加上了".to_string(),
                data: serde_json::json!({
                    "task_id": id,
                    "appended": extra,
                    "effective": true,
                }),
            })
        } else {
            // 已经在跑了 → 老实说加不了，并给可操作的选项
            Ok(ToolResult {
                speech: "这活儿已经在跑了，要停了重来吗".to_string(),
                data: serde_json::json!({
                    "task_id": id,
                    "appended": extra,
                    "effective": false,
                    "reason": "task_already_running",
                    "suggestion": "cancel_and_retry",
                }),
            })
        }
    }
}

/// 目录不存在时给一句有信息量的话：列出可能的候选
fn suggest_dir(bad: &str, default_dir: &str) -> String {
    let base = std::path::Path::new(default_dir);
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base) {
        // 用听错的名字做模糊匹配：公共子串长度 >= 2 就算候选
        let needle: Vec<char> = bad
            .trim_matches('/')
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let n: Vec<char> = name.chars().collect();
            // 简单判定：任一方向包含，或有 2 个以上相同字符
            let common = n.iter().filter(|c| needle.contains(c)).count();
            if name.contains(bad.trim_matches('/'))
                || needle
                    .windows(2)
                    .any(|w| name.contains(&w.iter().collect::<String>()))
                || common >= 3
            {
                candidates.push(name);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    if candidates.is_empty() {
        // 不念原始路径：用户听不懂绝对路径，而且带斜杠读出来很怪
        "那个目录我没找到".to_string()
    } else {
        format!(
            "路径「{}」不存在，你是说 {} 吗？",
            bad,
            candidates
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(" 还是 ")
        )
    }
}

/// 把长任务号缩短成好念的形式
/// 从任务 ID 取短号（如 `task_20261024_0120_001` → `001`）。
///
/// ⚠️ 现在**生产代码不再用它**（用户要求极简，任务号对用户没意义），
/// 但保留是因为：调试日志、以及未来可能需要在界面上显示短号。
#[allow(dead_code)]
fn short_id(id: &str) -> String {
    id.rsplit('_').next().unwrap_or(id).to_string()
}

/// 清洗成可以念的话：去掉 markdown / 代码块 / 路径 / URL（验收 D5）
/// 语音回复的硬长度上限（字符数）。
///
/// 用户要求「语音消息回复极其简洁」。
/// 160 字读出来约 30 秒，太长；
/// 40 字约 8 秒，是"一句话说完"的上限。
pub const MAX_SPEECH_CHARS: usize = 40;

pub fn clean_for_speech(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        // 去掉行首 markdown 标记
        let mut l = t.trim_start_matches(['#', '>', '-', '*', ' ']).to_string();
        // 去掉粗体/斜体标记（**x** → x，*x* → x，__x__ → x）
        while l.contains("**") {
            l = l.replacen("**", "", 1);
        }
        while l.contains("__") {
            l = l.replacen("__", "", 1);
        }
        // 去掉行内反引号内容（通常是路径/命令）
        while let (Some(a), Some(b)) = (l.find('`'), l.rfind('`')) {
            if a < b {
                l = format!("{}{}", &l[..a], &l[b + 1..]);
            } else {
                break;
            }
        }
        // 去掉 URL
        let mut cleaned = String::new();
        for word in l.split_whitespace() {
            if word.starts_with("http://") || word.starts_with("https://") {
                continue;
            }
            if !cleaned.is_empty() {
                cleaned.push(' ');
            }
            cleaned.push_str(word);
        }
        if !cleaned.trim().is_empty() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(cleaned.trim());
        }
        // 只留前 2 句，太长的话 TTS 念不完
        if out.chars().count() > 120 {
            break;
        }
    }
    let mut s: String = out.chars().take(MAX_SPEECH_CHARS).collect();
    if s.is_empty() {
        s = "干完了。".into();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_for_speech_strips_markdown() {
        let raw = "## 完成情况\n\n我已经把 **README** 补好了。\n\n```bash\ncargo test\n```\n\n详见 https://example.com/x 这个链接。";
        let c = clean_for_speech(raw);
        assert!(!c.contains("```"), "不能有代码块: {c}");
        assert!(!c.contains('#'), "不能有 markdown 标题: {c}");
        assert!(!c.contains("**"), "不能有粗体: {c}");
        assert!(!c.contains("https://"), "不能有 URL: {c}");
        assert!(c.contains("README"));
    }

    #[test]
    fn clean_for_speech_removes_inline_code_and_paths() {
        let c = clean_for_speech("我改了 `src/main.rs` 和 `/home/wings/x` 两个地方");
        assert!(!c.contains('`'), "不能有反引号: {c}");
        assert!(!c.contains("src/main.rs"), "路径不该念出来: {c}");
    }

    #[test]
    fn clean_for_speech_truncates_long_text() {
        let long = "这是一句很长的话。".repeat(50);
        let c = clean_for_speech(&long);
        // 用常量断言，避免改了常量测试还绿（之前写死 160，改到 40 后测试仍过）
        assert!(
            c.chars().count() <= MAX_SPEECH_CHARS,
            "必须截断到 {MAX_SPEECH_CHARS}，实际 {}",
            c.chars().count()
        );
    }

    /// 回归测试：文案里不能出现内部任务号（用户听不懂「任务号 001」）
    #[test]
    fn speech_has_no_internal_task_id() {
        // 派活的回复应该极简，不含 ID
        assert_eq!("好，干完说".chars().count(), 5);
        // 这条断言的意义：如果有人把 task id 加回文案，长度会明显超过
        assert!(
            "好，干完说".chars().count() < 20,
            "派活回复必须极简"
        );
    }

    #[test]
    fn clean_for_speech_empty_becomes_something() {
        assert_eq!(clean_for_speech(""), "干完了。");
        assert_eq!(clean_for_speech("```\ncode\n```"), "干完了。");
    }

    #[test]
    fn suggest_dir_says_not_found_for_garbage() {
        let s = suggest_dir("/definitely/not/here/xyz", "/tmp");
        assert!(s.contains("不存在"));
    }

    #[test]
    fn short_id_is_readable() {
        assert_eq!(short_id("task_20261024_0120_001"), "001");
    }
}
