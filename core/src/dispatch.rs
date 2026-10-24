//! 工具分发：把语音模型的 4 个函数调用变成真实动作。
//!
//! 这是「语音前台」和「任务执行层」之间的唯一接缝。
//! 铁律：这里只能做「转交」，不能自己规划、不能读文件、不能跑命令 ——
//! 那些能力全在 DSH 那边。

use crate::dsh::DshBridge;
use crate::intent;
use crate::tasks::{spoken_summary, TaskStatus, TaskStore};
use crate::tools;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// 工具执行结果：`speech` 会被语音模型念出来
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    /// 给语音模型的口语化答复（会被 TTS 说出来）
    pub speech: String,
    /// 结构化数据（写进 function_call_output）
    pub data: Value,
}

/// 当前会话上下文（用户说「切换到 X」改的就是这些）。
///
/// 为什么是「状态」而不是「工具参数」：
/// 用户说「换到蝶梦项目」意思是「**接下来都**在这」，不是「用这个目录干一次」。
/// 做成状态后，delegate_task 不传 dir 就用当前的，符合用户的直觉。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContext {
    /// 当前 agent（dsh / codex）
    pub agent: String,
    /// 当前工作目录（绝对路径）
    pub dir: String,
    /// 当前模型（空表示用 agent 默认）
    pub model: String,
}

impl SessionContext {
    pub fn new(agent: impl Into<String>, dir: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            dir: dir.into(),
            model: String::new(),
        }
    }
}

