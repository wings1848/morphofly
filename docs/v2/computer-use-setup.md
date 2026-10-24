# Computer Use 接入（2026-10-24）

## 为什么需要它

AEC 真机验证需要**真实 GUI 操作 + 真实麦克风**：
- 启动 App
- 喊「小蝶」唤醒
- 让它说话，同时喊「停」
- 观察是否自我打断

这些用脚本驱动不了（声音要从扬声器出来、被麦克风收回去），
必须操作真实桌面。

## 环境约束

| 设备 | 桌面 | 麦克风 | 能做 AEC |
|---|---|---|---|
| wings-tx-us1（本机，云主机）| ❌ 无 | ❌ | ❌ |
| wings-pc | ⚠️ 桌面未登录 | ❌ 无硬件 | ❌ |
| **wings-gpd** | ✅ Plasma Wayland | ✅ 有 | ✅ |

所以：**MCP 进程跑在 gpd 上，本机用 SSH stdio 桥接。**

## 安装（在 gpd 上）

```bash
export PATH=$HOME/.bun/bin:$HOME/.local/share/fnm/aliases/default/bin:$PATH
npm install -g --allow-scripts=@agent-sh/computer-use-linux @agent-sh/computer-use-linux
```

⚠️ **必须加 `--allow-scripts`** —— 否则 postinstall 不跑，
二进制下载不下来，命令找不到。

## 关键环境变量

```bash
export XDG_RUNTIME_DIR=/run/user/1000
export DISPLAY=:0
export XAUTHORITY=/run/user/1000/xauth_YRAalP   # ← 每次登录会变，要用 ls 查
```

`XAUTHORITY` 文件名是随机的（`xauth_XXXXXX`），
用 `ls /run/user/1000/xauth_*` 查当前值。

## 启用 AT-SPI

```bash
computer-use-linux setup
```

**必须做**，否则 `can_build_accessibility_tree: false`，
拿不到语义树，只能靠截图坐标点。

做完后 doctor 显示：
```
can_build_accessibility_tree: True
can_query_windows: True
can_focus_windows: True
can_send_development_input: True
can_capture_screenshots: False   ← 这个仍是 false，用 spectacle 替代
blockers: 无
```

## 注册到 DSH

在 `~/.dsh/profiles/web/cordis.patch.yml` 的 `mcp-lazy` 里加：

```yaml
      - serverName: computer-use-linux
        transport: stdio
        command: ssh
        args:
          - '-o'
          - 'BatchMode=yes'
          - '-o'
          - 'ConnectTimeout=15'
          - 'wings@100.107.9.102'
          - 'export PATH=$HOME/.bun/bin:$HOME/.local/share/fnm/aliases/default/bin:$PATH && export XDG_RUNTIME_DIR=/run/user/1000 DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_YRAalP && computer-use-linux mcp'
        lifecycle: lazy
```

验证：
```bash
dsh --profile web --dump-config | grep -A 3 computer-use-linux
```

## 验证 MCP 完整可用

结果（2026-10-24 实测）：
```
initialize: OK
tools (18): activate_window, click, doctor, drag, focused_window,
            get_app_state, list_apps, list_windows, move_window, perform_action, ...
list_windows: 1 个窗口
  - 蝶梦语音智能体开发 — DeepSeek Harness - Google Chrome
```

## 可用的 18 个工具

| 类别 | 工具 |
|---|---|
| 诊断 | doctor, setup_accessibility, setup_window_targeting |
| 观察 | get_app_state, screenshot, list_windows, focused_window, list_apps |
| 输入 | click, drag, scroll, press_key, type_text |
| 语义动作 | perform_action, set_value |
| 窗口管理 | activate_window, move_window, resize_window |

## 截图替代方案

`can_capture_screenshots: false`（portal 需要授权，SSH 会话拿不到）。

用 `spectacle` 替代：
```bash
export XAUTHORITY=/run/user/1000/xauth_YRAalP
spectacle -b -n -f -o /tmp/screen.png
```
实测能拿到 4096×3904 的 PNG。

## 这是本轮最重要的基础设施突破

**之前所有 GUI 相关的验证都做不了**：
- 拖动贴边
- 托盘菜单
- AEC 自我打断
- 完整语音流程

现在都能做了。
