//! 语音模型手上的工具白名单 —— 不变量 I1。
//!
//! 这不是提示词纪律，是物理隔离：云端模型拿到的 tools 数组里根本没有读写文件/跑命令的条目。
//!
//! ## 为什么从「恰好 4 个」改成「名字白名单 + 禁用名黑名单」
//!
//! 2026-10-24 V2 加第 5 个工具 `set_context`（多 agent 切换）时发现：
//! 「恰好 N 个」这个表述是**实现快照**，不是设计意图。
//! 真正的意图是「模型物理上没有文件/命令能力」，用数字表达反而脆——
//! 每次加工具都要改一堆地方，而且数字对不对和安全性没关系。
//!
//! 现在改成：
//! - 白名单：只允许 [`TOOL_NAMES`] 里的名字
//! - 黑名单：危险名字（bash/read_file/write_file/run_command…）永远不许出现
//!
//! 白名单保证「不会多」，黑名单保证「就算有人往白名单里加了危险名字也会被打红」。

use serde_json::{json, Value};

/// 当前工具数。加工具时改这里 + [`TOOL_NAMES`] + [`tool_defs`]。
pub const TOOL_COUNT: usize = 5;

pub const DELEGATE_TASK: &str = "delegate_task";
pub const QUERY_PROGRESS: &str = "query_progress";
pub const CANCEL_TASK: &str = "cancel_task";
pub const APPEND_REQUIREMENT: &str = "append_requirement";
/// 切换 agent / 工作目录 / 模型（V2 新增，多 agent 支持）
pub const SET_CONTEXT: &str = "set_context";

/// 工具名列表（测试与运行时断言共用）
pub const TOOL_NAMES: [&str; TOOL_COUNT] = [
    DELEGATE_TASK,
    QUERY_PROGRESS,
    CANCEL_TASK,
    APPEND_REQUIREMENT,
    SET_CONTEXT,
];

/// 绝对禁止的工具名 —— 出现任何一个就说明不变量 I1 被破了。
///
/// 这些名字即便被误加进 [`TOOL_NAMES`]，断言也会打红。
pub const FORBIDDEN_TOOL_NAMES: [&str; 12] = [
    "bash",
    "shell",
    "sh",
    "exec",
    "run_command",
    "run",
    "read_file",
    "write_file",
    "edit",
    "edit_file",
    "ls",
    "cat",
];

/// 组装给 realtime 模型的 tools 数组（OpenAI Realtime / StepFun / Qwen 同构）
pub fn tool_defs() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "name": DELEGATE_TASK,
            "description": "把一个任务交给编码 agent 执行。立即返回，不等待完成。task 必须自包含——对方看不到我们的对话历史，不要用「这个」「刚才说的」之类的指代，要把项目路径和具体要求讲清楚。",
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {"type": "string", "description": "完整、自包含的任务描述，含项目路径与具体要求"},
                    "dir": {"type": "string", "description": "工作目录的绝对路径，可选"},
                    "urgent": {"type": "boolean", "description": "是否加急"}
                },
                "required": ["task"]
            }
        }),
        json!({
            "type": "function",
            "name": QUERY_PROGRESS,
            "description": "查任务进度。用户问「那个完了吗」时调用。省略 task_id 则返回所有进行中的任务。",
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "任务号，省略则查全部进行中任务"}
                },
                "required": []
            }
        }),
        json!({
            "type": "function",
            "name": CANCEL_TASK,
            "description": "取消一个正在执行的任务。用户说「停」「别干了」时调用。",
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "要取消的任务号"}
                },
                "required": ["task_id"]
            }
        }),
        json!({
            "type": "function",
            "name": APPEND_REQUIREMENT,
            "description": "给正在执行的任务追加要求。用户说「顺便加个日志」时调用。",
            "parameters": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string", "description": "任务号"},
                    "extra": {"type": "string", "description": "追加的要求，自包含"}
                },
                "required": ["task_id", "extra"]
            }
        }),
        // ★ V2 新增：切换上下文（多 agent 支持）
        //
        // 为什么单独一个工具而不再塞进 delegate_task：
        //   「切换」改的是**持久状态**（接下来都用这个 agent/目录），
        //   「派活」是**执行一次动作**。语义不同，混在一起模型会困惑。
        //
        // 为什么 project 用别名不用绝对路径：
        //   中文语音识别英文路径几乎必错（实测「diemeng」→「/daimian」）。
        //   让模型复述用户说的词，宿主查别名表，比让它背路径可靠得多。
        json!({
            "type": "function",
            "name": SET_CONTEXT,
            "description": "切换接下来使用的工作环境。用户说「切换到某个 agent」「进入某个项目」「换个模型」时调用。三个参数都可选，只传要改的那个。",
            "parameters": {
                "type": "object",
                "properties": {
                    "agent": {
                        "type": "string",
                        "description": "要切换到的编码 agent，如 dsh 或 codex"
                    },
                    "project": {
                        "type": "string",
                        "description": "项目别名（用户说的那个词，比如「蝶梦」），不要传路径"
                    },
                    "model": {
                        "type": "string",
                        "description": "要切换到的模型名"
                    }
                },
                "required": []
            }
        }),
    ]
}

