#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T24: 4 个 timeout-crash 修复（高优先级）
hermes kanban create "T24: LLT 4 个 timeout 死循环修复" \
  --body "目标: 修复 LLT 表里 4 个 <timeout> crash（解析死循环），恢复 crash=0。
背景: T23 合并后 llt_baseline.txt 显示 crash=4（此前一直为 0）。都是 <timeout> 而非真崩溃，疑似 parser 死循环。
崩溃文件（llt_baseline.txt 中列出）:
- Diagnose/parse_test/parse_expected_no_modifier/enum_constructor.cj
- Parser/ar/parse_support_keyword_context/... (2 个)
- 另 1 个在 Sema

方法:
1. cat tools/llt_baseline.txt 找 [crash files] 完整列表
2. 对每个文件跑: cd /root/Code/cangjie/cj-lang && timeout 10 ./target/release/cj-frontend --dump-ast <file> 复现
3. 死循环通常在 parser 的某个 while 循环（比如未推进 pos）。用 timeout + 缩小输入定位
4. 修复循环推进逻辑

注意: 这是稳定性问题，优先级高。修复后 python3 tools/llt_baseline.py --jobs 8 确认 crash=0。不要改 cj-lsp。
验收: llt_baseline.py crash=0 + cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t24-timeout --skill cangjie-development --priority 90 2>&1 | tail -1

# T25: LLT 75.5% -> 85%
hermes kanban create "T25: LLT 成功率 75.5% -> 85%" \
  --body "目标: LLT 前端成功率从 75.5% (4040/5351) 提升到 85%+ (4550+)。
当前失败分布 (llt_baseline.txt, T23 后):
- Sema 674 失败 (成功 3188)
- Diagnose 382 失败
- Parser 259 失败

方法 (数据驱动):
1. cat tools/llt_baseline.txt 看最新失败消息聚类
2. 按频率从高到低修复, 每次: cargo build --release -p cj-frontend && python3 tools/llt_baseline.py --jobs 8 复测
3. 必须先 build release (debug 是旧行为, 会让测量失真)
4. Sema 最多失败: 优先 collector/resolver/typecheck 的覆盖

注意: 只改 crates/cj-parser 和 crates/cj-sema。不碰 cj-lsp (T26 领域)。与 T24 协调: 若 T24 已修 timeout, 你专注成功率。
验收: llt_baseline.py 成功率 >= 85%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t25-llt85 --skill cangjie-development --priority 85 2>&1 | tail -1

# T26: completion 24.2% -> 45%
hermes kanban create "T26: completion 用例对齐 24.2% -> 45%" \
  --body "目标: completion 通过率从 24.2% (39/161) 提升到 45%+ (72+ 用例)。
当前失败聚类 (T21 后 122 例):
- result-empty[]: 之前 46 例已部分修复
- result-nonempty-list: 内容/排序/richness 不符
- 剩余失败模式: 用 --report-only 或 artifacts 分析

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature completion --limit 15 看具体失败
2. 对照 testcases/autotestcase/completion/*.info 期望, 按聚类修复
3. T21 已完成 860 行重构 (context/LetPatternDestructor/statement detail)。剩余: 可能 missing member-access 补全、type 名kind 精度
4. 注意 completion.rs 现在 ~2100 行

注意: 不要改 tools/run_feature_cases.py 判定逻辑。lsp_config.txt 已指向 master。验收: run_feature_cases.py --feature completion 通过率 >= 45%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t26-completion45 --skill cangjie-development --priority 80 2>&1 | tail -1

hermes kanban list 2>&1 | grep -E "T2[456]"