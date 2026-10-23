# 进度 · 蝶梦语音智能体 V1

更新：2026-10-24 05:25 (+08:00)
run: `20261023-2339-v1-dsh-0`（standard 档 / desktop-app / inspect 模式）

## 一句话现状

**V1 的核心体验全部跑通并接进了 App**：喊「小蝶」或点小球 → 进入通话 → 说话派活 → DSH 执行 → **干完主动开口汇报**。
**唯一不达标的是延迟**（实测 2–7 秒，要求 800ms），原因是 StepFun 服务端 VAD 有 1.6 秒硬下限。

## 六个阶段

| 阶段 | 状态 | 关键证据 |
|---|---|---|
| 0 打通接缝 | ✅ 7/7（含「真的出声」） | `evidence/g0.*` |
| 1 能说话 | ✅ 双工 + 六态小球 | `evidence/g1.*`、`screenshots/` |
| 2 能派活 | ✅ 4 工具 + DSH 桥 | `evidence/g2.*` |
| 3 能主动汇报 | ✅ 已接进 App | `evidence/g3.1`、`g5.1` |
| 4 唤醒词 | ✅ Rust KWS 9/9 召回 0/8 误唤醒 | `evidence/g4.*` |
| 5 打磨打包 | ⏳ 约 60% | `evidence/g5.1`、`p2-release-metrics.md` |

## 测试现状

```
cargo test -p diemeng-core --lib              → 177 passed, 0 failed
cargo test -p diemeng-core --features kws     → 183 passed
node --test app/frontend/test/                →   8 passed
编译警告                                       →   0
```

## 已实测的指标

| 指标 | 实测 | 要求 | 结论 |
|---|---|---|---|
| 二进制体积 | 18MB（strip 13MB） | <100MB | ✅ |
| 冷启动到窗口 | 1476ms | <3s / 理想 1.5s | ✅ |
| 空闲内存 | 190MB | <150MB | ❌（软件渲染模式下测的） |
| 空闲 CPU | 3.7% | <5% | ✅ |
| 点击→能说话 | ~150ms | — | ✅ |
| WS 连接 | 103–176ms | — | ✅ |
| **端到端延迟** | **2000–7000ms** | <800ms | ❌ **不达标** |
| 唤醒召回（Rust） | 9/9 = 100% | ≥95% | ✅ |
| 唤醒误唤醒（Rust） | 0/8 = 0% | <1 次/8h | ✅ |
| 派活→DSH 执行 | 3.6s | — | ✅ |

## 阻塞项：P1 延迟（等用户决策）

`evidence/p1-vad-parameter-sweep.md` 的决定性实验：

| silence_duration_ms | 语音结束→speech_stopped |
|---|---|
| 100 / 300 / 500 / 1000 | 1604 / 1608 / 1608 / 1612 ms |

**参数拉 10 倍，结果差 4ms。StepFun 的 VAD 参数完全不生效，1.6 秒是服务端硬下限。**
手动模式（turn_detection=null + commit）更慢：commit 要 5 秒。

**等用户选 A/B/C**：
- A. 换阿里云百炼 Qwen（需要 DashScope key，代码已备好 `docs/切换语音服务商.md`）
- B. 改产品定位为「语音留言」模式
- C. 接受 2–7 秒

## 审计整改（8 项全处理）

见 `verify-report.md` §6。3 个真 bug：
1. S5 状态机缺口（待机断线不提示）
2. Tauri 2 缺 capabilities → IPC 全静默失败
3. 音频路由到虚拟设备（声音听不见）

## 踩坑记录（14 条，全部写进代码注释或文档）

1. StepFun `energy_awakeness_threshold` 默认 2500 太高 → VAD 不触发
2. StepFun 只支持 `server_vad`，参数不生效（1.6s 硬下限）
3. StepFun tools 要 OpenAI 嵌套格式
4. 同一次工具调用被报两遍 → 需按 call_id 去重
5. **Tauri 2 缺 capabilities 时 IPC 全静默失败**
6. CSP 没放行 `ipc:` → 同样静默
7. 改前端后 `cargo build` 有时不重新嵌入 → 要 `tauri build` 或清 build 缓存
8. KWin/Wayland 下 Tauri 窗口要 `GDK_BACKEND=x11` + 关 dmabuf
9. **`import -window` 在 KWin 下截到过期图层** → 用 spectacle 全屏再裁
10. 本机 MagicDNS 失效 → `tools/pc_ssh.sh` 回落 Tailscale IP
11. 本机无 root 无 ALSA 头 → deb 解包到 `~/.local/sysroot`
12. 本机磁盘会爆（target 4GB）→ `.cargo/config.toml` 关增量编译
13. 英文项目名对中文 ASR 不友好（diemeng → daimian/diamond/aimeng）
14. **sherpa-onnx Rust 绑定 `get_result` 没命中时返回 `Some(空)` 而非 `None`** → 静音也"命中"

## 下一步（不依赖用户决策的）

1. barge-in 真实打断实测（播放中喊「停」）
2. 静音 30s 自动退出跑满验证
3. release 打包（deb/AppImage）

## 等用户决策后才能做的

- P1 延迟（A/B/C）
- 真人完整验收（需要用户本人戴耳机跑）
