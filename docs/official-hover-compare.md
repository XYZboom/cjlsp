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

## 4. 我们缺/未对齐的小差异 (已确认官方套件未覆盖, 按"不破坏行为"原则暂不实现)

1. **多行 `//` / `/* */` / `/** */` 文档注释**: 官方 CollectCommentText 收集 inner/leading/trailing
   三组注释、逐条 ResolveComment (去前缀、去行首 *、最小缩进归一, 支持 `/** */`)并独立成段。
   我们只支持被 hover 声明上一行的单条 `//`, 多行/块注释会取到空。官方 181 例中注释全部是
   单行 `//`(10 个含注释的用例全过), 故不构成用例缺口; 若要支持 arkit 的 `/** @param */`
   参数文档格式需扩展 doc_comment。**依赖: 官方测试源里无 `/** */` 用例, 加了也测不到**, 且
   用户硬规则"最小必要改动、不破坏行为" → 记录不实现。
2. **注释文本 markdown 转义**: 官方 EscapeMarkdownText 对所有注释行转义 `\ * _ [] <>#` 等。
   我们 doc 原样输出。官方套件注释都是纯中文/短文本无特殊字符, 不影响 181 例。
3. **apiKey / @!APILevel**: Deveco / ohos 专属, VSCode 路径 (MessageHeaderEndOfLine::GetIsDeveco)
   与测试套件均不产出, 不实现。

## 5. 结论

- 我们的 render_hover 三段式 markdown (Declared in / Package info / ```cangjie 签名 / --- 注释)
  与官方 BuildHoverMarkdown 结构**逐字节对齐**（除去免不了的中文包名等差异源）。
- 修掉了 143 回归后, 官方 hover 套件 **181/181 = 100%**。
- 官方有而我们没有的字段: 多行/块/`/** */` 文档注释、注释 markdown 转义、apiKey(Deveco)、
  @!APILevel(ohos)。前两者官方套件无用例, 后两者非 VSCode 场景 —— 均按任务范围"保持行为不破坏、
  只补官方有而我们缺且可验证的字段"暂不实现, 记录于此。