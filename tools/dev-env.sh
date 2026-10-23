#!/usr/bin/env bash
# 本机（US 云主机）编译 Rust core 所需的环境。
# 本机没有 root，alsa 开发包被解包到 ~/.local/sysroot（见 docs/本机环境.md）。
# 有 sudo 的机器不需要这个脚本。
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:+$PKG_CONFIG_PATH:}$HOME/.local/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig"
export PKG_CONFIG="$HOME/.local/sysroot/usr/bin/pkg-config"
export LIBRARY_PATH="${LIBRARY_PATH:+$LIBRARY_PATH:}$HOME/.local/sysroot/usr/lib/x86_64-linux-gnu"
export C_INCLUDE_PATH="${C_INCLUDE_PATH:+$C_INCLUDE_PATH:}$HOME/.local/sysroot/usr/include"
export CPATH="${CPATH:+$CPATH:}$HOME/.local/sysroot/usr/include"
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}$HOME/.local/sysroot/usr/lib/x86_64-linux-gnu:$HOME/.local/sysroot/usr/lib/x86_64-linux-gnu/pkgconf"
export PATH="$HOME/.local/sysroot/usr/bin:$HOME/.cargo/bin:$PATH"
