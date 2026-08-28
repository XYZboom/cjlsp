# cj-lang

用 Rust 从零实现的**仓颉（Cangjie）语言**编译器前端与 LSP 服务器。以通过官方测试仓
`cangjie_test` 的相关检查为验收标准（**不做后端 codegen**）。

- **编译前端**：词法 / 语法 / 语义分析（`cj-frontend` 二进制，cjc 兼容诊断格式）
- **LSP 服务器**：标准 LSP over stdio（JSON-RPC 2.0 + Content-Length 帧），二进制
  必须命名为 `LSPServer`（`cj-lsp` crate）
- **VSCode 扩展**：`vscode-cangjie/`，薄客户端，随 VSIX 捆绑各平台 `LSPServer`

## 仓库结构

```
├── crates/                    # Rust workspace，8 个 crate
│   ├── cj-lexer               # 词法分析
│   ├── cj-ast                 # AST（由官方 ASTKind.inc 生成，勿手改）
│   ├── cj-parser              # 语法分析
│   ├── cj-cfg                 # 轻量控制流图（自研，不抄官方 CHIR）
│   ├── cj-sema                # 语义分析（含宏动态加载 dylib.rs）
│   ├── cj-diag                # 诊断（SCAN 格式逐字符对齐官方）
│   ├── cj-frontend            # 前端 CLI（cjc-frontend 兼容，--dump-ast）
│   └── cj-lsp                 # LSP 服务器（产物：LSPServer / LSPServer.exe）
├── vscode-cangjie/            # VSCode 扩展（见其 README）
├── vendor/cangjie-compiler/   # 官方源码引用（只读参考，vendor 化）
├── tools/                     # ci.sh、覆盖率/对齐/宏 E2E 等脚本
└── default_AST/               # 官方默认 AST 快照（对齐基线）
```

## 构建

### 前置条件

- Rust 工具链（`rustup` + `cargo`；macOS 原生构建另需 Xcode CLT，Windows 本机构建
  另需 MSVC 或 GNU 工具链）
- 跨平台宿主 Linux 上交叉编译 Windows 产物需额外安装（一次性）：

```bash
rustup target add x86_64-pc-windows-gnu        # rustup 目标
sudo apt install gcc-mingw-w64-x86-64          # 链接器（x86_64-w64-mingw32-gcc）
```

### Linux（宿主）构建

```bash
cd <仓库根目录>
cargo build --release -p cj-lsp -p cj-frontend
```

### Windows 交叉编译（Linux host）

```bash
cd <仓库根目录>
cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend
```

> 说明：`cj-lsp` 与 `cj-frontend` 两个 crate 拉入全部依赖（含宏动态加载
> `cj-sema/dylib.rs` 的 `cfg(windows)` 实现），因此交叉编译即覆盖全工作区。
> 如需整工作区校验，可换 `cargo build --target x86_64-pc-windows-gnu`。
> Windows 宏包的编译与 LSP 动态加载（LoadLibraryW/GetProcAddress）细节见
> [docs/windows-macros.md](docs/windows-macros.md)。

### 产物

| 目标平台 | 命令 | 产物 |
|---|---|---|
| Linux（宿主） | `cargo build --release -p cj-lsp -p cj-frontend` | `target/release/LSPServer`、`target/release/cj-frontend` |
| Windows（交叉编译） | `cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend` | `target/x86_64-pc-windows-gnu/release/LSPServer.exe`、`…/cj-frontend.exe` |

Windows 上对应本机构建命令为 `cargo build --release -p cj-lsp -p cj-frontend`（宿主
为 Windows 时，产物名同样为 `LSPServer.exe` / `cj-frontend.exe`）。

### VSCode 扩展捆绑

`vscode-cangjie` 扩展随 VSIX 捆绑各平台的 `LSPServer` 二进制（`bin/linux/LSPServer`、
`bin/win32/LSPServer.exe`），安装后开箱即用、零配置。插件按运行平台自动选择
`bin/<platform>/LSPServer`；仅当想使用自建服务器时才需手动设置
`cangjie.lsp.serverPath` 或环境变量 `CANGJIE_LSPSERVER`。发布流程见
[vscode-cangjie 的 docs/vscode-publish.md](vscode-cangjie/docs/vscode-publish.md) 与
其 [README](vscode-cangjie/README.md)。

