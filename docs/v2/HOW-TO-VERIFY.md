# 怎么验证蝶梦（操作手册）

**给后续接手的 agent 或人看。**

## 环境分工

| 设备 | 地址 | 用途 | 限制 |
|---|---|---|---|
| 本机（云主机）| - | 开发、单测、CI | 无桌面、无音频 |
| wings-pc | `wings@100.110.222.0` | GUI 验证 | **无麦克风硬件** |
| **wings-gpd** | `wings@100.107.9.102` | **真机全流程** | N6000 慢，编译 5 分钟 |

用 `tools/pc_ssh.sh` / `tools/gpd_ssh.sh` 拿目标地址（有兜底逻辑）。

## 一、本地验证（最快，不依赖设备）

```bash
cd ~/Code_Project/morphofly
source tools/dev-env.sh    # 本机必需（无 root，ALSA 解包在 ~/.local/sysroot）

# 单元测试（232 个）
cargo test -p morphofly-core --lib

# 含唤醒词的测试（243 个）
cargo test -p morphofly-core --features kws --lib

# 前端（8 个）
node --test 'app/frontend/test/*.test.mjs'

# e2e（mock 模式，不需要网络）
python3 tools/e2e/run.py --mode mock

# 格式与 lint
cargo fmt --all --check
cargo clippy -p morphofly-core --features kws --lib --tests
```

**本机踩坑**：`--features kws` 需要 `libstdc++.so`（不是 `.so.6`）。
已建软链接：`~/.local/sysroot/usr/lib/x86_64-linux-gnu/libstdc++.so`。

## 二、真机语音链路 e2e（本机能跑，需要网络 + key）

```bash
source tools/dev-env.sh
set -a; . ~/.diemeng/credentials; set +a

# 单条用例
python3 tools/e2e/run.py --mode real --case wake-01

# 全部 14 条
python3 tools/e2e/run.py --mode real
```

它会：
1. 用 edge-tts 合成语音 → miniaudio 解成裸 PCM
2. 调 `core/examples/e2e_drive.rs`（带 `--features kws`，走真实 KWS 前置门）
3. 比对期望，输出 `/tmp/e2e-report.json`

**期望：13/14 通过**（唯一失败是 edge-02「小蝶帮我」KWS 漏检）。

## 三、gpd 真机部署

### 3.1 拿 CI 编译的包（推荐）

```bash
# 触发构建
gh workflow run release.yml

# 等完成
gh run list --workflow=release.yml --limit 1

# 下载产物
gh run download <run-id> -D /tmp/gh-dist

# 传到 gpd
T=$(tools/gpd_ssh.sh)
scp /tmp/gh-dist/morphofly-linux-x86_64/deb/蝶梦_0.1.0_amd64.deb "$T:/tmp/morphofly.deb"

# 解包（gpd 没有 dpkg）
ssh "$T" 'cd /tmp && rm -rf mf-install && mkdir mf-install && cd mf-install && \
  ar x /tmp/morphofly.deb && tar xf data.tar.*'
```

**为什么不用 AppImage**：AppImage 会打包编译机的系统库
（含 AVX 指令），在 N6000（无 AVX）上会 SIGILL。deb 不打包系统库。

### 3.2 启动（关键：不能用 nohup）

```bash
# ❌ 错误：nohup setsid 在 SSH 会话退出时会被杀，App 启动后立刻消失（日志全空）
# ✅ 正确：systemd user service

ssh "$T" '
XAUTH=$(ls /run/user/1000/ | grep xauth | head -1)
mkdir -p ~/.config/systemd/user
cat > ~/.config/systemd/user/morphofly-test.service <<EOF
[Unit]
Description=MorphoFly (test)

[Service]
Type=simple
Environment=XDG_RUNTIME_DIR=/run/user/1000
Environment=DISPLAY=:0
Environment=XAUTHORITY=/run/user/1000/$XAUTH
Environment=MORPHOFLY_BUNDLE_DIR=/tmp/mf-install/usr/lib/蝶梦/resources
ExecStart=/tmp/mf-install/usr/bin/morphofly-app
Restart=no
EOF
systemctl --user daemon-reload
systemctl --user start morphofly-test
'
```

### 3.3 麦克风配置（必做，否则 KWS 起不来）

gpd 上 ALSA→PipeWire 的录音流有 bug，必须显式指定设备：

```bash
ssh "$T" 'cat > ~/.asoundrc <<EOF
pcm.!default {
    type pulse
    device "alsa_input.pci-0000_00_1f.3-platform-sof-essx8336.stereo-fallback"
}
ctl.!default { type pulse }
EOF'
```

