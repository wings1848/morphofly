# 验证报告（Windows 兼容）

**日期**：2026-10-25 01:10
**口径**：Windows 兼容 = 「在真实 Windows runner 上编得过、单测跑得过、能产出可执行文件」。
运行时行为没验（手头没有 Windows 机器），不声称「已验证兼容」。

## 本地全量回归（Linux，作对照）

```
cargo test -p morphofly-core --lib                → 250 passed
cargo test -p morphofly-core --features kws --lib → 261 passed
cargo clippy -p morphofly-core --lib --tests -D warnings → No issues
node --test app/frontend/test/                    → 13 passed
```

## Windows CI 结果

| 项 | 结果 |
|---|---|
| W1 仓库能在 Windows 上检出 | ✅ success |
| W2 core 在 Windows 上编译 | ✅ success |
| W3 core 248 个单测在 Windows 上跑 | ✅ 全过（11 秒） |
| W4 Tauri 壳在 Windows 上编译 | ✅ success |
| W5 编出 .exe | ⏳ 构建中（见 acceptance.md 的降级说明） |
| W5b NSIS 安装包 | ⏳ 可选 |

## 注意（不许含糊的地方）

1. **W5 拆成两步**是主动降级：NSIS 要整个 release 冷编译 + 打包，
   在 windows-latest 上 40+ 分钟没完。所以先保证 `--no-bundle` 能出 exe，
   安装包作为附加项。理由写在 acceptance.md。
2. **运行时完全没验证**。最可能踩的坑见 docs/v2/windows.md 的 5 条风险，
   其中「中文安装路径 + sherpa-onnx 加载唤醒词模型」风险最高。
