# 官方 LSP 开源实现调研（Ark）

调研日期: 2026-08-29, gitcode API 实测确认

## ⚠️ 关键差异（用户提醒, 已确认）
**官方 LSP 服务器深度依赖官方编译器源码**:
- `LSPCompilerInstance.cpp` include `cangjie/Driver/TempFileManager.h` + `CjoManager.h`
  (CjoManager = 编译器对象管理器, TempFileManager = 编译器驱动)
- LSP 通过 LSPCompilerInstance 直接调用官方 cjc 的 Parser/Sema/SymbolTable
- **因此官方 LSP 不能直接复用其实现** —— 它是"编译器的一层薄壳"

而我们: cj-lang 是 **Rust 独立仓颉编译器前端** (自研 lexer/parser/sema),
按 spec 工作, 不照抄官方源码。

## 我们的取用策略
只对齐 **协议层 / user-visible 行为**, 不复用内部实现:
1. capability 清单 → 功能完整性参考 (我们知道还缺 signatureHelp 等)
2. SemanticTokensAdaptor → 对齐 semantic token 编码/type 顺序 (协议层)
3. hyperlangExtension → 参考 client 配置 (serverPath/启动参数/诊断开关)
4. 官方 hover/completion 的**输出格式** (markdown 结构) 可对比对齐
5. 官方 LSP 的 symbol index (sql/) → 印证我们的 stdlib index.json 方向

NOT 参考: LSPCompilerInstance / CjoManager / ArkAST* (官方编译器内部 API)

## 结论
- 官方 LSP: gitcode.com/Cangjie/cangjie_tools → cangjie-language-server (C++/Ark)
- 官方 VSCode 扩展: 同仓库 hyperlangExtension
- 我们继续独立实现, 用官方做**行为对照 + 协议对齐**, 不依赖其编译器

## 官方 capability 清单 (20 项) vs 我们
| Capability | 官方 | 我们 (cj-lang) | 备注 |
|---|---|---|---|
| completion | ✓ | ✓ 85.7% (138/161) | |
| hover | ✓ | ✓ 100% (181/181) | |
| definition | ✓ | ✓ 9/9 + 跨文件 | |
| diagnostic | ✓ | ✓ 97.6% | |
| reference | ✓ | ✓ | |
| semanticHighlight | ✓ | ✓ | SemanticTokensAdaptor.cpp 可对比 |
| documentHighlight | ✓ | ✗ | 待实现 |
| workspaceSymbol | ✓ | ✗ | 待实现 |
| documentSymbol | ✓ | ✗ | 待实现 |
| signatureHelp | ✓ | ✗ | 待实现 (用户常用) |
| codeLens | ✓ | ✗ | 待实现 |
| rename / prepareRename | ✓ | ✗ | 待实现 |
| callHierarchy / typeHierarchy | ✓ | ✗ | 待实现 |
| codeAction / refactor / fileRefactor | ✓ | ✗ | 待实现 |
| overrideMethods | ✓ | ✗ | 待实现 |
| breakpoints | ✓ | ✗ | 待实现 |

## 官方内部结构参考
- languageserver/src/languageserver/:
  - ArkLanguageServer.cpp / ArkServer.cpp — 主循环
  - capabilities/<20 目录> — 各 capability Impl.cpp
  - index/ — 符号索引
  - sql/ — SQLite 持久化符号索引 (与我们 stdlib index.json 方案一致)
  - DocCache.cpp — 文档缓存 (对比我们的 scan_cache)
  - LSPCompilerInstance.cpp — 编译器集成
- semanticHighlight/: SemanticHighlightImpl + SemanticTokensAdaptor (协议对比源)

## 下一步
1. 拉取 cangjie-language-server 源码到本机 /root/Code/cangjie 作权威参考
   (git clone https://gitcode.com/Cangjie/cangjie_tools.git)
2. 对比 SemanticTokensAdaptor: 对齐我们 semantic.rs 的 token type 顺序/编码
3. 实现缺失 capability: signatureHelp > documentSymbol > rename > workspaceSymbol
   (按用户价值排序)
4. hyperlangExtension 官方扩展: 参考其 client 配置 (serverPath/启动参数)
