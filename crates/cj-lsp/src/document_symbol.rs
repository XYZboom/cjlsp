// cj-lsp: textDocument/documentSymbol — file outline / symbol tree.
//
// Walks the parsed File (file.decls) and emits LSP DocumentSymbol[]:
//   { name, kind, detail, range, selectionRange, children? }
// CodePos is 1-based; LSP positions are 0-based, so every span is shifted -1.
// Kind mapping (LSP SymbolKind): class=5, struct=23, interface=11, enum=10,
// func=12, var=13, prop=14; extend & type-alias use 19 (Object); enum bare
// cases = 13 (Variable), payload cases = 12 (Function). Container members
// (class/struct/interface/extend) become nested children; enum cases become
// children of the enum. Range covers the whole declaration (from the leading
// keyword to the matching closing brace); selectionRange is the name span.

use cj_ast::{Body, Decl, EnumCase, File, Type, TypeParam};
use cj_lexer::{Lexer, Token, TokenKind};
use serde_json::{json, Value};

/// LSP SymbolKind constants used by this module.
mod kind {
    pub const CLASS: u32 = 5;
    pub const ENUM: u32 = 10;
    pub const INTERFACE: u32 = 11;
    pub const FUNC: u32 = 12;
    pub const VAR: u32 = 13;
    pub const PROP: u32 = 14;
    pub const OBJECT: u32 = 19; // extend / type alias
    pub const STRUCT: u32 = 23;
}

/// Build the DocumentSymbol[] array for a parsed file.
pub fn document_symbols(file: &File, source: &str) -> Value {
    let toks = Lexer::new(source).tokenize();
    let out: Vec<Value> = file
        .decls
        .iter()
        .filter_map(|d| decl_to_symbol(d, None, &toks))
        .collect();
    json!(out)
}

/// One DocumentSymbol object. `children` key is omitted when empty (matches
/// the official server, which drops the key for leaf symbols).
fn symbol(
    name: &str,
    kind: u32,
    detail: &str,
    range: &Value,
    selection_range: &Value,
    children: Vec<Value>,
) -> Value {
    let mut s = json!({
        "name": name,
        "kind": kind,
        "detail": detail,
        "range": range,
        "selectionRange": selection_range,
    });
    if !children.is_empty() {
        s["children"] = json!(children);
    }
    s
}

/// Convert a 1-based CodePos span to a 0-based LSP range.
fn range_from(cp: cj_ast::CodePos) -> Value {
    json!({
        "start": {"line": cp.line - 1, "character": cp.col - 1},
        "end": {"line": cp.end_line - 1, "character": cp.end_col - 1},
    })
}

/// Start position of a CodePos (1-based -> 0-based line/character pair).
fn start_pos(cp: cj_ast::CodePos) -> Value {
    json!({"line": cp.line - 1, "character": cp.col - 1})
}

/// LSP position of a token's end (1-based -> 0-based).
fn lsp_pos(p: &cj_lexer::Position) -> Value {
    json!({"line": p.line - 1, "character": p.column - 1})
}

/// Full range of a braced decl: from the FIRST leading modifier token (or the
/// keyword when none — official starts at `open`/`public`/`static` for
/// `open class A {}`) to the end of the matching closing `}` (found via the
/// token stream). Falls back to the keyword span when no closing brace exists.
fn braced_range(keyword_pos: cj_ast::CodePos, toks: &[Token]) -> Value {
    let start = decl_start(keyword_pos, toks);
    match braced_close(toks, keyword_pos.offset) {
        Some(close) => json!({"start": start, "end": lsp_pos(&close.end)}),
        None => json!({"start": start, "end": {
            "line": keyword_pos.end_line - 1, "character": keyword_pos.end_col - 1
        }}),
    }
}

/// Start position (0-based) of a decl: walk backwards over any leading
/// modifier tokens (`open class`, `public static func`, `mut func` …) so the
/// range begins at the modifier, mirroring the official server.
fn decl_start(keyword_pos: cj_ast::CodePos, toks: &[Token]) -> Value {
    let Some(i) = toks
        .iter()
        .position(|t| t.begin.offset == keyword_pos.offset)
    else {
        return start_pos(keyword_pos);
    };
    let mut start = start_pos(keyword_pos);
    for t in toks[..i].iter().rev() {
        if is_modifier(t.kind) {
            start = json!({"line": t.begin.line - 1, "character": t.begin.column - 1});
        } else {
            break;
        }
    }
    start
}

