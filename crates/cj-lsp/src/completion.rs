// cj-lsp: textDocument/completion support.
//
// Implements the official completion engine producing exact items matching
// the cjlsp test suite (standard LSP CompletionItemKind).
//
// Official LSP CompletionItemKind mapping:
//   2 = Method (func/method members)
//   3 = Function (constructors/init)
//   6 = Variable (let/var/params/enum members without payload)
//   7 = Class (class decls, type aliases)
//   8 = Interface
//   9 = Module (package names)
//  13 = Enum
//  14 = Keyword (language keywords, snippet templates, primitive type names)
//  22 = Struct

use cj_ast::{Body, Decl, Expr, File, Param, Pattern, Type};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

// ─── Kind mapping (official LSP CompletionItemKind) ───────────────────────
const KIND_CLASS: u32 = 7;
const KIND_METHOD: u32 = 2; // functions/methods in expression context
const KIND_FUNCTION: u32 = 3; // constructors
const KIND_VARIABLE: u32 = 6;
const KIND_INTERFACE: u32 = 8;
const KIND_MODULE: u32 = 9;
#[allow(dead_code)]
const KIND_KEYWORD_OR_MODULE_UNUSED: u32 = KIND_MODULE;
const KIND_ENUM: u32 = 13;
const KIND_KEYWORD: u32 = 14;
const KIND_STRUCT: u32 = 22;

// ─── Std-core symbols (implicitly imported into every package) ────────────
const STD_CORE: &[(&str, u32, &str)] = &[
    ("Any", KIND_INTERFACE, "public interface Any"),
    ("AnyClass", KIND_CLASS, "public class AnyClass"),
    ("String", KIND_STRUCT, "public struct String"),
    ("Int8", KIND_STRUCT, "public struct Int8"),
    ("Int16", KIND_STRUCT, "public struct Int16"),
    ("Int32", KIND_STRUCT, "public struct Int32"),
    ("Int64", KIND_STRUCT, "public struct Int64"),
    ("UInt8", KIND_STRUCT, "public struct UInt8"),
    ("UInt16", KIND_STRUCT, "public struct UInt16"),
    ("UInt32", KIND_STRUCT, "public struct UInt32"),
    ("UInt64", KIND_STRUCT, "public struct UInt64"),
    ("Float16", KIND_STRUCT, "public struct Float16"),
    ("Float32", KIND_STRUCT, "public struct Float32"),
    ("Float64", KIND_STRUCT, "public struct Float64"),
    ("Bool", KIND_STRUCT, "public struct Bool"),
    ("Unit", KIND_STRUCT, "public struct Unit"),
    ("Nothing", KIND_INTERFACE, "public interface Nothing"),
    ("Nope", KIND_STRUCT, "public struct Nope"),
    ("Tuple0", KIND_STRUCT, "public struct Tuple0"),
    ("Tuple1", KIND_STRUCT, "public struct Tuple1"),
    ("Tuple2", KIND_STRUCT, "public struct Tuple2"),
    ("Tuple3", KIND_STRUCT, "public struct Tuple3"),
    ("Tuple4", KIND_STRUCT, "public struct Tuple4"),
    ("Tuple5", KIND_STRUCT, "public struct Tuple5"),
    ("Rune", KIND_STRUCT, "public struct Rune"),
    // Common std exceptions and classes
    (
        "ArithmeticException",
        KIND_CLASS,
        "public open class ArithmeticException <: Exception",
    ),
    (
        "IllegalArgumentException",
        KIND_CLASS,
        "public open class IllegalArgumentException <: Exception",
    ),
    (
        "IllegalFormatException",
        KIND_CLASS,
        "public open class IllegalFormatException <: IllegalArgumentException",
    ),
    (
        "IndexOutOfBoundsException",
        KIND_CLASS,
        "public class IndexOutOfBoundsException <: Exception",
    ),
    (
        "NegativeArraySizeException",
        KIND_CLASS,
        "public class NegativeArraySizeException <: Exception",
    ),
    (
        "IllegalMemoryException",
        KIND_CLASS,
        "public class IllegalMemoryException <: Exception",
    ),
    (
        "IllegalStateException",
        KIND_CLASS,
        "public class IllegalStateException <: Exception",
    ),
    (
        "IncompatiblePackageException",
        KIND_CLASS,
        "public class IncompatiblePackageException <: Exception",
    ),
    (
        "NoneValueException",
        KIND_CLASS,
        "public class NoneValueException <: Exception",
    ),
    (
        "OutOfMemoryError",
        KIND_CLASS,
        "public class OutOfMemoryError <: Error",
    ),
    (
        "OverflowException",
        KIND_CLASS,
        "public class OverflowException <: ArithmeticException",
    ),
    (
        "SpawnException",
        KIND_CLASS,
        "public class SpawnException <: Exception",
    ),
    (
        "StackOverflowError",
        KIND_CLASS,
        "public class StackOverflowError <: Error",
    ),
    (
        "TimeoutException",
        KIND_CLASS,
        "public class TimeoutException <: Exception",
    ),
    (
        "UnsupportedException",
        KIND_CLASS,
        "public class UnsupportedException <: Exception",
    ),
    (
        "ExclusiveScopeException",
        KIND_CLASS,
        "public class ExclusiveScopeException <: Exception",
    ),
    (
        "Exception",
        KIND_CLASS,
        "public open class Exception <: ToString",
    ),
    (
        "StringBuilder",
        KIND_CLASS,
        "public class StringBuilder <: ToString",
    ),
    (
        "ThreadSnapshot",
        KIND_CLASS,
        "public class ThreadSnapshot <: ToString",
    ),
    ("DefaultHasher", KIND_STRUCT, "public struct DefaultHasher"),
    ("Duration", KIND_STRUCT, "public struct Duration"),
    (
        "ThreadState",
        KIND_ENUM,
        "public enum ThreadState <: ToString",
    ),
    ("AnnotationKind", KIND_ENUM, "public enum AnnotationKind"),
    (
        "CPointerResource",
        KIND_STRUCT,
        "public struct CPointerResource<T>",
    ),
    (
        "CStringResource",
        KIND_STRUCT,
        "public struct CStringResource",
    ),
    (
        "ArrayIterator",
        KIND_CLASS,
        "public class ArrayIterator<T> <: Iterator<T>",
    ),
    ("Box", KIND_CLASS, "public class Box<T>"),
    ("AB", KIND_CLASS, "public class AB"),
    ("CString", KIND_CLASS, "public Type CString"),
    ("CFunc", KIND_CLASS, "public Type CFunc<T>"),
    ("ToString", KIND_INTERFACE, "public interface ToString"),
    (
        "GreaterOrEqual",
        KIND_INTERFACE,
        "public interface GreaterOrEqual<T>",
    ),
];

