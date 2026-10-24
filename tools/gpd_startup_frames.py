#!/usr/bin/env python3
"""启动瞬间的帧采集 + 分析（在 wings-gpd 上用 KWin ScreenShot2 D-Bus 跑）。

## 为什么不用 spectacle / portal

`spectacle -b -n -f -o` 实测一次 10–15 秒（走的是 xdg-desktop-portal），
启动气泡只存在 4.2 秒，用它能拍到的只有「已经恢复正常」的画面。
KWin 自己的 ScreenShot2 D-Bus 接口一次约 40–90ms，可以 8–10fps 连拍。

## 用法

    # 在 gpd 上连拍 13 秒（帧存到 /tmp/xxx，只存右下角 400x300 的小图，省传输）
    python3 gpd_startup_frames.py record /tmp/mf-startup 13 8

    # 分析：启动中（t_early）与稳定后（t_late）在「气泡区域」的差异
    python3 gpd_startup_frames.py analyze /tmp/mf-startup 3.0 12.0

## 为什么用「差分」而不是直接找黑框

黑框/气泡的位置和底色会随主题、壁纸、窗口边框变，直接按颜色阈值找很容易「假通过」。
差分法更稳：同一个位置，启动中有东西（气泡）和稳定后没东西，像素差会明显 > 0。
球本身的呼吸动画不在这块区域（区域取在球左侧），所以正常应该几乎无差异。

## 判定区域怎么来的

球是画面上最亮的东西（青色发光），先在右下角找它的亮度和位置，
再按「球半径 r」推出它左边的气泡区域（展开窗口向左多出的部分）。
这样换分辨率/缩放也不用改常数。
"""
import json
import os
import sys
import threading
import time

try:
    import dbus
    from PIL import Image, ImageChops
except ImportError as e:  # pragma: no cover
    sys.exit(f"缺依赖：{e}（gpd 上 python3-dbus + python3-pil 已有）")

# 右下角裁剪尺寸（截图逻辑像素）。400x300 足够装下球 + 左侧气泡，单帧约 20KB。
CROP_W = 400
CROP_H = 300


def make_capture():
    """返回一个每次调用抓一帧（RGB PIL.Image）的函数。"""
    bus = dbus.SessionBus()
    obj = bus.get_object("org.kde.KWin", "/org/kde/KWin/ScreenShot2")
    iface = dbus.Interface(obj, "org.kde.KWin.ScreenShot2")

    def grab():
        r, w = os.pipe()
        buf = []
        state = {"need": 1 << 62}

        def reader():
            while True:
                c = os.read(r, 1 << 20)
                if not c:
                    break
                buf.append(c)
                if sum(len(x) for x in buf) >= state["need"]:
                    break

        th = threading.Thread(target=reader, daemon=True)
        th.start()
        fd = dbus.types.UnixFd(w)
        opts = dbus.Dictionary({"include-cursor": dbus.Boolean(False)}, signature="sv")
        # ★ 别等 EOF：KWin 写完图也不一定马上关 write end，等 EOF 会白卡 5–10 秒。
        #   先拿 props 里的 width/height/stride，读满需要的字节数就停。
        props = iface.CaptureActiveScreen(opts, fd, timeout=8)
        p = {str(k): v for k, v in dict(props).items()}
        wd, ht, stride = int(p["width"]), int(p["height"]), int(p["stride"])
        state["need"] = stride * ht
        th.join(timeout=3)
        os.close(r)
        im = Image.frombuffer("RGBA", (wd, ht), b"".join(buf), "raw", "RGBX", stride, 1)
        return im.convert("RGB")

    return grab


def cmd_record(outdir, duration, fps):
    os.makedirs(outdir, exist_ok=True)
    for f in os.listdir(outdir):
        os.unlink(os.path.join(outdir, f))
    grab = make_capture()
    meta = open(os.path.join(outdir, "meta.jsonl"), "w")
    t0 = time.time()
    n = 0
    while time.time() - t0 < duration:
        t = time.time()
        try:
            im = grab()
            W, H = im.size
            box = (W - CROP_W, H - CROP_H, W, H)
            fn = "f%04d.jpg" % n
            im.crop(box).save(os.path.join(outdir, fn), quality=85)
            meta.write(json.dumps({"i": n, "t": round(time.time() - t0, 3),
                                   "epoch": round(time.time(), 3), "file": fn,
                                   "w": W, "h": H, "box": box}) + "\n")
            meta.flush()
            n += 1
        except Exception as e:  # 单帧失败不致命
            meta.write(json.dumps({"i": n, "t": round(time.time() - t0, 3),
                                   "error": str(e)}) + "\n")
        dt = time.time() - t
        time.sleep(max(0, 1.0 / fps - dt))
    meta.close()
    print("frames", n, "elapsed", round(time.time() - t0, 2))


def _frames(outdir):
    metas = []
    for line in open(os.path.join(outdir, "meta.jsonl")):
        line = line.strip()
        if not line:
            continue
        m = json.loads(line)
        if "file" in m:
            metas.append(m)
    return metas


def _load(outdir, m):
    return Image.open(os.path.join(outdir, m["file"])).convert("RGB")


