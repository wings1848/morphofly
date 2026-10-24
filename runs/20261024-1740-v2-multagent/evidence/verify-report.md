# 验证报告（S5）

## 单元测试

| 项 | 命令 | 结果 |
|---|---|---|
| core 默认 | `cargo test -p morphofly-core --lib` | **232 passed** |
| core + kws | `cargo test -p morphofly-core --features kws --lib` | **243 passed** |
| 前端 | `node --test 'app/frontend/test/*.test.mjs'` | **8 passed** |
| e2e mock | `python3 tools/e2e/run.py --mode mock` | **14/14** |
| **e2e 真机** | `python3 tools/e2e/run.py --mode real` | **13/14** |

## 静态检查

| 项 | 命令 | 结果 |
|---|---|---|
| 格式 | `cargo fmt --all --check` | 干净 |
| clippy | `cargo clippy --lib --tests` | **0 警告** |
| 编译 | `cargo build` | **0 警告** |
| CI | GitHub Actions | ✅ 通过 |
| 真机编译 | wings-pc release | ✅ 通过 |
| 真机测试 | wings-gpd（N6000 无 AVX）| **243 passed** |

## 不变量验证

| 不变量 | 测试 | 结果 |
|---|---|---|
| 模型无文件/命令能力 | `tools::tests::rejects_forbidden_tool_names` | ✅ |
| AEC 不牺牲打断 | `aec::tests` 双讲人声保留 43-55% | ✅ |
| 帧长恒 320 | `aec::tests` 帧长断言 | ✅ |
| 不会误唤醒 | KWS 对 3 条谐音 + 纯静音全 0 命中 | ✅ |

## 独立验证

用 subagent 做了技术侦察（2 份报告），我抽样复核了 4 条关键结论：
- ✅ 5 个现存缺陷（读代码确认）
- ✅ AEC 测试信号问题（subagent 纠正了我的错误判断）
- ⏳ PSS vs RSS 方法论（wings-pc 桌面不可用，未复核）
- ⏳ sonora 不需要 clang（间接确认：编译通过）

## 未验证项（如实列出）

| 项 | 为什么没验 |
|---|---|
| AEC 真机效果 | 需要真实 GUI 操作，正在用 computer use 做 |
| 多任务并发 | 需要 GUI 环境 |
| 拖动/托盘目视效果 | 需要 GUI 环境 |
| 内存 PSS 数据 | wings-pc 桌面未登录 |

## 发现并修复的 bug（真机 e2e 抓到）

| # | 问题 | 严重度 |
|---|---|---|
| 1 | 模型编造目录列表（6 个目录全不存在）| 🔴 致命 |
| 2 | 模型凭空捏造路径（Code_Project/M1）| 🔴 致命 |
| 3 | 模型假装干活（说「好查着呢」但不调工具）| 🔴 致命 |
| 4 | cancel_task 不真停 DSH | 🟠 欺骗用户 |
| 5 | append 不发给运行中 DSH | 🟠 欺骗用户 |
| 6 | set_context(model) 只改字段 | 🟠 假功能 |
| 7 | set_context(agent) 只改字段 | 🟠 假功能 |
| 8 | ACP 不按 sessionId 过滤 | 🟠 多 agent 地基 |
| 9 | 打断不停云端生成 | 🟡 浪费 token |
| 10 | 每任务 spawn（慢 2.8s）| 🟡 性能 |

**10 个里 7 个是「验收表写了通过但实际不成立」的假功能。**
