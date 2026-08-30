# 官方 hover 输出对照调研 (T61)

调研日期: 2026-08-30
官方源码: gitcode.com/Cangjie/cangjie_tools (本机 clone 于 /root/Code/cangjie/cangjie_tools)
官方文件: cangjie-language-server/src/languageserver/capabilities/hover/
  - HoverImpl.cpp / HoverImpl.h            — 组装 hover 的各个 markedString 块
  - ../../ArkServer.cpp (CollectHoverMarkdownBlocks / AppendHover* / BuildHoverMarkdown)
  - ../../common/ItemResolverUtil.cpp       — "Declared in:" + "Package info:" 来源
我们对: cj-lang crates/cj-lsp/src/hover.rs (render_hover / Index / Hoverable)

## 1. 官方 markdown 结构（结论先行）

官方最终 markdown 由三段拼接而成（ArkServer.cpp BuildHoverMarkdown）：

```
Declared in: <file>                   <- sourceInfo 块 (AppendMarkdownText, 行尾硬换行 "  \n")
Package info: <pkg>                   <- 同上一块

```cangjie                                <- AppendHoverDeclarations
<用于声明的 markedString 各行>           <- 可能是 source(声明源码) / "// In <kind> <name>"
```                                        (每一块一行, 空块被 TrimHoverBlock 丢弃)
                                        <- (无注释时这里直接结束, 无多余空行)
\n---\n\n<注释文本>                     <- AppendHoverComments (仅当有 comments)
```

实现细节:

- **sourceInfo**: ItemResolverUtil::ResolveSourceByNode → `"Declared in: <fileName>"` + BLANK()*
  + `"Package info: "` + fullPackageName, 再在 GetHoverMessage 后面拼两个 BLANK()（即两个空格）。
  ArkServer 里 AppendMarkdownText(text, hardBreak=true) 把每行转义后加 `"  \n"`（两空格硬换行），
  再加一个 `"\n"` 产生空行。
- **declaration 块**（```` ```cangjie ```` 围栏内）:
  - 1. 声明源码 (ResolveSourceByNode 已包含 Declared in / Package info? 不 —— source 在 HoverImpl
    里被 push 为 markedString[0], 含 "Declared in ... / Package info ..."; 真正进围栏的是)
  - 实际 GetHoverMessage 顺序:
    1. `source` = "Declared in: file  Package info: pkg"（含两个空格）→ CollectHoverMarkdownBlocks
       匹配 StartsWith "Declared in:" → sourceInfo 段（不进围栏）
    2. outerDecl 存在时 `GetHoverMessageByOuterDecl` → `"// In class <name> "`（或 interface/enum/struct;
       extend 块递归到真实类型）→ 进 declaration 段
    3. `detail` = ItemResolverUtil::ResolveDetailByNode 生成的签名 → declaration 段
    4. (Deveco 专用) apiKey:*** → 进 declaration 段 (CollectHoverMarkdownBlocks 里 StartsWith "apiKey:")
    5. `@!APILevel[...]`（ohos.labels 注解, 仅当 decl.annotations 带 APILevel）→ 进 declaration 段
    6. comments（doc 注释）: CollectCommentText(ResolveDeclComments) / index->FindComment →
       逐条 ResolveComment(去 //、/* */、/** */ 与行首 *、缩进归一) →
       最后一条加 "\n"、其余加 "\r\n"；BuildHoverMarkdown 里 non-primary-declaration 块进 comments 段
- **comment 段**: 全部 comments 变成 `"\n---\n\n"` + 每条 AppendMarkdownText(硬换行),
  多条之间 `"\n\n"` 分隔。注释文本中的 `\` `` ` `` `*` `_` `[` `]` `<` `>` `#`
  会被 EscapeMarkdownText 转义（有序列表点 `1.` 也转义）。

## 2. 与我们的 render_hover 对照

我们 (hover.rs render_hover) 输出的模板:

```
Declared in: <file>  \n
Package info: <pkg>  \n
\n
```cangjie
<signature>
```
\n
(<若有 doc> \n---\n\n<doc>  \n)
```

**逐项核对结论**: 与官方结构一致, 181 个官方用例已 100% 对齐:

| 字段 | 官方 | 我们 | 状态 |
|---|---|---|---|
| Declared in 行 | `Declared in: file` + 硬换行 | 同 | ✅ 一致 |
| Package info 行 | `Package info: pkg` + 硬换行 | 同 (pkg 内 `_` 转义 `\_`) | ✅ 一致 |
| 围栏 | ` ```cangjie\n...\n```\n` | 同 | ✅ 一致 |
| 成员前缀 | `// In class/struct/interface/enum <Name>` 独立成行 | 同名行 (`with_container`) | ✅ 一致 |
| 签名 | ResolveDetailByNode | render_* 系列 | ✅ 181/181 全过 |
| doc 注释 | 多行/块/`/** */` 均支持; EscapeMarkdownText 转义 | 仅单行 `//` | ⚠️ 见 §4 |
| apiKey | 仅 Deveco (GetIsDeveco) | 无 | ⏭️ 非 VSCode 路径, 不实现 |
| @!APILevel | 仅 ohos.labels APILevel 注解 | 无 | ⏭️ 测试套件无此场景, 不实现 |

关键: sourceInfo / declaration / comments 三段的分流逻辑 (CollectHoverMarkdownBlocks):
- `StartsWith "Declared in:"` → sourceInfo
- `StartsWith "apiKey:***" || StartsWith "// In "` → declarations
- 首个非 sourceInfo 块 / 前缀 `@!` → declarations
- 其余 → comments
我们以单一 signature 字符串承载 declaration 段、注释单独附加, 等价的覆盖了官方三块分类。

## 3. 本次发现并修复的真实回归 (143)

