T47 completion 修复路线图（从 blocked worker 日志抢救，代码未落盘）
=====================================================================
来源: hermes kanban log t_f4c453ad
进展: completion 112 → 120 (74.5%) — cluster-A 8 例全过，blocked 前未提交
失败: 41 例剩余

Cluster A 修复（emit_member_decl，已验证 112→120）:
1. init-strip: 成员 var 补全时去掉 init 表达式
2. public-only filter: member completion 只显示 public 成员
3. mut/open recovery: mut/open 关键字后的解析恢复

Cluster B（未完成）: func return type inference
- 需用 AST Binary/Pipe/Match 推断函数返回类型（infer_expr_type 扩展）

验证方法:
cd <repo> && cargo build -p cj-lsp && python3 tools/run_feature_cases.py --feature completion --workers 8
lsp_config.txt linux_path 指向被测分支 target/debug

注意: emit_member_decl 位于 crates/cj-lsp/src/completion.rs