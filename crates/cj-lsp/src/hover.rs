// cj-lsp: textDocument/hover + definition support.
//
// Given a cursor position, find the declaration (or the declaration a
// reference resolves to) whose name is under the cursor and return LSP Hover
// markdown:
//   Declared in: <file>  \nPackage info: <pkg>  \n\n```cangjie\n<signature>\n```\n
// plus, when the decl carries a doc comment:
//   \n---\n\n<comment>  \n
// and the range of the identifier under the cursor.
//
// Coverage (T18):
//   * top-level decls (class/interface/struct/enum/type-alias/func/var/main/macro)
//   * class/struct/interface members (funcs, vars, props, inits/primary ctors,
//     enum cases)
//   * local vars, params, local funcs and lambda params inside bodies
//   * reference resolution: value refs -> local/top-level decl, type refs ->
//     type decl, member access a.b -> member of a, ctor calls X() -> X's init,
//     this/super -> enclosing/parent type
//   * signature rendering matched to the official suite (internal visibility,
//     `<: A & B` parents for classes, `extends A, B` for interfaces, type
//     params `<T>`, param types, `var`/`let`, inferred initializer types)
//   * std.core symbols (String/Array/Exception/Any/...) with their own
//     "Declared in" file + `std.core` package

use cj_ast::{Body, CodePos, Decl, Expr, File, Pattern, PrimitiveKind, Type};
use serde_json::{json, Value};
use std::collections::HashMap;

// ================================================================
// Public entry
// ================================================================

/// Find the declaration or reference at (line, character) — LSP 0-based —
/// and return the LSP Hover result (or Value::Null when nothing is there).
pub fn hover_at(
    file: &File,
    source: &str,
    package: Option<&str>,
    file_name: &str,
    line: u32,
    character: u32,
) -> Value {
    let idx = Index::new(file, source, package, file_name);
    // 1) Hit-test the cursor against every known name span. The end is
    //    inclusive so a cursor just past an identifier still hits (official
    //    boundary behavior, e.g. `Row` when the cursor is on the `(` after it).
    if let Some(hi) = idx.hit_test(line, character) {
        return render_hover(hi, LspRange(hi.line - 1, hi.col - 1, hi.end_col - 1));
    }
    // 2) Reference resolution on the identifier word at the cursor.
    let Some(word) = word_at(source, line, character) else {
        return Value::Null;
    };
    if let Some(hi) = idx.resolve(&word.text, line, character, word.start) {
        return render_hover(hi, LspRange(word.line, word.start, word.end));
    }
    Value::Null
}

pub fn definition_at(file: &File, source: &str, uri: &str, line: u32, character: u32) -> Value {
    let idx = Index::new(file, source, None, "");
    // 1) The cursor is on a declared name span — jump straight there.
    // 2) Otherwise it's a use site: read the identifier word and resolve it
    //    LIGHTLY (type name -> lookup_type, value -> first by_name entry).
    //    We deliberately avoid the full `resolve()` (it runs overload /
    //    pipeline/this-super analysis for hover and can be too heavy on
    //    constructor calls) — definition only needs the declaration.
    let target = match idx.hit_test(line, character) {
        Some(hi) => Some(hi.name.clone()),
        None => match word_at(source, line, character) {
            Some(word) => Some(word.text),
            None => None,
        },
    };
    let Some(name) = target else {
        return Value::Null;
    };
    // A type name: jump to the type declaration (constructor calls resolve
    // through this, not to `init`).
    if let Some(hi) = idx.lookup_type(&name) {
        return json!({
            "uri": uri,
            "range": {
                "start": {"line": hi.line - 1, "character": hi.col - 1},
                "end": {"line": hi.line - 1, "character": hi.end_col - 1},
            }
        });
    }
    // A value / function: first by_name entry is the declaration.
    if let Some(hi) = idx
        .by_name
        .get(&name)
        .and_then(|v| v.first())
        .map(|&i| &idx.all[i])
    {
        return json!({
            "uri": uri,
            "range": {
                "start": {"line": hi.line - 1, "character": hi.col - 1},
                "end": {"line": hi.line - 1, "character": hi.end_col - 1},
            }
        });
    }
    // A local variable / parameter / lambda binding (e.g. `ii1.a1` on a local
    // `var ii1`): resolve via the scope-aware locals map.
    if let Some(hi) = idx.lookup_local(&name, line, character) {
        return json!({
            "uri": uri,
            "range": {
                "start": {"line": hi.line - 1, "character": hi.col - 1},
                "end": {"line": hi.line - 1, "character": hi.end_col - 1},
            }
        });
    }
    Value::Null
}
// ================================================================

#[derive(Clone, Copy)]
struct LspRange(u32, u32, u32); // line, start col, end col (0-based)

fn render_hover(hi: &Hoverable, r: LspRange) -> Value {
    // markdown-escape `_` in the package name (official renders it as `\_`)
    let pkg = hi.pkg.as_deref().unwrap_or("").replace('_', "\\_");
    let mut value = format!(
        "Declared in: {}  \nPackage info: {}  \n\n```cangjie\n{}\n```\n",
        hi.declared_in, pkg, hi.signature
    );
    if let Some(doc) = &hi.doc {
        value.push_str(&format!("\n---\n\n{doc}  \n"));
    }
    json!({
        "contents": { "kind": "markdown", "value": value },
        "range": {
            "start": { "line": r.0, "character": r.1 },
            "end": { "line": r.0, "character": r.2 },
        }
    })
}

// ================================================================
// Hoverable + Index
// ================================================================

/// A resoluble symbol: a declaration (hit-testable by name span) or a
/// std.core builtin (reached only through reference resolution).
#[derive(Clone)]
struct Hoverable {
    name: String,
    /// 1-based name token span.
    line: u32,
    col: u32,
    end_col: u32,
    /// Code-fence content of the hover (may carry the `// In <kind> <name>`
    /// member prefix).
    signature: String,
    /// Doc comment from the `//` line directly above the decl.
    doc: Option<String>,
    declared_in: String,
    pkg: Option<String>,
    /// Declared/inferred type (used for value reference resolution and var
    /// initializer type inference).
    ty: Option<String>,
    /// True when this symbol names a type (class/interface/struct/enum/alias).
    is_type: bool,
    /// Rendered parameter types for func-like decls; used to pick the right
    /// overload at a call site by matching the inferred argument types.
    param_tys: Vec<String>,
}

impl Hoverable {
    /// Does the (0-based) cursor fall on this symbol's name span?
    fn contains(&self, line: u32, character: u32) -> bool {
        self.line.saturating_sub(1) == line
            && character >= self.col.saturating_sub(1)
            && character <= self.end_col.saturating_sub(1)
    }
}

struct Container {
    name: String,
    name_line: u32,
    last_member_line: u32,
}

/// The identifier word (0-based span) at the cursor.
struct Word {
    text: String,
    line: u32,
    start: u32,
    end: u32,
}

struct Index<'a> {
    source: &'a str,
    /// The parsed file (used to walk call args for overload resolution).
    file: &'a File,
    total_lines: u32,
    package: Option<&'a str>,
    file_name: &'a str,
    /// Every declaration (top-level + members + local decls) — hit-test space.
    all: Vec<Hoverable>,
    /// Top-level + member decls by name (reference resolution).
    by_name: HashMap<String, Vec<usize>>,
    /// Local decls (vars, params, lambda params, local funcs) by name — for
    /// scope-aware reference resolution.
    locals: HashMap<String, Vec<usize>>,
    /// Type name -> index into `all` (classes/interfaces/structs/enums/aliases).
    types: HashMap<String, usize>,
    /// Type name -> kind label ("class"/"struct"/"interface"/"enum") — used
    /// to render `// In struct A` for extend-block members.
    type_kind: HashMap<String, String>,
    /// container type name -> member indices into `all`.
    members: HashMap<String, Vec<usize>>,
    containers: Vec<Container>,
    /// type name -> parent type display names (for `super` resolution).
    parents: HashMap<String, Vec<String>>,
    /// class/struct name -> synthesized default ctor (`public func init()`).
    /// Used when a type name in call position has no declared init member.
    implicit_inits: HashMap<String, Hoverable>,
    /// std.core builtin symbols.
    std: HashMap<String, Hoverable>,
}

impl<'a> Index<'a> {
    fn new(file: &'a File, source: &'a str, package: Option<&'a str>, file_name: &'a str) -> Self {
        let mut idx = Index {
            source,
            file,
            total_lines: source.lines().count() as u32,
            package,
            file_name,
            all: Vec::new(),
            by_name: HashMap::new(),
            locals: HashMap::new(),
            types: HashMap::new(),
            type_kind: HashMap::new(),
            members: HashMap::new(),
            containers: Vec::new(),
            parents: HashMap::new(),
            implicit_inits: HashMap::new(),
            std: HashMap::new(),
        };
        for s in STD_SYMS {
            idx.std.insert(
                s.name.to_string(),
                Hoverable {
                    name: s.name.to_string(),
                    line: 0,
                    col: 0,
                    end_col: 0,
                    signature: s.sig.to_string(),
                    doc: None,
                    declared_in: s.file.to_string(),
                    pkg: Some("std.core".to_string()),
                    ty: Some(s.name.to_string()),
                    is_type: true,
                    param_tys: Vec::new(),
                },
            );
        }
        idx.collect_file(file);
        idx.scan_local_funcs();
        idx
    }

    // ------------------------------------------------------------
    // Collection
    // ------------------------------------------------------------

    fn collect_file(&mut self, file: &'a File) {
        for d in &file.decls {
            self.collect_decl(d, None, false);
        }
    }