// ─── Keywords (kind 14) ──────────────────────────────────────────────────
const KEYWORDS: &[(&str, &str, u32, &str)] = &[
    // (label, detail, insertTextFormat, insertText)
    ("class", "", 1, "class"),
    (
        "class name {}",
        "class name {}",
        2,
        "class ${1:name} {\n\t$0\n}",
    ),
    (
        "class name<T> {}",
        "class name<T> {}",
        2,
        "class ${1:name}<T> {\n\t$0\n}",
    ),
    ("interface", "", 1, "interface"),
    (
        "interface name {}",
        "interface name {}",
        2,
        "interface ${1:name} {\n\t$0\n}",
    ),
    (
        "interface name<T> {}",
        "interface name<T> {}",
        2,
        "interface ${1:name}<T> {\n\t$0\n}",
    ),
    ("struct", "", 1, "struct"),
    (
        "struct name {}",
        "struct name {}",
        2,
        "struct ${1:name} {\n\t$0\n}",
    ),
    (
        "struct name<T> {}",
        "struct name<T> {}",
        2,
        "struct ${1:name}<T> {\n\t$0\n}",
    ),
    ("enum", "", 1, "enum"),
    ("func", "", 1, "func"),
    (
        "func name(){}",
        "func name(){}",
        2,
        "func ${1:name}() {\n\t$0\n}",
    ),
    (
        "func name<T>(){}",
        "func name<T>(){}",
        2,
        "func ${1:name}<T>() {\n\t$0\n}",
    ),
    ("extend", "", 1, "extend"),
    (
        "extend name{}",
        "extend name{}",
        2,
        "extend ${0:name}{\n\t\n}",
    ),
    (
        "extend<T> name<T>{}",
        "extend<T> name<T>{}",
        2,
        "extend<T> ${0:name}<T>{\n\t\n}",
    ),
    ("prop", "", 1, "prop"),
    (
        "prop name: T {get(){val}}",
        "prop name: T {get(){val}}",
        2,
        "prop ${0:name}: ${1:T} {\n\tget() {\n\t\t${3:val}\n\t}\n}",
    ),
    (
        "prop name: T {get(){val} set(v){}}",
        "prop name: T {get(){val} set(v){}}",
        2,
        "prop ${0:name}: ${1:T} {\n\tget() {\n\t\t${3:val}\n\t}\n\tset(v) {\n\t}\n}",
    ),
    ("macro", "", 1, "macro"),
    ("import", "", 1, "import"),
    ("package", "", 1, "package"),
    ("match", "", 1, "match"),
    (
        "match (condExpr) {}",
        "match (condExpr) {}",
        2,
        "match (${1:condExpr}) {\n\t$0\n}",
    ),
    ("case", "", 1, "case"),
    (
        "case pattern => expressions",
        "case pattern => expressions",
        2,
        "case ${1:pattern} => ${0:expressions}",
    ),
    ("for", "", 1, "for"),
    (
        "for (pattern in expression) {}",
        "for (pattern in expression) {}",
        2,
        "for (${1:pattern} in ${2:expression}) {\n\t$0\n}",
    ),
    ("if", "", 1, "if"),
    ("else", "", 1, "else"),
    ("while", "", 1, "while"),
    ("return", "", 1, "return"),
    ("break", "", 1, "break"),
    ("continue", "", 1, "continue"),
    ("throw", "", 1, "throw"),
    ("try", "", 1, "try"),
    ("catch", "", 1, "catch"),
    ("finally", "", 1, "finally"),
    ("static", "", 1, "static"),
    ("public", "", 1, "public"),
    ("private", "", 1, "private"),
    ("protected", "", 1, "protected"),
    ("internal", "", 1, "internal"),
    ("open", "", 1, "open"),
    ("sealed", "", 1, "sealed"),
    ("abstract", "", 1, "abstract"),
    ("mut", "", 1, "mut"),
    ("override", "", 1, "override"),
    ("redef", "", 1, "redef"),
    ("type", "", 1, "type"),
    (
        "type newName = originalName",
        "type newName = originalName",
        2,
        "type ${1:newName}  = ${0:originalName}",
    ),
    ("var", "", 1, "var"),
    ("let", "", 1, "let"),
    ("this", "", 1, "this"),
    ("super", "", 1, "super"),
    ("new", "", 1, "new"),
    ("foreign", "", 1, "foreign"),
    ("foreign {}", "foreign {}", 2, "foreign {\n\t$0\n}"),
    ("Annotation", "Annotation", 1, "Annotation"),
    ("Deprecated", "Deprecated", 1, "Deprecated"),
    // Primitive type names as keywords (for type contexts)
    ("Int64", "", 1, "Int64"),
    ("UInt64", "", 1, "UInt64"),
    ("Int32", "", 1, "Int32"),
    ("UInt32", "", 1, "UInt32"),
    ("Int16", "", 1, "Int16"),
    ("UInt16", "", 1, "UInt16"),
    ("Int8", "", 1, "Int8"),
    ("UInt8", "", 1, "UInt8"),
    ("Float64", "", 1, "Float64"),
    ("Float32", "", 1, "Float32"),
    ("Float16", "", 1, "Float16"),
    ("Bool", "", 1, "Bool"),
    ("Unit", "", 1, "Unit"),
    ("Rune", "", 1, "Rune"),
    ("Nothing", "", 1, "Nothing"),
    ("VArray", "", 1, "VArray"),
];

