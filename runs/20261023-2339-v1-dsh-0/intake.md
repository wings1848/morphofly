# intake · 蝶梦语音智能体 V1

- **需求来源**：`~/BrainWorkshop/Project/蝶梦智能/`（9 份文档 + 1 张画布，2026-10-23 完成规划）
- **一句话目标**：喊一声「小蝶」进入通话，用嘴指挥 DSH 干活，活干完它主动回来说 —— 全程零键鼠。
- **档位**：standard（lib/service 级、要长期维护、有硬性能/成本指标）
- **项目类型**：desktop-app（Rust core + Tauri 2 壳 + WebView 画球）
- **人机模式**：inspect（巡检）—— 只停 H2/H4，H1/H3 按默认走并把摘要推 TG
- **door**：two_way（阶段 0 是探针，随时可改架构 → 自动过 H2）
- **reversible_until**：阶段 0 全绿之后才冻结架构；在此之前改技术栈成本低
- **根目录**：`~/Code_Project/diemeng`
- **预算**：≤35 步 / ≤10 万输出 token；单阶段失败重试 ≤2

## 本次范围（V1）

只做**桌面端**（先 Linux/CachyOS 开发机，Windows/macOS 打包放后续），只做**阶段 0 + 阶段 1** 作为本轮交付：

1. 阶段 0：7 个接缝验证（G0.1–G0.7）全绿
2. 阶段 1：完整双工音频链路 + 小球 + 点球进出通话 + 字幕

**不做**（本轮）：AEC（要求戴耳机）、KWS 常驻集成（阶段 4）、任务表（阶段 2）、主动汇报（阶段 3）、打包分发（阶段 5）、ESP32/移动端。

## 已有资产（别重复造）

| 资产 | 位置 |
|---|---|
| 需求/技术选型/开发方案/验收标准/UI 提示词 | `~/BrainWorkshop/Project/蝶梦智能/*.md`（已复制到 `docs/`） |
| 15 个测试音频（含 5 个误触发负样本） | 仓库 `testdata/audio/` |
| 唤醒词一键验收脚本 | 仓库 `tools/kws/`（源自 vault `kws-harness/`） |
| 既有架构设计（含 StepAudio 真机实测） | `~/BrainWorkshop/想法/语音交互前台架构.canvas` |
| DSH 接入方式 | `dsh --profile acp`（stdio JSON-RPC）/ `--profile sdk` |
