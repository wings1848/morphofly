# 架构决策记录

## ADR-1：AEC 用 sonora 而不是 webrtc-audio-processing

- **status**: accepted
- **approvers**: [lead]
- **Context**：需要回声消除，避免外放自我打断
- **Decision**：用 sonora（纯 Rust AEC3 移植）
- **Alternatives 否掉的**：
  - webrtc-audio-processing：需要 clang/meson/ninja，本机没有且无 root
  - speexdsp：7 年未更新
  - 播放时暂停 VAD：实测真打断 0 次，牺牲核心能力
  - 提高 VAD 阈值：靠「回声恰好低于人声」硬撑，音量一变大就翻车
- **Consequences**：+607KB，CPU 0.85%，rust-version 提到 1.91

## ADR-2：多 agent 加第 5 个工具 set_context

- **status**: accepted
- **approvers**: [lead]
- **Context**：「切换 agent/目录/模型」是改持久状态，不是派一次活
- **Decision**：加 set_context，工具数 4→5；不变量 I1 改成「名字白名单+禁用名黑名单」
- **Alternatives 否掉的**：
  - 塞进 delegate_task 参数：语义错位
  - 监听 ASR 做关键词匹配：会误触发
  - 合并现有工具腾位置：query_progress 是最高频调用，合并增加歧义
- **Consequences**：以后加工具不被数字卡住，但 bash/read_file 等永久禁入

## ADR-3：缺陷 #4 用简化版复用而不是完整进程池

- **status**: accepted
- **approvers**: [lead]
- **Context**：每任务 spawn 一个 DSH 进程，启动开销 2.8s
- **Decision**：复用 1 个已握手进程（省 initialize），每次仍新建 session
- **Alternatives 否掉的**：
  - 完整多 session 进程池：需改 DshClient 的 session_id 路由，300-400 行，且必须有 GUI 验证并发
- **Consequences**：实测省 2.6 秒（比预估还好）；不支持并发多任务

## ADR-4：Computer Use 通过 SSH 桥接到 wings-gpd

- **status**: accepted
- **approvers**: [lead]
- **Context**：AEC 真机验证需要真实 GUI+麦克风，本机是云主机没桌面
- **Decision**：MCP 跑在 gpd 上，本机用 SSH stdio 桥接
- **Alternatives 否掉的**：
  - 在 wings-pc 上跑：无麦克风硬件
  - 脚本模拟：声音要从扬声器出来被麦克风收回去，脚本做不到
- **Consequences**：需要 gpd 在线；截图走 spectacle 替代 portal
