# 计划（S2）

## 三条工作流并行（worktree 隔离，写作用域不重叠）

| 流 | 分支 | 写作用域 | 谁做 |
|---|---|---|---|
| A 前端视觉 | `wt/ui-float` | `app/frontend/**` | 子代理 ui-visual |
| B 窗口行为 | 直接在 main（我自己） | `app/src-tauri/tauri.conf.json`、`app/src-tauri/src/main.rs`、`docs/`、`.github/` | lead |
| C 验证工具 | `wt/verify-float` | `tools/**` | 子代理 verify-harness |

**互不重叠**：A 只碰 frontend，B 只碰 Rust/配置，C 只碰 tools。合并顺序：B 已在 main，A/C 各自合并。

## DAG

```
B(窗口行为) ──┐
A(前端视觉) ──┼─→ 合并 ─→ 编译(gpd) ─→ 部署 ─→ 真机验收 ─→ CI 绿 ─→ 交付
C(验证工具) ──┘
```

A/B/C 之间无依赖，可同时开工。合并之后才需要编译，所以只付一次 6 分钟编译费。

## 每步的闸门

| 步骤 | 闸门 | 命令 |
|---|---|---|
| A 前端 | 单测 | `node --test 'app/frontend/test/*.test.mjs'` |
| B 窗口 | 编译 + 置顶/位置 | `cargo build --release -p morphofly-app`（gpd）+ `tools/verify_float_ui.sh top/pos` |
| C 工具 | 语法 + 真跑 | `bash -n tools/verify_float_ui.sh` + 在 gpd 上真跑一遍 |
| 合并后 | 全量 | `cargo test -p morphofly-core --lib` + `tools/verify_float_ui.sh all` |
| 交付 | CI | GitHub Actions CI 绿 |

## 预算与止损

- 编译次数上限：3 次（一次验证 Rust、一次合并后、一次必要时修）
- 前端视觉迭代不占用编译次数（无头 Chromium）
- 连续 2 轮修不好 → H4 升级给用户，不硬耗