    /// `container` = None for top-level, else the `class Foo` label.
    /// `member` = true when this decl lives inside a class-like body.
    fn collect_decl(&mut self, d: &'a Decl, container: Option<&str>, member: bool) {
        match d {
            Decl::Func {
                name,
                name_pos,
                is_public,
                is_abstract,
                type_params,
                params,
                ret,
                body,
                pos,
                ..
            } => {
                let mods =
                    self.effective_mods(pos, *is_public, container, member, body, *is_abstract);
                let ret_s = ret
                    .as_ref()
                    .map(render_type)
                    .or_else(|| self.infer_body_ret_ix(body));
                let ret_txt = ret_s
                    .as_deref()
                    .map(|t| format!(": {t}"))
                    .unwrap_or_else(|| ": Unit".to_string());
                let sig = format!(
                    "{mods}func {name}{}({}){}",
                    render_type_params(type_params),
                    render_params(params, false),
                    ret_txt
                );
                let sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: ret_s,
                    is_type: false,
                    param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),
                };
                let idx = self.push(hi, !member);
                if member {
                    if let Some(c) = container.and_then(container_type_name) {
                        self.members.entry(c).or_default().push(idx);
                    }
                }
                // params are hoverable locals (`let x: T`) and the cursor lands
                // on them in many official cases (e.g. `testArr(x!...)`).
                self.collect_params(params);
                if let Body::Block(stmts) = body {
                    self.collect_locals(stmts, name_pos.line);
                }
            }
            Decl::Class {
                name,
                name_pos,
                is_public,
                is_open,
                is_sealed,
                type_params,
                parents,
                members,
                pos,
                ..
            } => {
                let mut mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                if *is_open && !mods.contains("open ") {
                    mods.insert_str(mods.len(), "open ");
                }
                if *is_sealed && !mods.contains("sealed ") {
                    mods.push_str("sealed ");
                }
                let label = format!("class {name}");
                let sig = format!(
                    "{mods}class {name}{}{}",
                    render_type_params(type_params),
                    render_class_parents(parents)
                );
                let sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: true,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
                self.type_kind.insert(name.clone(), "class".to_string());
                self.collect_generic_params(type_params, &label);
                self.add_implicit_init(name, &label);
                if !parents.is_empty() {
                    self.parents
                        .insert(name.clone(), parents.iter().map(render_type).collect());
                }
                let ci = self.containers.len();
                self.containers.push(Container {
                    name: name.clone(),
                    name_line: name_pos.line,
                    last_member_line: 0,
                });
                self.collect_members(members, &label, ci);
                // the container body extends past the last member (closing
                // `}`), so everything below the declaration line is "inside"
                // it for `this`/`super`/member-access resolution
                self.containers[ci].last_member_line = self.total_lines;
            }
            Decl::Interface {
                name,
                name_pos,
                is_public,
                type_params,
                parents,
                members,
                pos,
                ..
            } => {
                let mut mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                if *is_public && !mods.contains("public ") {
                    mods = format!("public {mods}");
                }
                let label = format!("interface {name}");
                let sig = format!(
                    "{mods}interface {name}{}{}",
                    render_type_params(type_params),
                    render_interface_parents(parents)
                );
                let sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: true,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
                self.type_kind.insert(name.clone(), "interface".to_string());
                self.collect_generic_params(type_params, &label);
                if !parents.is_empty() {
                    self.parents
                        .insert(name.clone(), parents.iter().map(render_type).collect());
                }
                let ci = self.containers.len();
                self.containers.push(Container {
                    name: name.clone(),
                    name_line: name_pos.line,
                    last_member_line: 0,
                });
                self.collect_members(members, &label, ci);
                self.containers[ci].last_member_line = self.total_lines;
            }
            Decl::Struct {
                name,
                name_pos,
                is_public,
                is_open,
                type_params,
                members,
                pos,
                ..
            } => {
                let mut mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                if *is_open && !mods.contains("open ") {
                    mods.push_str("open ");
                }
                let label = format!("struct {name}");
                let sig = format!("{mods}struct {name}{}", render_type_params(type_params));
                let sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: true,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
                self.type_kind.insert(name.clone(), "struct".to_string());
                self.collect_generic_params(type_params, &label);
                self.add_implicit_init(name, &label);
                let ci = self.containers.len();
                self.containers.push(Container {
                    name: name.clone(),
                    name_line: name_pos.line,
                    last_member_line: 0,
                });
                self.collect_members(members, &label, ci);
                self.containers[ci].last_member_line = self.total_lines;
            }
            Decl::Enum {
                name,
                name_pos,
                is_public,
                type_params,
                cases,
                pos,
                ..
            } => {
                let mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                let label = format!("enum {name}");
                let sig = format!("{mods}enum {name}{}", render_type_params(type_params));
                let sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: true,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
                self.type_kind.insert(name.clone(), "enum".to_string());
                self.collect_generic_params(type_params, &label);
                let ci = self.containers.len();
                self.containers.push(Container {
                    name: name.clone(),
                    name_line: name_pos.line,
                    last_member_line: 0,
                });
                for case in cases {
                    let payload_s = render_type_list(&case.payloads);
                    let case_sig = if payload_s.is_empty() {
                        format!("// In {label}\n{name}.{}", case.name)
                    } else {
                        format!("// In {label}\n{name}.{}({payload_s})", case.name)
                    };
                    let hi = Hoverable {
                        name: case.name.clone(),
                        line: case.pos.line,
                        col: case.pos.col,
                        end_col: case.pos.end_col,
                        signature: case_sig,
                        doc: None,
                        declared_in: self.file_name.to_string(),
                        pkg: self.package.map(str::to_string),
                        ty: Some(name.clone()),
                        is_type: false,
                        param_tys: Vec::new(),
                    };
                    let idx = self.push(hi, false);
                    self.members.entry(name.clone()).or_default().push(idx);
                    self.containers[ci].last_member_line =
                        self.containers[ci].last_member_line.max(case.pos.line);
                }
            }
            Decl::TypeAlias {
                name,
                is_public,
                target,
                pos,
                ..
            } => {
                let mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                let sig = format!("{mods}type {name} = {}", render_type(target));
                let sig = self.with_container(&sig, container);
                // the AST stores only `pos` (the `type` keyword); recover the
                // alias-name span from the source line for hit-testing
                let src_line = self.source_line(pos.line);
                let name_col = src_line.find(name).unwrap_or(pos.col as usize);
                let hi = Hoverable {
                    name: name.clone(),
                    line: pos.line,
                    col: (name_col + 1) as u32,
                    end_col: (name_col + name.len() + 1) as u32,
                    signature: sig,
                    doc: self.doc_comment(pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: Some(render_type(target)),
                    is_type: true,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
            }
            Decl::Var {
                name,
                name_pos,
                is_mutable,
                is_public,
                ty,
                init,
                pos,
                ..
            } => {
                let mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                // collect locals inside the initializer first (lambdas in a
                // `let`/`var` initializer) so the type inference below can
                // resolve their params (e.g. `{a: Int64 => a}(1)` → Int64)
                if let Some(i) = init {
                    self.collect_local_expr(i, name_pos.line);
                }
                // declared/inferred type (bare, no `: ` prefix); the inferred
                // initializer type is used when no annotation is written
                // (official renders `var arr1: Array<Int64> = [...]`).
                let disp = ty
                    .as_ref()
                    .map(render_type)
                    .filter(|r| r != "?")
                    .or_else(|| {
                        if ty.is_none() {
                            init.as_ref()
                                .and_then(|i| infer_init_expr_at(i, self, name_pos.line, 0))
                        } else {
                            None
                        }
                    });
                let ty_disp = match ty {
                    Some(t) => {
                        let r = render_type(t);
                        // `??`/`???` option sugar mis-parses to Invalid — fall
                        // back to the literal source annotation
                        if r.contains('?') {
                            self.slice_type_from_source(name_pos.line, name)
                                .map(|s| format!(": {s}"))
                                .or_else(|| Some(format!(": {r}")))
                        } else {
                            Some(format!(": {r}"))
                        }
                    }
                    None => disp.as_deref().map(|t| format!(": {t}")),
                };
                let init_s = Some(self.init_slice(name_pos.line));
                let mut sig = format!(
                    "{mods}{} {name}{}{}",
                    if *is_mutable { "var" } else { "let" },
                    ty_disp.as_deref().unwrap_or(""),
                    init_s.as_deref().unwrap_or("")
                );
                sig = self.with_container(&sig, container);
                let hi = Hoverable {
                    name: name.clone(),
                    line: name_pos.line,
                    col: name_pos.col,
                    end_col: name_pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(name_pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: disp,
                    is_type: false,
                    param_tys: Vec::new(),
                };
                let idx = self.push(hi, !member);
                if member {
                    if let Some(c) = container.and_then(container_type_name) {
                        self.members.entry(c).or_default().push(idx);
                    }
                }
            }
            Decl::PrimaryCtor {
                is_public,
                params,
                body,
                pos,
            } => {
                let _ = is_public;
                let params_s = render_params(params, true);
                let sig = format!("internal func init({params_s})");
                let sig = self.with_container(&sig, container);
                let cname = container.and_then(container_type_name);
                let hi = Hoverable {
                    name: "init".to_string(),
                    line: pos.line,
                    col: pos.col,
                    end_col: pos.end_col,
                    signature: sig,
                    doc: self.doc_comment(pos.line),
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: false,
                    param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),
                };
                let idx = self.push(hi, false);
                if let Some(c) = cname {
                    self.members.entry(c).or_default().push(idx);
                }
                // `init(...)` ctor params are hoverable locals too
                if let Some(c) = container {
                    self.collect_ctor_props(params, c);
                }
                self.collect_params(params);
                if let Body::Block(stmts) = body {
                    self.collect_locals(stmts, pos.line);
                }
            }
            Decl::Extend {
                target: Type::Ref { name: tname, .. },
                members,
                pos,
                ..
            } => {
                // members of an `extend X { }` block belong to the target
                // type: collect them with the `// In class X` / `// In struct X`
                // container label for hit-testing and member resolution.
                let kind = self
                    .type_kind
                    .get(tname.as_str())
                    .map(String::as_str)
                    .unwrap_or("class");
                let label = format!("{kind} {tname}");
                let ci = self.containers.len();
                self.containers.push(Container {
                    name: tname.clone(),
                    name_line: pos.line,
                    last_member_line: self.total_lines,
                });
                self.collect_members(members, &label, ci);
            }
            Decl::Extend { .. } => {}
            Decl::Prop {
                name,
                is_public,
                ty,
                pos,
                ..
            } => {
                let mods =
                    self.effective_mods(pos, *is_public, container, member, &Body::Empty, false);
                let sig = format!("{mods}let {name}: {}", render_type(ty));
                let sig = self.with_container(&sig, container);
                // the AST stores only the `prop` keyword span — recover the
                // property NAME span from the source line
                let (ncol, nend) = self.prop_name_span(pos, name);
                let hi = Hoverable {
                    name: name.clone(),
                    line: pos.line,
                    col: ncol,
                    end_col: nend,
                    signature: sig,
                    doc: None,
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: Some(render_type(ty)),
                    is_type: false,
                    param_tys: Vec::new(),
                };
                let idx = self.push(hi, !member);
                if member {
                    if let Some(c) = container.and_then(container_type_name) {
                        self.members.entry(c).or_default().push(idx);
                    }
                }
            }
            Decl::Macro {
                name,
                is_public,
                pos,
                ..
            } => {
                let sig = if *is_public {
                    format!("public macro {name}")
                } else {
                    format!("macro {name}")
                };
                let hi = Hoverable {
                    name: name.clone(),
                    line: pos.line,
                    col: pos.col,
                    end_col: pos.end_col,
                    signature: sig,
                    doc: None,
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: false,
                    param_tys: Vec::new(),
                };
                self.push(hi, !member);
            }
            Decl::Main { pos, .. } => {
                let hi = Hoverable {
                    name: "main".to_string(),
                    line: pos.line,
                    col: pos.col,
                    end_col: pos.end_col,
                    signature: "func main()".to_string(),
                    doc: None,
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: None,
                    is_type: false,
                    param_tys: Vec::new(),
                };
                self.push(hi, true);
            }
            _ => {}
        }
    }

