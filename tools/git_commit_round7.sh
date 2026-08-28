#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T27: 性能基准（criterion）
hermes kanban create "T27: 性能基准 criterion (lexer/parser/LSP)" \
  --body "目标: 引入 criterion 性能基准，建立 lexer/parser/LSP 诊断的性能基线，防止覆盖率提升引入性能回归。
背景: LLT 覆盖率 61% -> 80.8% 的提升可能引入解析性能退化（T24 重写枚举解析、T25 加 range-step）。需要量化基线。

方法:
1. Cargo.toml 加 [dev-dependencies] criterion
2. benches/ 目录: bench_lexer.rs (tokenize 大文件), bench_parser.rs (parse 复杂文件), bench_lsp.rs (didOpen->diagnostics 延迟)
3. 基准样本: 用 LLT 里较大的 .cj 文件 + 合成大文件 (concatenate 多个 decl)
4. cargo bench 跑出基线数值, 记录到 tools/bench_baseline.txt
5. 不需要严格对齐官方编译器, 目标是内部回归检测

验收: cargo bench 可跑, tools/bench_baseline.txt 记录基线。lib 编译 + clippy 零警告 (benches 可在 workspace 外或用 ignore)。
参考: benchmark 放 benches/, 用 [[bench]] 声明。注意 dev-dependencies 不污染 release 构建。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t27-bench --skill cangjie-development --priority 75 2>&1 | tail -1

# T28: completion 24.2% -> 50%
hermes kanban create "T28: completion 用例对齐 24.2% -> 50%" \
  --body "目标: completion 通过率从 24.2% (39/161) 提升到 50%+ (80+ 用例)。
当前失败 122 例 (T21/T26 后):
- result-empty[]: 应在有候选时返回列表
- result-nonempty-list: 内容/排序/kind/detail 不符

方法 (数据驱动):
1. cd /root/Code/cangjie/cj-lang && python3 tools/run_feature_cases.py --feature completion --limit 15 看具体失败
2. 对照 testcases/autotestcase/completion/*.info 期望, 统计失败聚类
3. 重点: 官方 completion 在光标上下文为空后缀时也返回候选; member-access (obj.member) 场景; type 名 kind 精度
4. 注意 completion.rs 现在 ~2100 行 (T17/T21 重构), 改前先读结构

注意: 不要改 tools/run_feature_cases.py 判定逻辑。lsp_config.txt 已指向 master。改 crates/cj-lsp/src/completion.rs + server.rs。
验收: python3 tools/run_feature_cases.py --feature completion --workers 8 通过率 >= 50%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t28-completion50 --skill cangjie-development --priority 80 2>&1 | tail -1

# T29: LLT 80.8% -> 88%
hermes kanban create "T29: LLT 成功率 80.8% -> 88%" \
  --body "目标: LLT 前端成功率从 80.8% (4322/5351) 提升到 88%+ (4710+)。
当前失败分布 (llt_baseline.txt, T25 后):
- Sema 失败最多 (看具体数字)
- Diagnose 其次
- Parser 剩余

方法 (数据驱动):
1. cat tools/llt_baseline.txt 看失败聚类 (按频率从高到低)
2. 先 cargo build --release -p cj-frontend (必须 release, debug 失真)
3. 每次修复后: python3 tools/llt_baseline.py --jobs 8 复测
4. Sema 层 (collector/resolver/typecheck) 是最大头

注意: 只改 crates/cj-parser 和 crates/cj-sema。不碰 cj-lsp (T28 领域)。禁止引入新的 timeout/complexity 回归 (T24 刚修完 crash)。
验收: llt_baseline.py 成功率 >= 88%。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t29-llt88 --skill cangjie-development --priority 80 2>&1 | tail -1

hermes kanban list 2>&1 | grep -E "T2[789]"