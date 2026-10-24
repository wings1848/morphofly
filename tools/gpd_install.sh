#!/usr/bin/env bash
# 在任意 Linux 机器上安装 MorphoFly（不需要 sudo、不需要 root）
#
# 为什么用 deb 而不是 AppImage：
#   AppImage 会把**编译机的系统库**一起打包（libglib/libcrypto 等）。
#   如果编译机是 x86-64-v3（含 AVX）而目标机只有 SSE4.2，
#   一跑就 SIGILL 崩溃 —— wings-gpd 实测踩过这个问题，详见 docs/跨机传文件.md
#
#   deb 只包含我们的二进制，依赖系统库，天然匹配目标机 CPU。
#
# 用法（在目标机上）：
#   ./gpd_install.sh <deb 文件路径>
#   ./gpd_install.sh morphofly.deb
set -uo pipefail

DEB="${1:-}"
[ -n "$DEB" ] || { echo "用法: $0 <deb 文件>"; exit 1; }
[ -f "$DEB" ] || { echo "文件不存在: $DEB"; exit 1; }

DEST="$HOME/.local/opt/morphofly"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "=== 解包 $DEB ==="
cd "$TMP"
cp "$DEB" pkg.deb 2>/dev/null || { echo "复制失败"; exit 1; }

# 不同发行版工具不同：Arch 系没有 dpkg-deb，用 ar + tar
if command -v dpkg-deb >/dev/null 2>&1; then
    dpkg-deb -x pkg.deb .
else
    ar x pkg.deb || { echo "解包失败（缺 ar）"; exit 1; }
    for f in data.tar.*; do tar -xf "$f" 2>/dev/null; done
fi

[ -d usr/bin ] || { echo "解包后没有 usr/bin，包可能损坏"; exit 1; }

echo "=== 安装到 $DEST ==="
rm -rf "$DEST" && mkdir -p "$DEST"
cp -r usr/* "$DEST/"
chmod +x "$DEST/bin/morphofly-app" 2>/dev/null

# 找资源目录（Tauri 放在 usr/lib/<productName>/resources）
RES=$(find "$DEST" -type d -name resources | head -1)
echo "资源目录: ${RES:-未找到}"

echo "=== 生成启动脚本 ==="
cat > "$HOME/启动蝶梦.sh" <<LAUNCH
#!/usr/bin/env bash
# 蝶梦启动脚本（由 gpd_install.sh 生成）
export GDK_BACKEND=x11
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export LIBGL_ALWAYS_SOFTWARE=1
export MORPHOFLY_BUNDLE_DIR="$RES"
exec "$DEST/bin/morphofly-app" "\$@"
LAUNCH
chmod +x "$HOME/启动蝶梦.sh"

echo "=== 完成 ==="
echo "启动方式：~/启动蝶梦.sh"
echo
echo "首次使用需要配置密钥："
echo "  mkdir -p ~/.diemeng"
echo "  echo 'STEPFUN_KEY=你的key' > ~/.diemeng/credentials"
echo "  chmod 600 ~/.diemeng/credentials"
