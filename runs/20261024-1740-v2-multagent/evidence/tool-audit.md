# 工具真实性审计（2026-10-24）

## 背景

这轮抓到两个「假通过」功能（取消任务、模型切换），
都是「只改本地字段，没通知 DSH」。
所以系统审计了全部 5 个工具。

## 审计结果

| 工具 | 实现 | 真实性 | 证据 |
|---|---|---|---|
| delegate_task | 调 bridge.run_task_as | ✅ 真 | 任务真的在 DSH 里跑 |
| query_progress | 读 TaskStore | ✅ 真 | 能看到 running→done 流转 |
| cancel_task | bridge.cancel_task | ✅ 真 | 发 oneshot 信号杀进程 |
| append_requirement | 按状态区分 | ✅ 真 | Running 时如实说「要停了重来吗」 |
| set_context(agent) | bridge.run_task_as | ✅ 真 | 按 agent 选命令 |
| set_context(project) | ctx.dir → delegate 用 | ✅ 真 | 实测从 /tmp 切到项目目录 |
| set_context(model) | bridge.set_model | ✅ 真（本轮修）| DSH 真的接受了 |

## 实测证据

### 任务状态机
```
派活: 好，干完说 | id=task_20261024_1141_001
  1s: status=running
  4s: status=done   speech=干完了：收到
✅ 状态流动正常
```

### 目录切换
```
初始: SessionContext { agent: "dsh", dir: "/tmp" }
切项目: 好，换成蝶梦了
切换后: SessionContext { agent: "dsh", dir: "/home/wings/Code_Project/morphofly" }
派活: 好，干完说
✅ 真的用了新目录
```

### 模型切换
```
✅ ["deepseek-official","deepseek-v4-pro"] → 成功
⚠️ deepseek-v4-pro                        → unknown model option（如实拒绝）
⚠️ ["deepseek-official","不存在的模型"]     → unknown model option（如实拒绝）
```

## 之前假通过的两个（已修）

1. **cancel_task**：只改 `TaskStatus::Cancelled`，`self.bridge` 在函数里根本没用到
2. **set_context(model)**：只改 `ctx.model` 字段，bridge 里零 model 代码

## 为什么会连续出现这类问题

**共同模式**：写了「状态变更」但没写「通知外部系统」。

- TaskStore 里的状态是本地副本
- ctx 里的 agent/model/dir 是本地副本
- 只改副本 = 用户以为改了，实际没变

**防范**：
1. 每个「切换类」功能，必须有一条端到端测试（不是单测）
2. 留意 clippy 的 `unused variable` —— codex 那个 bug 就是它救的
3. 改完立刻实测，别只看单测绿

## 附带发现

`configOptions` 里 model 的 `options` 数组是**空的**，
只有 `currentValue` 有值 → 本地无法校验模型名合法性，
唯一可靠方式是「发过去看 DSH 接受还是拒绝」。
这个设计决定了 `set_config_option` 必须等响应。