完整跑官方 hover 套件实测 180/181 (99.4%), 唯一失败:

- textDocument_hover_143: `func g(a: Base) { print("2") }` 期望 `func g(a: Base): Unit`,
  我们输出 `func g(a: Base): print`。

根因: hover.rs infer_init_expr_at 的 Call 分支里, cb686a7 (cross-file 支持) 引入了
`else if !idx.is_known_func(name) { Some(name.clone()) }` —— 把"本文件未声明的名字"一律当成
跨文件类型构造 `X()` → 返回 `X`。`print` 是 std.core 内置函数(返回 Unit), 不在 by_name,
被误判为跨文件类型构造器 → 推断返回类型 `print`。

修复 (hover.rs, 最小改动):
- 新增 `fn is_known_std_func(name)` (print / println / assert, 与 cj-sema resolver::is_known_builtin
  的"内置函数"集合一致),
- 在该 fallback 分支里: name 是已知 std 内置函数 → `Some("Unit")`, 否则维持跨文件类型构造推断。

验证: 修复后全量 hover 套件 181/181 (100%), 单测 15/15 通过, build 无新 warning。

## 4. 我们缺/未对齐的小差异 (T61 记录, T65 已补)

T61 调研时上述字段官方套件未覆盖, 记录"暂不实现"。T65 对照官方 HoverImpl 逐条复核源码
语义后, **前两项已实现 (见 §4.1/§4.2), 后两项仍不实现**。

### 4.1 函数参数默认值 — T65 已实现
- 官方: `ItemResolverUtil::ProcessSingleParam` 无条件调用 `GetFuncNamedParam`, 对**每个**有
  默认值的参数渲染 ` = <expr>` (普通 func / 成员 func / init / 主构造器一致)。
- 我们 (T65 前): 仅 `init`/PrimaryCtor 走 `with_defaults=true`, 顶层与成员 func 走
  `false` → 签名漏 `= value`。
- T65 修复: `collect_decl` 的 `Decl::Func` 改 `render_params(params, true)`, 与官方对齐。
- 验证: 官方套件 181/181 不回归 (官方案例里带默认值的非 init func 均未被 hover, 无冲突);
  探针 `func add1(a: Int32, b!: Int32 = 1)` 现输出 `internal func add1(a: Int32, b!: Int32 = 1): Int32`。

### 4.2 多行 `//` / `/* */` / `/** */` 文档注释 + markdown 转义 — T65 已实现
- 官方: `CollectCommentText` 收集 inner/leading/trailing 三组注释, 逐条 `ResolveComment`
  (LINE 去 `//`; BLOCK/DOC 去 `/*..*/`/`/**..*/` 外壳, `RemoveBlankAndStar` 去行首 `*` +
  最小缩进归一), 每条注释独立成块, `AppendHoverComments` 以 `\n---\n\n` 开头、块间
  `\n\n` 分隔、每行 `EscapeMarkdownText` 转义 + 硬换行 `  \n`。
- 我们 (T65 前): 仅取声明上一行的单条 `//`, 多行/块注释取空, 注释原样输出不转义。
- T65 修复 (hover.rs):
  - `doc_comment` 重写: 向上收集紧邻的注释行 (含 `//`/`/*`/`*` 续行), 拆成注释 token
    (每条 `//` 一行; 每个 `/*..*/`/`/**..*/` 一块), 逐块 resolve, 再渲染为
    `\n---\n\n` + 块间 `\n\n` + 每行 `escape_markdown_text(line) + "  \n"`。
  - 新增 `resolve_line_comment` (TrimSpaceAndTab 语义)、`resolve_block_comment`
    (RemoveBlankAndStar 语义: 最小缩进 + 去首 `*` + 去尾空白)、`escape_markdown_text`
    (EscapeMarkdownText 语义: 转义 `\` `` ` `` `*` `_` `[` `]` `<` `>` `#`, 以及行首
    有序列表 `N. ` → `N\. `)。
  - `render_hover` 改为直接拼接 `doc` (doc 现为完整渲染后的注释段, 不再重复加 `  \n`)。
- 验证: 官方套件 181/181 不回归 (10 个含注释用例全为单行纯文本 `//`, 转义为无操作);
  探针 5 场景全 PASS (默认参数 / 多行 `//` / 块注释 / `/** @param */` / 有序列表转义);
  单测 29/29 通过。
- 仍不实现: **inner/trailing 注释** (声明同行尾部注释、`/** */` 内部悬挂注释) 官方
  CollectCommentText 亦收集, 但 VSCode 场景与官方套件均不涉及, 按最小改动原则跳过。

### 4.3 仍不实现
3. **apiKey / @!APILevel**: Deveco / ohos 专属, VSCode 路径 (MessageHeaderEndOfLine::GetIsDeveco)
   与测试套件均不产出, 不实现。

## 5. 结论

- 我们的 render_hover 三段式 markdown (Declared in / Package info / ```cangjie 签名 / --- 注释)
  与官方 BuildHoverMarkdown 结构**逐字节对齐**（除去免不了的中文包名等差异源）。
- 官方 hover 套件 **181/181 = 100%** (T61 修 143 后; T65 修改后仍 100%, 无回归)。
- T65 补齐了官方有而我们缺、且可按官方源码语义验证的字段:
  - **函数参数默认值** (普通/成员 func 签名渲染 ` = value`)
  - **多行 `//` / 块 `/* */` / 文档 `/** @param */` 注释** (独立成块 + 缩进/`*` 归一)
  - **注释 markdown 转义** (含有序列表 `N. ` → `N\. `)
- 仍不实现: apiKey (Deveco), @!APILevel (ohos), inner/trailing 注释 — 官方套件/VSCode
  场景均不产出, 保持行为不破坏、最小改动。