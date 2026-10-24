//! 意图分类：决定任务是「直接派」还是「先问确认」。
//!
//! ## 为什么需要它
//!
//! 用户要求：
//! > 简单任务直接递交给 work agent 处理，负责任务让 work agent 探索方案规划拆分等进行处理
//! > 当用户意图明确要开始进行任务时才派发 work agent，可主动询问用户是否确认进行任务
//!
//! 现状问题：语音模型判断完意图就**直接调 delegate_task**，
//! 一旦 ASR 听错一个字，就会开始做错的事，用户还得等它做完才发现。
//!
//! ## 为什么放在 Rust 层而不是提示词里
//!
//! 提示词约束不可靠（模型可能不遵守）。分类放在 `delegate` 之前，
//! 是**代码级强制**：复杂任务必须先经过确认逻辑。
//!
//! ## 分类规则
//!
//! 不看语义（那需要模型），看**语言学特征**：
//! - 动作词数量：一个还是多个
//! - 是否有"规划/重构/优化/整个/所有"这类大词
//! - 是否要求多步骤
//!
//! 这个启发式不完美，但比"全靠模型自觉"可靠得多。
//! 规则可以持续调，每次误判都是明确的改进点。

/// 任务复杂度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Complexity {
    /// 简单任务：单步、目标明确 → 直接派
    Simple,
    /// 复杂任务：多步、要规划 → 先问确认
    Complex,
}

impl Complexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Complex => "complex",
        }
    }
}

/// 一次派发的判断结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    pub complexity: Complexity,
    /// 为什么这么判断（写日志用，也方便调规则）
    pub reason: &'static str,
}

impl Intent {
    pub fn needs_confirm(&self) -> bool {
        self.complexity == Complexity::Complex
    }
}

/// 表示"大改动"的词：一出现就说明这事不简单
const BIG_WORDS: &[&str] = &[
    "整个", "全部", "所有", "重构", "重写", "架构", "迁移",
    "覆盖率", "性能优化", "安全审计", "清理", "整理一遍",
    "批量", "每个", "逐个", "全面",
];

/// 表示"要规划"的词
const PLAN_WORDS: &[&str] = &[
    "规划", "设计", "方案", "拆分", "评估", "调研", "分析",
    "怎么做", "如何实现", "可行性",
];

/// 多个动作的连接词（出现越多越可能多步）
const CHAIN_WORDS: &[&str] = &[
    "然后", "接着", "再", "并且", "同时", "顺便", "之后",
];

/// 判断任务复杂度。
///
/// 输入是交给 `delegate_task` 的 task 文本（已经是自包含的描述）。
pub fn classify(task: &str) -> Intent {
    let t = task.trim();

    // 空任务当作简单（实际不会发生，dispatch 层会先报错）
    if t.is_empty() {
        return Intent { complexity: Complexity::Simple, reason: "空任务" };
    }

    // 规则 1：大改动词
    if let Some(w) = BIG_WORDS.iter().find(|w| t.contains(*w)) {
        return Intent {
            complexity: Complexity::Complex,
            reason: big_word_reason(w),
        };
    }

    // 规则 2：要规划的词
    if let Some(w) = PLAN_WORDS.iter().find(|w| t.contains(*w)) {
        return Intent {
            complexity: Complexity::Complex,
            reason: plan_word_reason(w),
        };
    }

    // 规则 3：多步链路（出现 2 个以上连接词）
    let chain = CHAIN_WORDS.iter().filter(|w| t.contains(*w)).count();
    if chain >= 2 {
        return Intent { complexity: Complexity::Complex, reason: "多步骤任务" };
    }

    // 规则 4：句子很长（超过 50 字），大概率描述复杂需求
    if t.chars().count() > 50 {
        return Intent { complexity: Complexity::Complex, reason: "任务描述过长" };
    }

    Intent { complexity: Complexity::Simple, reason: "单步任务" }
}

