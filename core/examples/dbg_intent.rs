fn main() {
    let tasks = [
        "cd /home/wings/Code_Project/morphofly && 提升测试覆盖率到百分之八十",
        "把整个项目的测试覆盖率提升到百分之八十",
        "提升测试覆盖率",
        "ls /home/wings/Code_Project/morphofly",
    ];
    for t in tasks {
        let i = morphofly_core::intent::classify(t);
        println!("{:?}\n  -> {:?} ({})\n", t, i.complexity, i.reason);
    }
}