/// 运行时断言：tools 数组必须是白名单里的那些，不多不少，且不含黑名单名字（验收 A1）
pub fn assert_tool_invariant(tools: &[Value]) -> Result<(), String> {
    // ★ 黑名单检查放最前面：安全性优先于数量约束。
    //
    // 2026-10-24 变异测试抓到的真问题：原来数量检查在前，
    // 塞一个 bash 进去只会报「工具数必须是 5」，用户/开发者看不出
    // 真正原因是「bash 被禁了」。错误信息必须指向真问题。
    for t in tools {
        if let Some(n) = t.get("name").and_then(|v| v.as_str()) {
            if FORBIDDEN_TOOL_NAMES.contains(&n) {
                return Err(format!(
                    "工具 {n} 在不变量 I1 的黑名单里 —— 语音模型物理上不许有文件/命令能力"
                ));
            }
        }
    }
    if tools.len() != TOOL_COUNT {
        return Err(format!("工具数必须是 {TOOL_COUNT}，实际 {}", tools.len()));
    }
    let mut names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .collect();
    names.sort_unstable();
    let mut expect = TOOL_NAMES.to_vec();
    expect.sort_unstable();
    if names != expect {
        return Err(format!("工具名不匹配：{names:?} != {expect:?}"));
    }
    if tools
        .iter()
        .any(|t| t.get("type").and_then(|v| v.as_str()) != Some("function"))
    {
        return Err("所有工具 type 必须是 function".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不变量 I1：语音模型只有白名单工具，数量与 TOOL_COUNT 一致
    #[test]
    fn exactly_tool_count() {
        let tools = tool_defs();
        assert_eq!(tools.len(), TOOL_COUNT, "tool_defs 数量必须等于 TOOL_COUNT");
        assert!(assert_tool_invariant(&tools).is_ok());
    }

    /// 变异测试：把「bash」塞进工具列表，断言必须失败。
    ///
    /// 这条比「数量对不对」重要 —— 数量对了但混进危险工具才是灾难。
    #[test]
    fn rejects_forbidden_tool_names() {
        let mut tools = tool_defs();
        tools.push(json!({"type": "function", "name": "bash", "parameters": {}}));
        let err = assert_tool_invariant(&tools).expect_err("塞入 bash 必须被拒绝");
        assert!(
            err.contains("bash") || err.contains("黑名单"),
            "错误信息应指出黑名单: {err}"
        );
    }

    /// 变异测试：把 read_file 加进 TOOL_NAMES 的等价场景
    #[test]
    fn rejects_read_file() {
        let mut tools = tool_defs();
        tools.push(json!({"type": "function", "name": "read_file", "parameters": {}}));
        assert!(
            assert_tool_invariant(&tools).is_err(),
            "read_file 必须在黑名单里"
        );
    }

    /// 黑名单里的名字一个都不能在 tool_defs 里
    #[test]
    fn no_forbidden_name_in_real_tools() {
        let tools = tool_defs();
        for t in &tools {
            let n = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                !FORBIDDEN_TOOL_NAMES.contains(&n),
                "真实工具列表里混进了禁用名: {n}"
            );
        }
    }

    #[test]
    fn no_file_or_command_tools() {
        let tools = tool_defs();
        let granted: Vec<String> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();
        for forbidden in [
            "bash",
            "read_file",
            "write_file",
            "run_command",
            "ls",
            "edit",
        ] {
            assert!(
                !granted.iter().any(|g| g == forbidden),
                "不允许出现 {forbidden}"
            );
        }
    }

    #[test]
    fn invariant_catches_extra_tool() {
        let mut tools = tool_defs();
        tools.push(json!({"type": "function", "name": "bash", "parameters": {}}));
        assert!(assert_tool_invariant(&tools).is_err(), "多一个工具必须被拒");
    }

    #[test]
    fn invariant_catches_renamed_tool() {
        let mut tools = tool_defs();
        tools[0]["name"] = json!("do_anything");
        assert!(assert_tool_invariant(&tools).is_err());
    }

    #[test]
    fn delegate_task_requires_task_field() {
        let tools = tool_defs();
        let d = tools.iter().find(|t| t["name"] == DELEGATE_TASK).unwrap();
        let req = d["parameters"]["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "task"));
        // task 的描述必须强调自包含（验收 A6）
        let desc = d["parameters"]["properties"]["task"]["description"]
            .as_str()
            .unwrap();
        assert!(desc.contains("自包含"));
    }

    #[test]
    fn voice_tool_payloads_serialize() {
        let payload =
            json!({"name": DELEGATE_TASK, "task": "把 README 补一段怎么跑", "dir": "/tmp"});
        let s = serde_json::to_string(&payload).unwrap();
        assert!(s.contains("delegate_task"));
    }
}
