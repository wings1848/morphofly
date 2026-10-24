# 验收（Windows 兼容）

用户原话：**「确保项目兼容windows平台」**

| # | 标准 | 判定命令 | 期望 | 实测 |
|---|---|---|---|---|
| W1 | 仓库能在 Windows 上检出（无非法路径） | CI `build-windows` 的 checkout 步骤 | success | |
| W2 | core 能在 Windows 上编译 | CI `cargo check -p morphofly-core` | success | |
| W3 | core 单测能在 Windows 上跑过 | CI `cargo test -p morphofly-core --lib` | 全过 | |
| W4 | app 外壳能在 Windows 上编译 | CI `cargo check -p morphofly-app --no-default-features` | success | |
| W5 | 能打出 Windows 安装包 | release `cargo tauri build --bundles nsis` | 产出 .exe | |
| W6 | 数据目录不再依赖 HOME | 本机单测 `config::data_dir_*` | 3 个测试过 | ✅ 11:xx 已过 |
| W7 | 拉起 dsh 兼容 .cmd | 代码审查 + 注释（Windows 上无法单测） | 有 PATHEXT 分支 | ✅ |
| W8 | 平台代码隔离 | `grep -c cfg\(target_os = "linux"\)` | force_x11_backend 被隔离 | ✅ |

## 不做 / 做不到（如实写）

- **W9 运行时验证**：手头没有 Windows 机器，做不到。
  只做到「编译 + 打包 + 地雷清单」，**不能声称「已验证兼容」**。
- 中文安装路径 + sherpa-onnx 加载唤醒词模型：未验证，风险最高。