def _nearest(metas, t):
    return min(metas, key=lambda m: abs(m["t"] - t))


def _bubble_region(win_x, win_y, scale, cw, ch, box):
    """把「展开后窗口左半边（球左侧的气泡区）」换算成裁切图里的坐标。

    窗口几何：收起 148x148，展开 360x148（向左多出 212）。坐标是 X11 物理像素；
    截图是逻辑像素，所以要除以桌面缩放（gpd 实测 1.5）。
    """
    x0 = (win_x - 212) / scale - box[0]
    x1 = (win_x - 4) / scale - box[0]
    y0 = win_y / scale - box[1]
    y1 = (win_y + 148) / scale - box[1]
    return (max(0, int(x0)), max(0, int(y0)),
            min(cw, int(x1)), min(ch, int(y1)))


def cmd_analyze(outdir, t_late, win_x, win_y, scale, restart_epoch=0.0, until_epoch=0.0):
    """扫描整段录屏，找「球左侧气泡区」与稳定后相比最大的差异。

    为什么扫全部帧而不是固定 t_early：启动快的时候气泡 1.5s 就出现，
    慢的时候（真机冷启动/高负载）4–5s 才出现；固定时刻会假通过。
    取最大值对两种机器都成立。
    """
    metas = _frames(outdir)
    if restart_epoch and restart_epoch > 0:
        # 只保留「这次重启之后」的帧：重启前旧实例的字幕气泡是正常产品行为，
        # 不能算到启动又干净度的账上。
        metas = [m for m in metas if m.get("epoch", 0) >= restart_epoch]
    if until_epoch and until_epoch > 0:
        # 运行期字幕（真实语音/外部注入音频触发 S2）不是启动 bug：
        # 只分析「第一次运行期状态变化之前」的帧。实测验收时会碰到别的 agent
        # 在同一台 gpd 上灌音频，不切的话 T2 会把它当成启动气泡。
        metas = [m for m in metas if m.get("epoch", 0) < until_epoch]
    if len(metas) < 5:
        sys.exit("帧太少，录失败（重启后不足 5 帧）")

    def nearest_epoch(target):
        if target <= 0:
            return metas[-1]
        return min(metas, key=lambda m: abs(m.get("epoch", 0) - target))

    base = restart_epoch if restart_epoch and restart_epoch > 0 else metas[0].get("epoch", 0)
    late_m = nearest_epoch(base + t_late)
    late = _load(outdir, late_m)
    prev_m = nearest_epoch(base + max(0.0, t_late - 1.0))
    prev = _load(outdir, prev_m)
    box = metas[0]["box"]
    cw, ch = late.size
    region = _bubble_region(win_x, win_y, scale, cw, ch, box)

    def mean_diff(a, b):
        diff = ImageChops.difference(a, b).convert("L")
        h = diff.histogram()
        tot = sum(h) or 1
        mean = sum(i * c for i, c in enumerate(h)) / tot
        changed = sum(c for i, c in enumerate(h) if i > 12) / tot
        return mean, changed

    late_c = late.crop(region)
    noise_mean, _ = mean_diff(late.crop(region), prev.crop(region))
    worst = (0.0, 0.0, None)
    for m in metas:
        try:
            im = _load(outdir, m)
        except Exception:
            continue
        mean, changed = mean_diff(late_c, im.crop(region))
        if mean > worst[0]:
            worst = (mean, changed, m["t"])
    thr = max(1.5, noise_mean * 4 + 0.5)
    out = {"t_late": late_m["t"], "region": region, "scale": scale,
           "noise_mean": round(noise_mean, 2),
           "max_mean_diff": round(worst[0], 2),
           "max_changed_ratio": round(worst[1], 4),
           "max_at_t": worst[2],
           "threshold": round(thr, 2),
           "until_epoch": until_epoch or None}
    ok = worst[0] < thr
    out["verdict"] = "PASS" if ok else "FAIL"
    if not ok:
        out["reason"] = ("启动过程中球左侧出现了 4.2 秒的黑框/字幕（t≈%.1fs，差异 %.2f > 阈值 %.2f）"
                         % (worst[2] or 0, worst[0], thr))
    print(json.dumps(out, ensure_ascii=False))
    return 0 if ok else 1


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    cmd = sys.argv[1]
    if cmd == "record":
        cmd_record(sys.argv[2], float(sys.argv[3]), float(sys.argv[4]))
    elif cmd == "analyze":
        # analyze <dir> <t_late> <win_x> <win_y> <scale> [restart_epoch]
        if len(sys.argv) < 7:
            sys.exit("用法：analyze <dir> <t_late> <win_x> <win_y> <scale> [restart_epoch]")
        epoch = float(sys.argv[7]) if len(sys.argv) > 7 else 0.0
        until = float(sys.argv[8]) if len(sys.argv) > 8 else 0.0
        sys.exit(cmd_analyze(sys.argv[2], float(sys.argv[3]),
                             int(sys.argv[4]), int(sys.argv[5]), float(sys.argv[6]), epoch, until))
    else:
        sys.exit(f"未知子命令：{cmd}")


if __name__ == "__main__":
    main()
