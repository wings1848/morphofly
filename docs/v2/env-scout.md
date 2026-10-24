# V2 环境侦察结果

**日期**：2026-10-24 16:30
**目的**：确认用户提到的「wings-pc 的 DSH + GPU 算力」能否用于端到端测试

## 一、DSH 配置对齐（用户明确要求）

### 两边版本一致

| 项 | wings-tx-us1（本机） | wings-pc |
|---|---|---|
| DSH 版本 | `0.2.1-rc.2` | `0.2.1-rc.2` ✅ |
| 路径 | `/home/wings/.bun/bin/dsh` | `/home/wings/.bun/bin/dsh` ✅ |
| `acp` profile | 有 | 有 ✅ |
| acp bundle 列表 | `dsh-base` + `dsh-acp-app` | 完全相同 ✅ |
| `DEEPSEEK_API_KEY` | 有 | 有 ✅ |

**结论：两边的 acp profile 完全一致，不需要"对齐配置"——已经是同一份。**
差异只在凭据（wings-pc 还有火山、CommandCode 等其他 key，与蝶梦无关）。

### 远程 ACP 实测可用

从本机 SSH 到 wings-pc 跑 `dsh --profile acp`，`initialize` 握手成功：

```json
{"jsonrpc":"2.0","id":1,"result":{
  "protocolVersion":1,
  "agentInfo":{"name":"deepseek-harness-acp","version":"0.0.1"},
  "agentCapabilities":{"mcpCapabilities":{"http":true}}
}}
```

**这意味着：蝶梦可以连远程 DSH 做端到端测试**，不必在本机装。

### wings-pc 上已有的 DSH 服务

```
dsh-lab-web.service  运行中  dsh web --host 127.0.0.1 --port 3180
dsh-web.service      运行中  DeepSeek Harness Web UI
```

## 二、GPU 算力

### 硬件

| 项 | 值 |
|---|---|
| GPU | NVIDIA GeForce GTX 1660 Ti |
| 显存 | 6144 MiB，当前占用 1221 MiB，**空闲 4.9 GB** |
| 驱动 | 615.78.08 |
| cuDNN | **已装**（20 个库） |
| CUDA 编译器（nvcc） | **未装** |

### 能用 GPU 做什么（明确边界）

| 用途 | 可行性 |
|---|---|
| 跑 ONNX/CUDA 推理 | ⚠️ 需要装 ONNX Runtime GPU provider（当前只有 CPU 版） |
| 跑 PyTorch 推理 | ⚠️ 需要装 torch+cuda（当前未装） |
| 编译 CUDA 代码 | ❌ 无 nvcc |
| 本地大模型（Ollama/vLLM） | ⚠️ 6GB 显存，只能跑 7B 量化模型 |

**当前 GPU 处于"有硬件没软件"状态，需要搭环境才能用。**

### 对蝶梦的价值判断（重要）

蝶梦的音频链路里，**本地只跑两个小模型**：
- 唤醒词 KWS：约 4.4MB，CPU 已跑 1.1s 命中延迟
- VAD：643KB，CPU 已跑 80ms 判定

**这两个模型在 CPU 上已经足够快，GPU 加速收益很小。**

真正吃延迟的是**云端语音模型**（StepFun，802ms），那部分不在本机算。

**所以 GPU 对蝶梦当前架构的价值有限。** 除非：
1. 改用本地大模型做语音理解（但要 6GB 显存跑不动好的模型）
2. 用 GPU 跑批量测试（合成测试音频、批量跑 ASR 对比）← **这个有实际价值**

**建议**：GPU 先用于「批量测试加速」，不用于生产链路。

## 三、对 V2 计划的影响

| 用户要求 | 侦察结论 |
|---|---|
| 用 wings-pc 的 DSH 测试 | ✅ 可行，远程 ACP 已验证 |
| 对齐 tx-us1 的 DSH 配置 | ✅ 已一致，无需操作 |
| 用 GPU 算力 | ⚠️ 需先搭环境；但对当前架构收益有限，建议用于批量测试 |
| 编译走 GitHub CI/CD | ✅ 已配好（`.github/workflows/release.yml`） |
