# 验收 D4 · release 构建指标（阶段 5 部分完成）

**日期**：2026-10-24 04:05 (+08:00)
**机器**：wings-pc（16 核 / 15GB）
**构建**：`cargo tauri build --no-bundle`（release profile）

## 实测

| 指标 | 实测 | 验收要求（P4/P5/P6/P7） | 结论 |
|---|---|---|---|
| **二进制体积** | **18MB**（strip 后 13MB） | 安装包 <100MB / 理想 <40MB | ✅ 远优于要求 |
| **冷启动到窗口** | **1476ms** | <3s / 理想 <1.5s | ✅ 达到理想值 |
| **启动瞬间 RSS** | 92.9MB | — | — |
| **空闲稳定 RSS** | **190.1MB** | <150MB / 理想 <80MB | ❌ **超标 40MB** |
| **空闲 CPU** | **3.7%** | <5% / 理想 <2% | ✅ 合格（未达理想） |

启动日志：
```
INFO diemeng_app: 蝶梦启动 provider=stepfun model=stepaudio-3-realtime-preview key=1HeA****gBHe
```

## 内存超标的可能原因（未深入分析）

当前 demo 模式（`DIEMENG_DEMO_AUDIO=1`）每 33ms 合成 512 样本 PCM 并做一次 FFT。
虽然这是临时占位，但：
1. WebKitGTK 本身在软件渲染下占内存较大（`WEBKIT_DISABLE_DMABUF_RENDERER=1` + `LIBGL_ALWAYS_SOFTWARE=1` 是我为了在 KWin 下起窗口加的）
2. 若换成硬件渲染，内存可能显著下降
3. 粒子数量（720 个）与 Canvas 尺寸也会影响

**注意**：这个 190MB 是**在软件渲染 + demo 音频模式下**测的，不代表真实使用场景。
真实场景（接真音频、有 GPU）大概率更低，但**未验证**。

## bundle 未做

`--no-bundle` 只产出了二进制，没做 deb/AppImage 打包。
所以「安装包体积 <100MB」这条严格来说还未验证（二进制 18MB，打包后应该 50–70MB，但没实测）。

## 诚实标注

- ✅ 二进制体积、启动时间达标
- ⚠️ 内存超标，但测量条件不理想（软件渲染 + demo 音频），需在真实场景复测
- ❌ 安装包未打，P5 严格未验证
