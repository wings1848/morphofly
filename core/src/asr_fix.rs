//! 用户转写的本地纠正：把 ASR 常听错的专有名词换回正确写法。
//!
//! ## 为什么需要
//!
//! 真机全链路实测（2026-10-25）：用户说「帮我看看 morphofly 项目的 README
//! 第一行是什么」，云端转写出来是
//! 「帮我看看mosofly项目的vimi第一行是什么？」——项目名和文件名都错了。
//! 本地代派时这句原话会直接交给 DSH，错字会让它找错目录/读错文件。
//!
//! ## 试过但没用的路（留证据，别再走）
//!
//! 探针 `core/examples/asr_hint_probe.rs` 实测：给
//! `input_audio_transcription` 加 OpenAI 风格的 `prompt` 热词提示，
//! 服务端**静默忽略**（`session.updated` 回显里该字段一直是 null，
//! 转写结果与不带 prompt 完全一致）。所以只能本地纠正。
//!
//! ## 边界
//!
//! - 只认表里列出的「已知错法」，不做通用拼音/编辑距离猜测 ——
//!   乱猜会把用户正常的话改坏。
//! - 英文按大小写不敏感整体替换；中文只做整词替换。
//! - 表是给人读的，加起来不超过几十行；新踩到的错法往里加一条就行。

/// (正确写法, 见过的错法)
const TERMS: &[(&str, &[&str])] = &[
    (
        "morphofly",
        &[
            "mosofly",
            "morfofly",
            "morphofy",
            "morpho fly",
            "mobile fly",
            "moto fly",
            "摩托fly",
            "魔方fly",
            "魔方飞",
            "魔方 fly",
            "莫否fly",
            "mofangfly",
            "move_forward",
            "move forward",
        ],
    ),
    (
        "README",
        &[
            "vimi",
            "vim i",
            "read me",
            "reedme",
            "readmi",
            "微米",
            "里德米",
            "瑞德米",
        ],
    ),
    ("DSH", &["d s h", "第四h", "dsh"]),
    ("蝶梦", &["碟梦", "叠梦", "跌梦", "爹梦"]),
    ("Tauri", &["taoli", "套瑞", "陶瑞"]),
    ("canvas", &["canvs", "canva"]),
];

/// 纠正一句话里的专有名词。
///
/// 返回纠正后的文本；没有任何改动时返回原样（方便调用方判断要不要打日志）。
pub fn fix_transcript(text: &str) -> String {
    let mut out = text.to_string();
    for (canonical, variants) in TERMS {
        // 已经是正确写法就别动（避免 README → README 这种无意义替换）
        for wrong in *variants {
            if wrong.eq_ignore_ascii_case(canonical) {
                continue;
            }
            let next = if wrong.is_ascii() {
                replace_ascii_ci(&out, wrong, canonical)
            } else {
                out.replace(wrong, canonical)
            };
            out = next;
        }
    }
    out
}

/// 大小写不敏感地替换 ASCII 片段（只按 ASCII 边界匹配，避免把英文词替换进中文里）
fn replace_ascii_ci(haystack: &str, needle: &str, to: &str) -> String {
    let lower_hay = haystack.to_lowercase();
    let lower_needle = needle.to_lowercase();
    if !lower_hay.contains(&lower_needle) {
        return haystack.to_string();
    }
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0usize;
    let hay_bytes = haystack.as_bytes();
    let low_bytes = lower_hay.as_bytes();
    let needle_len = lower_needle.len();
    while i < hay_bytes.len() {
        // ASCII 边界：前一个字符不是字母数字，才允许开始匹配
        let prev_ok = i == 0 || !hay_bytes[i - 1].is_ascii_alphanumeric();
        if prev_ok
            && i + needle_len <= low_bytes.len()
            && &low_bytes[i..i + needle_len] == lower_needle.as_bytes()
        {
            // 后一个字符也不能是字母数字（避免 "dshx" 被替换一半）
            let after = i + needle_len;
            let next_ok = after >= hay_bytes.len() || !hay_bytes[after].is_ascii_alphanumeric();
            if next_ok {
                out.push_str(to);
                i = after;
                continue;
            }
        }
        // 按 UTF-8 字符推进
        let ch = haystack[i..].chars().next().unwrap_or_default();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_project_name_and_readme() {
        let got = fix_transcript("帮我看看mosofly项目的vimi第一行是什么？");
        assert_eq!(got, "帮我看看morphofly项目的README第一行是什么？");
    }

    #[test]
    fn fixes_chinese_mishearing() {
        let got = fix_transcript("帮我看看摩托fly项目的微米第一行");
        assert_eq!(got, "帮我看看morphofly项目的README第一行");
    }

    #[test]
    fn leaves_correct_and_unrelated_text_alone() {
        for t in [
            "帮我看看蝶梦项目的 README 第一行",
            "今天好累，随便聊聊",
            "把那个目录里的日志清一下",
        ] {
            assert_eq!(fix_transcript(t), t, "不该改：{t}");
        }
    }

    #[test]
    fn case_insensitive_for_ascii() {
        assert_eq!(fix_transcript("看看 MOSOFLY 项目"), "看看 morphofly 项目");
        assert_eq!(fix_transcript("把 VIMI 读一下"), "把 README 读一下");
    }

    #[test]
    fn fixes_mofangfly_from_real_run() {
        // 2026-10-25 真机第二轮原话：morphofly 被听成「魔方fly」
        let got = fix_transcript("帮我看看魔方fly项目的 limit 第一行是什么？");
        assert_eq!(got, "帮我看看morphofly项目的 limit 第一行是什么？");
    }

    #[test]
    fn does_not_replace_inside_longer_word() {
        // dshx 不是 DSH，别替换一半
        assert_eq!(fix_transcript("dshx 是什么"), "dshx 是什么");
    }
}
