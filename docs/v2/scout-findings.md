# V2 侦察结论汇总（含 5 个现存缺陷）

**日期**：2026-10-24
**来源**：subagent 技术侦察 + 我抽样复核

## 一、5 个现存缺陷（全部已复核确认）

这些是「验收表里写了通过、但实际不成立」的功能。**比新需求更优先修。**

### 缺陷 1：`cancel_task` 不真的停 DSH ⚠️

```rust
// core/src/dispatch.rs:168-188
async fn cancel(&self, v: &Value) -> Result<ToolResult> {
    let mut s = self.store.lock().await;
    s.set_status(&id, TaskStatus::Cancelled)?;   // 只改本地状态
    // 之后什么都没做 —— self.bridge 在这个函数里根本没用到
}
```

**后果**：用户说「停」，蝶梦回「好，停了」，但 DSH 里的活**还在跑**。
这就是「说停了其实没停」——比不支持取消更糟，因为它欺骗用户。

**修法**：`DshBridge` 保存每个任务的 cancel handle，`cancel()` 时真的杀进程/发 cancel。

### 缺陷 2：`append_requirement` 不发给运行中的 DSH ⚠️

`append()` 只把内容拼到本地 `task.goal`，`bridge.rs` 里**没有任何 append 通道**。

**后果**：用户说「顺便再加个要求」，蝶梦回「行，我给它补上了」，
但 DSH 那边根本不知道。**同样是欺骗用户。**

**修法**：要么真的往运行中的会话追加（ACP 有 `session/prompt` 可以再发一轮），
要么诚实地告诉用户「这活儿已经在跑了，加不了新要求，要我停了重来吗」。

### 缺陷 3：ACP 事件不按 sessionId 过滤 ⚠️ 多 agent 的地基

```rust
// core/src/dsh/acp.rs:309-313
"session/update" => {
    let update = params.get("update")...;   // 只读 update，不读 params.sessionId
    // 所有事件进同一个 events_tx
}
```

**实测**（subagent 做的）：一个 dsh 进程开两个 session 并发 prompt，
session A 能看到 session B 的 update。

**后果**：多 agent / 多任务并发时，事件会串台。
**这是多 agent 功能的前置必修项** —— 不修的话，加了多 agent 也是坏的。

**修法**：`handle_message` 加 `expected_session_id` 参数，不匹配就丢弃。

### 缺陷 4：每任务 spawn 一个 dsh 进程（慢 2.8 秒）

```rust
// core/src/dsh/bridge.rs
let client = DshClient::spawn(...)?;   // 每任务新建进程
client.connect().await?;              // initialize 1.35s
// session/new 再 1.45s
```

**实测**：纯启动开销 **2.8 秒**（3 次均值），任务还没开始干就先等 3 秒。

**修法**：常驻进程池 + 每任务一个 session。
实测验证：一个进程能同时开多 session（不同 cwd），并发 prompt 不串台。
且会话跨进程可恢复（`session/resume` 能续上历史）。

### 缺陷 5：打断不停云端生成

```rust
// app/src-tauri/src/voice.rs:261-269
player.stop_now();          // 只停了本地播放
// 从没调用 client.cancel_response()
```

`cancel_response()` 方法存在（`client.rs:393`），但**生产代码里零调用**，
只有测试用过（`client.rs:819`）。

**后果**：
1. 云端继续生成，浪费 token
2. 可能「刚停又接着说」（缓存的音频继续播）

**修法**：`stop_now()` 之后紧跟 `client.cancel_response()`。

## 二、多 agent 方案（采信 subagent 结论，我认同）

### 关键决策：加第 5 个工具 `set_context`

**为什么不是塞进 delegate_task 参数**：
「切换」是改持久状态，「派活」是执行一次动作。语义不同。
用户说「换到蝶梦项目」意思是「接下来都在这」，不是「用这个目录干一次」。

**为什么不是靠 ASR 关键词匹配**：
「切换到蝶梦项目」和「把蝶梦项目那个 bug 修了」都会命中「蝶梦项目」，必然误触发。

