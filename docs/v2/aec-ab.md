# A/B：AEC 会不会影响云端识别（从而影响模型调不调工具）

**状态：没做完（脚本卡在 journalctl -f 管道上），结论待补。**

## 为什么要做

探针一/二/三排除了三个假设（转写不支持、tool_choice 不支持、commit 抢跑不是原因），
而探针里**干净音频每次都正常调工具**。与 App 唯一的差别是音频链路：

```
探针：直推 wav
App ：虚拟音源 → PulseAudio 采集 48k → 重采样 16k → AEC(sonora) → 推流
```

所以下一个嫌疑是 AEC 动过音频，云端 ASR 听不清 → 模型只能敷衍。

## 已知的间接证据（偏向「ASR 质量」而非「AEC 坏了」）

真机实测抓到的派活参数：
```
{"task": "查看蝶梦项目中咪咪文件的第一行内容", "dir": ".../morphofly"}
```
- `README` 被听成「咪咪」→ 说明**音频送到了、能转写，但识别错了**
- 更早还有 `morphofly` → `move_forward`
- 而 AEC 若是把信号搞坏，通常是「整句听不出」而不是「关键词听错」

所以更可能是**云端 ASR 对英文/专有名词不友好**（产品已知问题 #3），
AEC 不是主因。但这只是推断，没做 A/B，不能写死。

## 怎么重做（下一轮）

别用 `journalctl -f | grep -m1`（ssh 管道不会自动退出，会把脚本挂住）。
改成「重启后轮询检查日志直到出现某行」，并且直接复用 gpd 上已有的
`/tmp/xiaodie.wav`（唤醒）和 `/tmp/task1.wav`（任务），不要每次用 edge-tts 重新合成。

```bash
# A：AEC 开（默认）
# B：systemd unit 里加 Environment=MORPHOFLY_AEC=0
# 各跑 3 次，统计 delegate_task 是否需要「纠正后才出现」
```

判据：A/B 两组「首轮就调工具」的比例。
