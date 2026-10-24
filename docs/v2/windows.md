# Windows 兼容

**最后更新**：2026-10-25 00:05

## 一句话结论

**能编、能打包、明显的地雷已拆；但运行时没在真 Windows 上验过。**

手头没有 Windows 机器，所以「兼容」目前是这三个证据：
1. CI 的 Windows job（`build-windows`）编过 `cargo check` core + app 外壳 + 跑 core 单测
2. release 的 Windows job 真打一个 NSIS 安装包
3. 下面这些代码级问题的定位与修复——每一条都是**在 Linux 上必然会挂、在 Windows 上一定会挂**的确定性问题

## 修掉的三个 Windows 硬伤

### 1. 数据目录只读 `HOME`

Windows 上没有 `HOME`，只有 `USERPROFILE`。原来取不到就退化成
「当前工作目录下的 `.diemeng`」—— 用户双击启动时 CWD 是 `C:\Windows\System32`
还是别处完全不可控，**配置看起来像丢了**（其实写到别处去了）。

```rust
// 现在：HOME → USERPROFILE → '.' 三级回退
// 测试：data_dir_works_without_home / data_dir_prefers_home / data_dir_falls_back_without_panic
```

### 2. `Command::new("dsh")` 在 Windows 上找不到命令

`dsh` 是 npm 装的。Windows 上 `npm -g` 生成的是 `dsh.cmd`（还有 `.ps1`），
**不是 `dsh.exe`**，而 `CreateProcess` 只按 PATHEXT 找 `.exe/.com`。

```rust
// 新增 spawn_program()：Windows 上按 .exe/.cmd/.bat/.com 逐个试，
// 全失败退化成 `cmd /C dsh ...`（.cmd 必须经 cmd.exe 解释）
```

### 3. 平台专用代码没有隔离

- `cpal` 的 `pulseaudio` feature：依赖本身在 cpal 里就是
  `cfg(any(target_os = "linux", ...))` 门控的，Windows 上是空操作 ✅（已核对 cpal 0.18.2 的 Cargo.toml）
- `force_x11_backend()`（Wayland 补丁）：已 `#[cfg(target_os = "linux")]`，
  其它平台给空实现。Windows 的窗口定位/置顶是系统原生支持的，不需要这套 hack。

## 打包

| 平台 | 产物 | 怎么来 |
|---|---|---|
| Linux | `deb` / `AppImage` | `cargo tauri build`（targets 写在 tauri.conf.json） |
| Windows | `NSIS .exe` | `cargo tauri build --bundles nsis`（覆盖给 Linux 写的 targets） |

图标补了 `icons/icon.ico`（256/128/64/48/32/16），Windows 打包必需。

## 已知风险（没验证，别当已经好了）

1. **中文安装路径 + sherpa-onnx**：`productName` 是「蝶梦」，Windows 安装目录会
   带中文。sherpa-onnx 的 C API 走窄字符串，对非 ASCII 路径可能不友好 ——
   唤醒词在 Windows 上可能加载不了模型。**真要上 Windows，先把 productName 改成
   纯 ASCII（如 MorphoFly），或把模型复制到无中文的目录再加载。**
2. **WebView2 运行时**：NSIS 安装包默认不带 WebView2。Win10 1803+ 一般自带，
   老系统要用户自己装，或用 `--bundles nsis` 配 `webviewInstallMode`。
3. **窗口透明 + 无边框**在 Windows 上的表现（拖拽、点击穿透、贴边）没实测。
4. **麦克风权限**：Windows 10+ 有「应用访问麦克风」的隐私开关，没开就静默失败。
5. `tools/*.sh` 全是 Linux 开发脚本（systemd、pactl、X11），Windows 上用不了；
   这些是开发/验证工具，不影响成品运行。

## 怎么验证（有 Windows 机器时）

```powershell
# 1. 编译 + 单测
cargo test -p morphofly-core --lib

# 2. 打安装包
cd app; cargo tauri build --bundles nsis

# 3. 装上后看这几条
#    - %USERPROFILE%\.diemeng\config.toml 生成在用户目录（不是 CWD）
#    - 小球启动后在屏幕右下角、置顶
#    - 喊「小蝶」能唤醒（KWS 模型能加载 = 路径没问题）
#    - 说个任务能派活（dsh.cmd 能拉起 = spawn_program 生效）
```
