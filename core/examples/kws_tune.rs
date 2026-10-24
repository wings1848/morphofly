//! 扫一遍 num_trailing_blanks，看它对「小蝶帮我」漏检和「小蝶儿」误唤醒的影响。
fn main() -> anyhow::Result<()> {
    let files = [
        ("wake-01", "小蝶"),
        ("edge-02", "小蝶帮我"),
        ("wake-03", "小蝶儿真好看"),
        ("wake-04", "蝴蝶飞走了"),
    ];
    println!("{:<10} {:<14} {}", "用例", "文本", "各 blanks 值命中数 (2/3/4/5)");
    for (id, text) in files {
        let path = format!("/tmp/diemeng-e2e-audio/{id}.pcm");
        let mut row = Vec::new();
        for blanks in [2, 3, 4, 5] {
            let mut cfg = morphofly_core::kws::KwsConfig::default();
            cfg.num_trailing_blanks = blanks;
            match morphofly_core::kws::detect_in_file(&cfg, std::path::Path::new(&path)) {
                Ok(h) => row.push(h.len().to_string()),
                Err(e) => row.push(format!("err")),
            }
        }
        println!("{:<10} {:<14} {}", id, text, row.join(" / "));
    }
    Ok(())
}
