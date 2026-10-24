# ASR 把项目名听错时：模糊匹配 + 任务文本纠正

**2026-10-25** ｜ 来源：真机全链路 e2e 抓到的失败（不是猜的）

## 现象

真机跑 `tools/gpd_voice_e2e.sh`，唤醒、建会话都正常，但**没派活**：

```
用户转写（整句） said=帮我看看mosofly项目的 limit 第一行是什么？
这一轮没有调用任何工具 —— 模型只说了话没干活 reply=好，查着呢
模型只说不做 → 本地用用户原话代派 said=帮我看看mosofly项目的 limit 第一行是什么？
WARN 任务里提到不认识的項目名 unknown=帮我...
INFO 本地代派成功 speech=没找到「帮我看看mosofly」，你是说蝶梦吗
```

两个问题叠在一起：

1. **项目名提取把动词带进去了**：从「项目」往前取连续中文/字母 → 得到
   `帮我看看mosofly`，于是追问话术变成「没找到「帮我看看mosofly」，你是说蝶梦吗」。
2. **没有模糊匹配**：`mosofly` 是 ASR 把 `morphofly` 听错的结果，
   但别名表只做精确匹配，于是把一次正常派活变成了提问。

## 修法（都在 core，可单测）

| 位置 | 改动 |
|---|---|
| `projects.rs` | 新增 `lookup_fuzzy`：编辑距离阈值按名字长度（≥4 字母容忍 长度/3；短名容忍 1）|
| `dispatch.rs` | `project_name_candidates` + `project_name_from_before`：英文名取尾部 ASCII 段，中文名剥「帮我/看看/在/查…」前缀 |
| `dispatch.rs` | `dir_for_task` / `extract_unknown_project` 增加模糊匹配：听错的项目当已知，不再追问 |
| `dispatch.rs` | `normalize_project_mentions`：派活前把任务文本里的错名换成正式名（`mosofly`→`morphofly`），DSH 才不会找错目录 |

单测 7 条新覆盖：`fuzzy_matches_asr_mishearing`、`fuzzy_does_not_swallow_unrelated_names`、
`verb_phrase_is_not_part_of_project_name`、`misheard_project_name_is_rewritten_in_task`、
`misheard_project_still_routes_to_right_dir`、`asr_misheard_known_project_is_allowed`、
`levenshtein_basics`。

## 边界（诚实）

- 只在「X 项目 / X 工程 / X 仓库」这个句式里做提取，闲聊里的词不会当项目名。
- 阈值故意保守：宁可回去追问，也不把任务派到错的项目。
- **ASR 把 README 听成 limit 这类内容词错误本地修不了**，只能靠上下文/追问。
  项目名这类「有限集合 + 可查表」的词才是本地能救的。

## 探针结论：StepFun 不支持转写热词 prompt（2026-10-25 实测）

`core/examples/asr_hint_probe.rs` 同一段音频跑两次：

| 配置 | `session.updated` 回显 | 转写结果 |
|---|---|---|
| `{"model":"whisper-1"}` | `input_audio_transcription = null` | 帮我看看**mosofly**项目的**vimi**第一行是什么？ |
| 同上 + `"prompt":"蝶梦 morphofly DSH README 阶跃星辰…"` | 仍是 `null` | 一模一样 |

→ prompt 被**静默忽略**（连回显都没有），这条路别再试。
（顺带确认：App 现在根本没发 `input_audio_transcription`，转写是服务端默认开的。）

## 于是改成「本地纠正表」

`core/src/asr_fix.rs`：一张「正确写法 ← 见过的错法」表，
在 `voice.rs` 收到整句转写后做一次纠正再往下用。
真机实测的错法全部收录：

- `morphofly ← mosofly / morfofly / mobile fly / 摩托fly / move_forward`
- `README ← vimi / read me / 微米 / 里德米`
- `蝶梦 ← 碟梦 / 叠梦 / 跌梦`

边界：只认表里的错法，不做通用猜测（乱猜会改坏用户正常的话）；
ASCII 按边界替换（`dshx` 不会被改成 `DSHx`），中文只做整词替换。
