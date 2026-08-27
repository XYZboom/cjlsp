#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T21: completion 9.9% -> 30%+
hermes kanban create "T21: completion 用例对齐 9.9% -> 30%+" \
  --body "目标: completion 通过率从 9.9% (16/161) 提升到 30%+ (48+ 用例)。
T16/T17 已量化失败聚类:
- result-empty[]: 46 例 (应返回列表但我们空)
- result-nonempty-list: ~99 例 (返回了但内容/排序/richness 不符)
- 当前 completion.rs ~1800 行 (T17 重写: std符号/成员访问/上下文触发)

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature completion --limit 10 看具体失败
2. 对照 testcases/autotestcase/completion/*.info 期望, 按聚类修复
3. 优先 result-empty[]: 检查位置/前缀/触发场景处理
4. 再修内容不符: 官方 CompletionItem 的 sortText/kind/detail 精确对齐

注意: 不要改 tools/run_feature_cases.py 的判定逻辑。lsp_config.txt linux_path 已是 /root/Code/cangjie/cj-lang/target/debug (master)。
验收: python3 tools/run_feature_cases.py --feature completion --workers 8 通过率 >= 30%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t21-completion --skill cangjie-development --priority 85 2>&1 | tail -1

# T22: hover 37.6% -> 60%+
hermes kanban create "T22: hover 用例对齐 37.6% -> 60%+" \
  --body "目标: hover 通过率从 37.6% (68/181) 提升到 60%+ (109+ 用例)。
T18 后失败聚类 (113 例):
- 引用处未解析到声明
- markdown 格式/签名细节不符
- range 差异

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature hover --limit 10 看失败
2. 对照 testcases/autotestcase/hover/*.info 期望
3. 统计剩余失败主因 (用 --report-only 或保留的 artifacts), 按聚类修复
4. 参考 T18 已实现的 Index/Container/成员访问解析

注意: hover.rs 现在 ~1870 行 (T18 重建)。lsp_config.txt 已指向 master。不要改 run_feature_cases.py。
验收: python3 tools/run_feature_cases.py --feature hover --workers 8 通过率 >= 60%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t22-hover --skill cangjie-development --priority 85 2>&1 | tail -1

# T23: LLT 61% -> 80%+
hermes kanban create "T23: LLT 成功率 61% -> 80%+" \
  --body "目标: LLT 前端成功率从 61.0% (3263/5351) 提升到 80%+ (4280+)。
当前失败分布 (llt_baseline.txt):
- Sema 层 3242 例失败最多 (成功 621)
- Diagnose 589 例失败
- Parser 473 例失败
- 剩余失败消息聚类: 看 tools/llt_baseline.txt 的 failure clusters

方法 (数据驱动):
1. cat tools/llt_baseline.txt 看最新失败聚类 (T20 后已更新)
2. 按频率从高到低修复, 每次改完: python3 tools/llt_baseline.py --jobs 8 复测
3. 注意用 release 二进制: cargo build --release -p cj-frontend (debug 是旧行为)
4. Sema 层失败多可能是 collector/resolver 的覆盖不足

注意: 只改 crates/cj-parser 和 crates/cj-sema。不碰 cj-lsp (那是 T21/T22 领域)。
验收: python3 tools/llt_baseline.py --jobs 8 成功率 >= 80%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t23-llt80 --skill cangjie-development --priority 80 2>&1 | tail -1

hermes kanban list 2>&1 | grep -E "T2[123]"