**最近的修法**：代码里已改 cpal 优先用 PulseAudio 后端（commit `e64c936`），
理论上不再依赖 `.asoundrc`。两者都保留更保险。

### 3.4 确认成功

```bash
ssh "$T" 'journalctl --user -u morphofly-test -n 20 --no-pager | \
  grep -E "托盘|贴边|AEC|KWS"'
```

期望看到：
```
系统托盘已就绪（左键显示/隐藏，右键菜单）
小球已贴边 x=3344 y=1456 right=true
AEC 已启用（sonora / AEC3） stream_delay_ms=0
KWS 常驻监听已启动
```

**这四行同时出现 = 蝶梦完整跑起来了。**

## 四、用 Computer Use 操作 gpd 桌面

已接入 `computer-use-linux`，配置在 `~/.dsh/profiles/web/cordis.patch.yml`。

```python
# 通过 SSH 起 MCP，然后 JSON-RPC 调用
CMD = ("export PATH=$HOME/.bun/bin:$HOME/.local/share/fnm/aliases/default/bin:$PATH && "
       "export XDG_RUNTIME_DIR=/run/user/1000 DISPLAY=:0 XAUTHORITY=/run/user/1000/xauth_XXX && "
       "computer-use-linux mcp")
```

18 个工具，常用的：
- `list_windows` — 列窗口（KWin 后端）
- `press_key` — 参数是 `{"key": "Ctrl+Alt+t"}`（**不是 `keys`**）
- `type_text` — `{"text": "...", "app_id": "org.kde.konsole"}`
- `screenshot` — 返回 base64 image
- `activate_window` — 切窗口

**踩坑**：
- `press_key` 的参数名是 `key`，写 `keys` 会报 `missing field`
- 截图分辨率会缩放，要按 `scale` 换算坐标
- 双屏时先确认哪块屏（gpd 是 3072×1728 + 1920×1200）

## 五、AEC 真机验证（最后一步）

需要真实人声。两条路：

### 路线 A：人站在 gpd 前
1. 启动 App（3.2）
2. 喊「小蝶」→ 看日志 `唤醒命中`
3. 让它说话，观察是否自我打断
4. 说话时喊「停」→ 应在 300ms 内停播

### 路线 B：注入真实 TTS 音频
```bash
# 生成「小蝶」音频（本机）
uv run --with edge-tts edge-tts --voice zh-CN-XiaoxiaoNeural \
  --text "小蝶" --write-media /tmp/xiaodie.mp3
uv run --with miniaudio python -c "
import miniaudio, wave
d = miniaudio.decode_file('/tmp/xiaodie.mp3',
    output_format=miniaudio.SampleFormat.SIGNED16, nchannels=1, sample_rate=16000)
w = wave.open('/tmp/xiaodie.wav','wb'); w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
w.writeframes(bytes(d.samples)); w.close()"

# 本地先验 KWS 能识别
MORPHOFLY_BUNDLE_DIR=$PWD/app/src-tauri/resources \
  cargo run -q -p morphofly-core --features kws --example kws_file_probe2 -- /tmp/xiaodie.wav
# 期望：命中 1 处: [(1.1, "小蝶")]

# 传 gpd 并播放（会被麦克风收到）
scp /tmp/xiaodie.wav "$T:/tmp/"
ssh "$T" 'export XDG_RUNTIME_DIR=/run/user/1000; paplay /tmp/xiaodie.wav'

# 看是否唤醒
ssh "$T" 'journalctl --user -u morphofly-test -n 10 --no-pager | grep -E "唤醒|S1|S2"'
```

**注意**：内置扬声器音量小，麦克风可能收不到。
先把音量调到 100%，或接外接音箱。

## 六、常见问题

| 症状 | 原因 | 解决 |
|---|---|---|
| `KWS 需要麦克风` | ALSA 桥接失败 | 见 3.3 麦克风配置，或确认 cpal PulseAudio 后端生效 |
| App 启动后日志全空、进程消失 | nohup 被杀 | 用 systemd service（3.2）|
| `PulseAudio: Unable to create stream: No such entity` | ALSA→PipeWire 录音流 bug | 同上 |
| 窗口跑到屏幕外 | 贴边坐标是按主屏算的 | 查 `小球已贴边 x=? y=?`，确认在哪块屏 |
| 双屏截图对不上坐标 | 截图缩放 + 多屏 | 先 `xrandr` 确认布局 |
| CI 产物在 gpd SIGILL | AppImage 打包了 AVX 库 | 用 deb |
