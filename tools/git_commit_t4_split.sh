#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang
hermes kanban block t_181d1a66 "任务粒度过大(1.8h超时+二次1.6h无提交), 与T8重叠(T8已合并, 覆盖率80%覆盖T4大部分目标)。server.rs未提交改动无tags/codeActions独有内容。按用户指示切分为T4a/T4b细粒度任务。"

hermes kanban create "T4a: LSP unused诊断补tags + codeActions字段" \
  --body "目标: cjlsp诊断输出对齐官方字段, 单个明确小任务。
背景: diagnostics_017等用例的unused诊断官方带 tags:[1] + codeActions(quickfix.removeUnusedSymbol, title=Remove unused function X, edit删除name范围)。
现状: server.rs push closure已输出category/code/codeActions:null/data, 缺tags和真实codeActions。
实现:
- analyze_source的unused诊断JSON补 tags:[1]
- codeActions: 需要每个unused诊断的删除范围。当前Diag只有line/col, 在unused.rs给Diag附加删除范围或server.rs按位置推算
- 参考textDocument_diagnostics_017.info的期望codeActions结构精确对齐
验收: python3 tools/lsp_cov.py中unused相关用例匹配提升, 或lsp_test.py的codeActions对比。cargo test全过+clippy零警告。git提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t4a-codeactions \
  --skill cangjie-development --priority 85 2>&1 | tail -1

hermes kanban create "T4b: 宏SDK缓存层 + cjc预编译接入" \
  --body "目标: 宏动态库(.so)的编译缓存, 支撑LSP用宏展开的性能要求。
背景: 已验证官网SDK1.1.3+cjpm把宏包编译为 lib-macro_X.define.so, 导出macroCall_c_X可调用展开函数。用户要求LSP中宏展开必须缓存, 避免每次didChange重编译。
实现(cj-sema新模块 macro_cache.rs):
- ①宏包源码hash -> 编译产物.so映射: 源码md5不变则复用.so (文件级缓存)
- ②宏展开结果缓存: (宏名+参数序列化)hash -> 展开结果, in-memory LRU
- ③LSP会话内缓存: didChange只对变更文件重算
- 接口: MacroCache::get_or_compile(pkg_src_hash) -> .so path; expand(so, macro_name, args_bytes) -> tokens
验收: 单测覆盖缓存命中(mock编译), cargo test + clippy零警告。git提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t4b-macro-cache \
  --skill cangjie-development --priority 75 2>&1 | tail -1

hermes kanban list 2>&1 | head -14