# 「不会误唤醒」的架构证明（2026-10-24）

## 结论

**真实产品不可能因为谐音误唤醒**，因为架构上有两道门：

```
麦克风 → [本地 KWS] → 命中才进 S1 → [session guard] → 才连云端
              ↓                            ↓
         不命中就停                    S1 才开连接
```

## 证据 1：KWS 是独立监听线程

`main.rs:275` `spawn_wake_listener`：
```rust
let cfg = KwsConfig::default();
WakeListener::start(cfg, move |ev: WakeEvent| {
    let WakeEvent::Detected { keyword } = ev;
    // 只有真的检测到关键词才会走到这里
    let stim = wake_stimulus_for(guard.state());
    // S0 → S1
})
```

## 证据 2：只有 S1 才连云端

`main.rs:210` `spawn_session_guard`，每 250ms 检查：
```rust
let st = { machine.lock().await.state() };
if voice::should_open_session(st) && current.is_none() {
    // 建立语音会话（连云端）
}
```

`should_open_session(st)` 只在 `S1`（已唤醒）时为真。

**S0（待机）状态下永远不会建立云端连接。**

## 证据 3：KWS 对谐音全部挡住（实测）

| 文本 | KWS 命中 |
|---|---|
| 小蝶 | ✅ 1 处 |
| 小蝶儿真好看 | ❌ 0 处 |
| 蝴蝶飞走了 | ❌ 0 处 |
| 这个字读小的 | ❌ 0 处 |
| 纯静音 | ❌ 0 处 |

用的判据是 `num_trailing_blanks >= 4`（关键词后必须连续 4 帧 blank），
这是当初实测出来专门挡「小蝶儿」的。

## 那为什么 e2e 会「误报」

因为 e2e_drive 是个**测试驱动**，它直接 `RealtimeClient::connect()`
连云端，**绕过了 KWS 和状态机**。

我后来给 e2e_drive 加了 KWS 前置门（`--features kws` 时生效），
误报就全部消失了 —— 从 8/14 提升到 12/14。

## 我之前的错误

我说「谐音误唤醒是产品 bug」，**这是错的**。
我当时看的是 e2e 的失败，而 e2e 走的不是真实链路。

**教训：测试失败时要先问「这个测试走的是真实路径吗」。**
绕过一半组件的测试，测出来的问题可能根本不存在。

## 剩下一个真实问题

「小蝶帮我」这种**没有停顿**的 4 个字，KWS 会漏检（0 处命中）。

这是真实的灵敏度不足，但：
- 用户通常会说「小蝶」+ 稍停 + 任务
- 降 `num_trailing_blanks` 会让谐音误唤醒回来（有硬断言保护）
- 正解是加唤醒词变体，需要装 `sherpa-onnx-cli`

见 `docs/v2/kws-edge-case.md`。