```rust
set_context(
  agent?: "dsh" | "codex",
  dir?:   string,           // 只接受别名，宿主查表
  model?: string,
)
```

**同时调整不变量 I1 的表述**：
从「恰好 4 个工具」改成「**名字白名单 + 禁用名黑名单**」。
`bash`/`read_file`/`write_file`/`run_command`/`ls`/`edit` 永远进不来。
语义更强，且以后加工具不被数字卡住。

### Codex 接入：几乎零代码 ✅

**关键发现**：`codex` CLI 没有 ACP，但官方有适配器
```
npx -y @zed-industries/codex-acp
```
实测 initialize 返回 `agentInfo.name="codex-acp"`。

**所以协议层完全复用现有 `acp.rs`**，只需：
1. 把 `DshClient` 泛化成「认任何 ACP agent」
2. 配置里加 `[agents.codex] command = ["npx", "-y", "@zed-industries/codex-acp"]`

### 项目别名表（避免 ASR 听错路径）

```toml
# ~/.diemeng/projects.toml
[[project]]
alias = ["蝶梦", "diemeng", "morphofly", "蝴蝶"]
path  = "/home/wings/Code_Project/morphofly"
```

`set_context(dir=...)` 只接受别名，宿主查表。
匹配前归一化：去空格、小写、全角转半角。

## 三、极简回复：真正的旋钮不是提示词

**关键洞察**：
- `DEFAULT_INSTRUCTIONS` 第 3 条只有「说话简短口语化」—— 纯定性，无数字
- 给了硬数字的是 `clean_for_speech`（任务汇报），但那只作用于**任务结果**
- **语音模型自己说的话完全没有约束**

**修法（按可靠性排序）**：
1. **压 `max_response_output_tokens`**（物理约束，最可靠）
   默认 4096，压到 ~120
2. 提示词给明确字数（已做：不超过 15 字）
3. `clean_for_speech` 补三处：
   - 裸路径没删（现有测试的路径恰好包在反引号里才被删）
   - 没有句子数控制（注释说 2 句，实现按字符）
   - 数字/符号没归一（`200ms`、`v0.1.0` 念出来是灾难）

## 四、桌面体验：Wayland 是硬坑

**用户环境是 KDE Wayland**（wings-pc 实测 `Type=wayland`，`kwin_wayland 6.8.0`）。

**Wayland 下 `set_position()` 被合成器忽略** —— 没有全局坐标概念。

三条路：
| 方案 | 可行性 |
|---|---|
| **A. 强制 XWayland**（`GDK_BACKEND=x11`） | ✅ 推荐，零新依赖，wings-pc 有 Xwayland |
| B. gtk-layer-shell | ❌ wings-pc 没装该库，deb 要加依赖 |
| C. 放弃贴边 | 只记上次位置 |

**Linux 托盘的限制**：
`libappindicator` 后端**不发点击事件**，只有菜单。
所以「点托盘显示小球」在 Linux 只能做成菜单项。
Windows/macOS 可以左键切换。

**启动不干扰，拆成 4 条可判定项**：
| # | 含义 | 实现 |
|---|---|---|
| 1 | 不抢焦点 | `"focus": false`（默认 true！） |
| 2 | 不遮挡 | 待机 `alwaysOnTop=false`（现在 true） |
| 3 | 不弹窗 | 无 dialog |
| 4 | 不白闪 | `"visible": false` → 算好坐标再 show |

**注意**：现在 config 里 `visible` 没设（默认 true），
所以 `main.rs` 里那句 `win.show()` 其实是空操作——窗口早就显示了。

## 五、优先级

1. **修 5 个缺陷**（尤其 #1 #2 是欺骗用户，#3 是多 agent 地基）
2. 加 `set_context` + 别名表
3. Codex 接入（配置级，几乎零代码）
4. 极简回复（先压 token 上限）
5. 桌面体验（Wayland 用 XWayland 兜底）
