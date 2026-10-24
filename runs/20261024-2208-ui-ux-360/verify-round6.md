# 第 6 轮验证（目录修复 + 主动汇报断线问题）

**日期**：2026-10-25 01:05

## 1. 目录修复：真机确认

```
16:56:07 WARN 模型只说不做 → 本地用用户原话代派 said=帮我看看蝶梦项目的vimi第1行是什么？
16:56:07 INFO 派活 agent=dsh dir=/home/wings/Code_Project/morphofly   ← 派对了
16:56:16 INFO DSH 会话已建 … cwd=/home/wings/Code_Project/morphofly   ← 会话也建起来了
```

之前是 `dir=/home/wings`（建会话失败）。现在按「任务里点名的项目」解析。

## 2. 但发现了更关键的一环：会话活不到汇报

```
16:59:10 派活（成功）
16:59:18 DSH 会话已建
17:00:38 WARN 读取 WebSocket 失败 … Connection reset
17:00:38 WARN 实时语音服务端返回 error: too long without operation
17:00:38 会话结束
```

DSH 干一个任务 1–3 分钟，而：
- 我们的状态机 30 秒空闲回 S0
- 云端 90 秒没操作直接断

**所以「主动汇报」从架构上就不可能兑现** —— 干完了会话早没了。

## 3. 修法（commit `9d92c40`）

会话守卫每 250ms 看一眼：S0 + 任务表里有「已完成但没念过」的结果
→ 发 `Stimulus::ProactiveReport`（模拟唤醒）→ 建会话 → 新会话的 report 循环立刻念。

带 15 秒冷却避免失败时反复开窗。+1 单测。

## 4. 验收脚本也修了

之前只数「工具调用 name=delegate_task」，本地代派不算 → 误判成没派活。
现在数「工具调用 name=delegate_task|派活 agent=」，并且等主动汇报用轮询（15–40 秒才回来）。

## 待验证

主动汇报端到端（等新包）。
