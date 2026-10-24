fn main() {
    let t = "cd /home/wings/Code_Project/M1 && ./run_test.sh";
    println!("task = {t:?}");
    for raw in t.split(|c: char| c.is_whitespace() || matches!(c, '，'|'。'|'、'|'；'|'"'|'\''|'`'|')'|'(')) {
        let tok = raw.trim_matches(|c: char| matches!(c, ','|':'|';'|'&'|'|'|'>'|'<'));
        let tok = tok.strip_suffix('.').unwrap_or(tok);
        if tok.is_empty() { continue; }
        let is_abs = tok.starts_with('/') && !tok.starts_with("./") && !tok.starts_with("../");
        println!("  raw={raw:?} tok={tok:?} is_abs={is_abs} exists={}",
                 if is_abs { std::path::Path::new(tok).exists() } else { true });
    }
}
