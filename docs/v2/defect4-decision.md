# 缺陷 #4（常驻进程池）为什么先不做

## 收益
每任务省 2.8 秒启动开销（initialize 1.35s + session/new 1.45s）。

## 改动范围（比我预想的大）

`DshClient` 现在是**单 session 设计**：
```rust
session_id: Arc<Mutex<Option<String>>>,   // 只有一个槽
```

要做常驻池，必须改成多 session 路由：
1. `session_id: Arc<Mutex<HashSet<String>>>` 或者 map
2. `handle_message` 按 sessionId 分发到不同的 broadcast 通道
3. `prompt()` / `cancel()` 要带 sessionId 参数
4. `DshBridge` 要管理进程生命周期（复用/健康检查/重启）
5. 任务完成后的 session 清理（否则内存泄漏）

**估计 300-400 行改动，涉及 3 个文件的接口变更。**

## 为什么现在不做

1. **改坏了发现不了** —— 这是并发相关改动，
   必须在有 GUI 的环境跑多任务并发才能验证
2. **当前桌面不可用**（wings-pc 卡登录界面，wings-gpd 离线）
3. **收益是「快 2.8 秒」，不是「能不能用」**
   —— 用户说的核心诉求是「让蝶梦正常可用」，
   编造结果、AEC、多 agent 这些才是关键路径

## 什么时候做

等真机可测试之后。届时可以先做「预热一个常驻进程」的简单版本：
- 只维护 1 个「空闲 client」，任务来了直接用
- 任务完成后把 client 放回池子（或销毁重建）
- 不做多 session 并发，只做「避免每次都 initialize」

这个简化版能省掉 1.35s 的 initialize，改动只有 ~50 行，
而且不涉及 session 路由，风险低得多。

## 结论

**先做简化版（预热单 client），不做完整的多 session 池。**
完整版等有真机验证条件时再说。
