# 方案决策（S2）

## 决策 1：Wayland 会话下强制 GTK 走 XWayland（GDK_BACKEND=x11）

- **状态**：accepted
- **背景**：见 facts.md F1/F2/F3 —— 纯 Wayland 下 `set_position` 和 `alwaysOnTop` 都不生效，
  而用户明确要求「默认置顶 + 启动在右下角」。
- **被否方案 B**：接 `gtk-layer-shell` / `tauri-plugin-decorum`。
  否掉原因：要引入新的原生依赖，且 wlr-layer-shell 在 KWin 上行为不完整，
  为了两个需求重写整套窗口逻辑，代价远大于收益。
- **被否方案 C**：接受 Wayland 限制，只做视觉不做定位/置顶。
  否掉原因：直接违背用户三句需求里的两句。
- **代价**：走 XWayland 后客户端是 X11 的，缩放手感/输入法可能有差异 → 标人工核对（A7）。

## 决策 2：字幕用「单窗口按需撑开」，不开第二个窗口

- **状态**：accepted
- **背景**：小球窗口只有 148×148，放不下字幕。
- **被否方案 B**：独立气泡窗口。否掉原因：多一个 WebView 约多 30–50MB 内存，
  而本项目 PSS 已经 371MB，内存是老问题。
- **做法**：`orb_tip(show)` 把窗口向左撑到 360 宽并让 x 同步减小，右边缘钉住 →
  屏幕上看球不动、左边冒气泡。前端把画布钉在右下角配合。

## 决策 3：视觉迭代用无头 Chromium，不靠重新编译

- **状态**：accepted
- **理由**：facts.md F8（秒级出图）对 F9（一次 release 编译 6 分钟）。
- **代价**：无头渲染没有窗口透明合成、没有真实桌面背景，所以「好不好看」的最终判断
  仍要真机截图 + 用户拍板。这一点必须诚实写进交付单。

## 本次必须保持不变的不变量

| # | 不变量 | 类别 | 怎么核 |
|---|---|---|---|
| I1 | 语音链路不回归：喊小蝶 → 派活 1 次 → 汇报 1 次 | 测试 | `tools/gpd_voice_e2e.sh` |
| I2 | 前端永远不碰麦克风（无 `getUserMedia`、不引入 npm 依赖） | 测试 | `grep -r getUserMedia app/frontend/` 必须为空 |
| I3 | 语音模型工具仍是 5 个，黑名单词汇不得出现 | 测试 | `cargo test -p morphofly-core --lib tools::` |
| I4 | 改动不碰 AEC / KWS / DSH 派活逻辑 | 人工核对 | `git diff --stat` 只含前端+窗口层 |
