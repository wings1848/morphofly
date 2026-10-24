//! 直接调 detect_in_file 看某段音频有没有唤醒词命中。
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut file = String::new();
    while let Some(a) = args.next() {
        if a == "--file" {
            file = args.next().unwrap_or_default();
        }
    }
    if file.is_empty() {
        anyhow::bail!("用法: kws_file_probe --file <音频>");
    }
    let cfg = morphofly_core::kws::KwsConfig::default();
    let hits = morphofly_core::kws::detect_in_file(&cfg, std::path::Path::new(&file))?;
    println!("命中 {} 处: {:?}", hits.len(), hits);
    Ok(())
}