/// Tokens that can prefix a decl and should be included in its range start:
/// visibility / member modifiers (public/private/protected/internal, static,
/// abstract, sealed, open, override, redef, mut, common/specific/features).
fn is_modifier(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::STATIC
            | TokenKind::PUBLIC
            | TokenKind::PRIVATE
            | TokenKind::INTERNAL
            | TokenKind::PROTECTED
            | TokenKind::OVERRIDE
            | TokenKind::REDEF
            | TokenKind::ABSTRACT
            | TokenKind::SEALED
            | TokenKind::OPEN
            | TokenKind::MUT
            | TokenKind::COMMON
            | TokenKind::SPECIFIC
            | TokenKind::FEATURES
    )
}

/// Find the closing `}` token of a braced decl whose body starts after the
/// token at `offset`. Param lists / generic args (paren/bracket depth) are
/// skipped so the first `{` at depth 0 is the body, then braces are matched.
fn braced_close(toks: &[Token], offset: usize) -> Option<&Token> {
    let start = toks.iter().position(|t| t.begin.offset == offset)?;
    let mut paren = 0i32;
    let mut i = start + 1;
    let mut body: Option<usize> = None;
    while let Some(t) = toks.get(i) {
        match t.kind {
            TokenKind::LCURL if paren == 0 => {
                body = Some(i);
                break;
            }
            TokenKind::LPAREN | TokenKind::LSQUARE => paren += 1,
            TokenKind::RPAREN | TokenKind::RSQUARE => paren = (paren - 1).max(0),
            _ => {}
        }
        i += 1;
    }
    let body = body?;
    let mut brace = 1i32;
    for t in toks.iter().skip(body + 1) {
        match t.kind {
            TokenKind::LCURL => brace += 1,
            TokenKind::RCURL => {
                brace -= 1;
                if brace == 0 {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// Range of a bodyless decl (abstract/interface func, type alias): from the
/// leading modifier/keyword to the end of the last token on its line — the
/// official server spans the whole signature line for bodyless funcs.
fn signature_range(keyword_pos: cj_ast::CodePos, toks: &[Token]) -> Value {
    let start = decl_start(keyword_pos, toks);
    match line_end(toks, keyword_pos.offset) {
        Some(t) => json!({"start": start, "end": lsp_pos(&t.end)}),
        None => json!({"start": start, "end": {
            "line": keyword_pos.end_line - 1, "character": keyword_pos.end_col - 1
        }}),
    }
}

/// The last token of a single-line decl (bodyless func, type alias): the last
/// token before the next newline / semicolon / closing brace at depth 0.
fn line_end(toks: &[Token], offset: usize) -> Option<&Token> {
    let start = toks.iter().position(|t| t.begin.offset == offset)?;
    let mut depth = 0i32;
    let mut last: Option<&Token> = None;
    for t in toks.iter().skip(start + 1) {
        match t.kind {
            TokenKind::NL | TokenKind::SEMI => {
                if depth == 0 {
                    return last;
                }
            }
            TokenKind::RCURL if depth == 0 => return last,
            TokenKind::LPAREN | TokenKind::LSQUARE | TokenKind::LCURL => {
                depth += 1;
                last = Some(t);
            }
            TokenKind::RPAREN | TokenKind::RSQUARE | TokenKind::RCURL => {
                depth = (depth - 1).max(0);
                last = Some(t);
            }
            _ => last = Some(t),
        }
    }
    last
}

/// The token right after the token at `offset` (used to locate a name that
/// the AST stores position-less, e.g. prop / type-alias names).
fn next_token(toks: &[Token], offset: usize) -> Option<&Token> {
    let i = toks.iter().position(|t| t.begin.offset == offset)?;
    toks.get(i + 1)
}

/// The matching `)` of a parenthesized enum payload starting at `offset`.
fn paren_end(toks: &[Token], offset: usize) -> Option<&Token> {
    let i = toks.iter().position(|t| t.begin.offset == offset)?;
    let mut depth = 0i32;
    for t in toks.iter().skip(i + 1) {
        match t.kind {
            TokenKind::LPAREN => depth += 1,
            TokenKind::RPAREN => {
                depth -= 1;
                if depth == 0 {
                    return Some(t);
                }
            }
            _ => {}
        }
    }
    None
}

/// `<T, U>` suffix for generic decl names (empty when no type params).
fn type_params_str(tps: &[TypeParam]) -> String {
    if tps.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            tps.iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Modifier detail of a func/member: "constructor" for ctors, else
/// static/public/abstract joined with ", " (empty when none — official
/// renders plain funcs with detail "").
fn func_detail(
    name: &str,
    container: Option<&str>,
    is_public: bool,
    is_static: bool,
    is_abstract: bool,
) -> String {
    if name == "init" || container == Some(name) {
        return "constructor".to_string();
    }
    let mut parts: Vec<&str> = Vec::new();
    if is_static {
        parts.push("static");
    }
    if is_public {
        parts.push("public");
    }
    if is_abstract {
        parts.push("abstract");
    }
    parts.join(", ")
}

/// Detail of a type container: "class"/"class, public"/"struct"/... —
/// interface & enum render "" matching the official server.
fn container_detail(label: &str, is_public: bool) -> String {
    if is_public {
        format!("{label}, public")
    } else {
        label.to_string()
    }
}

/// Recursively convert a decl to a DocumentSymbol. `container` is the
/// enclosing type's name (for constructor detection inside class bodies),
/// None for top-level decls.
fn decl_to_symbol(d: &Decl, container: Option<&str>, toks: &[Token]) -> Option<Value> {
    match d {
        Decl::Class {
            name,
            name_pos,
            is_public,
            type_params,
            members,
            pos,
            ..
        } => {
            let children = children_of(members, Some(name), toks);
            let full_name = format!("{name}{}", type_params_str(type_params));
            Some(symbol(
                &full_name,
                kind::CLASS,
                &container_detail("class", *is_public),
                &braced_range(*pos, toks),
                &range_from(*name_pos),
                children,
            ))
        }
        Decl::Struct {
            name,
            name_pos,
            is_public,
            type_params,
            members,
            pos,
            ..
        } => {
            let children = children_of(members, Some(name), toks);
            let full_name = format!("{name}{}", type_params_str(type_params));
            Some(symbol(
                &full_name,
                kind::STRUCT,
                &container_detail("struct", *is_public),
                &braced_range(*pos, toks),
                &range_from(*name_pos),
                children,
            ))
        }
        Decl::Interface {
            name,
            name_pos,
            type_params,
            members,
            pos,
            ..
        } => {
            let children = children_of(members, Some(name), toks);
            let full_name = format!("{name}{}", type_params_str(type_params));
            Some(symbol(
                &full_name,
                kind::INTERFACE,
                "",
                &braced_range(*pos, toks),
                &range_from(*name_pos),
                children,
            ))
        }
        Decl::Enum {
            name,
            name_pos,
            type_params,
            cases,
            pos,
            ..
        } => {
            let children: Vec<Value> = cases
                .iter()
                .filter_map(|c| enum_case_symbol(c, toks))
                .collect();
            let full_name = format!("{name}{}", type_params_str(type_params));
            Some(symbol(
                &full_name,
                kind::ENUM,
                "",
                &braced_range(*pos, toks),
                &range_from(*name_pos),
                children,
            ))
        }
        Decl::Extend {
            target,
            members,
            pos,
            ..
        } => {
            let children = children_of(members, container, toks);
            let target_name = render_type(target);
            // Official selectionRange = the extended type's own span.
            let sel = range_from(type_pos(target));
            Some(symbol(
                &target_name,
                kind::OBJECT,
                "extend",
                &braced_range(*pos, toks),
                &sel,
                children,
            ))
        }
        Decl::TypeAlias {
            name,
            type_params,
            pos,
            ..
        } => {
            let full_name = format!("{name}{}", type_params_str(type_params));
            // TypeAlias has no name_pos; the name is the token after `type`.
            // Official renders type aliases with range == name span (like vars).
            let sel = try_next_name_range(*pos, toks);
            Some(symbol(
                &full_name,
                kind::OBJECT,
                "type alias",
                &sel,
                &sel,
                Vec::new(),
            ))
        }
        Decl::Func {
            name,
            name_pos,
            is_public,
            is_static,
            is_abstract,
            type_params,
            params,
            ret,
            body,
            pos,
        } => {
            let detail = func_detail(name, container, *is_public, *is_static, *is_abstract);
            let _ = (type_params, params, ret);
            // Bodyless (abstract/interface) funcs have no closing brace — the
            // official range ends at the last token of the signature line.
            let range = match body {
                Body::Block(_) => braced_range(*pos, toks),
                Body::Empty => signature_range(*pos, toks),
            };
            Some(symbol(
                name,
                kind::FUNC,
                &detail,
                &range,
                &range_from(*name_pos),
                Vec::new(),
            ))
        }
        Decl::Main { pos, body } => {
            // bare `main()` entry — official renders it as a func named "main"
            let range = match body {
                Body::Block(_) => braced_range(*pos, toks),
                Body::Empty => signature_range(*pos, toks),
            };
            Some(symbol(
                "main",
                kind::FUNC,
                "",
                &range,
                &range_from(*pos),
                Vec::new(),
            ))
        }
        Decl::Var {
            name, name_pos, ty, ..
        } => {
            let detail = ty.as_ref().map(render_type).unwrap_or_default();
            let sel = range_from(*name_pos);
            Some(symbol(name, kind::VAR, &detail, &sel, &sel, Vec::new()))
        }
        Decl::VarWithPattern {
            pattern,
            ty,
            init,
            pos,
        } => {
            // destructuring `var (a, b) = ...` — no single name; fall back to
            // the first declared name for outline usefulness.
            let name = pattern_first_name(pattern).unwrap_or("_");
            let _ = (ty, init);
            let sel = range_from(*pos);
            Some(symbol(name, kind::VAR, "", &sel, &sel, Vec::new()))
        }
        Decl::Prop { name, ty, pos, .. } => {
            // Prop has no name_pos; the name is the token after `prop`.
            let sel = match next_token(toks, pos.offset) {
                Some(t) => json!({
                    "start": lsp_pos(&t.begin),
                    "end": lsp_pos(&t.end),
                }),
                None => range_from(*pos),
            };
            Some(symbol(
                name,
                kind::PROP,
                &render_type(ty),
                &braced_range(*pos, toks),
                &sel,
                Vec::new(),
            ))
        }
        Decl::PrimaryCtor {
            is_public,
            params,
            pos,
            ..
        } => {
            let _ = (is_public, params);
            Some(symbol(
                "init",
                kind::FUNC,
                "constructor",
                &braced_range(*pos, toks),
                &range_from(*pos),
                Vec::new(),
            ))
        }
        // Builtin / FuncParam / GenericParam / Package / MacroExpand / Invalid
        // carry no outline value — skip.
        _ => None,
    }
}

/// Children of a container body: every member decl rendered recursively.
fn children_of(members: &[Decl], container: Option<&str>, toks: &[Token]) -> Vec<Value> {
    members
        .iter()
        .filter_map(|m| decl_to_symbol(m, container, toks))
        .collect()
}

/// Enum case -> symbol. Bare cases (no payload) are Variable(13) with name =
/// the case name; payload cases are Function(12) with name `Case(T1, T2)`.
fn enum_case_symbol(c: &EnumCase, toks: &[Token]) -> Option<Value> {
    if c.payloads.is_empty() {
        let sel = range_from(c.pos);
        return Some(symbol(
            &c.name,
            kind::VAR,
            "public, constructor",
            &sel,
            &sel,
            Vec::new(),
        ));
    }
    let payloads: Vec<String> = c.payloads.iter().map(render_type).collect();
    let full_name = format!("{}({})", c.name, payloads.join(", "));
    let range = match paren_end(toks, c.pos.offset) {
        Some(rparen) => json!({
            "start": start_pos(c.pos),
            "end": lsp_pos(&rparen.end),
        }),
        None => range_from(c.pos),
    };
    Some(symbol(
        &full_name,
        kind::FUNC,
        "public, constructor",
        &range,
        &range_from(c.pos),
        Vec::new(),
    ))
}

/// First identifier name in a destructuring pattern (or None).
fn pattern_first_name(p: &cj_ast::Pattern) -> Option<&str> {
    use cj_ast::Pattern;
    match p {
        Pattern::Var { name, .. } => Some(name),
        // enum ctor pattern (`Foo(a)` on the LHS of `var Foo(x) = ...`) binds
        // through its args — recurse into them.
        Pattern::VarOrEnum { args, .. } | Pattern::Enum { args, .. } => {
            if args.is_empty() {
                None
            } else {
                args.iter().find_map(|a| pattern_first_name(a))
            }
        }
        Pattern::Tuple { elements, .. } => elements.iter().find_map(|e| pattern_first_name(e)),
        _ => None,
    }
}

/// Range for a position-less name: the token right after `pos` (e.g. the
/// identifier after `type`), falling back to the keyword's own span.
fn try_next_name_range(pos: cj_ast::CodePos, toks: &[Token]) -> Value {
    match next_token(toks, pos.offset) {
        Some(t) => json!({
            "start": lsp_pos(&t.begin),
            "end": lsp_pos(&t.end),
        }),
        None => range_from(pos),
    }
}

/// Render a type name (reused from hover — same string official shows in
/// detail for vars/props: e.g. "Int64", "Array<Int64>", "(Int32) -> Int32").
fn render_type(t: &Type) -> String {
    crate::hover::render_type(t)
}

/// The CodePos of a type node (every Type variant carries a `pos` span).
fn type_pos(t: &Type) -> cj_ast::CodePos {
    match t {
        Type::Ref { pos, .. }
        | Type::Qualified { pos, .. }
        | Type::Option { pos, .. }
        | Type::Constant { pos, .. }
        | Type::VArray { pos, .. }
        | Type::Primitive { pos, .. }
        | Type::Paren { pos, .. }
        | Type::Func { pos, .. }
        | Type::Tuple { pos, .. }
        | Type::This(pos)
        | Type::Invalid(pos) => *pos,
    }
}