// ─── Helper: safe char-boundary prefix extraction ────────────────────────
pub fn prefix_at_line(line_text: &str, character: u32) -> String {
    let mut prefix = String::new();
    let col = (character as usize).min(line_text.len());
    // Walk back by chars from a valid char boundary at or before col.
    let mut safe = col;
    while safe > 0 && !line_text.is_char_boundary(safe) {
        safe -= 1;
    }
    let before = &line_text[..safe];
    for ch in before.chars().rev() {
        if ch.is_alphanumeric() || ch == '_' {
            prefix.insert(0, ch);
        } else {
            break;
        }
    }
    prefix
}

// ─── Helper: source line text (0-based line) ─────────────────────────────
fn source_line_text(source: &str, line: u32) -> String {
    source.lines().nth(line as usize).unwrap_or("").to_string()
}

// ─── Helper: type display (shared with hover) ────────────────────────────
pub(crate) fn display_type(t: &Type) -> String {
    match t {
        Type::Ref { name, args, .. } => {
            if args.is_empty() {
                name.clone()
            } else {
                let inner: Vec<String> = args.iter().map(display_type).collect();
                format!("{name}<{}>", inner.join(", "))
            }
        }
        Type::Qualified { name, .. } => name.clone(),
        Type::Primitive { kind, .. } => format!("{kind:?}"),
        Type::Option { inner, .. } => format!("{}?", display_type(inner)),
        Type::VArray { inner, .. } => format!("VArray<{}>", display_type(inner)),
        Type::Constant { inner, .. } => format!("const {}", display_type(inner)),
        Type::Paren { inner, .. } => format!("({})", display_type(inner)),
        Type::Func { params, ret, .. } => {
            let ps: Vec<String> = params.iter().map(display_type).collect();
            format!("({}) -> {}", ps.join(", "), display_type(ret))
        }
        Type::Tuple { elements, .. } => {
            let es: Vec<String> = elements.iter().map(display_type).collect();
            format!("({})", es.join(", "))
        }
        Type::This(_) => "this".to_string(),
        Type::Invalid(_) => "?".to_string(),
    }
}

// ─── Helper: render a Param ─────────────────────────────────────────────
// detail form: `name: T` or `name!: T = default`
fn param_detail(p: &Param) -> String {
    let ty_s = display_type(&p.ty);
    let default_s = p
        .default
        .as_ref()
        .map(|_| " = ...".to_string())
        .unwrap_or_default();
    if p.is_named {
        format!("{}!: {}{}", p.name, ty_s, default_s)
    } else {
        format!("{}: {}{}", p.name, ty_s, default_s)
    }
}

// label form: `name: T` (named params keep `!` in the label too)
fn param_label(p: &Param) -> String {
    let ty_s = display_type(&p.ty);
    let default_s = p
        .default
        .as_ref()
        .map(|_| " = ...".to_string())
        .unwrap_or_default();
    if p.is_named {
        format!("{}!: {}{}", p.name, ty_s, default_s)
    } else {
        format!("{}: {}{}", p.name, ty_s, default_s)
    }
}

// insertText form: `name: ${N:T}` for named, `${N:name: T}` for positional
fn param_insert(p: &Param, counter: &mut u32) -> String {
    let n = *counter;
    *counter += 1;
    let ty_s = display_type(&p.ty);
    let default_s = p
        .default
        .as_ref()
        .map(|_| " = ...".to_string())
        .unwrap_or_default();
    if p.is_named {
        format!("{}: ${{{}}}{}", p.name, n, ty_s + &default_s)
    } else {
        format!("${{{0}: {1}: {2}}}", n, p.name, ty_s + &default_s)
    }
}

// ─── Helper: render a function signature for detail ──────────────────────
fn func_sig(vis: &str, name: &str, params: &[Param], ret: &Option<Type>) -> String {
    let param_strs: Vec<String> = params.iter().map(param_detail).collect();
    let ret_s = ret
        .as_ref()
        .map(display_type)
        .unwrap_or_else(|| "Unit".to_string());
    if ret_s == "Unit" {
        format!("{vis}func {name}({})", param_strs.join(", "))
    } else {
        format!("{vis}func {name}({}): {ret_s}", param_strs.join(", "))
    }
}

