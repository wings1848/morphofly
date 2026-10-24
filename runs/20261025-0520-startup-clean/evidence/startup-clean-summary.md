# 启动干净度验收（2026-10-25 06:21:39）

设备：wings@100.107.9.102  服务：morphofly-test
判据：T1 启动无字幕气泡 / T2 启动连拍无黑框 / T3 位置尺寸置顶不回归 / T4 前端单测全绿
启动窗口：本次启动后 60 行（约 3–5 秒）；验收期间重启次数：1

PASS  T1 启动时没有显示初始字幕（启动状态 state=S0 caption=无）
PASS  T2 真机连拍：球左侧气泡区无黑框/文字（"max_mean_diff": 0.0）
PASS  T3 位置/尺寸/置顶均未回归（pos: 实测 window=0x120000d 位置=(2908,1516) 尺寸=148x148 显示器=HDMI-A-1 3072x1728+0+0 → 右距=16px 底距=64px; top: 实测 _NET_WM_STATE(ATOM) = _NET_WM_STATE_ABOVE, _NET_WM_STATE_STAYS_ON_TOP, _NET_WM_STATE_SKIP_TASKBAR, _NET_WM_STATE_SKIP_PAGER; size: 实测 148x148（window=0x120000d 位置=(2908,1516)）; ）
PASS  T4 前端单测全绿（16 passed）

总计：通过 4 / 失败 0 / 跳过 0
