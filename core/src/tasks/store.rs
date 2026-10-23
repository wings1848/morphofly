//! 任务表：task_id ⇄ 状态 ⇄ 摘要，落盘到 `~/.diemeng/tasks.json`。
//!
//! 设计要点（docs/开发方案.md §5.3）：
//! - 重启不丢（验收 S5）
//! - 语音链路断了任务继续（验收 S4/A3）—— 任务表不持有 WS 句柄
//! - summary 会被直接念出来，必须是人话

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
    Cancelled,
    Blocked,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, TaskStatus::Done | TaskStatus::Failed | TaskStatus::Cancelled)
    }
    pub fn is_active(&self) -> bool {
        matches!(self, TaskStatus::Pending | TaskStatus::Running | TaskStatus::Blocked)
    }
    /// 念给用户听的中文
    pub fn spoken(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "排队中",
            TaskStatus::Running => "正在干",
            TaskStatus::Done => "干完了",
            TaskStatus::Failed => "失败了",
            TaskStatus::Cancelled => "已取消",
            TaskStatus::Blocked => "卡住了",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub goal: String,
    pub dir: String,
    pub status: TaskStatus,
    pub summary: String,
    #[serde(default)]
    pub agent_id: String,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

/// 一句话汇报用的措辞（别念代码块/路径/markdown —— 验收 A6 / D5）
pub fn spoken_summary(task: &Task) -> String {
    let s = task.summary.trim();
    if s.is_empty() {
        format!("{}：{}", task.goal, task.status.spoken())
    } else {
        format!("{}：{}", task.status.spoken(), s)
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// 生成 task id：task_<yyyymmdd>_<hhmm>_<seq>
fn make_id(seq: usize) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = now.as_secs();
    // 用 UTC 秒数拆日期时间（不引入 chrono）
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "task_{:04}{:02}{:02}_{:02}{:02}_{:03}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        seq
    )
}

/// Howard Hinnant 的 civil_from_days 算法
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskFile {
    #[serde(default)]
    tasks: Vec<Task>,
    #[serde(default)]
    seq: usize,
}

/// 任务表
#[derive(Debug)]
pub struct TaskStore {
    path: PathBuf,
    tasks: Vec<Task>,
    seq: usize,
}

impl TaskStore {
    /// 打开（不存在则空表）
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let mut store = Self { path, tasks: Vec::new(), seq: 0 };
        store.reload()?;
        Ok(store)
    }

    pub fn reload(&mut self) -> Result<()> {
        if self.path.exists() {
            let text = std::fs::read_to_string(&self.path)
                .with_context(|| format!("读任务表失败: {}", self.path.display()))?;
            let f: TaskFile = serde_json::from_str(&text)
                .with_context(|| format!("解析任务表失败: {}", self.path.display()))?;
            self.tasks = f.tasks;
            self.seq = f.seq;
        }
        Ok(())
    }

    fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let f = TaskFile { tasks: self.tasks.clone(), seq: self.seq };
        let text = serde_json::to_string_pretty(&f)?;
        // 原子写：先写临时文件再 rename，避免掉电写坏
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn create(&mut self, goal: &str, dir: &str) -> Result<Task> {
        self.seq += 1;
        let task = Task {
            id: make_id(self.seq),
            goal: goal.to_string(),
            dir: dir.to_string(),
            status: TaskStatus::Pending,
            summary: String::new(),
            agent_id: String::new(),
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        };
        self.tasks.push(task.clone());
        self.save()?;
        Ok(task)
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn all(&self) -> &[Task] {
        &self.tasks
    }

    pub fn active(&self) -> Vec<&Task> {
        self.tasks.iter().filter(|t| t.status.is_active()).collect()
    }

    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .with_context(|| format!("找不到任务 {id}"))?;
        t.status = status;
        t.updated_at_ms = now_ms();
        self.save()
    }

    pub fn set_summary(&mut self, id: &str, summary: &str) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .with_context(|| format!("找不到任务 {id}"))?;
        t.summary = summary.to_string();
        t.updated_at_ms = now_ms();
        self.save()
    }

    pub fn set_agent_id(&mut self, id: &str, agent_id: &str) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .with_context(|| format!("找不到任务 {id}"))?;
        t.agent_id = agent_id.to_string();
        t.updated_at_ms = now_ms();
        self.save()
    }

    /// 追加需求：拼到 goal 后面，保留原始诉求
    pub fn append_requirement(&mut self, id: &str, extra: &str) -> Result<()> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .with_context(|| format!("找不到任务 {id}"))?;
        t.goal = format!("{}\n\n补充要求：{}", t.goal, extra.trim());
        t.updated_at_ms = now_ms();
        self.save()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> TaskStore {
        let p = std::env::temp_dir().join(format!("diemeng-test-{}-{}.json", name, std::process::id()));
        let _ = std::fs::remove_file(&p);
        TaskStore::open(p).unwrap()
    }

    #[test]
    fn create_and_get() {
        let mut s = tmp_store("create");
        let t = s.create("列一下目录", "/tmp").unwrap();
        assert!(t.id.starts_with("task_"));
        assert_eq!(s.get(&t.id).unwrap().goal, "列一下目录");
        assert_eq!(s.get(&t.id).unwrap().status, TaskStatus::Pending);
    }

    #[test]
    fn persists_across_reopen() {
        let mut s = tmp_store("persist");
        let t = s.create("补一段 README", "/tmp").unwrap();
        s.set_status(&t.id, TaskStatus::Done).unwrap();
        s.set_summary(&t.id, "已经补好了").unwrap();
        let path = s.path().to_path_buf();
        drop(s);

        let s2 = TaskStore::open(path).unwrap();
        let t2 = s2.get(&t.id).unwrap();
        assert_eq!(t2.status, TaskStatus::Done);
        assert_eq!(t2.summary, "已经补好了");
    }

    #[test]
    fn active_excludes_finished() {
        let mut s = tmp_store("active");
        let a = s.create("a", "/tmp").unwrap();
        let b = s.create("b", "/tmp").unwrap();
        s.set_status(&a.id, TaskStatus::Done).unwrap();
        let act: Vec<_> = s.active().into_iter().map(|t| t.id.clone()).collect();
        assert_eq!(act, vec![b.id]);
    }

    #[test]
    fn append_requirement_keeps_original() {
        let mut s = tmp_store("append");
        let t = s.create("修 bug", "/tmp").unwrap();
        s.append_requirement(&t.id, "顺便加日志").unwrap();
        let g = &s.get(&t.id).unwrap().goal;
        assert!(g.contains("修 bug"));
        assert!(g.contains("顺便加日志"));
    }

    #[test]
    fn unknown_id_errors_not_panics() {
        let mut s = tmp_store("unknown");
        assert!(s.set_status("nope", TaskStatus::Done).is_err());
    }

    #[test]
    fn spoken_summary_has_no_markdown_or_paths() {
        let mut s = tmp_store("spoken");
        let t = s.create("补 README", "/tmp").unwrap();
        s.set_status(&t.id, TaskStatus::Done).unwrap();
        s.set_summary(&t.id, "README 补好了，没动别的文件").unwrap();
        let line = spoken_summary(s.get(&t.id).unwrap());
        assert!(!line.contains("```"));
        assert!(!line.contains('`'));
        assert!(line.starts_with("干完了"));
    }

    #[test]
    fn ids_are_unique() {
        let mut s = tmp_store("ids");
        let mut ids = std::collections::HashSet::new();
        for i in 0..50 {
            let t = s.create(&format!("t{i}"), "/tmp").unwrap();
            assert!(ids.insert(t.id.clone()), "id 重复: {}", t.id);
        }
    }

    #[test]
    fn status_flags() {
        assert!(TaskStatus::Done.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Running.is_active());
        assert!(!TaskStatus::Done.is_active());
    }
}
