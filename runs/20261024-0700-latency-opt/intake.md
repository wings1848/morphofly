# 输入

**来源**：上一轮 run 的遗留问题（`20261023-2339-v1-dsh-0/evidence/p1-vad-parameter-sweep.md`）

**上一轮的结论**：
> StepFun 的 VAD 判定「你说完了」固定要 1.6 秒，参数完全不生效。
> 用户感知端到端延迟 = 2~7 秒，P1（<800ms）**不达标**。
> 解法：换 provider 或改产品定位。

**本轮的怀疑**：
1. 我只测了 `stepaudio-3-realtime-preview` 一个模型，但 StepFun 有 6 个
2. 「VAD 参数无效」≠「无法优化」—— 能不能不用服务端 VAD？

**约束**：
- 不能换 provider（用户还没给百炼 key）
- 不能改产品定位（用户没确认）
- 不能动 UI
