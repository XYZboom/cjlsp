# 第 17 轮规划（并行 4 任务）

创建日期: 2026-08-30
基于官方 Ark LSP 20 项能力清单（docs/official-lsp-research.md）选官方有我们缺的。

## 任务
| id | 内容 | 目标 |
|---|---|---|
| T62 | rename + prepareRename（符号重命名） | 复用 documentHighlight 引用收集 → WorkspaceEdit |
| T63 | codeLens（引用计数透镜） | 顶层 decl 上方显示 "N references" |
| T64 | documentHighlight 嵌套成员精准方案 | 修复 008/009（类内方法名只高亮本类成员），39/124 不回归 |
| T65 | hover 参数文档对齐 | 对照官方 HoverImpl 补缺字段，10/10 不回归 |

## 里程碑（master）
- ff7ab82: documentHighlight 39/124 (31.5%) + LLT 88.2% crash=0
- 第 17 轮 worker 完成 → 验证 → 提交 → 推送

## 教训（本轮已固化）
- documentHighlight 嵌套成员收集要**作用域感知**（只收集光标所在容器内的成员），否则过度匹配（008 期望 1 处但递归收集到 2 处）
- 回退无净收益改动保持基线干净（decl_name_at_recursive 单独无收益已回退）
- kanban 任务落在当前 cwd 的 board（cj-lang），dispatch 用 `--board cj-lang` 启动
