#!/usr/bin/env bash
cd /root/Code/cangjie/cj-lang

# T10: LSP completion + hover（主 agent 认领，不 assign）
hermes kanban create "T10: LSP completion + hover 功能" \
  --body "目标: 为 cjlsp 实现 completion(补全) 和 hover(悬停提示), 基于已有符号表/解析器。
背景: LSP initialize 已声明 completionProvider/hoverProvider capability。lsp_test.py 套件有 completion/hover 用例。符号表(PackageTable)和 resolver 已就绪。
实现:
- completion: 基于文件作用域收集可补全名字(顶层decls+局部变量+import), 返回 CompletionItem[](label/kind/detail)。cpp 的缩写测试: 输入 'tes' 补全 testvar 等
- hover: 给定位置 -> 找到该名字的声明 -> 返回 Markdown (类型+签名+位置)
- 新文件 crates/cj-lsp/src/completion.rs, hover.rs
- server.rs 实现 textDocument/completion 和 textDocument/hover handler
验收: lsp_test.py completion/hover 用例通过, cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t10-completion \
  --skill cangjie-development --priority 80 2>&1 | tail -1

# T11: 宏 .so dlopen 展开（worker）
hermes kanban create "T11: 宏 dylib dlopen 实际展开(替代模板fallback)" \
  --body "目标: 用 libloading 加载 SDK cjpm 编译的宏 .so, 调用 macroCall_c_* 展开函数, 替换 expander.rs 的模板 fallback。
背景: T4b 已实现 macro_cache.rs 三层缓存(源码hash->.so)。T7 已把 expand_file_with_cache 接入 LSP(内置宏+缓存.so路径记录)。已验证 cjpm 生成 lib-macro_X.define.so 且导出 macroCall_c_Wrap_macro_calling_define 可调用符号。
实现:
- Cargo.toml cj-sema 加 libloading 依赖
- expander.rs expand_one_cached: 拿缓存.so后 dlopen, dlsym macroCall_c_<name>_<pkg> 符号, 构造 Tokens参数调用(官方Tokens序列化在 MacroEvalMsgSerializer.cpp, 先支持空参/字符串参)
- 保留内置宏和unresolved诊断
- 失败时优雅fallback到现有模板展开, 不影响LSP稳定性
验收: 写宏包 .so 的集成测试能 dlopen 并调用(至少符号存在性), cargo test 全过 + clippy 零警告, LSP 诊断不回归(84.8%)。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t11-dylib-macro \
  --skill cangjie-development --priority 75 2>&1 | tail -1

# T12: 诊断覆盖率 84.8% -> 95%（worker）
hermes kanban create "T12: 诊断覆盖率冲刺 84.8% -> 95%" \
  --body "目标: lsp_cov.py 从 106/125(84.8%) 提升到 95%+ (119/125)。
方法(数据驱动):
1. python3 tools/lsp_cov.py 看各用例匹配, 找仍未匹配的 ~19 条
2. 优先高频: 常见缺口类型(未覆盖的警告/类型转换/参数检查)
3. 对照 /root/Code/cangjie/cangjie_test/testsuites/HLT/Tools/cjlsp/testcases/autotestcase/diagnostics/ 期望精确对齐措辞+位置
4. 每改一处立刻跑 lsp_cov.py 验证提升, 不堆功能
注意: 只改 crates/cj-sema 和 crates/cj-lsp 诊断相关, 不要改 tools/lsp_cov.py 匹配逻辑。与 T11 不同文件(避免冲突): T11 改 expander.rs/macro_cache.rs, 你改 checks.rs/typecheck.rs/unused.rs 等。
验收: lsp_cov.py >= 95%(119/125)。cargo test 全过 + clippy 零警告。git 提交。" \
  --workspace worktree:/root/Code/cangjie/cj-lang --branch wt/t12-cov95 \
  --skill cangjie-development --priority 85 2>&1 | tail -1

hermes kanban list 2>&1 | head -12