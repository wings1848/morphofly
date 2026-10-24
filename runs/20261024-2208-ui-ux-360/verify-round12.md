# 第 12 轮验证：语音全链路（会话关闭后仍主动汇报）+ pc 车道

**2026-10-25 04:37** ｜ 被测 sha `11dc82e`（GitHub Actions run 40130109936，build-app-linux）
**设备**：wings-gpd（KWin + XWayland，虚拟音源 mfsink 注入唤醒/任务语音）

## 一、全链路（核心承诺）

```
20:35:20  App 启动
20:35:40  会话已建立 state="S2"
20:35:58  用户转写本地纠正（ASR 听错专有名词）
          before=帮我看看mosofly项目的 limit 第一行是什么？
          after =帮我看看morphofly项目的 limit 第一行是什么？
20:35:58  派活 agent=dsh dir=/home/wings/Code_Project/morphofly
20:36:21  DSH initialize 握手完成 elapsed_ms=22815
20:36:22  DSH 会话已建 session=17ed7904-b604-4a9f-97a7-58d9d2be9e0b
20:37:27  会话结束（云端 90s 空闲断开，DSH 还在干）
20:37:31  有任务结果还没念，主动开一轮汇报 pending=1   ← 守卫自己重开会话
20:37:34  会话已建立 state="S2"
20:37:35  🔔 主动汇报 task=task_20261024_2035_029 report=干完了：…
```

`tools/gpd_voice_e2e.sh` 输出：

```
派活次数:          1   （期望 1）
模型工具调用:      0   （走的是本地代派）
DSH 会话新建:      1   （期望 1）
主动汇报:          1   （期望 1）
✅ 全链路通过：唤醒 1 次、派活 1 次、汇报 1 次
```

**关键点**：语音会话 20:37:27 就结束了，DSH 到 20:37:31 才出结果 ——
App 自己发现「有结果没念」，重新开了会话念出来。这才是「关屏后它回来说」的承诺。

## 二、这轮修掉的 5 个真 bug（全部来自真机 e2e，不是猜的）

| # | 现象 | 真因 | 修法 | 证据 |
|---|---|---|---|---|
| 1 | DSH 干完、一条汇报都没有（重启才补报） | 会话和汇报守卫各自 `TaskStore::open(同一路径)` 建了**两个实例**，守卫看不到会话派出的任务 | 会话改为接收共用 store/dispatcher | `verify-round11` 之后的日志：task 027 done 但 reported=false，无任何汇报 |
| 2 | 「看一下 README」被反问确认 | 路径占 37 字符 → 整句 57 字 > 50 阈值 | 只读动词 + 无改动动词 → 直接派 | `intent` 单测 + 本次 e2e 不再拦 |
| 3 | morphofly 听成 mosofly / 魔方fly / 魔方fly | 别名表只精确匹配；项目名提取把动词也算进去（拿到「帮我看看mosofly」） | `lookup_fuzzy`（编辑距离）+ 提取重写 + 任务文本纠正 | 本次 `after=帮我看看morphofly…` |
| 4 | README 听成 vimi / 微米 | ASR 专有名词错误 | 本地纠正表 `core/src/asr_fix.rs`；探针证明 prompt 热词被服务端忽略 | `docs/v2/asr-project-mishearing.md` |
| 5 | 汇报「DSH 握手超时」 | 高负载时 initialize 实测 **22.8s**，超时只给 30s | 75s / 100s + 打耗时日志 | `elapsed_ms=22815` |

## 三、pc 车道（subagent 并行完成）

- wings-pc 已登录桌面，用 CI 产物部署 + 建 user service，`pos / top / size` 三项 **PASS**
- `tools/e2e_matrix.sh --with-pc`：3 项 PASS，rc=0（默认关，避免依赖 pc 在线）
- 坑：pc 的 X11 缩放 = 2（窗口 296 = 148×2）→ 阈值按 `PC_SCALE=2` 换算；
  pc 没有扬声器 → 主动汇报念不出来会每 ~30s 重开一次 StepFun 会话（计费 + 抢 key）
  → **pc 服务不常驻**，用前 start、用完 stop

## 四、判定

- 语音全链路：**PASS**（唤醒 1 / 派活 1 / DSH 会话 1 / 汇报 1）
- 汇报闭环（会话关闭后自行重开）：**PASS**
- pc 车道：**PASS**（三项几何检查）
- core 单测：271 passed；CI 全绿

## 五、留给下一轮

1. 完成内容词 ASR 错误（README→limit）本地救不了，只能靠模型直接听音频
2. 无扬声器机器上「汇报念不出」会无限重试开会计费会话 → 需要失败退避/上限
3. 打断（<300ms）仍未端到端验证
4. Windows 运行时仍未真机验证
