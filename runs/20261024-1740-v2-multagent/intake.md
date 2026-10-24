# 输入

## 用户原话（11 条新需求）
见 `~/BrainWorkshop/Project/蝶梦智能/蝶梦语音智能体.canvas` 的 ⑥ 区。

## 侦察输入
两份 subagent 技术侦察报告，结论见 `docs/v2/scout-full.md`。

## 上一轮遗留
- `runs/20261023-2339-v1-dsh-0`：V1 完成，延迟 802ms / 打断 300ms
- 5 个现存缺陷（本轮修了 4 个）

## 环境限制
- wings-pc 桌面会话已注销 → GUI 验证暂不可做
- 本机（云主机）无音频无桌面 → 只能跑纯逻辑测试
- GPU 环境未搭（缺 nvcc / ONNX GPU provider）
