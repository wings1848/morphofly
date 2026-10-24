# 第 2 轮验证（S6 修到绿）

**日期**：2026-10-25 00:25
**触发**：用户新要求「确保兼容 windows 平台」+「不要用 sleep 阻塞」

## 这一轮修的 5 个问题（都来自真机/真 CI）

| # | 现象 | 真因 | 证据 |
|---|---|---|---|
| 1 | 右下角歪 212px | 启动时字幕抢在贴边前调 `orb_tip`，归位点拿屏幕正中当基准 | 日志 `小球已贴边 x=2696`（应为 2908）+ `want=1249 got=2696` |
| 2 | 模型只回「好，查着呢」不派活、且不纠正 | 纠正判据只认云端 `speech_started`；本地 VAD 提前 commit 时那条不来 | 日志有 `本地判定说完 → commit` 但无 `WARN 这一轮没有调用任何工具` |
| 3 | 项目名被 ASR 听错就放弃 | `delegate` 里 dir 不存在只追问，没先查别名表 | 日志 `dir=/home/wings/Code_Project/move_forward` |
| 4 | **Windows 上仓库检不出来** | 根目录被跟踪的空文件 `...`（Windows 文件名不能以点结尾） | CI `error: invalid path '...'` + `git.exe exit 128` |
| 5 | 仓库 423MB | `ci-dist/` 把 AppImage + squashfs-root 提交了（400 文件，单个 libwebkit2gtk 86MB） | `git ls-files | grep -c ci-dist` = 400 |

## 修完的实测

### 位置（真机）
```
pos: 实测 window=0x120000d 位置=(2908,1516) 尺寸=148x148 → 右距=16px 底距=64px
pos: PASS
```
日志顺序也对：`贴边 x=2908` → `气泡 show=true target_x=2696` → `show=false target_x=2908`。

### 纠正机制（真机）
```
00:01:41 本地判定开口
00:01:45 本地判定说完 → commit
00:01:45 云端没发 SpeechStarted，按本地 VAD 判定算用户开口   ← 新
00:01:46 WARN 这一轮没有调用任何工具 —— 模型只说了话没干活 reply=好，查着呢  ← 新
00:01:46 已注入纠正提醒，等它重新响应
```
**纠正现在会触发**（之前完全静默）。但纠正后模型仍然没调工具 →
已加「把当前项目 dir 直接写进提醒」的改动，待下一轮验证。

### Windows（CI）
```
步骤 2 actions/checkout      : success   ← 修好 `...` 之后才可能
步骤 5 cargo check core      : success
步骤 6 core 单元测试          : （见 CI 页面）
步骤 7 app 外壳 check        : （见 CI 页面）
```

## 诚实交代

- Windows 只验证到「编译 + 单测」，**运行时没验**（没有 Windows 机器）
- 中文安装路径 + sherpa-onnx 加载模型是最大未知，已写进 docs/v2/windows.md
- 修正后「模型仍不派活」这条没彻底解决，只加了一个更具体的提醒；
  真解法要靠「拿到用户原话 + 本地确定性派活」，那是下一轮的事