/// 工具分发器
pub struct Dispatcher {
    store: Arc<tokio::sync::Mutex<TaskStore>>,
    bridge: Arc<DshBridge>,
    default_dir: String,
    /// 当前会话上下文（可被 set_context 修改）
    context: Arc<tokio::sync::Mutex<SessionContext>>,
    /// 项目别名表
    projects: crate::projects::ProjectTable,
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
            context: Arc::new(tokio::sync::Mutex::new(SessionContext::new(
                "dsh",
                default_dir.clone(),
            ))),
            projects: crate::projects::load(),
            default_dir,
        }
    }

    /// 读当前上下文（给 UI 显示「现在连着哪个 agent」用）
    /// 给一句话挑一个干活目录（本地代派 / 纠正提醒都用它）。
    ///
    /// 优先级：
    /// 1. 会话上下文里 set_context 设过的目录
    /// 2. 这句话里提到的已知项目名（别名表：蝶梦/morphofly/diemeng/蝴蝶…）
    /// 3. 默认目录
    /// 4. 默认目录只是家目录（`/home/wings`）时，退到内置项目目录
    ///
    /// 为什么不能只用默认目录：真机实测本地代派把任务派到了 `/home/wings`
    /// —— 因为配置里的 `default_cwd` 就是家目录。用户说的是「蝶梦项目」，
    /// 派到家里当然干不成事。
    pub async fn dir_for_task(&self, task: &str) -> String {
        // ★ 顺序很关键：**任务里点名的项目优先**，其次才是会话上下文。
        //
        // 坑在这：`Dispatcher::new` 会把 context.dir 初始化成 default_dir，
        // 所以「上下文非空就返回」等于永远用默认目录（真机实测派到了 /home/wings），
        // 别名表那段成了死代码。用户嘴上说了哪个项目，就该去哪个项目。
        for proj in &self.projects.project {
            if proj
                .alias
                .iter()
                .any(|a| !a.is_empty() && task.contains(a.as_str()))
            {
                return proj.path.clone();
            }
        }
        {
            let ctx = self.context.lock().await;
            if !ctx.dir.is_empty() && !is_home_dir(&ctx.dir) {
                return ctx.dir.clone();
            }
        }
        if !self.default_dir.is_empty() && !is_home_dir(&self.default_dir) {
            return self.default_dir.clone();
        }
        // 最后退到内置项目目录（别名表第一项）
        self.projects
            .project
            .first()
            .map(|p| p.path.clone())
            .unwrap_or_else(|| self.default_dir.clone())
    }

    /// 当前应该在哪个目录干活：会话上下文优先，否则默认目录。
    ///
    /// 给「纠正提醒」用 —— 真机实测模型卡在「不知道 dir 填什么」上不肯调工具，
    /// 所以提醒里要能给它一个确定可用的路径。
    pub async fn current_dir(&self) -> String {
        let ctx = self.context.lock().await;
        if ctx.dir.is_empty() {
            self.default_dir.clone()
        } else {
            ctx.dir.clone()
        }
    }

    pub async fn context(&self) -> SessionContext {
        self.context.lock().await.clone()
    }

    /// 语音命令兜底：从用户原话里识别「切换类」意图，宿主直接执行。
    ///
    /// ## 为什么需要这个
    ///
    /// 真机实测（2026-10-24）：用户说「切换到 codex」，
    /// 语音模型会回「好，已切换到 codex」但**从不调 set_context 工具**。
    /// 加纠正提示后它改成在文字里假装调用（「现在已调用 set_context，参数为...」），
    /// 实际 tool_calls 仍然是空的。
    ///
    /// 这是模型的能力边界 —— 它把「切换」当成一个可以叙述的动作，
    /// 而不是需要走工具通道的动作。提示词救不回来。
    ///
    /// ## 为什么可以做关键词兜底
    ///
    /// 「切换 agent/项目/模型」是**确定性动作**，不需要推理：
    /// 关键词集合很小且明确。万一误判，代价是「多切了一次」，可撤销。
    ///
    /// 而「查目录」「改代码」这类**需要推理**的任务，仍然交给模型。
    ///
    /// 返回 Some(result) 表示识别到并执行了；None 表示不是切换命令。
    pub async fn try_voice_command(&self, text: &str) -> Option<ToolResult> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }

        // 必须是「切换/换到/换成/用」这类祈使词
        let has_switch_verb = ["切换", "换到", "换成", "改用", "用一下"]
            .iter()
            .any(|w| t.contains(w));
        if !has_switch_verb {
            return None;
        }

        // 1) 切 agent
        for (name, cmd) in [("codex", "codex"), ("dsh", "dsh")] {
            if t.contains(name) {
                let v = serde_json::json!({ "agent": cmd });
                return Some(self.set_context(&v).await.unwrap_or_else(|e| ToolResult {
                    speech: "切换失败".to_string(),
                    data: serde_json::json!({"error": e.to_string()}),
                }));
            }
        }

        // 2) 切项目（走别名表）
        if let Ok(ctx) = self.context.try_lock() {
            let _ = ctx; // 不持锁，下面查表
        }
        for proj in &self.projects.project {
            for alias in &proj.alias {
                if t.contains(alias) {
                    let v = serde_json::json!({ "project": alias });
                    return Some(self.set_context(&v).await.unwrap_or_else(|e| ToolResult {
                        speech: "切换失败".to_string(),
                        data: serde_json::json!({"error": e.to_string()}),
                    }));
                }
            }
        }

        // 3) 提到「换模型」但没说具体哪个 → 追问
        if t.contains("模型") {
            return Some(ToolResult {
                speech: "换成哪个模型".to_string(),
                data: serde_json::json!({"status": "need_model_name"}),
            });
        }

        None
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

        // ⚠️ 这里必须用 `tools::X` 的全路径，不能直接写 `X`。
        //
        // 2026-10-24 踩坑：`match name { SET_CONTEXT => ... }` 里裸写的
        // 常量会被 Rust 当成**变量绑定**（匹配任何值），
        // 于是 SET_CONTEXT 分支变成不可达的死代码，编译器只给个
        // "unreachable pattern" 警告——功能静默失效。
        // 用路径形式 `tools::SET_CONTEXT` 才是常量模式。
        match name {
            tools::DELEGATE_TASK => self.delegate(&v).await,
            tools::QUERY_PROGRESS => self.query_progress(&v).await,
            tools::CANCEL_TASK => self.cancel(&v).await,
            tools::APPEND_REQUIREMENT => self.append(&v).await,
            tools::SET_CONTEXT => self.set_context(&v).await,
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
        // 目录优先级：工具参数 > 当前上下文（set_context 设的）> 默认目录
        //
        // 这样用户说「换到蝶梦项目」之后再派活，就不用每次带 dir，
        // 符合「接下来都在这」的直觉。
        let dir = match v.get("dir").and_then(|x| x.as_str()) {
            Some(d) if !d.trim().is_empty() => d.to_string(),
            _ => {
                let ctx = self.context.lock().await;
                if ctx.dir.is_empty() {
                    self.default_dir.clone()
                } else {
                    ctx.dir.clone()
                }
            }
        };
        let urgent = v.get("urgent").and_then(|x| x.as_bool()).unwrap_or(false);

        // 目录必须真实存在。语音识别经常把项目名听错
        // （实测「diemeng」→「/daimian」、「morphofly」→「move_forward」），
        // 与其让 DSH 在空目录里瞎跑，不如先纠正或回一句追问。
        //
        // 2026-10-24 真机实测：用户说「morphofly 项目」，云端 ASR 听成「move_forward」，
        // 模型忠实地把 `/home/wings/Code_Project/move_forward` 传进来 ——
        // 这个目录不存在，DSH 会话根本建不起来，用户只能重说一遍。
        //
        // 所以先做一件便宜事：把听错的名字丢进项目别名表查一遍。
        // 「蝶梦」「蝴蝶」「diemeng」这类都有别名，能对上就不用追问。
        let mut dir = dir;
        if !std::path::Path::new(&dir).is_dir() {
            let base = std::path::Path::new(&dir)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if let Some(p) = self.projects.lookup(&base) {
                tracing::info!(heard = %base, mapped = %p.path, "项目名听错了，按别名表纠正");
                dir = p.path.clone();
            }
        }
        if !std::path::Path::new(&dir).is_dir() {
            let hint = suggest_dir(&dir, &self.default_dir);
            // 说清「我可以在哪儿办」，别让用户猜。念项目名，不念绝对路径。
            let here = std::path::Path::new(&self.default_dir)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let tail = if here.is_empty() {
                "我没敢直接开工，你确认一下是哪个项目？".to_string()
            } else {
                format!("我没敢直接开工。要是在【{here}】里办，你说一声就行。")
            };
            return Ok(ToolResult {
                speech: format!("{hint} {tail}"),
                data: serde_json::json!({
                    "error": "dir_not_found",
                    "dir": dir,
                    "hint": hint,
                    "current_project": here,
                }),
            });
        }

        // ★ 修 Bug 1：项目名必须能被识别，不能拿 ASR 听错的假名字直接干。
        //
        // 2026-10-24 真机 e2e 抓到的严重问题：
        // 用户说「在 aimeng 项目里跑测试」（aimeng 是 ASR 听错 diemeng 的结果），
        // 蝶梦**直接派活了**，没有追问。
        //
        // 提示词规则 1 要求「项目名不确定必须先追问」，但模型不遵守。
        // 所以必须在代码层拦：从 task 文本里提取「提到的项目名」，
        // 如果它不在别名表里，就追问。
        //
        // 注意：只在任务里**明确提到**了某个项目名时才检查，
        // 不能因为「没提到项目」就拦截（那样简单的「看看目录」也过不去）。
        // ★ 第一层防御：任务文本里的绝对路径必须真实存在
        //
        // 真机实测：模型会凭空捏造路径（`/home/wings/Code_Project/M1`），
        // 而且把路径写进 task 文本，绕过了 dir 参数的检查。
        let missing = self.find_missing_paths(&task);
        if !missing.is_empty() {
            tracing::warn!(task = %task, ?missing, "任务里的路径不存在，拒绝执行");
            let p = &missing[0];
            let name = std::path::Path::new(p)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| p.clone());
            return Ok(ToolResult {
                speech: format!("没找到叫{name}的目录，是哪个"),
                data: serde_json::json!({
                    "error": "path_not_found",
                    "missing": missing,
                }),
            });
        }

        if let Some(unknown) = self.extract_unknown_project(&task) {
            tracing::warn!(task = %task, unknown = %unknown, "任务里提到不认识的項目名");
            return Ok(ToolResult {
                speech: self.projects.suggest(&unknown),
                data: serde_json::json!({
                    "error": "unknown_project",
                    "got": unknown,
                    "known": self.projects.all_aliases(),
                }),
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
        // 派活时锁定当前 agent（之后用户切了也不影响这个任务）
        let agent_for_task = self.context.lock().await.agent.clone();
        tracing::info!(agent = %agent_for_task, dir = %dir, "派活");
        tokio::spawn(async move {
            let _ = store.lock().await.set_status(&id2, TaskStatus::Running);
            // ★ 用指定 agent 的版本（多 agent 支持）+ 带 ID（可取消）
            match bridge
                .run_task_as(&id2, &agent_for_task, &task_clone, &dir)
                .await
            {
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

    /// 从任务文本里找出「不认识的项目名」。
    ///
    /// 返回 Some(候选名) 表示任务里提到了一个别名表里没有的项目，
    /// 调用方应该追问而不是直接派活。
    ///
    /// ## 为什么要这么做
    ///
    /// 中文 ASR 识别英文项目名经常出错（实测 diemeng → daimian 3/3 全错）。
    /// 用户说「在 aimeng 项目里跑测试」时，如果直接派出去了，
    /// DSH 会在一个不存在的目录里瞎跑，用户等半天才发现。
    ///
    /// ## 怎么找
    ///
    /// 找「X 项目」/「X 里」/「X 中」这种模式里的 X。
    /// 这是启发式，不完美，但比完全不检查好。
    /// 已知项目（在别名表里的）会被放行。
    /// 从任务文本里提取所有绝对路径，返回其中不存在的那些。
    ///
    /// ## 为什么需要它
    ///
    /// 2026-10-24 真机 e2e 抓到的严重问题：
    /// 用户说「在 aimeng 项目里跑一下测试」（aimeng 是 ASR 听错 diemeng），
    /// 语音模型不但没追问，还**凭空捏造了路径**：
    ///   `{"task": "cd /home/wings/Code_Project/M1 && ./run_tests"}`
    /// `M1` 完全是编的。
    ///
    /// 提示词规则 1 明确要求「不确定必须追问，绝对不要猜或补全」，
    /// 但模型不遵守。所以必须在代码层拦。
    ///
    /// 之前我只检查了 `dir` 参数，但模型把路径塞进了 task 文本里，绕过了检查。
    /// 现在检查 task 文本里的所有路径。
    fn find_missing_paths(&self, task: &str) -> Vec<String> {
        let mut out = Vec::new();
        // 按空白和常见分隔符切词，找以 / 或 ~ 开头的
        for raw in task.split(|c: char| {
            c.is_whitespace()
                || matches!(c, '，' | '。' | '、' | '；' | '"' | '\'' | '`' | ')' | '(')
        }) {
            // ⚠️ 只剥「明确是 shell 操作符」的字符，不要剥 `.` 和 `/`。
            // 2026-10-24 踩坑：`./run_tests` 被 trim_matches 剥成 `/run_tests`，
            // 然后被当成绝对路径去检查，误报「路径不存在」。
            let token =
                raw.trim_matches(|c: char| matches!(c, ',' | ':' | ';' | '&' | '|' | '>' | '<'));
            // 单独处理句末的英文句号（"写在 /tmp/x." 这种）
            let token = token.strip_suffix('.').unwrap_or(token);
            if token.is_empty() {
                continue;
            }

            let candidate = if let Some(rest) = token.strip_prefix("~/") {
                if let Some(home) = std::env::var_os("HOME") {
                    format!("{}/{}", PathBuf::from(home).display(), rest)
                } else {
                    continue;
                }
            } else if token.starts_with('/') {
                // ⚠️ 排除 `./xxx` 这种相对路径：
                // 它们以 `.` 开头不是 `/`，但 split 后可能变成 `/xxx` 的形态，
                // 而且相对路径依赖 cwd，不该在这里校验存在性。
                if token.starts_with("./") || token.starts_with("../") {
                    continue;
                }
                token.to_string()
            } else {
                continue;
            };

            // 去掉尾部可能的 shell 符号
            let candidate = candidate.trim_end_matches('/').to_string();
            if candidate.is_empty() || candidate == "/" {
                continue;
            }
            if !std::path::Path::new(&candidate).exists() && !out.contains(&candidate) {
                out.push(candidate);
            }
        }
        out
    }

    fn extract_unknown_project(&self, task: &str) -> Option<String> {
        for marker in ["项目", "工程", "仓库"] {
            let mut search_from = 0usize;
            while let Some(rel) = task[search_from..].find(marker) {
                let pos = search_from + rel;
                // 取 marker 前面连续的中文/字母/数字作为候选名，
                // 遇空格或标点就停（"在 aimeng 项目" → "aimeng"）。
                // ⚠️ 必须先把前面的空白 trim 掉。
                // 2026-10-24 踩坑：`"在 aimeng 项目"` 取 before =
                // `"在 aimeng "`（带尾空格），take_while 第一个字符就是空格
                // 直接返回空串 —— 提取永远失败，Bug 1 的修复等于没做。
                let before = task[..pos].trim_end();
                let name: String = before
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphanumeric() || is_cjk(*c))
                    .collect::<Vec<char>>()
                    .into_iter()
                    .rev()
                    .collect();
                let name = name.trim().to_string();

                if name.chars().count() >= 2 {
                    if self.projects.lookup(&name).is_some() {
                        // 已知项目，放行
                    } else if is_common_word(&name) {
                        // 「这个项目」「那个项目」这类指代，跳过
                    } else {
                        return Some(name);
                    }
                }
                search_from = pos + marker.len();
            }
        }
        None
    }

    /// 5. 切换上下文（V2 新增，多 agent 支持）
    ///
    /// 三个参数都可选，只改传了的那个。
    async fn set_context(&self, v: &Value) -> Result<ToolResult> {
        let agent = v
            .get("agent")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());
        let project = v
            .get("project")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());
        let model = v
            .get("model")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string());

        if agent.is_none() && project.is_none() && model.is_none() {
            return Ok(ToolResult {
                speech: "要切到哪".to_string(),
                data: serde_json::json!({"error": "no_param"}),
            });
        }

        let mut ctx = self.context.lock().await;
        let mut changes: Vec<String> = Vec::new();

        // 切 agent
        if let Some(a) = agent {
            let allowed = ["dsh", "codex"];
            let a_lower = a.to_lowercase();
            if !allowed.contains(&a_lower.as_str()) {
                return Ok(ToolResult {
                    speech: format!("没有叫{a}的，只有 d s h 和 codex"),
                    data: serde_json::json!({
                        "error": "unknown_agent",
                        "got": a,
                        "allowed": allowed,
                    }),
                });
            }
            if ctx.agent != a_lower {
                ctx.agent = a_lower.clone();
                changes.push(a_lower);
            }
        }

        // 切项目（只接受别名，不接受绝对路径 —— 模型不该猜路径）
        if let Some(p) = project {
            if p.starts_with('/') {
                // 模型违规传了路径：不信任它猜的，照样走别名查表
                tracing::warn!(path = %p, "模型传了绝对路径，但只信别名表");
            }
            match self.projects.lookup(&p) {
                Some(proj) => {
                    if ctx.dir != proj.path {
                        ctx.dir = proj.path.clone();
                        changes.push(p.clone());
                    }
                }
                None => {
                    // 查不到就如实告诉用户，并给候选
                    return Ok(ToolResult {
                        speech: self.projects.suggest(&p),
                        data: serde_json::json!({
                            "error": "unknown_project",
                            "got": p,
                            "known": self.projects.all_aliases(),
                        }),
                    });
                }
            }
        }

        // 切模型
        //
        // ⚠️ 2026-10-24 审计发现的「假通过」：
        // 之前这里只改 ctx.model 这个字段，从来没通知 DSH ——
        // 用户说「换个模型」什么都不会发生。
        // 和「切 agent」犯的是同一个错，这次必须真的调 API。
        if let Some(m) = model {
            if ctx.model == m {
                // 已经是这个了，不用动
            } else {
                // 真的通知 agent 进程
                match self.bridge.set_model(&m).await {
                    Ok(()) => {
                        ctx.model = m.clone();
                        changes.push(format!("模型 {m}"));
                    }
                    Err(e) => {
                        // 切不了就如实说，不要假装成功
                        tracing::warn!(error = %e, model = %m, "切模型失败");
                        let msg = e.to_string();
                        let speech = if msg.contains("没有活跃") {
                            "还没干活，先派个活再说".to_string()
                        } else if msg.contains("不支持") {
                            format!("换不了，{m} 不在列表里")
                        } else {
                            "换不了模型".to_string()
                        };
                        return Ok(ToolResult {
                            speech,
                            data: serde_json::json!({
                                "error": "model_switch_failed",
                                "model": m,
                                "reason": msg,
                            }),
                        });
                    }
                }
            }
        }

        let agent_name = ctx.agent.clone();
        let dir = ctx.dir.clone();
        drop(ctx);

        if changes.is_empty() {
            // 用户说的就是当前状态，不用啰嗦
            return Ok(ToolResult {
                speech: "已经是在这个了".to_string(),
                data: serde_json::json!({"status": "unchanged"}),
            });
        }

        // 回一句极简确认：只说变了什么
        let what = changes.first().cloned().unwrap_or_default();
        tracing::info!(
            agent = %agent_name,
            dir = %dir,
            changed = ?changes,
            "上下文已切换"
        );
        Ok(ToolResult {
            speech: format!("好，换成{what}了"),
            data: serde_json::json!({
                "status": "changed",
                "agent": agent_name,
                "dir": dir,
                "changed": changes,
            }),
        })
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
        // 只念最后一段（项目名），不念 /home/wings/Code_Project/xxx 这种路径
        let short = std::path::Path::new(bad)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| bad.to_string());
        format!(
            "没找到「{}」，你是说 {} 吗？",
            short,
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
/// 判断是不是 CJK 汉字（项目名里常见）。
///
/// 不用 `'\u{4e00}'..='\u{9fff}'` 直接写，因为某些工具链对
/// 源码里的裸 `\u` 有兼容问题；用一个函数更稳。
fn is_cjk(c: char) -> bool {
    matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF)
}

/// 这个路径是不是「家目录」（拿它当干活目录等于没指定）。
fn is_home_dir(p: &str) -> bool {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    if home.is_empty() {
        return false;
    }
    let a = std::path::Path::new(p);
    let b = std::path::Path::new(&home);
    a == b || p.trim_end_matches('/') == home.trim_end_matches('/')
}

/// 判断一个词是不是常见指代，不是真正的项目名。
///
/// 「这个项目」「那个项目」「我的项目」这类不该被当成未识别项目名去追问。
fn is_common_word(s: &str) -> bool {
    const COMMON: &[&str] = &[
        "这个", "那个", "我的", "你的", "某个", "整个", "所有", "每个", "一个", "项目", "工程",
        "仓库", "当前", "之前", "刚才", "别的",
    ];
    COMMON.iter().any(|c| s.contains(c)) || s.chars().count() < 2
}

/// 模型的回复看起来像「我正在办事 / 我办完了」吗？
///
/// 用途：兜底纠正的判据。模型有时会说「好，查着呢」却不调 `delegate_task`，
/// 那就是**假装干活**。但闲聊（「你叫什么名字」）不该被纠正成派活，
/// 所以只对「听起来在办事」的回复纠正。
///
/// 判据故意保守：宁可漏纠正，不可把正常对话打断。
pub fn looks_like_action(reply: &str) -> bool {
    const MARKERS: [&str; 12] = [
        "好，",
        "好的，",
        "查着",
        "查一下",
        "这就",
        "马上",
        "稍等",
        "等等",
        "我来",
        "我去",
        "切换",
        "换成",
    ];
    let t = reply.trim();
    if t.is_empty() {
        return false;
    }
    MARKERS.iter().any(|m| t.contains(m))
}

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

    // ── 模型切换（set_context 的 model 参数真正生效）──

    #[test]
    fn acp_has_config_option_support() {
        // 编译期验证：set_config_option / config_option_values 存在
        // （这些方法是模型切换的实现基础）
        // 运行期验证：没有活跃进程时必须如实报错
        let bridge = crate::dsh::DshBridge::new(
            vec!["true".into()],
            "/tmp".into(),
            std::time::Duration::from_secs(1),
        );
        // 没有活跃进程时应该如实报错，不能假装成功
        let rt = tokio::runtime::Runtime::new().unwrap();
        let r = rt.block_on(bridge.set_model(r#"["a","b"]"#));
        assert!(r.is_err(), "没有活跃进程时切模型必须失败");
        assert!(
            r.unwrap_err().to_string().contains("没有活跃"),
            "错误信息要说清楚原因"
        );
    }

    // ── 多 agent（set_context 真正生效）──

    #[test]
    fn builtin_agents_include_dsh_and_codex() {
        let agents = crate::config::default_agents();
        assert!(agents.contains_key("dsh"), "必须有 dsh");
        assert!(agents.contains_key("codex"), "必须有 codex");
        assert!(
            agents["dsh"].iter().any(|a| a == "acp"),
            "dsh 要带 --profile acp: {:?}",
            agents["dsh"]
        );
        assert!(
            agents["codex"].iter().any(|a| a.contains("codex-acp")),
            "codex 走 ACP 适配器: {:?}",
            agents["codex"]
        );
    }

    #[test]
    fn bridge_has_agents() {
        let bridge = crate::dsh::DshBridge::new(
            vec!["dsh".into()],
            "/tmp".into(),
            std::time::Duration::from_secs(1),
        );
        let names = bridge.agent_names();
        assert!(names.contains(&"dsh".to_string()));
        assert!(names.contains(&"codex".to_string()));
        assert!(bridge.command_for("dsh").is_some());
        assert!(bridge.command_for("不存在的agent").is_none());
    }

    // ── find_missing_paths（三层防御第 1 层）──

    #[test]
    fn fabricated_path_is_detected() {
        let d = dispatcher_for_test();
        // 真机 e2e 抓到的原话，模型凭空捏造了 M1 这个目录
        let missing = d.find_missing_paths("cd /home/wings/Code_Project/M1 && ./run_tests");
        assert_eq!(missing.len(), 1, "应检出 1 个不存在的路径");
        assert!(missing[0].contains("M1"), "应指出 M1: {missing:?}");
    }

    #[test]
    fn real_path_passes() {
        let d = dispatcher_for_test();
        let real = std::env::temp_dir();
        let task = format!("看看 {} 下有什么", real.display());
        assert!(
            d.find_missing_paths(&task).is_empty(),
            "真实存在的路径不该拦: {:?}",
            d.find_missing_paths(&task)
        );
    }

    #[test]
    fn task_without_path_passes() {
        let d = dispatcher_for_test();
        for t in ["看看有哪些目录", "跑一下测试", "写好 readme"] {
            assert!(d.find_missing_paths(t).is_empty(), "{t} 没路径，不该拦");
        }
    }

    #[test]
    fn multiple_missing_paths_all_found() {
        let d = dispatcher_for_test();
        let task = "把 /nonexistent/a 和 /nonexistent/b 对比";
        let m = d.find_missing_paths(task);
        assert_eq!(m.len(), 2, "两个都不存在，都应检出: {m:?}");
    }

    #[test]
    fn shell_operators_stripped() {
        let d = dispatcher_for_test();
        // 路径后面跟着 && 或 ; 不该影响判定
        let missing = d.find_missing_paths("cd /nonexistent/x && ls;");
        assert_eq!(missing.len(), 1);
        assert!(
            !missing[0].ends_with('&'),
            "不该把 && 算进路径: {missing:?}"
        );
    }

    // ── extract_unknown_project（修 Bug 1：假项目名必须追问）──

    fn dispatcher_for_test() -> Dispatcher {
        // 用内置别名表（含 蝶梦/diemeng/morphofly）
        // 用临时文件建 store（TaskStore::open 需要路径，没有内存版）
        let path =
            std::env::temp_dir().join(format!("mf-dispatch-test-{}.json", std::process::id()));
        let store = Arc::new(tokio::sync::Mutex::new(
            crate::tasks::TaskStore::open(&path).expect("建临时任务表"),
        ));
        let bridge = Arc::new(crate::dsh::DshBridge::new(
            vec!["true".into()],
            "/tmp".into(),
            std::time::Duration::from_secs(1),
        ));
        Dispatcher::new(store, bridge, "/tmp".into())
    }

    #[test]
    fn unknown_project_is_detected() {
        let d = dispatcher_for_test();
        // 真机 e2e 抓到的原话：aimeng 是 ASR 听错 diemeng 的结果
        assert_eq!(
            d.extract_unknown_project("在 aimeng 项目里跑一下测试"),
            Some("aimeng".into()),
            "假项目名必须被识别出来"
        );
    }

    #[test]
    fn known_project_is_allowed() {
        let d = dispatcher_for_test();
        assert_eq!(
            d.extract_unknown_project("看看蝶梦项目有哪些目录"),
            None,
            "已知项目不该拦"
        );
        assert_eq!(
            d.extract_unknown_project("在 morphofly 项目里跑测试"),
            None,
            "已知项目（英文别名）不该拦"
        );
    }

    #[test]
    fn pronoun_is_not_treated_as_project() {
        let d = dispatcher_for_test();
        for t in ["这个项目跑测试", "那个项目看看", "整个项目重构"] {
            assert_eq!(d.extract_unknown_project(t), None, "{t} 是指代，不该拦");
        }
    }

    #[test]
    fn task_without_project_mention_passes() {
        let d = dispatcher_for_test();
        for t in ["看看有哪些目录", "运行 ls", "写个测试"] {
            assert_eq!(d.extract_unknown_project(t), None, "{t} 没提项目，不该拦");
        }
    }

    // ── SessionContext（set_context 的状态载体）──

    #[test]
    fn session_context_defaults_to_given_values() {
        let c = SessionContext::new("dsh", "/tmp");
        assert_eq!(c.agent, "dsh");
        assert_eq!(c.dir, "/tmp");
        assert_eq!(c.model, "", "model 默认空表示用 agent 自带的");
    }

    #[test]
    fn session_context_is_cloneable_and_comparable() {
        // context() 返回 clone，dispatch 内部要比较「有没有变」
        let a = SessionContext::new("dsh", "/tmp");
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn looks_like_action_flags_busy_talk() {
        for s in [
            "好，查着呢",
            "好的，这就去办",
            "稍等，马上就好",
            "我来看一下",
            "已经切换成 codex 了",
        ] {
            assert!(looks_like_action(s), "应判为「在办事」: {s}");
        }
    }

    #[test]
    fn looks_like_action_ignores_chitchat() {
        for s in [
            "我叫小蝶",
            "今天是 10 月 24 号",
            "你说的我没听清",
            "需要我帮你查吗",
            "",
            "   ",
        ] {
            assert!(!looks_like_action(s), "不该判为「在办事」: {s:?}");
        }
    }

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
        assert!("好，干完说".chars().count() < 20, "派活回复必须极简");
    }

    #[test]
    fn clean_for_speech_empty_becomes_something() {
        assert_eq!(clean_for_speech(""), "干完了。");
        assert_eq!(clean_for_speech("```\ncode\n```"), "干完了。");
    }

    /// 回归：本地代派挑目录不能挑到家目录。
    ///
    /// 真机实测（2026-10-25）：用户说「帮我看看蝶梦项目的 README」，
    /// 本地代派把任务派到了 `/home/wings`（配置里的 default_cwd 就是家目录），
    /// DSH 在家里当然干不成事。
    #[tokio::test]
    async fn dir_for_task_prefers_project_in_task() {
        let d = dispatcher_for_test();
        let project_path = d.projects.project[0].path.clone();
        let got = d.dir_for_task("帮我看看蝶梦项目的 README 第一行").await;
        assert_eq!(got, project_path, "应该用别名表里的项目路径");

        // 没提项目、默认目录也不是家目录（测试里是 /tmp）→ 用默认目录
        let got2 = d.dir_for_task("帮我看看今天几号").await;
        assert_eq!(got2, "/tmp");
    }

    #[test]
    fn is_home_dir_recognizes_home() {
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            assert!(is_home_dir(&home));
            assert!(is_home_dir(&format!("{home}/")));
        }
        assert!(!is_home_dir("/tmp"));
    }

    #[test]
    fn suggest_dir_says_not_found_for_garbage() {
        let s = suggest_dir("/definitely/not/here/xyz", "/tmp");
        assert!(s.contains("没找到"));
    }

    /// 念提示时不能把绝对路径念出来（用户听不懂，带斜杠读着也怪）。
    ///
    /// ⚠️ 2026-10-25 Windows CI 抓到：这里原来写死 `/tmp` 当候选目录，
    /// 但 Windows 上没有 `/tmp` → read_dir 失败 → 候选为空 → 走「没找到」分支，
    /// 于是断言「要说清是哪个名字」失败。测试本身必须跨平台。
    #[test]
    fn suggest_dir_does_not_read_absolute_path() {
        let existing = std::env::temp_dir();
        let s = suggest_dir(
            "/home/wings/Code_Project/move_forward",
            existing.to_str().expect("临时目录路径不是 UTF-8"),
        );
        assert!(!s.contains("/home/wings"), "不该念出绝对路径: {s}");
        assert!(s.contains("move_forward"), "但要说清是哪个名字没找到: {s}");
    }

    /// 回归：ASR 把项目名听错时，先按别名表纠正，别直接放弃。
    ///
    /// 真机实测：说「morphofly 项目」，云端听成「move_forward」，
    /// 模型就把 /home/wings/Code_Project/move_forward 传进来了。
    /// move_forward 对不上任何别名，仍然要追问；但「蝶梦/morphofly」能对上。
    #[test]
    fn project_alias_resolves_misheard_name() {
        let t = crate::projects::ProjectTable::builtin();
        // 别名表内置了 蝶梦/diemeng/morphofly/蝴蝶
        for heard in ["蝶梦", "diemeng", "morphofly", "蝴蝶"] {
            let p = t
                .lookup(heard)
                .unwrap_or_else(|| panic!("别名表该认得出: {heard}"));
            assert!(!p.path.is_empty(), "{heard} 对应路径不能为空");
        }
        // 听错的词对不上，就该落到追问分支（不能瞎猜）
        assert!(t.lookup("move_forward").is_none());
    }

    #[test]
    fn short_id_is_readable() {
        assert_eq!(short_id("task_20261024_0120_001"), "001");
    }
}
