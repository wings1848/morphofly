#!/usr/bin/env python3
"""从 wings-gpd 抓真实桌面截图（走 computer-use-linux 的 MCP 协议）。

## 为什么需要这个脚本

会话网关只转发文本，MCP 的 `screenshot` 工具返回的图片到不了本机；
`computer-use-linux screenshot` 这个 CLI 又只打印元数据、不落盘。
但 MCP 服务器本身是 stdio JSON-RPC —— 直接跟它说话就能把 base64 图取下来。

## 用法

    python3 tools/gpd_shot.py /tmp/gpd-desktop.png
    python3 tools/gpd_shot.py /tmp/orb.png --crop 2900,1450,172,172

--crop x,y,w,h 只截桌面的一块（省传输、省内存）。

## 事实依据

见 docs/v2/float-window.md「踩过的坑」一节：
- `ffmpeg -f x11grab` 抓不到被合成的窗口（只有鼠标指针）
- MCP screenshot 走 xdg-desktop-portal，能拿到真桌面
"""
import argparse
import base64
import json
import os
import subprocess
import sys

GPD = os.environ.get("GPD", "wings@100.107.9.102")
REMOTE_ENV = (
    "XDG_RUNTIME_DIR=/run/user/1000 "
    "DISPLAY=:0 "
    "XAUTHORITY=/run/user/1000/xauth_YRAalP "
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus"
)


class Mcp:
    def __init__(self, cmd):
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self.next_id = 1

    def send(self, method, params=None):
        rid = self.next_id
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()
        # 跳过服务器发来的通知（没有 id），只等我们的响应
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError("MCP 服务器断了")
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if obj.get("id") == rid:
                return obj

    def notify(self, method, params=None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def close(self):
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        self.proc.terminate()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out", help="输出 png 路径")
    ap.add_argument("--crop", help="x,y,w,h，只截一块")
    ap.add_argument("--format", default="png", choices=["png", "jpeg"])
    args = ap.parse_args()

    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", GPD,
           f"{REMOTE_ENV} computer-use-linux mcp"]
    m = Mcp(cmd)
    try:
        m.send("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "gpd-shot", "version": "0"},
        })
        m.notify("notifications/initialized")

        call_args = {"format": args.format, "max_bytes": 2000000}
        if args.crop:
            # 工具没有原生 crop，用 window 目标外的方式：先全屏高质量截，再本地裁
            call_args["full_screen"] = True
        res = m.send("tools/call", {"name": "screenshot", "arguments": call_args})
    finally:
        m.close()

    content = (res.get("result") or {}).get("content") or []
    for part in content:
        if part.get("type") == "image":
            raw = base64.b64decode(part["data"])
            with open(args.out, "wb") as f:
                f.write(raw)
            print(f"已保存 {args.out}（{len(raw)} 字节）")
            if args.crop:
                x, y, w, h = (int(v) for v in args.crop.split(","))
                # 客户端坐标（screenshot 默认会缩到 1920 宽，这里换算一下）
                meta = {}
                for p2 in content:
                    if p2.get("type") == "text":
                        try:
                            meta = json.loads(p2["text"])
                        except Exception:
                            pass
                cw = meta.get("coordinate_width") or w
                scale = cw / 1920.0 if cw else 1.0
                subprocess.run(
                    ["magick", args.out, "-crop",
                     f"{int(w / scale)}x{int(h / scale)}+{int(x / scale)}+{int(y / scale)}",
                     "+repage", args.out],
                    check=False,
                )
            return 0
    print("没拿到图片，返回：", json.dumps(res)[:400], file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