    /// Collect members of a class-like body. `ci` indexes `self.containers`.
    fn collect_members(&mut self, members: &'a [Decl], container: &str, ci: usize) {
        for m in members {
            match m {
                // class-name constructor: `Test(a: Int64, ...) { }` inside the
                // class — official hover renders it as `internal func init(...)`.
                // Distinguished from a same-named `func Test()` METHOD by the
                // absence of the `func` keyword before the name in the source.
                Decl::Func {
                    name,
                    name_pos,
                    params,
                    ..
                } if (name == container || container.ends_with(&format!(" {name}")))
                    && !self.before_kind(name_pos).trim_end().ends_with("func") =>
                {
                    let params_s = render_params(params, true);
                    // ctor signatures never carry a return type
                    let sig = format!("// In {container}\ninternal func init({params_s})");
                    let hi = Hoverable {
                        name: "init".to_string(),
                        line: name_pos.line,
                        col: name_pos.col,
                        end_col: name_pos.end_col,
                        signature: sig,
                        doc: None,
                        declared_in: self.file_name.to_string(),
                        pkg: self.package.map(str::to_string),
                        ty: None,
                        is_type: false,
                        param_tys: params.iter().map(|p| render_type(&p.ty)).collect(),
                    };
                    let idx = self.push(hi, false);
                    if let Some(cn) = container_type_name(container) {
                        self.members.entry(cn).or_default().push(idx);
                    }
                    self.containers[ci].last_member_line =
                        self.containers[ci].last_member_line.max(name_pos.line);
                    // also collect the ctor params as locals (they are hoverable)
                    self.collect_ctor_props(params, container);
                    self.collect_params(params);
                }
                _ => {
                    let before = self.all.len();
                    self.collect_decl(m, Some(container), true);
                    for h in &self.all[before..] {
                        self.containers[ci].last_member_line =
                            self.containers[ci].last_member_line.max(h.line);
                    }
                }
            }
        }
        if self.containers[ci].last_member_line == 0 {
            self.containers[ci].last_member_line = self.containers[ci].name_line;
        }
    }

    /// Collect parameters of a func/init as locals (param hover: `let x: T`).
    fn collect_params(&mut self, params: &'a [cj_ast::Param]) {
        for p in params {
            let sig = format!("let {}: {}", p.name, render_type(&p.ty));
            let hi = Hoverable {
                name: p.name.clone(),
                line: p.pos.line,
                col: p.pos.col,
                end_col: p.pos.end_col,
                signature: sig,
                doc: None,
                declared_in: self.file_name.to_string(),
                pkg: self.package.map(str::to_string),
                ty: Some(render_type(&p.ty)),
                is_type: false,
                param_tys: Vec::new(),
            };
            let idx = self.push(hi, false);
            self.locals.entry(p.name.clone()).or_default().push(idx);
        }
    }

    /// True when a ctor param is a property member (`var x!: T` in a
    /// constructor's parens) — the `var` keyword precedes the name in source.
    fn param_is_property(&self, p: &cj_ast::Param) -> bool {
        let l = self.source_line(p.pos.line);
        let col = p.pos.col.saturating_sub(1) as usize;
        if col > l.len() {
            return false;
        }
        let before = &l[..col];
        let trimmed = before.trim_end();
        trimmed.ends_with("var")
            && trimmed
                .get(..trimmed.len().saturating_sub(3))
                .and_then(|r| r.chars().next_back())
                .map(|c| !c.is_alphanumeric())
                .unwrap_or(true)
    }

    /// Ctor property params (`var x!: T`) become member vars: they are added
    /// to `members` with the `// In <container>` prefix so hovering them shows
    /// `internal var x: T` (no initializer, no `!`).
    fn collect_ctor_props(&mut self, params: &'a [cj_ast::Param], container: &str) {
        let Some(cname) = container_type_name(container) else {
            return;
        };
        for p in params {
            if !self.param_is_property(p) {
                continue;
            }
            let mods =
                self.effective_mods(&p.pos, false, Some(container), true, &Body::Empty, false);
            let sig = format!(
                "// In {container}\n{mods}var {}: {}",
                p.name,
                render_type(&p.ty)
            );
            let hi = Hoverable {
                name: p.name.clone(),
                line: p.pos.line,
                col: p.pos.col,
                end_col: p.pos.end_col,
                signature: sig,
                doc: None,
                declared_in: self.file_name.to_string(),
                pkg: self.package.map(str::to_string),
                ty: Some(render_type(&p.ty)),
                is_type: false,
                param_tys: Vec::new(),
            };
            let idx = self.push(hi, false);
            self.members.entry(cname.clone()).or_default().push(idx);
        }
    }

    /// Type parameters of a class-like decl are hoverable (`// In class C7\n
    /// genericParam T`) and resolvable from type references.
    fn collect_generic_params(&mut self, tps: &'a [cj_ast::TypeParam], container: &str) {
        for tp in tps {
            let sig = format!("// In {container}\ngenericParam {}", tp.name);
            let hi = Hoverable {
                name: tp.name.clone(),
                line: tp.pos.line,
                col: tp.pos.col,
                end_col: tp.pos.end_col,
                signature: sig,
                doc: None,
                declared_in: self.file_name.to_string(),
                pkg: self.package.map(str::to_string),
                ty: None,
                is_type: false,
                param_tys: Vec::new(),
            };
            self.push(hi, true);
        }
    }

    /// Register the synthesized default constructor (`public func init()`) for
    /// a class/struct so a constructor call `Name()` with no declared init
    /// still resolves to the official `// In class Name` / `// In struct Name`
    /// hover instead of the type declaration itself.
    fn add_implicit_init(&mut self, name: &str, label: &str) {
        self.implicit_inits.insert(
            name.to_string(),
            Hoverable {
                name: "init".to_string(),
                line: 0,
                col: 0,
                end_col: 0,
                signature: format!("// In {label}\npublic func init()"),
                doc: None,
                declared_in: self.file_name.to_string(),
                pkg: self.package.map(str::to_string),
                ty: None,
                is_type: false,
                param_tys: Vec::new(),
            },
        );
    }