// reason 用 &'static str 是为了零分配；这里为每个词返回固定文案
fn big_word_reason(w: &&str) -> &'static str {
    if *w == "重构" || *w == "重写" || *w == "架构" || *w == "迁移" {
        "涉及架构改动"
    } else if *w == "覆盖率" || *w == "性能优化" || *w == "安全审计" {
        "涉及全面改进"
    } else {
        "涉及范围大"
    }
}

fn plan_word_reason(w: &&str) -> &'static str {
    if *w == "规划" || *w == "设计" || *w == "方案" {
        "需要先规划"
    } else {
        "需要先调研分析"
    }
}

/// 生成确认问句（用户听到的）。
///
/// 原则：极简、给出关键信息、不要念内部术语。
pub fn confirm_question(task: &str) -> String {
    // 只取任务的前 12 个字做概述，避免念一长串
    let brief: String = task.chars().take(12).collect();
    let ellipsis = if task.chars().count() > 12 { "…" } else { "" };
    format!("要我{}{}吗", brief, ellipsis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_task_does_not_need_confirm() {
        for t in [
            "看看蝶梦项目有哪些目录",
            "运行 ls 看看有什么文件",
            "查一下测试数量",
        ] {
            let i = classify(t);
            assert_eq!(i.complexity, Complexity::Simple, "{t} 应是简单任务");
            assert!(!i.needs_confirm());
        }
    }

    #[test]
    fn big_change_needs_confirm() {
        for t in [
            "把整个项目的测试覆盖率提升到 80%",
            "重构一下音频模块",
            "全面清理无用代码",
            "批量改所有文件的 import",
        ] {
            let i = classify(t);
            assert_eq!(i.complexity, Complexity::Complex, "{t} 应是复杂任务");
            assert!(i.needs_confirm(), "{t} 应该先问确认");
        }
    }

    #[test]
    fn planning_task_needs_confirm() {
        for t in [
            "帮我设计一下多端架构",
            "调研一下 AEC 方案",
            "这个功能怎么做比较好",
        ] {
            assert!(classify(t).needs_confirm(), "{t} 应该先问确认");
        }
    }

    #[test]
    fn multi_step_needs_confirm() {
        let t = "先跑测试，然后修复失败的，接着重新跑一遍";
        assert!(classify(t).needs_confirm(), "多步骤任务应该先问确认");
    }

    #[test]
    fn long_description_needs_confirm() {
        // 必须真的超过阈值（第一次写测试时串只有 43 字，没到 50，测试假通过）
        let t = "我需要你帮我仔细看一下这个项目里面的那个音频处理模块，                 检查它到底是不是存在什么潜在的问题，如果有的话就帮我修复一下，                 另外再确认一下是不是会影响其他模块，最后告诉我具体的改动点";
        assert!(t.chars().count() > 50, "测试串必须超过阈值，实际 {}", t.chars().count());
        assert!(classify(t).needs_confirm(), "超长描述应该先问确认");
    }

    /// 阈值边界：刚好 50 字仍算简单，51 字算复杂
    #[test]
    fn length_threshold_boundary() {
        let base = "看一下这个项目";
        let at_50: String = std::iter::repeat('测').take(50).collect();
        let at_51: String = std::iter::repeat('测').take(51).collect();
        let _ = base;
        assert_eq!(classify(&at_50).complexity, Complexity::Simple, "50 字仍是简单");
        assert_eq!(classify(&at_51).complexity, Complexity::Complex, "51 字算复杂");
    }

    #[test]
    fn confirm_question_is_short() {
        let q = confirm_question("把整个项目的测试覆盖率提升到百分之八十");
        // 「要我」+ 12 字 + 「…吗」= 最多 17 字
        assert!(q.chars().count() <= 18, "确认问句太长: {q}");
        assert!(q.contains("要我"), "应包含「要我」: {q}");
        assert!(q.ends_with("吗"), "应是问句: {q}");
    }

    #[test]
    fn empty_task_is_simple() {
        assert_eq!(classify("").complexity, Complexity::Simple);
        assert_eq!(classify("   ").complexity, Complexity::Simple);
    }
}