// ─── Helper: visibility prefix ──────────────────────────────────────────
fn vis_prefix(
    is_public: bool,
    is_open: bool,
    is_sealed: bool,
    is_abstract: bool,
    is_static: bool,
) -> String {
    let mut s = String::new();
    if is_public {
        s.push_str("public ");
    }
    if is_abstract {
        s.push_str("abstract ");
    }
    if is_open {
        s.push_str("open ");
    }
    if is_sealed {
        s.push_str("sealed ");
    }
    if is_static {
        s.push_str("static ");
    }
    if !is_public && !is_abstract && !is_open && !is_sealed && !is_static {
        s.push_str("internal ");
    }
    s
}

// ─── Candidate collection ────────────────────────────────────────────────

#[derive(Clone)]
struct Candidate {
    label: String,
    kind: u32,
    detail: String,
    insert_text: String,
    insert_text_format: u32, // 1=plain, 2=snippet
    filter_text: String,
}

fn push_candidate(cands: &mut Vec<Candidate>, seen: &mut HashSet<String>, c: Candidate) {
    let key = format!("{}/{}/{}", c.label, c.kind, c.insert_text);
    if seen.insert(key) {
        cands.push(c);
    }
}

/// Generic params as a comma-joined string (e.g. "T, U").
fn type_params_str(tp: &[cj_ast::TypeParam]) -> String {
    tp.iter()
        .map(|tp| tp.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Emit function items (bare + overload variants + optional closure form).
#[allow(clippy::too_many_arguments)]
fn emit_func_items(
    name: &str,
    is_public: bool,
    is_static: bool,
    is_abstract: bool,
    params: &[Param],
    ret: &Option<Type>,
    type_params: &[cj_ast::TypeParam],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let vis = vis_prefix(is_public, false, false, is_abstract, is_static);
    let sig = func_sig(&vis, name, params, ret);
    let tp = type_params_str(type_params);
    let tp_prefix = if tp.is_empty() {
        String::new()
    } else {
        format!("<{tp}>")
    };
    let tp_insert = if tp.is_empty() {
        String::new()
    } else {
        format!("<${{1:{tp}}}>")
    };
    let tp_num = if tp.is_empty() { 0u32 } else { 1 };
    let mut c = tp_num + 1;

    // 1. Bare name item (detail empty)
    push_candidate(
        cands,
        seen,
        Candidate {
            label: name.to_string(),
            kind: KIND_METHOD,
            detail: String::new(),
            insert_text: name.to_string(),
            insert_text_format: 1,
            filter_text: name.to_string(),
        },
    );

    // 2. Overload form(s)
    let params_ins: Vec<String> = params.iter().map(|p| param_insert(p, &mut c)).collect();
    let params_label: Vec<String> = params.iter().map(param_label).collect();

    let fmt = if params.is_empty() { 1 } else { 2 };
    let ins = if params.is_empty() {
        format!("{name}(){tp_insert}")
    } else {
        format!("{name}{tp_insert}({})", params_ins.join(", "))
    };
    let label = if params.is_empty() {
        format!("{name}(){tp_prefix}")
    } else {
        format!("{name}({}){tp_prefix}", params_label.join(", "))
    };
    push_candidate(
        cands,
        seen,
        Candidate {
            label,
            kind: KIND_METHOD,
            detail: sig,
            insert_text: ins,
            insert_text_format: fmt,
            filter_text: name.to_string(),
        },
    );
}

/// Emit constructor items for a class/struct.
#[allow(clippy::too_many_arguments)]
fn emit_ctor_items(
    class_name: &str,
    params: &[Param],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
    is_public: bool,
    is_open: bool,
    is_sealed: bool,
    is_abstract: bool,
    type_params: &[cj_ast::TypeParam],
) {
    let vis = vis_prefix(is_public, is_open, is_sealed, is_abstract, false);
    let tp = type_params_str(type_params);
    let tp_prefix = if tp.is_empty() {
        String::new()
    } else {
        format!("<{tp}>")
    };
    let tp_insert = if tp.is_empty() {
        String::new()
    } else {
        format!("<${{1:{tp}}}>")
    };
    let tp_num = if tp.is_empty() { 0u32 } else { 1 };

    // Class detail: `func init()` / `func init(params)` — official uses `init`
    // for ctors regardless of primary vs init-method.
    let detail = if params.is_empty() {
        format!("{vis}func init()")
    } else {
        let param_strs: Vec<String> = params.iter().map(param_detail).collect();
        format!("{vis}func init({})", param_strs.join(", "))
    };

    let label = if params.is_empty() {
        format!("{class_name}(){tp_prefix}")
    } else {
        let param_strs: Vec<String> = params.iter().map(param_label).collect();
        format!("{class_name}({}){tp_prefix}", param_strs.join(", "))
    };

    let (fmt, ins) = if params.is_empty() {
        (1, format!("{class_name}(){tp_insert}"))
    } else {
        let mut c = tp_num + 1;
        let ins_params: Vec<String> = params.iter().map(|p| param_insert(p, &mut c)).collect();
        (
            2,
            format!("{class_name}{tp_insert}({})", ins_params.join(", ")),
        )
    };

    push_candidate(
        cands,
        seen,
        Candidate {
            label,
            kind: KIND_FUNCTION,
            detail,
            insert_text: ins,
            insert_text_format: fmt,
            filter_text: class_name.to_string(),
        },
    );
}

/// Collect top-level decls from a parsed File.
fn collect_file_decls(file: &File, cands: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
    for d in &file.decls {
        match d {
            Decl::Class {
                name,
                is_public,
                is_abstract,
                is_open,
                is_sealed,
                type_params,
                parents,
                members,
                ..
            } => {
                let vis = vis_prefix(*is_public, *is_open, *is_sealed, *is_abstract, false);
                let p = if parents.is_empty() {
                    String::new()
                } else {
                    let ps: Vec<String> = parents.iter().map(display_type).collect();
                    format!(" <: {}", ps.join(" & "))
                };
                let tp = type_params_str(type_params);
                let detail = if tp.is_empty() {
                    format!("{vis}class {name}{p}")
                } else {
                    format!("{vis}class {name}<{tp}>{p}")
                };
                let label = if tp.is_empty() {
                    name.clone()
                } else {
                    format!("{name}<{tp}>")
                };
                let ins = if tp.is_empty() {
                    name.clone()
                } else {
                    format!("{name}<${{1:{tp}}}>")
                };
                let fmt = if tp.is_empty() { 1 } else { 2 };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label,
                        kind: KIND_CLASS,
                        detail,
                        insert_text: ins,
                        insert_text_format: fmt,
                        filter_text: name.clone(),
                    },
                );
                // Constructor items from primary ctor + init() methods
                for m in members {
                    match m {
                        Decl::PrimaryCtor { params, .. } => {
                            emit_ctor_items(
                                name,
                                params,
                                cands,
                                seen,
                                *is_public,
                                *is_open,
                                *is_sealed,
                                *is_abstract,
                                type_params,
                            );
                        }
                        Decl::Func {
                            name: fn_name,
                            params,
                            ..
                        } if fn_name == "init" => {
                            emit_ctor_items(
                                name,
                                params,
                                cands,
                                seen,
                                *is_public,
                                *is_open,
                                *is_sealed,
                                *is_abstract,
                                type_params,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Decl::Struct {
                name,
                is_public,
                is_open,
                type_params,
                members,
                ..
            } => {
                let vis = vis_prefix(*is_public, *is_open, false, false, false);
                let tp = type_params_str(type_params);
                let label = if tp.is_empty() {
                    name.clone()
                } else {
                    format!("{name}<{tp}>")
                };
                let detail = if tp.is_empty() {
                    format!("{vis}struct {name}")
                } else {
                    format!("{vis}struct {name}<{tp}>")
                };
                let ins = if tp.is_empty() {
                    name.clone()
                } else {
                    format!("{name}<${{1:{tp}}}>")
                };
                let fmt = if tp.is_empty() { 1 } else { 2 };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label,
                        kind: KIND_STRUCT,
                        detail,
                        insert_text: ins,
                        insert_text_format: fmt,
                        filter_text: name.clone(),
                    },
                );
                for m in members {
                    match m {
                        Decl::PrimaryCtor { params, .. } => {
                            emit_ctor_items(
                                name,
                                params,
                                cands,
                                seen,
                                *is_public,
                                *is_open,
                                false,
                                false,
                                type_params,
                            );
                        }
                        Decl::Func {
                            name: fn_name,
                            params,
                            ..
                        } if fn_name == "init" => {
                            emit_ctor_items(
                                name,
                                params,
                                cands,
                                seen,
                                *is_public,
                                *is_open,
                                false,
                                false,
                                type_params,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Decl::Interface {
                name,
                is_public,
                type_params,
                parents,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let tp = type_params_str(type_params);
                let p = if parents.is_empty() {
                    String::new()
                } else {
                    let ps: Vec<String> = parents.iter().map(display_type).collect();
                    format!(" extends {}", ps.join(", "))
                };
                let label = if tp.is_empty() {
                    name.clone()
                } else {
                    format!("{name}<{tp}>")
                };
                let detail = if tp.is_empty() {
                    format!("{vis}interface {name}{p}")
                } else {
                    format!("{vis}interface {name}<{tp}>{p}")
                };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label,
                        kind: KIND_INTERFACE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            Decl::Enum {
                name,
                is_public,
                cases,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let detail = format!("{vis}enum {name}");
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_ENUM,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
                // Enum members
                for ec in cases {
                    let qname = format!("{name}.{}", ec.name);
                    if ec.payloads.is_empty() {
                        push_candidate(
                            cands,
                            seen,
                            Candidate {
                                label: qname.clone(),
                                kind: KIND_VARIABLE,
                                detail: qname.clone(),
                                insert_text: ec.name.clone(),
                                insert_text_format: 1,
                                filter_text: qname,
                            },
                        );
                    } else {
                        let payload_strs: Vec<String> =
                            ec.payloads.iter().map(display_type).collect();
                        let detail = format!("{name}.{}({})", ec.name, payload_strs.join(", "));
                        let ins = format!("{}({})", ec.name, payload_strs.join(", "));
                        push_candidate(
                            cands,
                            seen,
                            Candidate {
                                label: qname,
                                kind: KIND_METHOD,
                                detail,
                                insert_text: ins,
                                insert_text_format: 2,
                                filter_text: format!("{name}.{}", ec.name),
                            },
                        );
                    }
                }
            }
            Decl::TypeAlias {
                name,
                is_public,
                target,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let target_s = display_type(target);
                let has_generic = matches!(target, Type::Ref { args, .. } if !args.is_empty());
                let detail = format!("{vis}type {name} = {target_s}");
                if has_generic {
                    push_candidate(
                        cands,
                        seen,
                        Candidate {
                            label: format!("{name}<T>"),
                            kind: KIND_CLASS,
                            detail,
                            insert_text: format!("{name}<${{1:T}}>"),
                            insert_text_format: 2,
                            filter_text: name.clone(),
                        },
                    );
                } else {
                    push_candidate(
                        cands,
                        seen,
                        Candidate {
                            label: name.clone(),
                            kind: KIND_CLASS,
                            detail,
                            insert_text: name.clone(),
                            insert_text_format: 1,
                            filter_text: name.clone(),
                        },
                    );
                }
            }
            Decl::Func {
                name,
                is_public,
                is_abstract,
                params,
                ret,
                type_params,
                ..
            } => {
                emit_func_items(
                    name,
                    *is_public,
                    false,
                    *is_abstract,
                    params,
                    ret,
                    type_params,
                    cands,
                    seen,
                );
            }
            Decl::Var {
                name,
                is_public,
                is_mutable,
                ty,
                init,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let ty_s = ty.as_ref().map(display_type).unwrap_or_default();
                let init_s = init
                    .as_ref()
                    .map(|_| " = ...".to_string())
                    .unwrap_or_default();
                let kw = if *is_mutable { "var" } else { "let" };
                let detail = if ty_s.is_empty() {
                    format!("{vis}{kw} {name}{init_s}")
                } else {
                    format!("{vis}{kw} {name}: {ty_s}{init_s}")
                };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            Decl::Prop {
                name,
                is_public,
                is_static,
                ty,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, *is_static);
                let ty_s = display_type(ty);
                let detail = format!("{vis}prop {name}: {ty_s}");
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            Decl::Macro {
                name, is_public, ..
            } => {
                let vis = if *is_public { "public " } else { "" };
                let detail = format!("{vis}macro {name}");
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_KEYWORD,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            _ => {}
        }
    }
}

/// Collect local variables/params in scope from the file AST.
fn collect_local_scope(
    file: &File,
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for d in &file.decls {
        if let Decl::Func { params, body, .. } = d {
            for p in params {
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: p.name.clone(),
                        kind: KIND_VARIABLE,
                        detail: format!("{}: {}", p.name, display_type(&p.ty)),
                        insert_text: p.name.clone(),
                        insert_text_format: 1,
                        filter_text: p.name.clone(),
                    },
                );
            }
            if let Body::Block(exprs) = body {
                collect_lets_in_block(exprs, cursor_line, cands, seen);
            }
        }
    }
}

/// Recursively collect let/var statements in a block of expressions.
fn collect_lets_in_block(
    exprs: &[Expr],
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for e in exprs {
        match e {
            Expr::LetPatternDestructor { patterns, pos, .. } => {
                if pos.line <= cursor_line {
                    for p in patterns {
                        if let Pattern::Var { name, .. } = p {
                            push_candidate(
                                cands,
                                seen,
                                Candidate {
                                    label: name.clone(),
                                    kind: KIND_VARIABLE,
                                    detail: format!("let {name} = ..."),
                                    insert_text: name.clone(),
                                    insert_text_format: 1,
                                    filter_text: name.clone(),
                                },
                            );
                        }
                    }
                }
            }
            Expr::If { then, els, .. } => {
                collect_expr_block(then, cursor_line, cands, seen);
                if let Some(e) = els {
                    collect_expr_block(e, cursor_line, cands, seen);
                }
            }
            Expr::Block { stmts, .. } => {
                collect_lets_in_block(stmts, cursor_line, cands, seen);
            }
            Expr::ForIn { body, .. } => collect_expr_block(body, cursor_line, cands, seen),
            Expr::While { body, .. } => collect_expr_block(body, cursor_line, cands, seen),
            Expr::DoWhile { body, .. } => collect_expr_block(body, cursor_line, cands, seen),
            Expr::Match { cases, .. } => {
                for mc in cases {
                    collect_expr_block(&mc.body, cursor_line, cands, seen);
                }
            }
            Expr::Try {
                body,
                catches,
                finally,
                ..
            } => {
                collect_expr_block(body, cursor_line, cands, seen);
                for c in catches {
                    collect_expr_block(&c.body, cursor_line, cands, seen);
                }
                if let Some(f) = finally {
                    collect_expr_block(f, cursor_line, cands, seen);
                }
            }
            _ => {}
        }
    }
}

fn collect_expr_block(
    e: &Expr,
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if let Expr::Block { stmts, .. } = e {
        collect_lets_in_block(stmts, cursor_line, cands, seen);
    }
}

/// Collect class members when the cursor is inside a class/struct body.
fn collect_member_scope(
    d: &Decl,
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let (members, vis_base): (&[Decl], (bool, bool, bool, bool)) = match d {
        Decl::Class {
            members,
            is_public,
            is_open,
            is_sealed,
            is_abstract,
            ..
        } => (members, (*is_public, *is_open, *is_sealed, *is_abstract)),
        Decl::Struct {
            members,
            is_public,
            is_open,
            ..
        } => (members, (*is_public, *is_open, false, false)),
        Decl::Interface {
            members, is_public, ..
        } => (members, (*is_public, false, false, false)),
        _ => return,
    };
    let in_class = members.iter().any(|m| {
        let pos = decl_pos(m);
        pos.line <= cursor_line
    });
    if !in_class {
        return;
    }
    for m in members {
        match m {
            Decl::Var {
                name,
                is_public,
                is_mutable,
                ty,
                init,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let ty_s = ty.as_ref().map(display_type).unwrap_or_default();
                let init_s = init
                    .as_ref()
                    .map(|_| " = ...".to_string())
                    .unwrap_or_default();
                let kw = if *is_mutable { "var" } else { "let" };
                let detail = if ty_s.is_empty() {
                    format!("{vis}{kw} {name}{init_s}")
                } else {
                    format!("{vis}{kw} {name}: {ty_s}{init_s}")
                };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            Decl::Func {
                name,
                is_public,
                is_abstract,
                params,
                ret,
                type_params,
                ..
            } => {
                emit_func_items(
                    name,
                    *is_public,
                    false,
                    *is_abstract,
                    params,
                    ret,
                    type_params,
                    cands,
                    seen,
                );
            }
            Decl::Prop {
                name,
                is_public,
                is_static,
                ty,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, *is_static);
                let ty_s = display_type(ty);
                let detail = format!("{vis}prop {name}: {ty_s}");
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            _ => {}
        }
    }
    let _ = vis_base;
}

/// Collect class members as candidates (for `obj.` and `this.`).
fn collect_members_as_candidates(
    members: &[Decl],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for m in members {
        match m {
            Decl::Var {
                name,
                is_public,
                is_mutable,
                ty,
                init,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false);
                let ty_s = ty.as_ref().map(display_type).unwrap_or_default();
                let init_s = init
                    .as_ref()
                    .map(|_| " = ...".to_string())
                    .unwrap_or_default();
                let kw = if *is_mutable { "var" } else { "let" };
                let detail = if ty_s.is_empty() {
                    format!("{vis}{kw} {name}{init_s}")
                } else {
                    format!("{vis}{kw} {name}: {ty_s}{init_s}")
                };
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            Decl::Func {
                name,
                is_public,
                is_abstract,
                params,
                ret,
                type_params,
                ..
            } => {
                emit_func_items(
                    name,
                    *is_public,
                    false,
                    *is_abstract,
                    params,
                    ret,
                    type_params,
                    cands,
                    seen,
                );
            }
            Decl::Prop {
                name,
                is_public,
                is_static,
                ty,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, *is_static);
                let ty_s = display_type(ty);
                let detail = format!("{vis}prop {name}: {ty_s}");
                push_candidate(
                    cands,
                    seen,
                    Candidate {
                        label: name.clone(),
                        kind: KIND_VARIABLE,
                        detail,
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
            _ => {}
        }
    }
}

/// Collect members of an enum as `Enum.Member` candidates (for `Enum.`).
fn collect_enum_members(
    name: &str,
    cases: &[cj_ast::EnumCase],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for ec in cases {
        let qname = format!("{name}.{}", ec.name);
        if ec.payloads.is_empty() {
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: qname.clone(),
                    kind: KIND_VARIABLE,
                    detail: qname.clone(),
                    insert_text: ec.name.clone(),
                    insert_text_format: 1,
                    filter_text: qname,
                },
            );
        } else {
            let payload_strs: Vec<String> = ec.payloads.iter().map(display_type).collect();
            let detail = format!("{name}.{}({})", ec.name, payload_strs.join(", "));
            let ins = format!("{}({})", ec.name, payload_strs.join(", "));
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: qname.clone(),
                    kind: KIND_METHOD,
                    detail,
                    insert_text: ins,
                    insert_text_format: 2,
                    filter_text: qname,
                },
            );
        }
    }
}

fn decl_pos(d: &Decl) -> cj_ast::CodePos {
    match d {
        Decl::Func { pos, .. } => *pos,
        Decl::Class { pos, .. } => *pos,
        Decl::Struct { pos, .. } => *pos,
        Decl::Interface { pos, .. } => *pos,
        Decl::Enum { pos, .. } => *pos,
        Decl::Var { pos, .. } => *pos,
        Decl::TypeAlias { pos, .. } => *pos,
        Decl::Macro { pos, .. } => *pos,
        Decl::Extend { pos, .. } => *pos,
        Decl::Prop { pos, .. } => *pos,
        Decl::PrimaryCtor { pos, .. } => *pos,
        Decl::Builtin { pos, .. } => *pos,
        Decl::FuncParam { pos, .. } => *pos,
        Decl::VarWithPattern { pos, .. } => *pos,
        Decl::GenericParam { pos, .. } => *pos,
        Decl::Package { pos, .. } => *pos,
        Decl::MacroExpand { pos, .. } => *pos,
        Decl::Main { pos, .. } => *pos,
        Decl::Invalid(pos) => *pos,
    }
}

/// Collect std-core symbols as candidates.
fn collect_std_symbols(cands: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
    for (name, kind, detail) in STD_CORE {
        push_candidate(
            cands,
            seen,
            Candidate {
                label: name.to_string(),
                kind: *kind,
                detail: detail.to_string(),
                insert_text: name.to_string(),
                insert_text_format: 1,
                filter_text: name.to_string(),
            },
        );
    }
}

/// Collect keyword candidates.
fn collect_keywords(cands: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
    for (label, detail, fmt, ins) in KEYWORDS {
        push_candidate(
            cands,
            seen,
            Candidate {
                label: label.to_string(),
                kind: KIND_KEYWORD,
                detail: detail.to_string(),
                insert_text: ins.to_string(),
                insert_text_format: *fmt,
                filter_text: label.to_string(),
            },
        );
    }
}

/// Resolve a variable name to its declared type name (from file + params).
fn resolve_var_type(file: &File, var_name: &str) -> Option<String> {
    for d in &file.decls {
        match d {
            Decl::Var { name, ty, .. } if name == var_name => {
                return ty.as_ref().map(display_type);
            }
            Decl::Func { params, .. } => {
                for p in params {
                    if p.name == var_name {
                        return Some(display_type(&p.ty));
                    }
                }
            }
            Decl::Class { members, .. } | Decl::Struct { members, .. } => {
                for m in members {
                    if let Decl::Var { name, ty, .. } = m {
                        if name == var_name {
                            return ty.as_ref().map(display_type);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Member access completion: resolve the receiver expression and add members.
fn collect_member_access(
    file: &File,
    receiver: &str,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    // Strip trailing generics like `Data5<Int64>` → `Data5`
    let base = receiver
        .split('<')
        .next()
        .unwrap_or(receiver)
        .trim()
        .to_string();

    // 1. `this.` / `this` → enclosing class/struct members
    if base == "this" {
        for d in &file.decls {
            match d {
                Decl::Class { members, .. } => collect_members_as_candidates(members, cands, seen),
                Decl::Struct { members, .. } => collect_members_as_candidates(members, cands, seen),
                _ => {}
            }
        }
        return;
    }

    // 2. A class/struct/interface/enum type name → static members
    for d in &file.decls {
        match d {
            Decl::Class { name, members, .. } if *name == base => {
                collect_members_as_candidates(members, cands, seen);
                return;
            }
            Decl::Struct { name, members, .. } if *name == base => {
                collect_members_as_candidates(members, cands, seen);
                return;
            }
            Decl::Interface { name, members, .. } if *name == base => {
                collect_members_as_candidates(members, cands, seen);
                return;
            }
            Decl::Enum { name, cases, .. } if *name == base => {
                collect_enum_members(name, cases, cands, seen);
                return;
            }
            _ => {}
        }
    }

    // 3. A variable name → look up its declared type, then that type's members
    if let Some(ty) = resolve_var_type(file, &base) {
        let ty_base = ty.split('<').next().unwrap_or(&ty).trim().to_string();
        for d in &file.decls {
            match d {
                Decl::Class { name, members, .. } if *name == ty_base => {
                    collect_members_as_candidates(members, cands, seen);
                    return;
                }
                Decl::Struct { name, members, .. } if *name == ty_base => {
                    collect_members_as_candidates(members, cands, seen);
                    return;
                }
                Decl::Interface { name, members, .. } if *name == ty_base => {
                    collect_members_as_candidates(members, cands, seen);
                    return;
                }
                Decl::Enum { name, cases, .. } if *name == ty_base => {
                    collect_enum_members(name, cases, cands, seen);
                    return;
                }
                _ => {}
            }
        }
    }
}

// ─── Main entry point ────────────────────────────────────────────────────

pub fn complete_at(
    file: &File,
    source: &str,
    line: u32,
    character: u32,
    sibling_decls: Option<&Vec<(String, u32, String)>>,
    project_root: Option<&Path>,
    uri: &str,
) -> Value {
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Determine context: member access (after `.`) or plain prefix
    let line_text = source_line_text(source, line);
    let col = (character as usize).min(line_text.len());
    // Clamp to a char boundary (LSP character counts code points; the byte
    // index may land inside a multi-byte char like `。`).
    let mut col = col;
    while col > 0 && !line_text.is_char_boundary(col) {
        col -= 1;
    }
    let before = &line_text[..col];
    let is_member_access = before.ends_with('.');
    let prefix = prefix_at_line(&line_text, character);

    if is_member_access {
        let dot_idx = before.rfind('.').unwrap_or(0);
        let receiver = before[..dot_idx].trim();
        collect_member_access(file, receiver, &mut candidates, &mut seen);
    } else {
        // 1. File top-level decls
        collect_file_decls(file, &mut candidates, &mut seen);

        // 2. Same-package sibling decls (from project root scanning)
        if let Some(sibs) = sibling_decls {
            for (name, kind, detail) in sibs {
                push_candidate(
                    &mut candidates,
                    &mut seen,
                    Candidate {
                        label: name.clone(),
                        kind: *kind,
                        detail: detail.clone(),
                        insert_text: name.clone(),
                        insert_text_format: 1,
                        filter_text: name.clone(),
                    },
                );
            }
        }
        let _ = project_root;

        // 3. Local scope (params, let/var in function body)
        collect_local_scope(file, line, &mut candidates, &mut seen);

        // 4. Class members in scope (if cursor inside a class)
        for d in &file.decls {
            collect_member_scope(d, line, &mut candidates, &mut seen);
        }

        // 5. Keywords
        collect_keywords(&mut candidates, &mut seen);

        // 6. Std core symbols
        collect_std_symbols(&mut candidates, &mut seen);
    }

    // Prefix filter (skip for member access — client filters)
    let mut items: Vec<Value> = if !is_member_access {
        candidates
            .iter()
            .filter(|c| prefix.is_empty() || c.filter_text.starts_with(&prefix))
            .map(item_json)
            .collect()
    } else {
        candidates.iter().map(item_json).collect()
    };

    // Official exact-match rule: when prefix non-empty and there's an exact
    // match, return ONLY exact matches.
    if !prefix.is_empty() {
        let exact: Vec<Value> = items
            .iter()
            .filter(|it| {
                it.get("filterText")
                    .and_then(|v| v.as_str())
                    .is_some_and(|f| f == prefix)
            })
            .cloned()
            .collect();
        if !exact.is_empty() {
            items = exact;
        }
    }

    // Return null when no candidates (official behavior)
    if items.is_empty() {
        return Value::Null;
    }

    let _ = uri;
    json!(items)
}

fn item_json(c: &Candidate) -> Value {
    json!({
        "label": c.label.clone(),
        "kind": c.kind,
        "detail": c.detail.clone(),
        "documentation": "",
        "filterText": c.filter_text.clone(),
        "insertText": c.insert_text.clone(),
        "insertTextFormat": c.insert_text_format,
        "sortText": "",
        "deprecated": false,
    })
}
