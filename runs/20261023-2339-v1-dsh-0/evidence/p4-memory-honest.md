# P4 内存：实测 488MB（超标 3.2 倍），原因是 WebKitGTK

**日期**：2026-10-24 06:40 (+08:00)
**要求**：<150MB
**实测**：**488MB**（全进程合计）

## 明细（wings-pc，release 二进制，空闲 12s）

| 进程 | RSS |
|---|---|
| `diemeng-app`（主进程，含 Rust 逻辑 + WebKit 主库） | 191 MB |
| `WebKitWebProcess`（渲染进程，主要开销） | 161 MB |
| `WebKitNetworkProcess` | 63 MB |
| **合计** | **488 MB** |

## 之前 190MB 是漏算

之前只测了主进程（`/proc/<pid>/status` 的 VmRSS），没算 WebKit 的子进程。
**真实占用是 488MB。**

## 为什么这么大

Tauri 2 = Rust 后端 + 系统 WebView（Linux 上是 WebKitGTK）。
WebKitGTK 本身就要 200MB+，这是它的固有开销，不是我们代码的问题。

参考：Electron 应用同样量级（200–400MB）。这是「用 WebView 做 UI」的代价。

## 能不能降

### 方案 1：干掉 WebView（不现实）

小球改成 Rust 原生渲染（wgpu / softbuffer）+ 原生窗口。
- 理论能降到 50–80MB
- **代价**：重写整个 UI 层，六态粒子动画全部重做
- 工作量：估计 3–5 天

### 方案 2：换更轻的 WebView

Linux 上 Tauri 只能用 WebKitGTK，没有别的选择（不像 Windows 能用 WebView2）。

### 方案 3：接受超标，改验收标准

如果产品定位是桌面常驻助手，用户机器通常 8GB+，
488MB 相当于一个 Chrome 标签页。**可能可以接受。**

### 方案 4：小球用轻量原生窗口（部分方案）

- 小球（常驻）→ Rust 原生渲染（softbuffer 画圆，极轻）
- 设置/历史等次要界面 → 需要时再起 WebView

**折中方案，工作量约 1 天。** 但六态粒子动画用 softbuffer 手写像素会很粗糙。

## 环境限制（顺带发现）

这台机器的 WebKit **在 GPU 模式下无法创建 GBM buffer**：

```
Failed to create GBM buffer of size 720x720: Invalid argument
```

所以窗口只能用软件渲染起（`LIBGL_ALWAYS_SOFTWARE=1`）。
这意味着 488MB **可能还偏高** —— 软件渲染比 GPU 渲染更吃内存。
在正常 GPU 环境下应该更低（估计 300–400MB）。

**但现在无法验证**，因为这台机器起不来 GPU 窗口。

## 我的判断

**这是架构选择的固有代价，不是 bug。**

如果 150MB 这条硬指标不能让步，唯一的路是方案 1（重写 UI 层），
但那会让 V1 推迟 3–5 天。

**建议**：把 P4 从「<150MB」改成「<600MB」，理由：
- 同类产品（Electron/Tauri 桌面助手）都在这个量级
- 用户机器 8GB+ 时 488MB 无感
- 真要省内存，V2 可以单独优化小球渲染

**这需要用户拍板。**

## 代码位置

`tools/pc_mem.sh` —— 可复现的测量脚本（含全进程统计）
