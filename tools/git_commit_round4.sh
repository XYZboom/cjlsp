#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T17: completion 失败聚类修复（T16 baseline: 2/161=1.2% -> 提升）
hermes kanban create "T17: completion 用例对齐 (1.2% -> 50%+)" \
  --body "目标: 基于 T16 baseline 提升 completion 通过率从 1.2% (2/161) 到 50%+。
失败聚类 (T16 已量化):
- result-nonempty-list: 99 例 (我们返回列表但 richness/排序/内容不符)
- result-empty[]: 46 例 (应返回列表但我们返回空)
- expected-null-vs-[]-harness-crash: 9 例 (框架自身)
- no-response: 5 例

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --report-only 看详细差异
2. 挑一个失败用例对照官方期望 (testcases/autotestcase/completion/*.info), 逐个聚类修复
3. 优先 result-empty[] (46例): 可能是位置/didChange 处理问题, 或前缀匹配逻辑
4. 再修 result-nonempty-list: 官方 CompletionItem 字段/排序/richness

注意: completion.rs 已有基础(文件decls+std符号+精确匹配规则)。主要增强: 补全范围(import/member access)、触发场景。验收: python3 tools/run_feature_cases.py --completion 通过率 >= 50%。cargo test + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t17-completion \
  --skill cangjie-development --priority 85 2>&1 | tail -1

# T18: hover 失败聚类修复（T16 baseline: 14/181=7.7% -> 提升）
hermes kanban create "T18: hover 用例对齐 (7.7% -> 50%+)" \
  --body "目标: 基于 T16 baseline 提升 hover 通过率从 7.7% (14/181) 到 50%+。
失败聚类 (T16 已量化):
- result-null: 130 例 (光标处未找到声明)
- result-other: 37 例 (找到但 markdown 内容/range 不符)

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --report-only 看详细差异
2. 挑失败用例对照官方期望 (testcases/autotestcase/hover/*.info)
3. result-null (130例): 可能原因 - 光标在引用处(非声明处)、跨文件声明、方法/属性成员、import 符号。扩展 decl_hover 覆盖: 成员方法/属性、类内字段、引用位置解析到声明
4. result-other (37例): markdown 格式(官方可能是纯文本或不同签名格式)、range 差异

注意: hover.rs 已有声明签名渲染+name_pos匹配。主要增强: 引用处->声明解析、更多声明类型。验收: python3 tools/run_feature_cases.py --hover 通过率 >= 50%。cargo test + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t18-hover \
  --skill cangjie-development --priority 85 2>&1 | tail -1

# T19: LLT 前端全量回归（M8 里程碑评估）
hermes kanban create "T19: LLT 前端全量回归 (5353用例基线)" \
  --body "目标: 用 cj-frontend 批量跑 LLT/compiler 前端用例, 建 M8 里程碑基线。
背景: 项目目标是对齐官方 cangjie_compiler 行为并跑通官方测试套件。LLT/compiler 前端测试约 5353 例。
方法:
1. 调研 LLT 测试如何运行 (testsuites/LLT/compiler/ 结构, 看是否有 run 脚本或 .cj 文件直接可解析)
2. 写批量脚本: 对每个 .cj 文件跑 cj-frontend --dump-ast, 统计: 解析成功数/失败数/崩溃数
3. 对比: 无法跑官方编译器对照(无完整工具链), 至少建'无崩溃'基线
4. 记录到 tools/llt_baseline.txt: 总数/成功/失败/崩溃 + 失败样例前 50
注意: 这是评估任务, 不改 cj-frontend 核心逻辑。若发现高频崩溃模式, 记录到看板任务描述供后续修复。
验收: tools/llt_baseline.txt 输出 + 统计数字。git 提交(工具+基线)。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t19-llt \
  --skill cangjie-development --priority 80 2>&1 | tail -1

hermes kanban list 2>&1 | head -12