#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# 清理已合并的 worktree（T11/T12 已合并，T4 blocked 保留）
git worktree remove .worktrees/t_86181d47 --force 2>&1 | tail -1
git worktree remove .worktrees/t_f5c6aaf6 --force 2>&1 | tail -1
git branch -D wt/t11-dylib-macro wt/t12-cov95 2>/dev/null

# T13: LSP definition/references（主 agent 认领）
hermes kanban create "T13: LSP definition + references 跳转" \
  --body "目标: 实现 textDocument/definition 和 textDocument/references, 基于符号表跳转到声明。
背景: initialize 已声明 definitionProvider:true, referencesProvider:true。符号表(PackageTable)+resolver+name_pos 已就绪, hover.rs 已有 name_pos 匹配逻辑可复用。
实现:
- definition: 给定位置 -> 找到名字引用 -> 返回声明位置(uri+range, 用 name_pos)
- references: 给定位置 -> 找文件中所有该名字引用点
- 复用 hover.rs 的 decl_hover 位置匹配, 新增 definition.rs/references.rs 或合并进 server.rs
- 官方用例: /root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp/testcases/autotestcase/definition/ 和 references/
验收: lsp_test.py definition_001 + references_001 通过。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t13-def-ref \
  --skill cangjie-development --priority 85 2>&1 | tail -1

# T14: 宏展开诊断预览 note（worker）
hermes kanban create "T14: 宏展开诊断预览 note(官方 note: the code after the macro is expanded)" \
  --body "目标: 当诊断发生在宏展开生成的代码中时, 附加 note 显示展开预览, 对齐官方 cjc 行为。
背景: 官方 cjc 报错自带:
  note: the code after the macro is expanded as follows
    /* 5.1 */print(x)
我们的 expander.rs 已记录 Expansion{call_line, call_col, expanded} 展开轨迹, 但 LSP 诊断未使用。
实现:
- expander.rs expand_file_with_cache 已返回 (expansions, diags)
- server.rs: 当普通诊断的位置落在某 Expansion 的 expanded 文本范围内时, 给该诊断附加 note: 'the code after the macro is expanded as follows' + 展开代码
- 用 Diag 的 notes 字段(已存在)
- 参考官方 cjc 输出格式精确对齐
验收: 写一个 @Wrap(42) 宏调用导致后续错误诊断, 验证诊断带展开预览 note。cargo test 全过 + clippy 零警告 + lsp_cov 不回归(91.2%)。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t14-macro-preview \
  --skill cangjie-development --priority 75 2>&1 | tail -1

# T15: 诊断覆盖率 91.2% -> 95%（worker）
hermes kanban create "T15: 诊断覆盖率冲刺 91.2% -> 95%" \
  --body "目标: lsp_cov.py 从 114/125(91.2%) 提升到 95%+ (119/125)。
方法(数据驱动):
1. python3 tools/lsp_cov.py 看各用例匹配, 找仍未匹配的 ~11 条
2. 对照 /root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp/testcases/autotestcase/diagnostics/ 期望精确对齐
3. 每改一处立刻跑 lsp_cov.py 验证提升
注意: 与 T14 不同文件(T14 改 expander.rs/server.rs 的 note 逻辑, 你改 checks.rs/typecheck.rs/unused.rs 等诊断规则)。不要改 tools/lsp_cov.py。
验收: lsp_cov.py >= 95%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t15-cov95 \
  --skill cangjie-development --priority 85 2>&1 | tail -1

# T16: completion/hover 官方全目录基线（worker）
hermes kanban create "T16: completion/hover 全目录批量基线测量" \
  --body "目标: 批量跑官方 completion(161用例) + hover(181用例) 全目录, 建通过率基线, 不改代码只测量。
方法:
- 更新 lsp_config.txt linux_path = /root/Code/cangjie/cj-lang/target/debug (已改过, 确认)
- 用 tools/run_completion_cases.py 逻辑批量跑, 或写类似脚本
- 注意 lsp_test.py 有 env_info None 的 bug(framework自身), 用 grep 'testcase pass' 判定
- completion/hover 各统计: 通过/总数/失败列表, 输出到 tools/feature_baseline.txt
验收: 输出 baseline 文件, 报告通过率数字。不提交代码改动(除非工具脚本)。git 提交(如有工具)。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t16-baseline \
  --skill cangjie-development --priority 70 2>&1 | tail -1

hermes kanban list 2>&1 | head -12