## 开发：CI 管道（tools/ci.sh）

仓库根目录的 `tools/ci.sh` 是完整 CI 管道（T45 起含**全平台门禁**），推送前跑一次
即可全部验证。脚本退出非零即失败，CI 据此提前终止。

```bash
cd <仓库根目录>
./tools/ci.sh            # 完整管道
./tools/ci.sh -j 12      # 并行构建（12 核；-h 显示用法）
CI=1 ./tools/ci.sh       # 非交互（行为相同）
```

### 13 步门禁总览

| # | 步骤 | 命令 | 说明 |
|---|---|---|---|
| 1 | 格式检查 | `cargo fmt --all --check` | 不允许有格式差异 |
| 2 | Clippy | `cargo clippy --workspace -- -D warnings` | 警告即错误 |
| 3 | 单元 / 集成测试 | `cargo test --workspace` | 输出通过用例计数 |
| 4 | LSP 诊断覆盖率 | `python3 tools/lsp_cov.py` | 覆盖率不得低于 75% |
| 5 | 宏展开 E2E | `python3 tools/test_macro_e2e.py` | 未解析宏被正确报告 |
| 5b | 宏预览 note E2E | `python3 tools/test_macro_preview.py` | 展开后的代码预览 note |
| 6 | SCAN Parser 对齐 | `python3 tools/scan_compare.py --dir "$SCAN_DIR"` | 与官方 LLT Parser 用例逐字符对齐 |
| 7 | LSP 功能用例 | `python3 tools/run_feature_cases.py` | completion + hover；默认 smoke（`--limit 2`），`FEATURE_FULL=1` 跑全量 |
| 8 | Linux debug 构建 | `cargo build --workspace` | 全工作区 |
| 8b | Linux release 构建 | `cargo build --release --workspace` | 全工作区 |
| 8c | Windows debug 交叉编译 | `cargo build --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend` | 仅当 Windows 工具链就绪 |
| 8d | Windows release 交叉编译 | `cargo build --release --target x86_64-pc-windows-gnu -p cj-lsp -p cj-frontend` | 仅当 Windows 工具链就绪 |
| 8e | Windows target Clippy | `cargo clippy --workspace --target x86_64-pc-windows-gnu -- -D warnings` | 仅当 Windows 工具链就绪 |

**Windows 交叉编译门槛**：步骤 8c–8e 仅在同时满足下述条件时执行（否则打印
`SKIP  windows-gnu cross-build ...` 并继续，保证 Linux-only 开发机也能跑通管道）：

- `command -v x86_64-w64-mingw32-gcc` 存在（即已 `apt install gcc-mingw-w64-x86-64`）
- `rustup target list --installed` 含 `x86_64-pc-windows-gnu`

**环境变量**：`SCAN_DIR`（步骤 6 的用例目录，默认指向本机官方测试仓）、
`FEATURE_FULL=1`（步骤 7 跑全量功能用例而非 smoke）。

> 已知良性提示：Windows debug 链接时 mingw ld 可能打印
> `corrupt .drectve at end of def file`（release 链接不出现），是 ld 的误报，
> 不影响产物，ci.sh 中该步骤不因 warning 失败。

## 硬性规则

- AST 节点类型由官方 `ASTKind.inc` 生成（`tools/gen_ast.py`），不要手改节点；改则
  改生成器后重新生成。
- Rust 最佳实践：clippy `-D warnings` 零警告、不 panic、高性能（避免不必要
  分配/克隆）。
- 每改一处先小批验证再叠加；完成一个任务必须 `cargo build` + `cargo test -p <crate>`
  + `cargo clippy -D warnings` 全绿后 git 提交。

## License

Apache-2.0（workspace package license）。VSCode 扩展单独为 MIT，见
`vscode-cangjie/LICENSE`。