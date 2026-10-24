//! 语音模型手上只有这 4 个函数 —— 不变量 I1。
//!
//! 这不是提示词纪律，是物理隔离：云端模型拿到的 tools 数组里根本没有读写文件/跑命令的条目。
//! `TOOL_COUNT` 被单测锁死，改动会立刻打红。

use serde_json::{json, Value};

/// 恰好 4 个。任何新增都必须同时修改验收标准 A1 与 docs/开发方案.md。
pub const TOOL_COUNT: usize = 4;

pub const DELEGATE_TASK: &str = "delegate_task";
pub const QUERY_PROGRESS: &str = "query_progress";
pub const CANCEL_TASK: &str = "cancel_task";
pub const APPEND_REQUIREMENT: &str = "append_requirement";

/// 工具名列表（测试与运行时断言共用）
pub const TOOL_NAMES: [&str; TOOL_COUNT] = [
    DELEGATE_TASK,
    QUERY_PROGRESS,
    CANCEL_TASK,
    APPEND_REQUIREMENT,
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
    ]
}

/// 运行时断言：tools 数组必须恰好是这 4 个，不多不少（验收 A1）
pub fn assert_tool_invariant(tools: &[Value]) -> Result<(), String> {
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

    /// 不变量 I1：语音模型只有 4 个工具
    #[test]
    fn exactly_four_tools() {
        let tools = tool_defs();
        assert_eq!(tools.len(), 4);
        assert!(assert_tool_invariant(&tools).is_ok());
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
