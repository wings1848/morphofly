# wings-gpd 真机语音全链路测试

**最后更新**：2026-10-24 22:00
**脚本**：`tools/gpd_voice_e2e.sh`

这是本项目**第一次在真机上跑通完整链路**的记录。之前所有「跑通」都是在 x86 云主机上用
预录 PCM 文件喂 `e2e_drive` 示例，绕过了麦克风和 App 外壳。

## 结论先说

✅ **跑通了**：喊「小蝶」→ 唤醒 → 建云端会话 → 说任务 → 派活给 DSH →
DSH 真读文件 → 主动汇报正确答案。

同时**炸出一个严重 bug**（已修）：一句话派了 4 个任务。

## 三个关键事实（都实测过）

### 1. gpd 没有接实体音箱

| 测什么 | RMS | Peak |
|---|---|---|
| 放「小蝶」时录 **monitor 源**（看有没有出声）| 1397 | 17802 |
| 放「小蝶」时录 **麦克风**（看有没有听到）| 266 | 1406 |

麦克风那 266 就是底噪，全程平直（每秒 RMS 都在 258–279 之间），听不到任何东西。

**含义**：gpd 的 SOF 声卡在出声（软件侧有信号），但喇叭/耳机口没接设备，
所以「放出来再用麦克风听到」这条声学回环在 gpd 上**不可能做**。

### 2. 所以真机测试改用「数字注入」

造一个虚拟音源，让 App 从它录音：

```bash
pactl load-module module-null-sink sink_name=mfsink
pactl set-default-source mfsink.monitor
paplay --device=mfsink /tmp/xiaodie.wav     # 灌唤醒词
```

除了「空气」这一段，KWS、AEC、云端会话、工具调用、DSH 派活、主动汇报全是真的。

### 3. 麦克风原本打不开，真因是两个叠加

现象：

```
ALSA lib pcm_pulse.c:758:(pulse_prepare) [error.] PulseAudio: Unable to create stream: No such entity
ERROR morphofly_core::kws: KWS 监听线程异常退出 error=KWS 需要麦克风
```

**真因一：跑的是旧二进制。** systemd 服务 `ExecStart=/tmp/mf-install/usr/bin/morphofly-app`，
那是从 GitHub Actions 的 deb 解出来的（commit 更早，`61.8MB`），
里面**没有** cpal 的 PulseAudio 后端代码。本机 `target/release/morphofly-app`（61.6MB）
是新编的，但根本没被用上。

**真因二：`.asoundrc` 里 `device "default"` 在 pipewire-pulse 上不成立。**

```
pcm.!default { type pulse device "default" }
```

ALSA 的 pulse 插件会拿 `default` 当**源名字**去连，而 pipewire-pulse 没有叫
`default` 的源 → `PA_ERR_NOENTITY`。

**修法**：用 cpal 的 PulseAudio 后端直连 Pulse 协议（绕开 ALSA 桥接层），
并把服务指向新编的二进制。

验证日志（修复后）：

```
INFO morphofly_core::audio::capture: 音频后端：PulseAudio
INFO morphofly_core::kws: KWS 常驻监听已启动
```

## 完整跑通的那一次（原始日志）

```
21:50:39 🔊 检测到唤醒词 keyword=小蝶 at_sec=0.0
21:50:39 🔊 唤醒 keyword=小蝶 state="S1"
21:50:39 建立语音会话…
21:50:42 会话已建立 state="S2"

# 用户说「帮我看看 morphofly 项目的 README 第一行是什么」
21:51:33 本地判定说完 → commit
21:51:34 🔧 工具调用 name=delegate_task arguments={"task": "查看 morphofly 项目 README 文件的第一行", ...}
21:51:34 派活 agent=dsh dir=/home/wings/Code_Project/morphofly
21:51:38 DSH 会话已建 session=cdaf9179-…
21:51:43 🔔 主动汇报 report=干完了：README 的第一行是：也就是项目名的大标题 "MorphoFly"。
```

**汇报内容是对的** —— DSH 真去读了文件，不是编的。

## 同时炸出的 bug #11：一句话派 4 次任务

同一句话在 9 秒内派了 4 个任务、起了 4 个 DSH 会话、报了 4 遍：

```
21:51:34 🔧 delegate_task   → task_..._001
21:51:37 🔧 delegate_task   → task_..._002
21:51:40 🔧 delegate_task   → task_..._003
21:51:43 🔧 delegate_task   → task_..._004
```

### 根因（两条，都是判据错误）

1. **统计单位错了。** 兜底纠正逻辑按「一次模型响应」清零计数器，
   但一个用户请求天然跨多个 response：
   `用户说话 → response1(调 delegate_task) → 工具结果回灌 → response2(说「好，干完说」)`。
   response2 没调工具，就被误判成「假装干活」→ 注入纠正 → 模型再派一次 → 循环。

2. **判据用错了事件。** 代码把 `TranscriptDelta` 当「用户说了什么」，
   但它映射的是 `response.audio_transcript.delta` —— **模型自己说的话**。
   用户的话（`conversation.item.input_audio_transcription.completed`）
   在解析层就被丢弃了。所以「用户说过话」这个条件几乎恒真。

### 修法

- 统计单位改成「一次用户说话」：只在 `SpeechStarted` 清零；
- 新增 `pending_tool_results`：工具结果还没回灌时不参与「没干活」判断；
- 用新增的 `dispatch::looks_like_action` 判断「像不像在装忙」，闲聊不逼着派活；
- 一次用户说话最多纠正一次。

修复后复验见下节。

## 复验（复验命令与实测计数）

```bash
tools/gpd_voice_e2e.sh
```

脚本做的事：建虚拟音源 → 重启 App → 灌唤醒词 → 等唤醒 → 灌任务语音 →
统计 `delegate_task` / DSH 会话 / 主动汇报 / 纠正 的次数。

判据：`delegate_task == 1`、DSH 会话 ≤ 1、纠正 == 0。

_（实测结果见 `docs/v2/e2e-status.md` 的「gpd 真机」一节）_

## 踩坑清单

1. **改完代码要确认服务真的用了新二进制**。`systemctl cat morphofly-test` 看 `ExecStart`。
   这次就是改完代码、编完了，跑的却还是旧 deb，白折腾一轮。
2. **`paplay` 报 `Failed to drain stream: Timeout` 不代表没放出来**，看 monitor RMS 更可靠。
3. **别用 `pactl set-default-source` 设成硬件 monitor 源**（`alsa_output...monitor`）：
   pipewire 会把它自己录进去形成回环，`paplay` 会卡住。用 `module-null-sink` 干净。
4. **gpd 上 `cargo build --release -p morphofly-app` 约 7 分钟**（N6000，4 核）。
5. **本机（US 云主机）编不了 app crate**：缺 gtk/webkit 开发头。只能编 core，app 得上 gpd 或 CI。
