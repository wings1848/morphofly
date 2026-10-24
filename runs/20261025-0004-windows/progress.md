# 进展（Windows 兼容）

**最后更新**：2026-10-25 01:00

## 已完成（有 CI 实测证据）

| 项 | 结果 |
|---|---|
| W1 仓库能在 Windows 上检出 | ✅ success（删掉 `...` 非法文件名之后） |
| W2 core 能在 Windows 上编译 | ✅ success |
| W3 core 248 个单测在 Windows 上跑 | ✅ **248/248 全过**（11 秒） |
| W4 Tauri 壳在 Windows 上编译 | ✅ success（`--no-default-features`） |
| W6 数据目录不依赖 HOME | ✅ 本机 3 个单测 |
| W7 拉 dsh 兼容 .cmd | ✅ 代码 + PATHEXT 分支 |
| W8 平台代码隔离 | ✅ `#[cfg(target_os="linux")]` |

## 进行中

- W5 **NSIS 安装包**：release 的 Windows job 还在 `cargo install tauri-cli`（Windows 上冷编译很慢，已 40+ 分钟）

## 修掉的问题（按发现顺序）

1. **仓库根目录有个被跟踪的空文件 `...`** —— Windows 文件名不能以点结尾，整个仓库检不出来
2. **`ci-dist/` 423MB 构建产物入库**（400 文件，单个 libwebkit2gtk 86MB）→ 停止跟踪
3. **数据目录只读 `HOME`** → HOME/USERPROFILE/'.' 三级回退
4. **`Command::new("dsh")` 不认 `dsh.cmd`** → PATHEXT 逐试
5. **`cmd /C` 兜底让「命令不存在」也能 spawn 成功** → 去掉（Windows 单测挂了 40 分钟才发现）
6. **测试写死 `/tmp`** → 改用 `std::env::temp_dir()`
7. **平台代码未隔离** → `force_x11_backend` cfg 到 linux

## 没做到的（诚实写）

- **运行时没在真 Windows 上验过**（手头没有 Windows 机器）
- 中文安装路径 + sherpa-onnx 加载唤醒词模型：风险最高，未验证
