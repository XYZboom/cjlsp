# Cangjie Rust LSP + Compiler Frontend — Project Guidelines

## Mission
用 Rust 从零实现仓颉（Cangjie）语言的**编译器前端**（词法/语法/语义分析）和
**LSP 服务器**，以通过官方测试仓 `cangjie_test` 的相关检查为验收标准。

## Acceptance (验收标准 — 用户确认)
| 组件 | 测试 | 规模 |
|---|---|---|
| LSP 服务器 | `testsuites/HLT/Tools/cjlsp/testcases` | 1689 用例 / 26 功能类 |
| 编译前端 | `testsuites/LLT/compiler/{Lexer,Parser,Sema,Frontend,Diagnose}` | ≈5353 文件 |

**不做后端 codegen。**

## Hard rules（用户硬性规则）
1. **不修改 aifuzzer 等无关项目**；本仓库只动 cj-lang 和官方源码引用。
2. **AST 已从官方 ASTKind.inc 生成**（tools/gen_ast.py），节点类型不要手改；
   要改就改生成器后重新生成。官方 Node.h 是权威结构来源，允许照搬结构。
3. **Rust 最佳实践**：clippy `-D warnings` 零警告、不 panic、高性能
   （避免不必要分配/克隆，用迭代器/Box 语义）。
4. **每改一处先小批验证再叠加**；回归用 git + 测试。
5. 诊断格式是最大难点：SCAN 块要求错误消息+行列+`^^` 标线逐字符匹配官方。
6. 完成一个任务必须：`cargo build` + `cargo test -p <crate>` + `cargo clippy -D warnings`
   全绿，git 提交，并在任务 summary 写验收证据。

## Repos / Layout
- 项目：`/root/Code/cangjie/cj-lang`（Rust workspace，8 crates）
- 测试仓：`/root/Code/cangjie/cangjie_test`（只读验收金标准）
- 框架：`/root/Code/cangjie/cangjie_test_framework`（Maple）
- 官方源码蓝本：`/root/Code/cangjie/cangjie_compiler`（只读参考）
- 生成器：`tools/gen_ast.py`

## Key architecture decisions
- 语义检查 95%+ 在 AST 层（src/Sema/），仅 3 类诊断在 CHIR 层需数据流
  （VarInitCheck/UnreachableBranchCheck/OverflowChecking）→ 自研轻量 cj-cfg，
  不抄官方 CHIR。
- 诊断格式对齐 cjc：`--diagnostic-format=json` 和文本 SCAN 格式都要支持。
- LSP 二进制必须命名 `LSPServer`，支持 `--test --disableAutoImport --enable-log=true`。
