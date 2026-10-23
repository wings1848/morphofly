# assumptions · 本轮采用默认值（巡检模式，不阻塞等回答）

| # | 问题 | 采用默认 | 依据 | 反悔成本 |
|---|---|---|---|---|
| 1 | 首选哪家实时语音 | **StepFun `stepaudio-3-realtime-preview`** | 画布降级链首选、限免、key 已在手（vault canvas）；Qwen 做成第二个 provider 配置项 | 低：换 provider 只改配置文件 |
| 2 | V1 平台 | **Linux 优先（wings-pc）** | 唯一有真实麦克风 + 显示器的机器；US 云主机无音频设备、无桌面 | 低：core 与 Tauri 跨平台 |
| 3 | 唤醒词 | **「小蝶」+ `num_trailing_blanks=4`** | 已有 8/8 召回 0/8 误唤醒实测 | 低：关键词文件可改 |
| 4 | 前端要不要打包器 | **不用**（原生 ES module + 静态目录） | 少一层构建，Tauri 直接吃静态目录 | 低 |
| 5 | 密钥放哪 | `~/.diemeng/config.toml`（600）+ 环境变量覆盖；**不入库** | 不变量 A1-5 | 低 |
| 6 | 人机模式 | inspect（巡检） | devflow 默认 | — |

## 需要用户知道但不阻塞的事

- vault 画布里那条阶跃密钥是**明文**写在 `.canvas` 里的，Obsidian 仓库若同步到公网/分享会泄漏。
  建议去阶跃控制台轮换一次。**我没有把它写进仓库任何文件**，只读取到本机临时文件后立即用于环境变量。
- 阿里云百炼 key 尚未提供；等 key 到位再补 Qwen provider 实测。
