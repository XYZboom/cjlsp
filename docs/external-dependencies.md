# 外部项目与生态依赖全景说明 (External Dependencies & Upstream Ecosystem)

本文档系统性梳理 `cj-lang` (Cangjie Rust Frontend & LSP Server) 项目对外部生态、官方仓库及标准库的依赖关系、解耦设计与协同策略。

---

## 1. 核心设计原则：核心自包含，外围松耦合

本项目在架构上遵循严格的分层设计：
- **核心前端与 LSP 核心（零外部依赖）**：词法（`cj-lexer`）、语法（`cj-parser`）、语义分析（`cj-sema`）、轻量控制流图（`cj-cfg`）、诊断（`cj-diag`）以及基于 stdio 的语言服务器（`cj-lsp`）完全使用纯 Rust 实现。在不安装仓颉官方编译器、无需网络、无需外部任何仓库的情况下，即可完成 `cargo build` 与 `cargo test --workspace`（135+ 个单元测试）。
- **生态与工具链（明确契约与按需依赖）**：测试验收、标准库跳转、宏运行时展开等高级能力按需对接仓颉官方生态。

---

## 2. 外部项目与官方仓库依赖清单

| 依赖实体 / 仓库 | 官方/开源地址 | 在本项目中的定位与用途 | 依赖方式与耦合程度 |
| :--- | :--- | :--- | :--- |
| **`cangjie_test`**<br>（官方测试集） | `https://gitcode.com/Cangjie/cangjie_test.git` | **验收金标准与端到端回归套件**<br>包含 1689+ 个 HLT 协议用例及 5300+ 个 LLT 编译前端测试集。 | **Git Submodule**<br>挂载于 `tests/cangjie_test/`。未拉取时项目仍可自包含构建与单测，驱动脚本自动探测并平滑回退。 |
| **`cangjie_compiler`**<br>（官方编译器源码） | `https://gitcode.com/Cangjie/cangjie_compiler.git` | **规范参考与 AST/诊断元数据来源**<br>用于提供 AST 规范（`ASTKind.inc`）及错误码定义（`Diagnostic*.def`）。 | **元数据已 Vendor 化 / 离线生成**<br>关键元数据已静态保存在 `vendor/cangjie-compiler/ASTKind.inc` 与 `crates/cj-diag/src/templates.rs`，日常构建与测试**完全不依赖**官方源码。 |
| **`cangjie_runtime`**<br>（官方运行时与标准库核心） | `https://gitcode.com/Cangjie/cangjie_runtime.git` | **标准库源码与跨文件跳转定义源**<br>提供 `core`、`collection`、`io` 等 30+ 官方核心标准库 `.cj` 源码。 | **按需动态下载 / 符号索引解耦**<br>通过 `tools/stdlib_download.py` 按需下载至 `~/.cangjie-lsp/std/<version>/`，并通过 `tools/stdlib_index.py` 构建为离线 `index.json`，LSP 运行时零网络查表跳转。 |
| **`cangjie_stdx`**<br>（官方扩展库） | `https://gitcode.com/Cangjie/cangjie_stdx.git` | **扩展标准库源码**<br>包含官方网络、加密、扩展包等。 | **按需动态下载**<br>与 `cangjie_runtime` 类似，按需获取并在只读路径建立索引。 |
| **`cangjie_tools`**<br>（官方工具链与语言服务） | `https://gitcode.com/Cangjie/cangjie_tools.git` | **LSP 协议与输出格式对照参考**<br>用于权威对比 `Hover` Markdown 拼接、`SemanticTokens` 编码顺序等行为。 | **只读设计参考**<br>纯技术设计与协议格式对照，代码层零物理依赖。 |
| **仓颉官方 SDK**<br>（官方发布二进制包） | `https://cangjie-lang.cn/download` | **宏动态编译展开运行时**<br>当且仅当需要执行复杂的 `@Macro` 真实调用并加载 `.so` / `.dll` 时作为宿主编译器。 | **环境变量动态发现**<br>优先读取 `CANGJIE_HOME` 或遍历系统 `PATH` 查找 `cjc`。未安装 SDK 时，宏分析平滑退化为静态语法分析，不影响基础开发与单测。 |

---

## 3. 仓颉标准库支持与跳转机制

仓颉官方预编译 SDK 中仅分发 LLVM Bitcode (`.bc`)，默认不包含 `.cj` 源码文件。为了在 IDE 中实现对标准库类型与函数（如 `Array`、`String`、`println`）的代码补全、文档悬停与“跳转到定义（Go to Definition）”，项目实现了以下机制：

1. **离线静态符号表 (`crates/cj-lsp/src/hover.rs`)**：
   - 内置了常用核心类型与操作的保底静态签名，即使无外部标准库也能提供基本悬停。
2. **动态标准库源码索引 (`tools/stdlib_download.py` & `stdlib_index.py`)**：
   - 工具自动从官方 `cangjie_runtime` 拉取对应版本的源码至本地缓存目录 `~/.cangjie-lsp/std/<version>/`；
   - 提取全局符号生成 `index.json`（包含 8000+ 符号坐标：文件、行号、列号、类别）；
   - LSP 服务器启动时自动加载该索引，跳转命中时返回 `file://~/.cangjie-lsp/std/...` 绝对 URI；
3. **只读保护 (`vscode-cangjie/`)**：
   - VSCode 扩展自动将下载的 `~/.cangjie-lsp/std/**` 注册入 `files.readonlyInclude`，防止用户误修改标准库源码。

---

## 4. 依赖配置与环境变量总览

| 环境变量 | 作用 | 默认回退行为 |
| :--- | :--- | :--- |
| `CANGJIE_HOME` | 指定仓颉官方 SDK 根目录（用于宏展开与官方编译器调用） | 自动在系统 `PATH` 中探测 `cjc` 路径；探测失败则不启用动态宏展开 |
| `CANGJIE_TEST_BASE` | 指定官方 HLT 测试套件根目录 | 优先探测 `tests/cangjie_test` 子模块，再探测上级同级目录 `../cangjie_test` |
| `CANGJIE_LLT_DIR` | 指定官方 LLT 编译器前端测试集目录 | 优先使用 `tests/cangjie_test/testsuites/LLT/compiler` |
| `CJ_FRONTEND` | 指定被测 `cj-frontend` 二进制路径 | 默认为工作区编译产物 `target/debug/cj-frontend` |
| `CANGJIE_LSPSERVER` | 指定 `LSPServer` 二进制路径 | 默认为工作区产物 `target/debug/LSPServer` 或插件内置二进制 |
