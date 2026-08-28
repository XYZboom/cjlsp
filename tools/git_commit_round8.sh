#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T30: LLT 83.5% -> 90%
hermes kanban create "T30: LLT 成功率 83.5% -> 90%" \
  --body "目标: LLT 前端成功率从 83.5% (4470/5351) 提升到 90%+ (4820+)。
当前失败分布 (llt_baseline.txt, T29 后 fail=881):
- 按目录看哪个最多 (Sema 应该仍是大头)
- 失败消息聚类看 tools/llt_baseline.txt

方法 (数据驱动):
1. cat tools/llt_baseline.txt 看失败聚类 (按频率从高到低)
2. 先 cargo build --release -p cj-frontend (必须 release, debug 失真)
3. 每次修复后: python3 tools/llt_baseline.py --jobs 8 复测
4. 优先 Sema 层 (collector/resolver/typecheck)

注意: 只改 crates/cj-parser 和 crates/cj-sema。不碰 cj-lsp (T31/T32 领域)。禁止引入 timeout (T24 已清零 crash, 保持 crash=0)。
验收: llt_baseline.py 成功率 >= 90%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t30-llt90 --skill cangjie-development --priority 85 2>&1 | tail -1

# T31: completion 31.7% -> 50%
hermes kanban create "T31: completion 用例对齐 31.7% -> 50%" \
  --body "目标: completion 通过率从 31.7% (51/161) 提升到 50%+ (80+ 用例)。
当前失败 110 例 (T28 后):
- MEMBER 补全约 58 例 (obj.member 需类型推断, 是最大头)
- 其他: result-empty / richness 不符

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature completion --limit 15 看具体失败
2. 对照 testcases/autotestcase/completion/*.info 期望
3. MEMBER 是核心: 需要简单类型推断 (let x: C = ... 后 x. 补全 C 的成员)。参考 T18 hover.rs 的 Index/成员收集 (它已能做类型推断)
4. 可复用 cj-sema 的符号表/collector

注意: 只改 crates/cj-lsp/src/completion.rs + server.rs。T30 是 cj-parser/cj-sema 领域, 不冲突 (但若 T30 也动 sema 需注意合并)。lsp_config.txt 已指向 master。
验收: python3 tools/run_feature_cases.py --feature completion --workers 8 通过率 >= 50%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t31-completion50 --skill cangjie-development --priority 80 2>&1 | tail -1

# T32: hover 80.7% -> 90%
hermes kanban create "T32: hover 用例对齐 80.7% -> 90%" \
  --body "目标: hover 通过率从 80.7% (146/181) 提升到 90%+ (163+ 用例)。
当前失败 35 例 (T22 后, 已是 80.7%):
- 剩余失败模式: 用 python3 tools/run_feature_cases.py --feature hover --limit 15 看
- 可能: 特殊声明 (macro/prop/operator)、嵌套引用、doc comment 细节

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature hover --limit 15 看具体失败
2. 对照 testcases/autotestcase/hover/*.info 期望, 统计剩余聚类
3. 只改 crates/cj-lsp/src/hover.rs + server.rs

注意: hover.rs ~1870 行 (T18 重建), 改前先读结构。lsp_config.txt 已指向 master。不要改 run_feature_cases.py。
验收: python3 tools/run_feature_cases.py --feature hover --workers 8 通过率 >= 90%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t32-hover90 --skill cangjie-development --priority 75 2>&1 | tail -1

hermes kanban list 2>&1 | grep -E "T3[012]"