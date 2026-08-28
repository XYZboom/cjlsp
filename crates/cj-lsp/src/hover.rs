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

/// Find the declaration at the cursor and return it as an LSP Location
/// (uri + name span). Reuses the hover index's span matching.
pub fn definition_at(file: &File, uri: &str, line: u32, character: u32) -> Value {
    // definition only needs name-span matching; source text is not required
    // for span matching (we match against the parsed decl spans).
    let idx = Index::new(file, "", None, "");
    if let Some(hi) = idx.hit_test(line, character) {
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
// Rendering
// ================================================================

#[derive(Clone, Copy)]
struct LspRange(u32, u32, u32); // line, start col, end col (0-based)

fn render_hover(hi: &Hoverable, r: LspRange) -> Value {
    let mut value = format!(
        "Declared in: {}  \nPackage info: {}  \n\n```cangjie\n{}\n```\n",
        hi.declared_in,
        hi.pkg.as_deref().unwrap_or(""),
        hi.signature
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
            total_lines: source.lines().count() as u32,
            package,
            file_name,
            all: Vec::new(),
            by_name: HashMap::new(),
            locals: HashMap::new(),
            types: HashMap::new(),
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
                    .or_else(|| infer_body_ret(body));
                let sig = format!(
                    "{mods}func {name}{}({}){}",
                    render_type_params(type_params),
                    render_params(params, false),
                    ret_s
                        .as_deref()
                        .map(|t| format!(": {t}"))
                        .unwrap_or_default()
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
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
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
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
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
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
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
                };
                self.push(hi, true);
                self.types.insert(name.clone(), self.all.len() - 1);
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
                // declared/inferred type (bare, no `: ` prefix); the inferred
                // initializer type is used when no annotation is written
                // (official renders `var arr1: Array<Int64> = [...]`).
                let disp = ty
                    .as_ref()
                    .map(render_type)
                    .filter(|r| r != "?")
                    .or_else(|| {
                        if ty.is_none() {
                            init.as_ref().and_then(|i| infer_init_expr(i, self))
                        } else {
                            None
                        }
                    });
                let ty_disp = match ty {
                    Some(t) => {
                        let r = render_type(t);
                        if r == "?" {
                            self.slice_type_from_source(name_pos.line, name)
                                .map(|s| format!(": {s}"))
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
                };
                let idx = self.push(hi, false);
                if let Some(c) = cname {
                    self.members.entry(c).or_default().push(idx);
                }
                // `init(...)` ctor params are hoverable locals too
                self.collect_params(params);
                if let Body::Block(stmts) = body {
                    self.collect_locals(stmts, pos.line);
                }
            }
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
                let hi = Hoverable {
                    name: name.clone(),
                    line: pos.line,
                    col: pos.col,
                    end_col: pos.end_col,
                    signature: sig,
                    doc: None,
                    declared_in: self.file_name.to_string(),
                    pkg: self.package.map(str::to_string),
                    ty: Some(render_type(ty)),
                    is_type: false,
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
                    && !self.before_kind(name_pos).trim_end().ends_with("func") => {
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
                    };
                    let idx = self.push(hi, false);
                    if let Some(cn) = container_type_name(container) {
                        self.members
                            .entry(cn)
                            .or_default()
                            .push(idx);
                    }
                    self.containers[ci].last_member_line =
                        self.containers[ci].last_member_line.max(name_pos.line);
                    // also collect the ctor params as locals (they are hoverable)
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
            };
            let idx = self.push(hi, false);
            self.locals.entry(p.name.clone()).or_default().push(idx);
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
                let inferred =
                    infer_init_expr_at(initializer, self, pos.line.saturating_sub(1), 0);
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
                        let ty_disp = ty
                            .as_ref()
                            .map(render_type)
                            .or_else(|| inferred.clone());
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
            .or_else(|| infer_body_ret(body));
        let sig = format!(
            "func {name}({}){}",
            render_params(params, false),
            ret_s
                .as_deref()
                .map(|t| format!(": {t}"))
                .unwrap_or_default()
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
        })
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
        if !has_vis {
            if is_public {
                mods.insert(0, "public");
            } else {
                mods.insert(0, "internal");
            }
        }
        // interface member funcs: abstract (no body) / open (has body)
        let in_interface = container
            .map(|c| c.starts_with("interface "))
            .unwrap_or(false)
            && member;
        if in_interface
            && !mods.contains(&"abstract")
            && !mods.contains(&"open")
            && !mods.contains(&"static")
        {
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
        if after.is_empty() || after.starts_with('{') {
            String::new()
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

    fn resolve(&self, word: &str, line: u32, character: u32, word_start: u32) -> Option<&Hoverable> {
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
                if let Some(hi) = self.member_lookup(&recv_ty, word) {
                    return Some(hi);
                }
            }
        }
        // call position: `word(` or `word<T>(` → ctor/init or func
        if self.is_call_at(word, line, character) {
            if self.types.contains_key(word) {
                if let Some(hi) = self.member_lookup(word, "init") {
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
                return self
                    .lookup_local(word, line, character)
                    .or_else(|| self.pick_non_type(hits));
            }
            return self.lookup_local(word, line, character);
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

    fn is_call_at(&self, word: &str, line: u32, character: u32) -> bool {
        let l = self.source_line(line + 1);
        let col = character as usize;
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

/// Infer a function's return type from its body when not declared.
fn infer_body_ret(body: &Body) -> Option<String> {
    let stmts = match body {
        Body::Empty => return Some("Unit".to_string()),
        Body::Block(stmts) => stmts,
    };
    match stmts.last() {
        Some(e) => infer_expr_type(e),
        None => Some("Unit".to_string()),
    }
}

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

/// Type of the last statement of a block (for return-type inference).
fn infer_expr_type(e: &Expr) -> Option<String> {
    match e {
        Expr::Lit { kind, .. } => Some(lit_type(kind)),
        Expr::Name { .. } | Expr::Call { .. } => None,
        Expr::Binary { lhs, .. } => infer_expr_type(lhs),
        Expr::Block { stmts, .. } => stmts.last().and_then(infer_expr_type),
        Expr::Lambda { .. } => Some("Unit".to_string()),
        _ => None,
    }
}

/// Infer the declared type of a `var x = <init>` when no explicit type is
/// written. Returns a display string (e.g. `Array<Int64>`, `Option<Base1>`).
fn infer_init_expr(e: &Expr, idx: &Index) -> Option<String> {
    infer_init_expr_at(e, idx, 0, 0)
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
        Expr::Call { callee, .. } => match callee.as_ref() {
            // type args live on the callee Name, not on the Call node
            // (`Array<String>()` → Name{ name: "Array", type_args: [String] })
            Expr::Name {
                name, type_args, ..
            } => {
                if type_args.is_empty() {
                    Some(name.clone())
                } else {
                    Some(format!(
                        "{name}<{}>",
                        type_args
                            .iter()
                            .map(render_type)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
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
                            if let Some(t) = &hi.ty {
                                return Some(t.clone());
                            }
                        }
                    }
                    Some(on.clone())
                }
                _ => None,
            },
            _ => None,
        },
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
        Expr::StrInterpolation { .. } | Expr::Interpolation { .. } => Some("String".to_string()),
        Expr::Lambda { body, .. } => infer_expr_type(body),
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
