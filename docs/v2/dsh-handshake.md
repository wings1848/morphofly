# DSH 握手为什么要 15 秒（以及它怎么把蝶梦搞挂）

**2026-10-25**

## 现象

蝶梦派活后经常报：

```
失败了：出错了：建 DSH 会话失败: DSH 握手超时（30s 没回 initialize 结果）
```

稳定性测试连跑 3 次，0 通过 —— 而且每次都在这一步挂。

## 实测

手写探针（`/tmp/dsh_probe.py`，直连 `dsh --profile acp` 走 JSON-RPC）：

```
initialize : 15.29s / 14.75s / 15.51s     ← 稳定 15 秒
session/new:  1.49s
```

**initialize 稳定要 15 秒**，不是偶发。

## 为什么这会把蝶梦搞挂

原来的代码这么写的：

```rust
// 等一下握手回复（最多 2 秒）
for _ in 0..100 { if initialized { break } sleep(20ms) }
Ok(())   // ← 2 秒没等到也往下走，不报错
```

于是：
1. 2 秒后就去发 `session/new`
2. 服务端还在握手，`session/new` 排队 → 原来只等 20 秒 → 超时
3. 用户听到「失败了：建 DSH 会话失败」

## 已改（commit `d28b5a3`）

- `initialize()`：**真等到结果**，最多 30 秒；等不到明确报「DSH 握手超时」
- `session/new`：20s → 50s

## 还没解决的部分（下一轮）

真机上仍然会 30 秒超时，说明**应用环境下握手比 15 秒更慢**。已经给服务开了
`RUST_LOG=...,dsh=debug`（stderr 现在会带 dsh 自己的输出），下一步看它卡在哪。

怀疑方向：`--profile acp` 加载的插件太多（`@deepseek-ai/dsh-plugin-package-inventory-deepseek`
会扫 node_modules，`dsh-hmr` 会 watch 文件），机器又是 N6000 + 同时在跑 Chrome。

对策候选：
1. 给蝶梦单独做一个精简 acp profile（少加载几个插件）
2. 进程池复用（`end_turn` 才回收 —— 但握手失败时永远进不了池，成死循环）
3. 后台预热：app 启动时就先握手一个 dsh 备着