    /// Collect local declarations from a function/init body: local vars,
    /// params are added separately by the caller when available.
    fn collect_locals(&mut self, stmts: &'a [Expr], fn_line: u32) {
        // the enclosing function's params are added by the caller if it has
        // them (collect_decl Func arm does not — add here from AST):
        // NOTE: params are collected via collect_decl -> collect_params? No —
        // Func params are collected in collect_decl directly. Locals only.
        let _ = fn_line;
        for s in stmts {
            self.collect_local_expr(s, fn_line);
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_local_expr(&mut self, e: &'a Expr, fn_line: u32) {
        match e {
            Expr::LetPatternDestructor {
                patterns,
                initializer,
                pos,
                ..
            } => {
                // position-aware inference so name refs in the initializer
                // resolve against in-scope locals/params (e.g. `var a = x`)
                let inferred = infer_init_expr_at(initializer, self, pos.line.saturating_sub(1), 0);
                for pat in patterns {
                    if let Pattern::Var {
                        name,
                        name_pos,
                        is_mutable,
                        ty,
                        pos,
                        ..
                    } = pat
                    {
                        let kind = if *is_mutable { "var" } else { "let" };
                        // an explicit annotation (`var ii: Any = AA()`) wins
                        // over the type inferred from the initializer
                        let ty_disp = ty.as_ref().map(render_type).or_else(|| inferred.clone());
                        let td = ty_disp
                            .as_deref()
                            .map(|t| format!(": {t}"))
                            .unwrap_or_default();
                        let init_s = self.init_slice(name_pos.line);
                        let sig = format!("{kind} {name}{td}{init_s}");
                        let hi = Hoverable {
                            name: name.clone(),
                            line: name_pos.line,
                            col: name_pos.col,
                            end_col: name_pos.end_col,
                            signature: sig,
                            doc: None,
                            declared_in: self.file_name.to_string(),
                            pkg: self.package.map(str::to_string),
                            ty: ty_disp.clone(),
                            is_type: false,
                            param_tys: Vec::new(),
                        };
                        let idx = self.push(hi, false);
                        self.locals.entry(name.clone()).or_default().push(idx);
                        let _ = pos;
                    }
                }
                let _ = pos;
                self.collect_local_expr(initializer, fn_line);
            }
            Expr::Lambda { params, body, .. } => {
                self.collect_params(params);
                self.collect_local_expr(body, fn_line);
            }
            Expr::Block { stmts, .. } => {
                for s in stmts {
                    self.collect_local_expr(s, fn_line);
                }
            }
            Expr::If {
                cond, then, els, ..
            } => {
                self.collect_local_expr(cond, fn_line);
                self.collect_local_expr(then, fn_line);
                if let Some(e) = els {
                    self.collect_local_expr(e, fn_line);
                }
            }
            Expr::While { cond, body, .. } => {
                self.collect_local_expr(cond, fn_line);
                self.collect_local_expr(body, fn_line);
            }
            Expr::DoWhile { cond, body, .. } => {
                self.collect_local_expr(body, fn_line);
                self.collect_local_expr(cond, fn_line);
            }
            Expr::ForIn {
                pattern,
                iter,
                body,
                ..
            } => {
                self.collect_local_expr(iter, fn_line);
                if let Pattern::Var {
                    name,
                    name_pos,
                    is_mutable,
                    pos,
                    ..
                } = pattern
                {
                    let kind = if *is_mutable { "var" } else { "let" };
                    let sig = format!("{kind} {name}");
                    let hi = Hoverable {
                        name: name.clone(),
                        line: name_pos.line,
                        col: name_pos.col,
                        end_col: name_pos.end_col,
                        signature: sig,
                        doc: None,
                        declared_in: self.file_name.to_string(),
                        pkg: self.package.map(str::to_string),
                        ty: None,
                        is_type: false,
                        param_tys: Vec::new(),
                    };
                    let idx = self.push(hi, false);
                    self.locals.entry(name.clone()).or_default().push(idx);
                    let _ = pos;
                }
                self.collect_local_expr(body, fn_line);
            }
            Expr::Match {
                scrutinee, cases, ..
            } => {
                self.collect_local_expr(scrutinee, fn_line);
                for c in cases {
                    self.collect_local_expr(&c.body, fn_line);
                }
            }
            Expr::Try {
                body,
                catches,
                finally,
                ..
            } => {
                self.collect_local_expr(body, fn_line);
                for c in catches {
                    if let Some(cn) = &c.name {
                        let sig = format!("let {cn}");
                        let hi = Hoverable {
                            name: cn.clone(),
                            line: c.pos.line,
                            col: c.pos.col,
                            end_col: c.pos.end_col,
                            signature: sig,
                            doc: None,
                            declared_in: self.file_name.to_string(),
                            pkg: self.package.map(str::to_string),
                            ty: None,
                            is_type: false,
                            param_tys: Vec::new(),
                        };
                        let idx = self.push(hi, false);
                        self.locals.entry(cn.clone()).or_default().push(idx);
                    }
                    self.collect_local_expr(&c.body, fn_line);
                }
                if let Some(f) = finally {
                    self.collect_local_expr(f, fn_line);
                }
            }
            Expr::Spawn { inner, .. }
            | Expr::Synchronized { inner, .. }
            | Expr::Perform { inner, .. }
            | Expr::Resume { inner, .. }
            | Expr::Throw { inner, .. }
            | Expr::Pointer { inner, .. }
            | Expr::Optional { inner, .. }
            | Expr::OptionalChain { inner, .. }
            | Expr::Paren { inner, .. }
            | Expr::Unary { inner, .. }
            | Expr::IncOrDec { inner, .. } => self.collect_local_expr(inner, fn_line),
            Expr::Binary { lhs, rhs, .. }
            | Expr::Range {
                start: lhs,
                end: rhs,
                ..
            } => {
                self.collect_local_expr(lhs, fn_line);
                self.collect_local_expr(rhs, fn_line);
            }
            Expr::Assign { lhs, rhs, .. } => {
                self.collect_local_expr(lhs, fn_line);
                self.collect_local_expr(rhs, fn_line);
            }
            Expr::Subscript { object, index, .. } => {
                self.collect_local_expr(object, fn_line);
                self.collect_local_expr(index, fn_line);
            }
            Expr::Is { inner, .. } | Expr::As { inner, .. } => {
                self.collect_local_expr(inner, fn_line)
            }
            Expr::Call { callee, args, .. } => {
                self.collect_local_expr(callee, fn_line);
                for a in args {
                    self.collect_local_expr(&a.value, fn_line);
                }
            }
            Expr::Member { object, .. } => self.collect_local_expr(object, fn_line),
            Expr::Return { value: Some(v), .. } => self.collect_local_expr(v, fn_line),
            Expr::Return { value: None, .. } => {}
            Expr::Interpolation { parts, .. } | Expr::StrInterpolation { parts, .. } => {
                for p in parts {
                    if let cj_ast::InterpPart::Expr(e) = p {
                        self.collect_local_expr(e, fn_line);
                    }
                }
            }
            _ => {}
        }
    }

    /// Local funcs are not parsed by the main parser (they parse as garbage
    /// inside bodies), so scan the source for indented `func` decls that the
    /// AST didn't capture and parse each one standalone.
    fn scan_local_funcs(&mut self) {
        let lines: Vec<&str> = self.source.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            if indent == 0 {
                continue; // top-level — handled by the AST
            }
            if !(trimmed.starts_with("func ")
                || trimmed.starts_with("mut func ")
                || trimmed.starts_with("operator func "))
            {
                continue;
            }
            // the func keyword column (0-based)
            let fkw = trimmed.find("func").unwrap() + indent;
            // skip if this position already has a collected decl (member func)
            if self
                .all
                .iter()
                .any(|h| h.line as usize == i + 1 && h.col as usize == fkw + 1)
            {
                continue;
            }
            if let Some(hi) = self.parse_local_func(i as u32, &lines) {
                let idx = self.push(hi, false);
                self.locals
                    .entry(self.all[idx].name.clone())
                    .or_default()
                    .push(idx);
            }
        }
    }

    /// Parse one local-func statement starting at line `start` (0-based),
    /// spanning until its matching closing brace, as a standalone top-level
    /// func, then re-map positions to the real file.
    #[allow(clippy::question_mark)]
    fn parse_local_func(&self, start: u32, lines: &[&str]) -> Option<Hoverable> {
        // find the end: the `func` line + matching braces until a blank-line
        // separated next decl or EOF.
        let mut end = start;
        let mut depth = 0i32;
        let mut started = false;
        for (j, l) in lines.iter().enumerate().skip(start as usize) {
            for ch in l.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        started = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if started && depth <= 0 {
                end = j as u32;
                break;
            }
            if j as u32 > start + 2 && l.trim().is_empty() {
                // allow empty line inside body; stop if a new decl appears
                let nxt = lines.get(j + 1).map(|x| x.trim().to_string());
                if let Some(n) = nxt {
                    if n.starts_with("func ") || n.starts_with("let ") || n.starts_with("var ") {
                        break;
                    }
                }
            }
        }
        let slice: String = lines[start as usize..=end as usize].join("\n");
        let wrapped = format!("package p\n{slice}\n");
        let file = cj_parser::parse_source(&wrapped).0;
        let Some(d) = file.decls.first() else {
            return None;
        };
        let (name, name_pos, params, ret, body) = match d {
            Decl::Func {
                name,
                name_pos,
                params,
                ret,
                body,
                ..
            } => (name, name_pos, params, ret, body),
            _ => return None,
        };
        let ret_s = ret
            .as_ref()
            .map(render_type)
            .or_else(|| self.infer_body_ret_with_params(body, params));
        let ret_txt = ret_s
            .as_deref()
            .map(|t| format!(": {t}"))
            .unwrap_or_else(|| ": Unit".to_string());
        // a scanned local func may actually be a member of a class-like body
        // that the parser dropped (e.g. `operator func +` in an enum) —
        // recover the container + modifiers from the source
        let container = self.enclosing_container_label(start, lines);
        let prefix = container
            .as_deref()
            .map(|c| format!("// In {c}\n"))
            .unwrap_or_default();
        let mods = if container.is_some() {
            self.local_member_mods(start, lines)
        } else {
            String::new()
        };
        let sig = format!(
            "{prefix}{mods}func {name}({}){ret_txt}",
            render_params(params, false)
        );
        let line = start + name_pos.line.saturating_sub(1);
        Some(Hoverable {
            name: name.clone(),
            line,
            col: name_pos.col,
            end_col: name_pos.end_col,
            signature: sig,
            doc: None,
            declared_in: self.file_name.to_string(),
            pkg: self.package.map(str::to_string),
            ty: ret_s,
            is_type: false,
            param_tys: Vec::new(),
        })
    }

    /// Visibility modifiers for a scanned local func that is a class-like
    /// member (defaults to `internal`; `mut`/`static`/... are recovered).
    fn local_member_mods(&self, start: u32, lines: &[&str]) -> String {
        let line = lines.get(start as usize).unwrap_or(&"");
        let trimmed = line.trim_start();
        let mut mods: Vec<&str> = Vec::new();
        for m in [
            "private",
            "protected",
            "public",
            "internal",
            "static",
            "mut",
            "open",
            "abstract",
        ] {
            if word_in(trimmed, m) && !mods.contains(&m) {
                mods.push(m);
            }
        }
        let has_vis = mods
            .iter()
            .any(|m| matches!(*m, "private" | "protected" | "public" | "internal"));
        if !has_vis {
            mods.insert(0, "internal");
        }
        format!("{} ", mods.join(" "))
    }

    /// Nearest preceding `class/struct/interface/enum <Name>` opener label —
    /// but a local func nested inside another func is NOT a class member, so
    /// the scan stops at the nearest `func ... {` opener with None.
    fn enclosing_container_label(&self, start: u32, lines: &[&str]) -> Option<String> {
        for j in (0..start as usize).rev() {
            let line = lines[j];
            let trimmed = line.trim();
            // a func opener that nests this func (ends with `{`) → local func
            if trimmed.ends_with('{') && trimmed.contains("func ") {
                return None;
            }
            for kw in ["class", "struct", "interface", "enum"] {
                let mut rest = line;
                while let Some(i) = rest.find(kw) {
                    let before_ok =
                        i == 0 || !rest[..i].chars().next_back().unwrap().is_alphanumeric();
                    let after = &rest[i + kw.len()..];
                    let after_ok = after
                        .chars()
                        .next()
                        .map(|c| c == ' ' || c == '<')
                        .unwrap_or(false);
                    if before_ok && after_ok {
                        let name = after
                            .trim_start()
                            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                            .next()
                            .unwrap_or("");
                        if !name.is_empty() {
                            return Some(format!("{kw} {name}"));
                        }
                    }
                    rest = after;
                }
            }
        }
        None
    }

    // ------------------------------------------------------------
    // Signature helpers
    // ------------------------------------------------------------

    fn with_container(&self, sig: &str, container: Option<&str>) -> String {
        match container {
            Some(c) => format!("// In {c}\n{sig}"),
            None => sig.to_string(),
        }
    }

    /// Infer a func's return type from its body (position-insensitive: every
    /// in-scope local of the body is considered visible).
    fn infer_body_ret_ix(&self, body: &Body) -> Option<String> {
        match body {
            Body::Empty => Some("Unit".to_string()),
            Body::Block(stmts) => stmts
                .last()
                .and_then(|e| infer_init_expr_at(e, self, u32::MAX, u32::MAX)),
        }
    }

    /// Like `infer_body_ret_ix` but also resolves Name refs against the given
    /// params (for scanned local funcs whose params aren't in `locals`).
    fn infer_body_ret_with_params(&self, body: &Body, params: &[cj_ast::Param]) -> Option<String> {
        match body {
            Body::Empty => Some("Unit".to_string()),
            Body::Block(stmts) => stmts
                .last()
                .and_then(|e| self.infer_expr_with_params(e, params)),
        }
    }

    fn infer_expr_with_params(&self, e: &Expr, params: &[cj_ast::Param]) -> Option<String> {
        match e {
            Expr::Name { name, .. } => {
                if let Some(p) = params.iter().find(|p| &p.name == name) {
                    return Some(render_type(&p.ty));
                }
                infer_init_expr_at(e, self, u32::MAX, u32::MAX)
            }
            Expr::Binary { lhs, .. } => self.infer_expr_with_params(lhs, params),
            Expr::Paren { inner, .. } => self.infer_expr_with_params(inner, params),
            Expr::Block { stmts, .. } => stmts
                .last()
                .and_then(|s| self.infer_expr_with_params(s, params)),
            _ => infer_init_expr_at(e, self, u32::MAX, u32::MAX),
        }
    }

    /// Reconstruct effective modifiers from the decl start + the source line
    /// (the AST stores only `is_public`, so private/protected/static/mut/etc
    /// are recovered from the source text before the kind keyword).
    fn effective_mods(
        &self,
        pos: &CodePos,
        is_public: bool,
        container: Option<&str>,
        member: bool,
        body: &Body,
        is_abstract: bool,
    ) -> String {
        let before = self.before_kind(pos);
        let mut mods: Vec<&str> = Vec::new();
        for m in [
            "private",
            "protected",
            "public",
            "internal",
            "static",
            "abstract",
            "open",
            "sealed",
            "mut",
        ] {
            if word_in(before, m) && !mods.contains(&m) {
                mods.push(m);
            }
        }
        let has_vis = mods
            .iter()
            .any(|m| matches!(*m, "private" | "protected" | "public" | "internal"));
        // interface (and interface-extend) members are implicitly public
        let in_interface = container
            .map(|c| c.starts_with("interface "))
            .unwrap_or(false)
            && member;
        if !has_vis {
            if is_public || in_interface {
                mods.insert(0, "public");
            } else {
                mods.insert(0, "internal");
            }
        }
        // interface member funcs: abstract (no body) / open (has body) —
        // even static ones are abstract when they have no body
        if in_interface && !mods.contains(&"abstract") && !mods.contains(&"open") {
            if matches!(body, Body::Empty) || is_abstract {
                mods.push("abstract");
            } else {
                mods.push("open");
            }
        }
        format!("{} ", mods.join(" "))
    }

    fn before_kind(&self, pos: &CodePos) -> &str {
        let line = self.source_line(pos.line);
        let col = pos.col.saturating_sub(1) as usize;
        line.get(..col.min(line.len())).unwrap_or("")
    }

    /// Slice the initializer text from the source line: from the first `=`
    /// after the variable's name up to the end of the line (minus a trailing
    /// comment or comma). Lambda initializers (`= {x => x}`) are omitted —
    /// the official hover renders only the declared signature.
    fn init_slice(&self, line: u32) -> String {
        let line = self.source_line(line);
        // the assignment `=` may not exist on the line (declaration-only
        // member var) — then there is no initializer to slice
        let Some(eq) = line.find('=') else {
            return String::new();
        };
        let after = line[eq + 1..].trim();
        let after = strip_trailing_comment(after);
        if after.is_empty() {
            String::new()
        } else if after.starts_with('{') {
            // keep immediately-invoked lambdas `{a => ...}(1, 2)`; omit bare
            // lambdas / blocks
            if let Some(close) = after.rfind('}') {
                let tail = after[close + 1..].trim_start();
                if tail.starts_with('(') {
                    format!(" = {after}")
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            format!(" = {after}")
        }
    }

    /// Slice an explicit-but-unrenderable type (e.g. `???Int64`) from source:
    /// the text between `:` and `=`/`{` on the var's line.
    fn slice_type_from_source(&self, line: u32, name: &str) -> Option<String> {
        let line = self.source_line(line);
        let idx = line.find(name)?;
        let rest = &line[idx + name.len()..];
        let after_colon = rest.find(':')?;
        let rest2 = &rest[after_colon + 1..];
        let ty = rest2
            .split(['=', '{'])
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(',')
            .trim_end_matches(';')
            .trim();
        if ty.is_empty() {
            None
        } else {
            Some(ty.to_string())
        }
    }

    /// Recover a property's NAME span (1-based, inclusive/exclusive) from the
    /// `prop` keyword span + the source line.
    fn prop_name_span(&self, pos: &CodePos, name: &str) -> (u32, u32) {
        let line = self.source_line(pos.line);
        let mut i = pos.end_col.saturating_sub(1) as usize; // just after `prop`
        if i > line.len() {
            return (pos.col, pos.end_col);
        }
        while i < line.len()
            && !(line.as_bytes()[i].is_ascii_alphanumeric() || line.as_bytes()[i] == b'_')
        {
            i += 1;
        }
        let start = i;
        while i < line.len()
            && (line.as_bytes()[i].is_ascii_alphanumeric() || line.as_bytes()[i] == b'_')
        {
            i += 1;
        }
        if line.get(start..i) == Some(name) {
            (start as u32 + 1, i as u32 + 1)
        } else {
            (pos.col, pos.end_col)
        }
    }

    fn source_line(&self, line: u32) -> &str {
        if line == 0 {
            return "";
        }
        self.source.lines().nth((line - 1) as usize).unwrap_or("")
    }

    /// Doc comment: the `//` line immediately above the decl name line.
    fn doc_comment(&self, name_line: u32) -> Option<String> {
        if name_line <= 1 {
            return None;
        }
        let above = self.source_line(name_line - 1).trim();
        if let Some(body) = above.strip_prefix("//") {
            let t = body.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        } else {
            None
        }
    }

    // ------------------------------------------------------------
    // Indexing
    // ------------------------------------------------------------

    fn push(&mut self, hi: Hoverable, index_by_name: bool) -> usize {
        let i = self.all.len();
        if index_by_name {
            self.by_name.entry(hi.name.clone()).or_default().push(i);
        }
        self.all.push(hi);
        i
    }

    // ------------------------------------------------------------
    // Hit-testing + reference resolution
    // ------------------------------------------------------------

    fn hit_test(&self, line: u32, character: u32) -> Option<&Hoverable> {
        self.all.iter().find(|h| h.contains(line, character))
    }

    fn resolve(
        &self,
        word: &str,
        line: u32,
        character: u32,
        word_start: u32,
    ) -> Option<&Hoverable> {
        if word.is_empty() {
            return None;
        }
        // special receivers
        if word == "this" {
            return self
                .enclosing_container(line, character)
                .and_then(|c| self.lookup_type(&c.name));
        }
        if word == "super" {
            if let Some(c) = self.enclosing_container(line, character) {
                if let Some(p) = self.parents.get(&c.name).and_then(|p| p.first()) {
                    return self.lookup_type(p);
                }
            }
            return None;
        }
        // member access `recv.word`
        if let Some(recv) = self.receiver_before(line, character) {
            if let Some(recv_ty) = self.receiver_type(&recv, line, character) {
                // pipeline `arg |> recv.word` passes `arg` as the call's sole
                // argument — resolve the overload (own + inherited) that
                // accepts it (e.g. `m3 |> m2.h1` where m3: Rune picks the
                // inherited `I1.h1(a: Rune)` over the own `h1(a: Int64)`).
                if let Some(arg_ty) = self.pipeline_arg(&recv, line, character) {
                    if let Some(hi) =
                        self.member_lookup_for_pipeline(&recv_ty, word, &[Some(arg_ty)])
                    {
                        return Some(hi);
                    }
                }
                if let Some(hi) = self.member_lookup(&recv_ty, word) {
                    return Some(hi);
                }
            }
        }
        // call position: `word(` or `word<T>(` → ctor/init or func. Slice
        // from the END of the word (the cursor often sits mid-word).
        let word_end = word_start + word.len() as u32;
        if self.is_call_at(word, line, word_end) {
            if self.types.contains_key(word) {
                let args = self.call_arg_tys(word, line, word_start, word_end);
                if let Some(hi) = self.member_lookup_for_call(word, "init", &args) {
                    return Some(hi);
                }
                // no declared init — official shows the synthesized default
                // ctor (`// In class X\npublic func init()`)
                return self
                    .implicit_inits
                    .get(word)
                    .or_else(|| self.lookup_type(word));
            }
            if let Some(hits) = self.by_name.get(word) {
                if let Some(hi) = self.lookup_local(word, line, character) {
                    return Some(hi);
                }
                let args = self.call_arg_tys(word, line, word_start, word_end);
                return self.pick_func_by_args(hits, &args);
            }
            return self.lookup_local(word, line, character);
        }
        // type name used as a static / enum-case receiver: `Foo.f`, `Time.Day`
        if self.types.contains_key(word) && self.byte_after(line, word_end) == Some(b'.') {
            return self.lookup_type(word);
        }
        // type position (preceded by : < as is extend etc)
        if self.is_type_position(word, line, word_start) {
            if let Some(hi) = self.lookup_type(word) {
                return Some(hi);
            }
        }
        // plain value reference
        if let Some(hi) = self.lookup_local(word, line, character) {
            return Some(hi);
        }
        if let Some(hits) = self.by_name.get(word) {
            return self.pick_non_type(hits);
        }
        self.lookup_std(word)
    }

    fn receiver_before(&self, line: u32, character: u32) -> Option<String> {
        let l = self.source_line(line + 1);
        let col = character as usize;
        if col > l.len() {
            return None;
        }
        let before = &l[..col];
        let idx = before.rfind('.')?;
        let head = &before[..idx];
        // optional receiver `c2?.item`: the `?` sits before the dot
        let head = head.trim_end_matches('?');
        let recv = head
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next()?;
        if recv.is_empty() {
            None
        } else {
            Some(recv.to_string())
        }
    }

    /// Resolve a member-access receiver to a TYPE name (display form).
    fn receiver_type(&self, recv: &str, line: u32, character: u32) -> Option<String> {
        if recv == "this" {
            return self
                .enclosing_container(line, character)
                .map(|c| c.name.clone());
        }
        if recv == "super" {
            return self
                .enclosing_container(line, character)
                .and_then(|c| self.parents.get(&c.name))
                .and_then(|p| p.first().cloned());
        }
        if self.types.contains_key(recv) {
            return Some(recv.to_string());
        }
        // a value → its declared type
        if let Some(hi) = self
            .lookup_local(recv, line, character)
            .or_else(|| self.lookup_top_value(recv))
        {
            return strip_std_wrap(hi.ty.clone());
        }
        None
    }

    fn member_lookup(&self, container: &str, name: &str) -> Option<&Hoverable> {
        let base = container.split('<').next().unwrap_or(container);
        let idxs = self.members.get(base)?;
        let mut best: Option<&Hoverable> = None;
        for i in idxs {
            let h = &self.all[*i];
            if h.name == name {
                // prefer the init with the most parameters (overload pick)
                if best.is_none() || (name == "init" && h_sig_len(h) > h_sig_len(best.unwrap())) {
                    best = Some(h);
                }
            }
        }
        best
    }

    /// Pick a ctor/func overload at a call site by matching the inferred
    /// argument types against each candidate's parameter types.
    fn member_lookup_for_call(
        &self,
        container: &str,
        name: &str,
        args: &[Option<String>],
    ) -> Option<&Hoverable> {
        let base = container.split('<').next().unwrap_or(container);
        let idxs = self.members.get(base)?;
        let mut best: Option<&Hoverable> = None;
        let mut best_score = i32::MIN;
        for i in idxs {
            let h = &self.all[*i];
            if h.name != name {
                continue;
            }
            let score = match_score(&h.param_tys, args);
            if best.is_none() || score > best_score {
                best = Some(h);
                best_score = score;
            }
        }
        best
    }

    /// Collect indices of every member named `name` on `container` — its own
    /// members plus, recursively, members inherited from its parents (the
    /// `parents` table maps a type name to its declared parent type names).
    fn member_candidates(&self, container: &str, name: &str, out: &mut Vec<usize>) {
        let base = container.split('<').next().unwrap_or(container);
        if let Some(idxs) = self.members.get(base) {
            for &i in idxs {
                if self.all[i].name == name {
                    out.push(i);
                }
            }
        }
        if let Some(ps) = self.parents.get(base) {
            for p in ps {
                self.member_candidates(p, name, out);
            }
        }
    }

    /// Pick the member overload (own + inherited, via [`Self::member_candidates`])
    /// whose parameters best accept the given argument types. Own members win
    /// ties because they are collected first.
    fn member_lookup_for_pipeline(
        &self,
        container: &str,
        name: &str,
        args: &[Option<String>],
    ) -> Option<&Hoverable> {
        let mut cands = Vec::new();
        self.member_candidates(container, name, &mut cands);
        let mut best: Option<&Hoverable> = None;
        let mut best_score = i32::MIN;
        for i in cands {
            let h = &self.all[i];
            let score = match_score(&h.param_tys, args);
            if best.is_none() || score > best_score {
                best = Some(h);
                best_score = score;
            }
        }
        best
    }

    /// When the member access `recv.word` is the right-hand side of a
    /// pipeline `arg |> recv.word`, the pipeline passes `arg` as the call's
    /// single argument (`recv.word(arg)`). Return `arg`'s resolved type.
    fn pipeline_arg(&self, recv: &str, line: u32, character: u32) -> Option<String> {
        let l = self.source_line(line + 1);
        let col = character as usize;
        if col > l.len() {
            return None;
        }
        let before = &l[..col];
        let idx = before.rfind('.')?;
        let head = &before[..idx];
        // the pipeline operator must feed directly into this receiver (only
        // whitespace between `|>` and the receiver word), otherwise the `|>`
        // belongs to a different subexpression.
        let p = head.rfind("|>")?;
        if head[p + 2..].trim() != recv {
            return None;
        }
        let left = &head[..p];
        let arg = left
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .find(|s| !s.is_empty())?;
        // the pipeline argument is a value → its declared/inferred type
        self.lookup_local(arg, line, character)
            .or_else(|| self.lookup_top_value(arg))
            .and_then(|h| h.ty.clone())
    }

    /// Pick the best function overload (by arg types) from the by_name hits.
    fn pick_func_by_args(&self, hits: &[usize], args: &[Option<String>]) -> Option<&Hoverable> {
        let mut best: Option<&Hoverable> = None;
        let mut best_score = i32::MIN;
        for &i in hits {
            let h = &self.all[i];
            if h.is_type {
                continue;
            }
            let score = match_score(&h.param_tys, args);
            if best.is_none() || score > best_score {
                best = Some(h);
                best_score = score;
            }
        }
        best
    }

    /// Infer the declared types of the arguments of the call whose callee is
    /// the identifier at (line, word_start..word_end).
    fn call_arg_tys(
        &self,
        word: &str,
        line: u32,
        word_start: u32,
        word_end: u32,
    ) -> Vec<Option<String>> {
        let mut calls: Vec<&Expr> = Vec::new();
        for d in &self.file.decls {
            self.collect_calls_in_decl(d, &mut calls);
        }
        let call = calls.into_iter().find(|c| {
            if let Expr::Call { callee, .. } = c {
                if let Expr::Name { name, pos, .. } = callee.as_ref() {
                    return name == word
                        && pos.line == line + 1
                        && pos.col == word_start + 1
                        && pos.end_col == word_end + 1;
                }
            }
            false
        });
        match call {
            Some(Expr::Call { args, .. }) => args
                .iter()
                .map(|a| infer_init_expr_at(&a.value, self, line, word_end))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn collect_calls_in_decl<'x>(&self, d: &'x Decl, out: &mut Vec<&'x Expr>) {
        match d {
            Decl::Func { body, .. } => self.collect_calls_in_body(body, out),
            Decl::PrimaryCtor { body, .. } => self.collect_calls_in_body(body, out),
            Decl::Class { members, .. }
            | Decl::Struct { members, .. }
            | Decl::Interface { members, .. }
            | Decl::Extend { members, .. } => {
                for m in members {
                    self.collect_calls_in_decl(m, out);
                }
            }
            Decl::Var { init: Some(i), .. } => self.collect_calls_in_expr(i, out),
            Decl::Var { .. } => {}
            _ => {}
        }
    }

    fn collect_calls_in_body<'x>(&self, body: &'x Body, out: &mut Vec<&'x Expr>) {
        if let Body::Block(stmts) = body {
            for s in stmts {
                self.collect_calls_in_expr(s, out);
            }
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn collect_calls_in_expr<'x>(&self, e: &'x Expr, out: &mut Vec<&'x Expr>) {
        if let Expr::Call { .. } = e {
            out.push(e);
        }
        match e {
            Expr::Call { callee, args, .. } => {
                self.collect_calls_in_expr(callee, out);
                for a in args {
                    self.collect_calls_in_expr(&a.value, out);
                }
            }
            Expr::Member { object, .. } => self.collect_calls_in_expr(object, out),
            Expr::Lambda { body, .. } => self.collect_calls_in_expr(body, out),
            Expr::Block { stmts, .. } => {
                for s in stmts {
                    self.collect_calls_in_expr(s, out);
                }
            }
            Expr::LetPatternDestructor { initializer, .. } => {
                self.collect_calls_in_expr(initializer, out)
            }
            Expr::Binary { lhs, rhs, .. }
            | Expr::Range {
                start: lhs,
                end: rhs,
                ..
            } => {
                self.collect_calls_in_expr(lhs, out);
                self.collect_calls_in_expr(rhs, out);
            }
            Expr::Assign { lhs, rhs, .. } => {
                self.collect_calls_in_expr(lhs, out);
                self.collect_calls_in_expr(rhs, out);
            }
            Expr::Subscript { object, index, .. } => {
                self.collect_calls_in_expr(object, out);
                self.collect_calls_in_expr(index, out);
            }
            Expr::Paren { inner, .. }
            | Expr::Unary { inner, .. }
            | Expr::IncOrDec { inner, .. }
            | Expr::Return {
                value: Some(inner), ..
            }
            | Expr::Is { inner, .. }
            | Expr::As { inner, .. }
            | Expr::Optional { inner, .. }
            | Expr::OptionalChain { inner, .. }
            | Expr::TrailingClosure { closure: inner, .. } => {
                self.collect_calls_in_expr(inner, out)
            }
            Expr::If {
                cond, then, els, ..
            } => {
                self.collect_calls_in_expr(cond, out);
                self.collect_calls_in_expr(then, out);
                if let Some(e) = els {
                    self.collect_calls_in_expr(e, out);
                }
            }
            Expr::While { cond, body, .. } | Expr::DoWhile { cond, body, .. } => {
                self.collect_calls_in_expr(cond, out);
                self.collect_calls_in_expr(body, out);
            }
            Expr::ForIn { iter, body, .. } => {
                self.collect_calls_in_expr(iter, out);
                self.collect_calls_in_expr(body, out);
            }
            Expr::Match {
                scrutinee, cases, ..
            } => {
                self.collect_calls_in_expr(scrutinee, out);
                for c in cases {
                    self.collect_calls_in_expr(&c.body, out);
                }
            }
            Expr::Interpolation { parts, .. } | Expr::StrInterpolation { parts, .. } => {
                for p in parts {
                    if let cj_ast::InterpPart::Expr(pe) = p {
                        self.collect_calls_in_expr(pe, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// Byte right after the word (0-based column), for `.`-receiver checks.
    fn byte_after(&self, line: u32, col: u32) -> Option<u8> {
        let l = self.source_line(line + 1);
        l.as_bytes().get(col as usize).copied()
    }

    fn is_call_at(&self, word: &str, line: u32, word_end: u32) -> bool {
        let l = self.source_line(line + 1);
        let col = word_end as usize;
        let after = &l[col..];
        let after = after.trim_start();
        if after.starts_with('(') {
            return true;
        }
        if let Some(lt) = after.find('<') {
            if after[lt..]
                .find('>')
                .is_some_and(|gt| after[lt + gt + 1..].trim_start().starts_with('('))
            {
                return true;
            }
        }
        // word is the last identifier (call without parens — e.g. pipe `f ~>`)
        let _ = word;
        false
    }

    fn is_type_position(&self, word: &str, line: u32, word_start: u32) -> bool {
        let l = self.source_line(line + 1);
        let col = word_start as usize;
        if col > l.len() {
            return false;
        }
        let before = &l[..col];
        // check explicit keyword prefixes (as, is, extend, ->, =>) — these
        // are the most common type-position indicators in the official suite
        for kw in ["as ", "is ", "extend ", "-> ", "=> "] {
            if before.ends_with(kw) {
                return true;
            }
        }
        let lastch = before.trim_end().chars().next_back();
        match lastch {
            Some(':') | Some('<') | Some('&') | Some('|') | Some('(') | Some('>') | Some(',') => {
                word.chars()
                    .next()
                    .map(|c| c.is_uppercase() || c == '_')
                    .unwrap_or(false)
                    && (self.types.contains_key(word) || self.std.contains_key(word))
            }
            _ => false,
        }
    }

    fn lookup_local(&self, word: &str, line: u32, character: u32) -> Option<&Hoverable> {
        let idxs = self.locals.get(word)?;
        let mut best: Option<&Hoverable> = None;
        for i in idxs {
            let h = &self.all[*i];
            // visible only after its own declaration point
            if h.line > line + 1 || (h.line == line + 1 && h.col > character) {
                continue;
            }
            if best.is_none() || (h.line, h.col) > (best.unwrap().line, best.unwrap().col) {
                best = Some(h);
            }
        }
        best
    }

    fn lookup_top_value(&self, word: &str) -> Option<&Hoverable> {
        let hits = self.by_name.get(word)?;
        hits.iter().find_map(|&i| {
            let h = &self.all[i];
            if !h.is_type {
                Some(h)
            } else {
                None
            }
        })
    }

    fn pick_non_type(&self, hits: &[usize]) -> Option<&Hoverable> {
        hits.iter().find_map(|&i| {
            let h = &self.all[i];
            if !h.is_type {
                Some(h)
            } else {
                None
            }
        })
    }

    fn lookup_type(&self, name: &str) -> Option<&Hoverable> {
        self.types
            .get(name)
            .and_then(|&i| self.all.get(i))
            .or_else(|| self.lookup_std(name))
    }

    fn lookup_std(&self, name: &str) -> Option<&Hoverable> {
        self.std.get(name)
    }

    /// Innermost container whose member block spans `line` (0-based).
    fn enclosing_container(&self, line: u32, _character: u32) -> Option<&Container> {
        self.containers
            .iter()
            .filter(|c| c.name_line <= line + 1 && c.last_member_line > line)
            .max_by_key(|c| c.name_line)
    }
}

// ================================================================
// Rendering helpers
// ================================================================

fn render_type(t: &Type) -> String {
    match t {
        Type::Ref { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                format!("{name}<{}>", render_type_list(args))
            }
        }
        Type::Qualified { name, .. } => name.clone(),
        Type::Option { inner, .. } => format!("?{}", render_type(inner)),
        Type::Constant { inner, .. } => format!("const {}", render_type(inner)),
        Type::VArray { inner, .. } => format!("VArray<{}>", render_type(inner)),
        Type::Primitive { kind, .. } => primitive_name(kind).to_string(),
        Type::Paren { inner, .. } => format!("({})", render_type(inner)),
        Type::Func { params, ret, .. } => {
            format!("({}) -> {}", render_type_list(params), render_type(ret))
        }
        Type::Tuple { elements, .. } => format!("({})", render_type_list(elements)),
        Type::This(_) => "this".to_string(),
        Type::Invalid(_) => "?".to_string(),
    }
}

fn render_type_list(ts: &[Type]) -> String {
    ts.iter().map(render_type).collect::<Vec<_>>().join(", ")
}

fn primitive_name(k: &PrimitiveKind) -> &'static str {
    match k {
        PrimitiveKind::Int8 => "Int8",
        PrimitiveKind::Int16 => "Int16",
        PrimitiveKind::Int32 => "Int32",
        PrimitiveKind::Int64 => "Int64",
        PrimitiveKind::IntNative => "Int64",
        PrimitiveKind::UInt8 => "UInt8",
        PrimitiveKind::UInt16 => "UInt16",
        PrimitiveKind::UInt32 => "UInt32",
        PrimitiveKind::UInt64 => "UInt64",
        PrimitiveKind::UIntNative => "UInt64",
        PrimitiveKind::Float16 => "Float16",
        PrimitiveKind::Float32 => "Float32",
        PrimitiveKind::Float64 => "Float64",
        PrimitiveKind::Rune => "Rune",
        PrimitiveKind::Bool => "Bool",
        PrimitiveKind::Nothing => "Nothing",
        PrimitiveKind::Unit => "Unit",
        PrimitiveKind::VArray => "VArray",
        PrimitiveKind::String => "String",
    }
}

fn render_type_params(tps: &[cj_ast::TypeParam]) -> String {
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

/// Class/struct parents (`<: A & B`).
fn render_class_parents(parents: &[Type]) -> String {
    if parents.is_empty() {
        String::new()
    } else {
        format!(
            " <: {}",
            parents
                .iter()
                .map(render_type)
                .collect::<Vec<_>>()
                .join(" & ")
        )
    }
}

/// Interface parents (`extends A, B`).
fn render_interface_parents(parents: &[Type]) -> String {
    if parents.is_empty() {
        String::new()
    } else {
        format!(
            " extends {}",
            parents
                .iter()
                .map(render_type)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// Render a param list: `a: Int64, b!: Float64, c: Bool = true`.
fn render_params(params: &[cj_ast::Param], with_defaults: bool) -> String {
    params
        .iter()
        .map(|p| {
            let bang = if p.is_named { "!" } else { "" };
            let mut s = format!("{}{bang}: {}", p.name, render_type(&p.ty));
            if with_defaults {
                if let Some(def) = &p.default {
                    let d = render_default_expr(def);
                    if !d.is_empty() {
                        s.push_str(&format!(" = {d}"));
                    }
                }
            }
            s
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_default_expr(e: &Expr) -> String {
    match e {
        Expr::Lit { kind, value, .. } => match kind {
            cj_ast::LitKind::String | cj_ast::LitKind::JString => {
                format!("\"{}\"", value.replace('"', "\\\""))
            }
            cj_ast::LitKind::Rune | cj_ast::LitKind::RuneByte => format!("r'{}'", value),
            cj_ast::LitKind::Bool => value.clone(),
            cj_ast::LitKind::Unit => "()".to_string(),
            _ => value.clone(),
        },
        Expr::Name { name, .. } => name.clone(),
        Expr::ArrayLit { elements, .. } => format!(
            "[{}]",
            elements
                .iter()
                .map(render_default_expr)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Call { callee, args, .. } => {
            if let Expr::Name { name, .. } = callee.as_ref() {
                let a = args
                    .iter()
                    .map(|a| render_default_expr(&a.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}({a})")
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

/// Infer a literal's type from its kind.
fn lit_type(kind: &cj_ast::LitKind) -> String {
    match kind {
        cj_ast::LitKind::Integer => "Int64".to_string(),
        cj_ast::LitKind::Float => "Float64".to_string(),
        cj_ast::LitKind::Rune | cj_ast::LitKind::RuneByte => "Rune".to_string(),
        cj_ast::LitKind::String | cj_ast::LitKind::JString => "String".to_string(),
        cj_ast::LitKind::Bool => "Bool".to_string(),
        cj_ast::LitKind::Unit => "Unit".to_string(),
        cj_ast::LitKind::None => "Nothing".to_string(),
    }
}

/// Position-aware variant: `line`/`character` (0-based) let name references
/// in the initializer resolve against locals that are in scope there.
fn infer_init_expr_at(e: &Expr, idx: &Index, line: u32, character: u32) -> Option<String> {
    match e {
        Expr::Lit { kind, .. } => Some(lit_type(kind)),
        Expr::ArrayLit { elements, .. } => {
            let elem = elements
                .first()
                .and_then(|e| infer_init_expr_at(e, idx, line, character));
            elem.map(|t| format!("Array<{t}>"))
        }
        Expr::Call { callee, .. } => {
            // immediately-invoked lambda: `{a: Int64, b: Int64 => a + b}(1, 2)`
            // has the lambda's body type
            if let Expr::Lambda { body, .. } = callee.as_ref() {
                return infer_init_expr_at(body, idx, line, character);
            }
            match callee.as_ref() {
                // type args live on the callee Name, not on the Call node
                // (`Array<String>()` → Name{ name: "Array", type_args: [String] })
                Expr::Name {
                    name, type_args, ..
                } => {
                    if !type_args.is_empty() {
                        // type constructor `Array<String>()` → `Array<String>`
                        Some(format!(
                            "{name}<{}>",
                            type_args
                                .iter()
                                .map(render_type)
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    } else if idx.lookup_type(name).is_some() {
                        // type constructor `C7()` → `C7`
                        Some(name.clone())
                    } else {
                        // a plain function call → its declared return type
                        if let Some(hits) = idx.by_name.get(name) {
                            if let Some(hi) = idx.pick_non_type(hits) {
                                if let Some(t) = &hi.ty {
                                    return Some(t.clone());
                                }
                            }
                        }
                        Some("Unit".to_string())
                    }
                }
                Expr::Member { object, name, .. } => match object.as_ref() {
                    // enum case / static ctor with explicit type args: the result
                    // type is the receiver type as written (`Time3<Array<Int64>>`)
                    Expr::Name {
                        name: on,
                        type_args,
                        ..
                    } if !type_args.is_empty() => Some(format!(
                        "{on}<{}>",
                        type_args
                            .iter()
                            .map(render_type)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                    Expr::Name { name: on, .. } => {
                        // value receiver `recv.member(...)`: resolve through the
                        // member's declared type (`c1.C2()` → `Unit`)
                        if let Some(recv_ty) = idx.receiver_type(on, line, character) {
                            if let Some(hi) = idx.member_lookup(&recv_ty, name) {
                                return Some(hi.ty.clone().unwrap_or_else(|| "Unit".to_string()));
                            }
                        }
                        None
                    }
                    Expr::Optional { inner, .. } => {
                        // optional receiver `c2?.m()`: unwrap, resolve, re-wrap
                        if let Expr::Name { name: on, .. } = inner.as_ref() {
                            if let Some(recv_ty) = idx.receiver_type(on, line, character) {
                                if let Some(hi) = idx.member_lookup(&recv_ty, name) {
                                    let t = hi.ty.clone().unwrap_or_else(|| "Unit".to_string());
                                    return Some(format!("Option<{t}>"));
                                }
                            }
                        }
                        None
                    }
                    _ => None,
                },
                _ => None,
            }
        }
        // member access on a value/static receiver: resolve the member's type
        // (`ab.c1` → Int32, `Option<OptionC>.None` → Option<OptionC>)
        Expr::Member { object, name, .. } => match object.as_ref() {
            Expr::Name {
                name: on,
                type_args,
                ..
            } if !type_args.is_empty() => Some(format!(
                "{on}<{}>",
                type_args
                    .iter()
                    .map(render_type)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            Expr::Name { name: on, .. } => {
                if let Some(recv_ty) = idx.receiver_type(on, line, character) {
                    if let Some(hi) = idx.member_lookup(&recv_ty, name) {
                        return Some(hi.ty.clone().unwrap_or_else(|| "Unit".to_string()));
                    }
                }
                None
            }
            Expr::Optional { inner, .. } => {
                // optional receiver `c2?.item`: unwrap, resolve, re-wrap
                if let Expr::Name { name: on, .. } = inner.as_ref() {
                    if let Some(recv_ty) = idx.receiver_type(on, line, character) {
                        if let Some(hi) = idx.member_lookup(&recv_ty, name) {
                            let t = hi.ty.clone().unwrap_or_else(|| "Unit".to_string());
                            return Some(format!("Option<{t}>"));
                        }
                    }
                }
                None
            }
            _ => None,
        },
        // optional chain `c2?.item`: the member type wrapped in Option
        Expr::OptionalChain { inner, .. } => {
            let t = infer_init_expr_at(inner, idx, line, character)?;
            Some(format!("Option<{t}>"))
        }
        Expr::As { ty, .. } => Some(format!("Option<{}>", render_type(ty))),
        Expr::Name { name, .. } => {
            // resolve the referenced symbol's declared type; locals (params
            // and earlier local vars) take precedence over top-level names
            if let Some(hi) = idx
                .lookup_local(name, line, character)
                .or_else(|| idx.lookup_top_value(name))
            {
                return hi.ty.clone();
            }
            idx.lookup_std(name).and_then(|s| s.ty.clone())
        }
        Expr::Subscript { object, .. } => {
            let oty = infer_init_expr_at(object, idx, line, character)?;
            if let Some(inner) = oty.strip_prefix("Array<") {
                inner.strip_suffix('>').map(str::to_string)
            } else {
                None
            }
        }
        Expr::Binary { lhs, .. } => infer_init_expr_at(lhs, idx, line, character),
        Expr::Block { stmts, .. } => stmts
            .last()
            .and_then(|s| infer_init_expr_at(s, idx, line, character)),
        Expr::Paren { inner, .. } => infer_init_expr_at(inner, idx, line, character),
        Expr::StrInterpolation { .. } | Expr::Interpolation { .. } => Some("String".to_string()),
        Expr::Lambda { params, body, .. } => {
            // a lambda value has a function type `(P...) -> R`
            let ptys = params
                .iter()
                .map(|p| render_type(&p.ty))
                .collect::<Vec<_>>()
                .join(", ");
            let ret =
                infer_init_expr_at(body, idx, line, character).unwrap_or_else(|| "?".to_string());
            Some(format!("({ptys}) -> {ret}"))
        }
        _ => None,
    }
}

/// Strip `Option<X>` / `?X` wrappers for member-lookup inner type.
fn strip_std_wrap(ty: Option<String>) -> Option<String> {
    let ty = ty?;
    if let Some(inner) = ty.strip_prefix("Option<") {
        if let Some(i) = inner.strip_suffix('>') {
            return Some(i.to_string());
        }
    }
    if let Some(inner) = ty.strip_prefix('?') {
        return Some(inner.to_string());
    }
    Some(ty)
}

fn h_sig_len(h: &Hoverable) -> usize {
    h.signature.matches('(').count()
}

/// Score a call candidate against the inferred argument types: exact
/// param-type matches weigh heavily; arity distance is a penalty.
fn match_score(ptys: &[String], args: &[Option<String>]) -> i32 {
    let mut exact = 0i32;
    for (p, a) in ptys.iter().zip(args.iter()) {
        if let Some(a) = a {
            if a == p {
                exact += 1;
            }
        }
    }
    exact * 10 - (ptys.len() as i32 - args.len() as i32).abs() * 5
}

// ================================================================
// std.core symbols
// ================================================================

struct StdSym {
    name: &'static str,
    sig: &'static str,
    file: &'static str,
}

const STD_SYMS: &[StdSym] = &[
    StdSym {
        name: "String",
        sig: "public struct String",
        file: "string.cj",
    },
    StdSym {
        name: "Array",
        sig: "public struct Array<T>",
        file: "array.cj",
    },
    StdSym {
        name: "VArray",
        sig: "public struct VArray<T>",
        file: "varray.cj",
    },
    StdSym {
        name: "Exception",
        sig: "public open class Exception <: ToString",
        file: "exception.cj",
    },
    StdSym {
        name: "Any",
        sig: "public interface Any",
        file: "any.cj",
    },
    StdSym {
        name: "ToString",
        sig: "public interface ToString",
        file: "toString.cj",
    },
    StdSym {
        name: "Option",
        sig: "public enum Option<out T>",
        file: "option.cj",
    },
];

// ================================================================
// Misc helpers
// ================================================================

fn word_at(source: &str, line: u32, character: u32) -> Option<Word> {
    let l = source.lines().nth(line as usize)?;
    let col = character as usize;
    if col > l.len() {
        return None;
    }
    let bytes = l.as_bytes();
    let mut start = col;
    while start > 0 && is_word_byte(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < bytes.len() && is_word_byte(bytes[end]) {
        end += 1;
    }
    if start == end {
        // cursor right after an identifier (e.g. on `(` after `Row`)
        if col > 0 && is_word_byte(bytes[col - 1]) {
            let mut s = col - 1;
            while s > 0 && is_word_byte(bytes[s - 1]) {
                s -= 1;
            }
            return Some(Word {
                text: l[s..col].to_string(),
                line,
                start: s as u32,
                end: col as u32,
            });
        }
        return None;
    }
    Some(Word {
        text: l[start..end].to_string(),
        line,
        start: start as u32,
        end: end as u32,
    })
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// True when `name` appears as a whole word in `hay`.
fn word_in(hay: &str, name: &str) -> bool {
    let mut rest = hay;
    while let Some(i) = rest.find(name) {
        let before_ok = i == 0 || !rest[..i].chars().next_back().unwrap().is_alphanumeric();
        let after = &rest[i + name.len()..];
        let after_ok = after
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        rest = after;
    }
    false
}

fn strip_trailing_comment(s: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut prev = '\0';
    for ch in s.chars() {
        if in_str {
            out.push(ch);
            if ch == '"' && prev != '\\' {
                in_str = false;
            }
            prev = ch;
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if ch == '/' && prev == '/' {
            out.pop();
            break;
        } else {
            out.push(ch);
        }
        prev = ch;
    }
    out.trim().trim_end_matches(',').trim_end().to_string()
}

fn container_type_name(label: &str) -> Option<String> {
    // The label format is `class Foo` / `struct Foo` / `interface Foo` /
    // `enum Foo`; extract the bare type name (may be followed by type-param
    // or inheritance text in the label, so take the first token).
    for prefix in ["class ", "struct ", "interface ", "enum "] {
        if let Some(rest) = label.strip_prefix(prefix) {
            let name = rest.split(|c: char| c.is_whitespace() || c == '<').next()?;
            return Some(name.to_string());
        }
    }
    None
}
