# 验收（Windows 兼容）

用户原话：**「确保项目兼容windows平台」**

| # | 标准 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| W1 | 仓库能在 Windows 上检出（无非法路径） | CI `build-windows` 的 checkout 步骤 | success | ✅ success（删掉 `...` 之后） |
| W2 | core 能在 Windows 上编译 | CI `cargo check -p morphofly-core` | success | ✅ success |
| W3 | core 单测能在 Windows 上跑过 | CI `cargo test -p morphofly-core --lib` | 全过 | ⚠️ 首次真跑 246 过 / 2 挂 → 已修，待复跑 |
| W4 | app 外壳能在 Windows 上编译 | CI `cargo check -p morphofly-app --no-default-features` | success | |
| W5 | 能打出 Windows 安装包 | release `cargo tauri build --bundles nsis` | 产出 .exe | ⏳ 构建中 |
| W6 | 数据目录不再依赖 HOME | 本机单测 `config::data_dir_*` | 3 个测试过 | ✅ 11:xx 已过 |
| W7 | 拉起 dsh 兼容 .cmd | 代码审查 + 注释（Windows 上无法单测） | 有 PATHEXT 分支 | ✅ |
| W8 | 平台代码隔离 | `grep -c cfg\(target_os = "linux"\)` | force_x11_backend 被隔离 | ✅ |

## 不做 / 做不到（如实写）

- **W9 运行时验证**：手头没有 Windows 机器，做不到。
  只做到「编译 + 打包 + 地雷清单」，**不能声称「已验证兼容」**。
- 中文安装路径 + sherpa-onnx 加载唤醒词模型：未验证，风险最高。

## Windows 闸门第一次真跑就抓到 2 个真 bug（2026-10-25）

```
test result: FAILED. 246 passed; 2 failed; finished in 8.99s
  test dispatch::tests::suggest_dir_does_not_read_absolute_path ... FAILED
  test dsh::acp::tests::spawn_missing_binary_errors_cleanly ... FAILED
```

| 失败 | 真因 | 修法 |
|---|---|---|
| `spawn_missing_binary_errors_cleanly` | 我加的 `cmd /C` 兜底让「不存在的命令」也能 spawn 成功，于是「拉不起要立刻报错」这条路径在 Windows 上消失 | 去掉 `cmd /C`；CreateProcess 本来就能执行 .cmd/.bat |
| `suggest_dir_does_not_read_absolute_path` | 测试写死 `/tmp` 当候选目录，Windows 没有 /tmp → read_dir 失败 → 走了另一个分支 | 改用 `std::env::temp_dir()` |

**这就是加 Windows 闸门的价值**：这两个问题在 Linux 上永远不会暴露。
