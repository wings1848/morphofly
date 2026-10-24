//! 兜底纠正的状态机：模型「只说了话没干活」该不该打断它重来。
//!
//! ## 为什么要有这个模块
//!
//! 提示词要求「需要查信息时必须真的调 `delegate_task`」，但模型不总是遵守 ——
//! 真机实测它会回一句「好，查着呢」却什么都没做。那是**假装干活**，同样是骗用户。
//! 所以 App 里有一层兜底：发现它没干活就注入一条系统提醒，逼它真派活。
//!
//! ## 为什么要把这段逻辑从 App 里抽出来
//!
//! 抽出来之前它是 `voice.rs` 事件循环里的几个局部变量，**没有任何测试覆盖**，
//! 2026-10-24 在 wings-gpd 真机上直接炸了：一句「看看 README 第一行」
//! 派了 4 个任务、起了 4 个 DSH 会话、报了 4 遍。
//!
//! 两条判据都错得很隐蔽：
//!
//! 1. **统计单位错了。** 按「一次模型响应」清零计数器，但一个用户请求天然跨多个
//!    response：`用户说话 → resp1(调 delegate_task) → 工具结果回灌 → resp2(最终答复)`。
//!    resp2 没调工具是**正常的**，却被判成「没干活」→ 纠正 → 再派一次 → 循环。
//! 2. **事件用错了。** 把 `TranscriptDelta`（实际是 `response.audio_transcript.delta`，
//!    **模型自己说的话**）当成「用户说了什么」用，「用户说过话」这个条件几乎恒真。
//!
//! 所以：统计单位 = **一次用户说话**；判断依据 = **模型自己的回复像不像在装忙**。
//!
//! ## 用法
//!
//! 事件循环里照着事件调对应方法，`on_response_done` 的返回值决定下一步：
//!
//! ```
//! use morphofly_core::correction::{CorrectionGuard, Decision};
//!
//! let mut g = CorrectionGuard::new();
//! g.on_user_speech();                       // SpeechStarted
//! g.on_tool_call();                         // ToolCall
//! assert_eq!(g.on_response_done(), Decision::Wait);   // 工具那轮的 response.done
//! g.on_assistant_text("好，干完说");
//! assert_eq!(g.on_response_done(), Decision::TurnEnd); // 最终答复：不纠正
//! ```

use crate::dispatch::looks_like_action;

/// `on_response_done` 的处理建议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 还有工具结果没回灌，这不是最终答复，什么都别做
    Wait,
    /// 模型在装忙 → 该注入纠正提醒逼它真派活
    Correct,
    /// 这一轮正常结束
    TurnEnd,
}

/// 兜底纠正的判据状态。
#[derive(Debug, Default, Clone)]
pub struct CorrectionGuard {
    /// 本次用户说话以来调了几次工具
    tool_calls: usize,
    /// 工具调用已派出、结果还没回灌的数量
    pending: usize,
    /// 用户说过话、这一轮还没收尾
    user_spoke: bool,
    /// 模型本轮说的话（判断「像不像在装忙」）
    reply: String,
    /// 这次用户说话已经纠正过（一次最多纠正一次）
    corrected: bool,
}

impl CorrectionGuard {
    pub fn new() -> Self {
        Self::default()
    }

    /// 用户开口了（`SpeechStarted`）——**只有这里能重置统计**。
    pub fn on_user_speech(&mut self) {
        self.tool_calls = 0;
        self.pending = 0;
        self.user_spoke = true;
        self.reply.clear();
        self.corrected = false;
    }

    /// 模型说的话（`TranscriptDelta` / `TextDelta`，都是模型的，不是用户的）。
    pub fn on_assistant_text(&mut self, text: &str) {
        if self.reply.len() < 200 {
            self.reply.push_str(text);
        }
    }

    /// 调了一次工具（`ToolCall`）。
    pub fn on_tool_call(&mut self) {
        self.tool_calls += 1;
        self.pending += 1;
    }

    /// 一个 response 结束（`ResponseDone`）。
    pub fn on_response_done(&mut self) -> Decision {
        if self.pending > 0 {
            self.pending -= 1;
            self.reply.clear();
            return Decision::Wait;
        }
        if self.tool_calls == 0
            && self.user_spoke
            && !self.corrected
            && looks_like_action(&self.reply)
        {
            self.corrected = true;
            self.reply.clear();
            return Decision::Correct;
        }
        self.user_spoke = false;
        self.reply.clear();
        Decision::TurnEnd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归测试：bug #11。
    ///
    /// 一次用户说话里调了工具，工具结果回灌后的「最终答复」不能再被纠正。
    /// 修复前这里会返回 `Correct`，于是模型再派一次任务 —— 一句话变成四句话。
    #[test]
    fn tool_call_then_final_reply_is_not_corrected() {
        let mut g = CorrectionGuard::new();
        g.on_user_speech();
        g.on_tool_call();
        // 工具调用那一轮的 response.done
        assert_eq!(g.on_response_done(), Decision::Wait);
        // 工具结果回灌后的最终答复
        g.on_assistant_text("好，干完说");
        assert_eq!(
            g.on_response_done(),
            Decision::TurnEnd,
            "已经在干活了，不该再纠正（否则会重复派活）"
        );
    }

    /// 真的装忙 → 纠正一次，且只纠正一次。
    #[test]
    fn fake_work_is_corrected_once() {
        let mut g = CorrectionGuard::new();
        g.on_user_speech();
        g.on_assistant_text("好，查着呢");
        assert_eq!(g.on_response_done(), Decision::Correct);

        // 纠正之后的响应，即使还是没调工具，也不能再纠正
        g.on_assistant_text("好，查着呢");
        assert_eq!(
            g.on_response_done(),
            Decision::TurnEnd,
            "一次用户说话最多纠正一次"
        );
    }

    /// 闲聊不该被逼着派活。
    #[test]
    fn chitchat_is_not_corrected() {
        let mut g = CorrectionGuard::new();
        g.on_user_speech();
        g.on_assistant_text("我叫小蝶，是你的语音助手");
        assert_eq!(g.on_response_done(), Decision::TurnEnd);
    }

    /// 新一轮用户说话要重新开始统计。
    #[test]
    fn new_utterance_resets_state() {
        let mut g = CorrectionGuard::new();
        g.on_user_speech();
        g.on_tool_call();
        assert_eq!(g.on_response_done(), Decision::Wait);
        g.on_assistant_text("好了");
        assert_eq!(g.on_response_done(), Decision::TurnEnd);

        // 用户又问一句，这次没派活 → 该纠正（说明计数确实重置了）
        g.on_user_speech();
        g.on_assistant_text("好，查着呢");
        assert_eq!(g.on_response_done(), Decision::Correct);
    }

    /// 多个工具调用对应多个 response.done，都要等齐。
    #[test]
    fn waits_for_every_pending_tool() {
        let mut g = CorrectionGuard::new();
        g.on_user_speech();
        g.on_tool_call();
        g.on_tool_call();
        assert_eq!(g.on_response_done(), Decision::Wait);
        assert_eq!(g.on_response_done(), Decision::Wait);
        g.on_assistant_text("都办完了");
        assert_eq!(g.on_response_done(), Decision::TurnEnd);
    }

    /// 系统注入的响应（主动汇报等）不是用户开的头，不该被纠正。
    #[test]
    fn system_triggered_reply_is_not_corrected() {
        let mut g = CorrectionGuard::new();
        // 没有 on_user_speech，直接来一条回复
        g.on_assistant_text("好，那边干完了");
        assert_eq!(g.on_response_done(), Decision::TurnEnd);
    }
}
