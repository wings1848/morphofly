# 贡献指南

## 这个仓库的规矩

三条硬约束，破了就是设计失败，不接受 PR：

1. **WebView 永不碰麦克风** —— 音频全在 Rust 侧用 cpal/rodio 原生处理
2. **语音模型只有 4 个函数** —— 没有文件/命令能力，这是物理隔离不是提示词
3. **语音链路与任务链路独立** —— 语音轮次先结束，任务后台跑完再主动汇报

## 开发环境

```bash
# 依赖
rustc 1.82+ / cargo
node 20+（前端测试）
webkit2gtk-4.1 + gtk3（Tauri 壳）

# 跑测试
cargo test -p morphofly-core --lib          # 178 个纯逻辑测试
node --test app/frontend/test/            # 8 个前端测试
```

**唤醒词需要额外 feature**：

```bash
cargo test -p morphofly-core --features kws --lib   # 189 个
```

`sherpa-onnx` 是静态库，部分精简系统缺 `libstdc++` 会链接失败，所以默认不开。

## 提交前自查

```bash
cargo build -p morphofly-core    # 必须 0 警告
cargo test -p morphofly-core --lib
node --test app/frontend/test/
```

**不要提交**：
- 任何密钥（`~/.diemeng/credentials` 已在 .gitignore）
- `target/`、`dist/`
- 实测数据里编造的数字

## 实测数据的规矩

这个项目所有性能数字都来自真机实测。改代码后如果影响性能：

1. 重跑对应命令（见 `docs/实测数据.md`）
2. **只写实测数字**
3. 数字变差要写清原因，不要悄悄改

## 代码风格

- Rust：`cargo fmt` + `cargo clippy`
- 注释写「为什么」，不写「是什么」
- 踩过的坑写进代码注释（仓库里有大量这类记录，是有意为之）

## 分支与合并

- 主分支 `main` 保护：分支 → PR → CI 绿 → squash 合并
- commit message 用中文或英文都行，但要能看出改了什么、为什么

## 项目结构

```
core/           Rust 核心（音频/协议/状态机/工具/DSH 桥）
app/src-tauri/  Tauri 2 桌面壳
app/frontend/   六态蓝色小球（原生 ESM，无打包器）
core/examples/  13 个可运行探针
tools/          运维脚本
docs/           技术文档
runs/           devflow 开发流程记录（含实测证据）
```
