# 事实核对（S2）

每条断言都配可复现命令和实测输出。

## DSH ACP 能力

| 断言 | 命令 | 实测 |
|---|---|---|
| 支持 `session/set_config_option` | 探针 initialize + session/new | configOptions 返回 2 项（model, reasoning_effort）|
| 不支持 `session/set_mode` | 同上 | `Method not found` |
| model 值是 JSON 数组字符串 | `session/set_config_option` 传裸名 | `Invalid params: unknown model option` |
| configOptions 的 options 是空数组 | session/new 返回值 | `options: []`，只有 currentValue |

## Codex 接入

| 断言 | 命件 | 实测 |
|---|---|---|
| codex CLI 无 ACP 子命令 | `codex help \| grep -c acp` | 0 |
| 官方适配器可用 | `npx -y @zed-industries/codex-acp` | `agentInfo.name="codex-acp"` |
| 需要登录 | `codex login status` | `Not logged in` |

## 音频硬件

| 设备 | 命令 | 实测 |
|---|---|---|
| wings-pc 无麦克风 | `cat /proc/asound/cards` | 只有 Loopback / PCH / NVidia，全是输出 |
| wings-gpd 有麦克风 | `arecord -l` | `card 0: sofessx8336`，有输入 |
| gpd 麦克风能收到扬声器 | parecord + paplay | RMS 0.00987，峰值 3293 |

## AEC

| 断言 | 命令 | 实测 |
|---|---|---|
| 合成信号 34.5dB | `cargo test --lib aec` | 19 passed |
| 固定正弦只有 1.9dB | 同上（旧信号） | 延迟估计锁错 lag |

## 进程复用

| 断言 | 命令 | 实测 |
|---|---|---|
| 冷启动 3698ms | `--example pool_bench` | 3698ms |
| 复用后 1102ms | 同上 | 1102ms / 1347ms |
