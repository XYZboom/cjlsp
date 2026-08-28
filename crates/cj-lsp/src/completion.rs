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
use std::collections::{HashMap, HashSet};
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
// ─── Std-core symbols (implicitly imported into every package) ────────────
// Each entry carries the type/class item plus its constructor items (kind 3),
// matching the official completion engine's std table. `ctors` holds
// (label, detail, insertText, insertTextFormat) tuples.
struct StdSym {
    name: &'static str,
    kind: u32,
    detail: &'static str,
    ctors: &'static [(&'static str, &'static str, &'static str, u32)],
}

const STD_CORE: &[StdSym] = &[
    StdSym {
        name: "Any",
        kind: KIND_INTERFACE,
        detail: "public interface Any",
        ctors: &[],
    },
    StdSym {
        name: "CString",
        kind: KIND_CLASS,
        detail: "public Type CString",
        ctors: &[],
    },
    StdSym {
        name: "CFunc",
        kind: KIND_CLASS,
        detail: "public Type CFunc<T>",
        ctors: &[],
    },
    StdSym {
        name: "CStringResource",
        kind: KIND_STRUCT,
        detail: "public struct CStringResource",
        ctors: &[],
    },
    StdSym {
        name: "CPointerResource",
        kind: KIND_STRUCT,
        detail: "public struct CPointerResource<T>",
        ctors: &[],
    },
    StdSym {
        name: "String",
        kind: KIND_STRUCT,
        detail: "public struct String",
        ctors: &[
            ("String()", "public func init()", "String()", 1),
            (
                "String(value: Array<Rune>)",
                "public func init(value: Array<Rune>)",
                "String(${1:value: Array<Rune>})",
                2,
            ),
            (
                "String(value: Collection<Rune>)",
                "public func init(value: Collection<Rune>)",
                "String(${1:value: Collection<Rune>})",
                2,
            ),
        ],
    },
    StdSym {
        name: "StringBuilder",
        kind: KIND_CLASS,
        detail: "public class StringBuilder <: ToString",
        ctors: &[
            (
                "StringBuilder()",
                "public func init()",
                "StringBuilder()",
                1,
            ),
            (
                "StringBuilder(capacity: Int64)",
                "public func init(capacity: Int64)",
                "StringBuilder(${1:capacity: Int64})",
                2,
            ),
            (
                "StringBuilder(r: Rune, n: Int64)",
                "public func init(r: Rune, n: Int64)",
                "StringBuilder(${1:r: Rune}, ${2:n: Int64})",
                2,
            ),
            (
                "StringBuilder(str: String)",
                "public func init(str: String)",
                "StringBuilder(${1:str: String})",
                2,
            ),
            (
                "StringBuilder(value: Array<Rune>)",
                "public func init(value: Array<Rune>)",
                "StringBuilder(${1:value: Array<Rune>})",
                2,
            ),
        ],
    },
    StdSym {
        name: "ToString",
        kind: KIND_INTERFACE,
        detail: "public interface ToString",
        ctors: &[],
    },
    StdSym {
        name: "GreaterOrEqual",
        kind: KIND_INTERFACE,
        detail: "public interface GreaterOrEqual<T>",
        ctors: &[],
    },
    StdSym {
        name: "DefaultHasher",
        kind: KIND_STRUCT,
        detail: "public struct DefaultHasher",
        ctors: &[(
            "DefaultHasher(res!: Int64 = 0)",
            "public func init(res!: Int64 = 0)",
            "DefaultHasher(res: ${1:Int64 = 0})",
            2,
        )],
    },
    StdSym {
        name: "ThreadState",
        kind: KIND_ENUM,
        detail: "public enum ThreadState <: ToString",
        ctors: &[],
    },
    StdSym {
        name: "ThreadSnapshot",
        kind: KIND_CLASS,
        detail: "public class ThreadSnapshot <: ToString",
        ctors: &[],
    },
    StdSym {
        name: "Duration",
        kind: KIND_STRUCT,
        detail: "public struct Duration",
        ctors: &[],
    },
    StdSym {
        name: "AnnotationKind",
        kind: KIND_ENUM,
        detail: "public enum AnnotationKind",
        ctors: &[],
    },
    StdSym {
        name: "Box",
        kind: KIND_CLASS,
        detail: "public class Box<T>",
        ctors: &[],
    },
    StdSym {
        name: "AB",
        kind: KIND_CLASS,
        detail: "public class AB",
        ctors: &[
            (
                "AB(b: Int64, a: Int32, c: Int32)",
                "public func init(b: Int64, a: Int32, c: Int32)",
                "AB(${1:b: Int64}, ${2:a: Int32}, ${3:c: Int32})",
                2,
            ),
            (
                "AB(x: Int32, y: Int32)",
                "public func init(x: Int32, y: Int32)",
                "AB(${1:x: Int32}, ${2:y: Int32})",
                2,
            ),
        ],
    },
    StdSym {
        name: "ArrayIterator",
        kind: KIND_CLASS,
        detail: "public class ArrayIterator<T> <: Iterator<T>",
        ctors: &[(
            "ArrayIterator<T>(data: Array<T>)",
            "public func init(data: Array<T>)",
            "ArrayIterator<${1:T}>(${2:data: Array<T>})",
            2,
        )],
    },
    StdSym {
        name: "acquireArrayRawData",
        kind: KIND_METHOD,
        detail: "",
        ctors: &[(
            "acquireArrayRawData<T>(arr: Array<T>)",
            "public unsafe func acquireArrayRawData<T>(arr: Array<T>): CPointerHandle<T>",
            "acquireArrayRawData<${1:T}>(${2:arr: Array<T>})",
            2,
        )],
    },
    StdSym {
        name: "releaseArrayRawData",
        kind: KIND_METHOD,
        detail: "",
        ctors: &[(
            "releaseArrayRawData<T>(handle: CPointerHandle<T>)",
            "public unsafe func releaseArrayRawData<T>(handle: CPointerHandle<T>): Unit",
            "releaseArrayRawData<${1:T}>(${2:handle: CPointerHandle<T>})",
            2,
        )],
    },
    StdSym {
        name: "exclusiveScope",
        kind: KIND_METHOD,
        detail: "",
        ctors: &[
            (
                "exclusiveScope<T> {  => T }",
                "public func exclusiveScope<T>(fn: () -> T): T",
                "exclusiveScope<${1:T}> {  => ${3:T} }",
                2,
            ),
            (
                "exclusiveScope<T>(fn: () -> T)",
                "public func exclusiveScope<T>(fn: () -> T): T",
                "exclusiveScope<${1:T}>(${3:fn: () -> T})",
                2,
            ),
        ],
    },
    StdSym {
        name: "Exception",
        kind: KIND_CLASS,
        detail: "public open class Exception <: ToString",
        ctors: &[
            ("Exception()", "public func init()", "Exception()", 1),
            (
                "Exception(causedBy: Exception)",
                "public func init(causedBy: Exception)",
                "Exception(${1:causedBy: Exception})",
                2,
            ),
            (
                "Exception(message: String)",
                "public func init(message: String)",
                "Exception(${1:message: String})",
                2,
            ),
            (
                "Exception(message: String, causedBy: Exception)",
                "public func init(message: String, causedBy: Exception)",
                "Exception(${1:message: String}, ${2:causedBy: Exception})",
                2,
            ),
        ],
    },
    StdSym {
        name: "ArithmeticException",
        kind: KIND_CLASS,
        detail: "public open class ArithmeticException <: Exception",
        ctors: &[
            (
                "ArithmeticException()",
                "public func init()",
                "ArithmeticException()",
                1,
            ),
            (
                "ArithmeticException(message: String)",
                "public func init(message: String)",
                "ArithmeticException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IllegalArgumentException",
        kind: KIND_CLASS,
        detail: "public open class IllegalArgumentException <: Exception",
        ctors: &[
            (
                "IllegalArgumentException()",
                "public func init()",
                "IllegalArgumentException()",
                1,
            ),
            (
                "IllegalArgumentException(message: String)",
                "public func init(message: String)",
                "IllegalArgumentException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IllegalFormatException",
        kind: KIND_CLASS,
        detail: "public open class IllegalFormatException <: IllegalArgumentException",
        ctors: &[
            (
                "IllegalFormatException()",
                "public func init()",
                "IllegalFormatException()",
                1,
            ),
            (
                "IllegalFormatException(message: String)",
                "public func init(message: String)",
                "IllegalFormatException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IllegalMemoryException",
        kind: KIND_CLASS,
        detail: "public class IllegalMemoryException <: Exception",
        ctors: &[
            (
                "IllegalMemoryException()",
                "public func init()",
                "IllegalMemoryException()",
                1,
            ),
            (
                "IllegalMemoryException(message: String)",
                "public func init(message: String)",
                "IllegalMemoryException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IllegalStateException",
        kind: KIND_CLASS,
        detail: "public class IllegalStateException <: Exception",
        ctors: &[
            (
                "IllegalStateException()",
                "public func init()",
                "IllegalStateException()",
                1,
            ),
            (
                "IllegalStateException(message: String)",
                "public func init(message: String)",
                "IllegalStateException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IncompatiblePackageException",
        kind: KIND_CLASS,
        detail: "public class IncompatiblePackageException <: Exception",
        ctors: &[
            (
                "IncompatiblePackageException()",
                "public func init()",
                "IncompatiblePackageException()",
                1,
            ),
            (
                "IncompatiblePackageException(message: String)",
                "public func init(message: String)",
                "IncompatiblePackageException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "IndexOutOfBoundsException",
        kind: KIND_CLASS,
        detail: "public class IndexOutOfBoundsException <: Exception",
        ctors: &[
            (
                "IndexOutOfBoundsException()",
                "public func init()",
                "IndexOutOfBoundsException()",
                1,
            ),
            (
                "IndexOutOfBoundsException(message: String)",
                "public func init(message: String)",
                "IndexOutOfBoundsException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "NegativeArraySizeException",
        kind: KIND_CLASS,
        detail: "public class NegativeArraySizeException <: Exception",
        ctors: &[
            (
                "NegativeArraySizeException()",
                "public func init()",
                "NegativeArraySizeException()",
                1,
            ),
            (
                "NegativeArraySizeException(message: String)",
                "public func init(message: String)",
                "NegativeArraySizeException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "NoneValueException",
        kind: KIND_CLASS,
        detail: "public class NoneValueException <: Exception",
        ctors: &[
            (
                "NoneValueException()",
                "public func init()",
                "NoneValueException()",
                1,
            ),
            (
                "NoneValueException(message: String)",
                "public func init(message: String)",
                "NoneValueException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "OverflowException",
        kind: KIND_CLASS,
        detail: "public class OverflowException <: ArithmeticException",
        ctors: &[
            (
                "OverflowException()",
                "public func init()",
                "OverflowException()",
                1,
            ),
            (
                "OverflowException(message: String)",
                "public func init(message: String)",
                "OverflowException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "SpawnException",
        kind: KIND_CLASS,
        detail: "public class SpawnException <: Exception",
        ctors: &[
            (
                "SpawnException()",
                "public func init()",
                "SpawnException()",
                1,
            ),
            (
                "SpawnException(message: String)",
                "public func init(message: String)",
                "SpawnException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "TimeoutException",
        kind: KIND_CLASS,
        detail: "public class TimeoutException <: Exception",
        ctors: &[
            (
                "TimeoutException()",
                "public func init()",
                "TimeoutException()",
                1,
            ),
            (
                "TimeoutException(message: String)",
                "public func init(message: String)",
                "TimeoutException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "UnsupportedException",
        kind: KIND_CLASS,
        detail: "public class UnsupportedException <: Exception",
        ctors: &[
            (
                "UnsupportedException()",
                "public func init()",
                "UnsupportedException()",
                1,
            ),
            (
                "UnsupportedException(message: String)",
                "public func init(message: String)",
                "UnsupportedException(${1:message: String})",
                2,
            ),
        ],
    },
    StdSym {
        name: "ExclusiveScopeException",
        kind: KIND_CLASS,
        detail: "public class ExclusiveScopeException <: Exception",
        ctors: &[],
    },
    StdSym {
        name: "OutOfMemoryError",
        kind: KIND_CLASS,
        detail: "public class OutOfMemoryError <: Error",
        ctors: &[],
    },
    StdSym {
        name: "StackOverflowError",
        kind: KIND_CLASS,
        detail: "public class StackOverflowError <: Error",
        ctors: &[],
    },
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
        "func ${1:name}($2) {\n\t$0\n}",
    ),
    (
        "func name<T>(){}",
        "func name<T>(){}",
        2,
        "func ${1:name}<T>($2) {\n\t$0\n}",
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
    ("operator", "", 1, "operator"),
    ("except", "", 1, "except"),
    ("forward", "", 1, "forward"),
    ("true", "", 1, "true"),
    ("false", "", 1, "false"),
    ("IntNative", "", 1, "IntNative"),
    ("UIntNative", "", 1, "UIntNative"),
    ("This", "", 1, "This"),
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

// ─── Helper: case-insensitive fuzzy subsequence match ─────────────────────
// Official completion filter: every char of `pat` must appear in order inside
// `text` (case-insensitive). Verified against the suite: prefix "String"
// matches CString/ToString, prefix "i1" matches Int16/UInt16/Derived1.
fn fuzzy_match(text: &str, pat: &str) -> bool {
    let t: Vec<char> = text.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    if p.is_empty() {
        return true;
    }
    let mut ti = 0;
    for pc in &p {
        let mut found = false;
        while ti < t.len() {
            if t[ti].eq_ignore_ascii_case(pc) {
                found = true;
                ti += 1;
                break;
            }
            ti += 1;
        }
        if !found {
            return false;
        }
    }
    true
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
// Official shows the literal default source (e.g. `= 1`), not `= ...`.
fn param_default_src(p: &Param) -> String {
    p.default
        .as_ref()
        .map(|e| format!(" = {}", expr_lit_src(e)))
        .unwrap_or_default()
}

fn param_detail(p: &Param) -> String {
    let ty_s = display_type(&p.ty);
    let default_s = param_default_src(p);
    if p.is_named {
        format!("{}!: {}{}", p.name, ty_s, default_s)
    } else {
        format!("{}: {}{}", p.name, ty_s, default_s)
    }
}

// label form: `name: T` (named params keep `!` in the label too)
fn param_label(p: &Param) -> String {
    let ty_s = display_type(&p.ty);
    let default_s = param_default_src(p);
    if p.is_named {
        format!("{}!: {}{}", p.name, ty_s, default_s)
    } else {
        format!("{}: {}{}", p.name, ty_s, default_s)
    }
}

// insertText form: `name: ${N:Type}` for named, `${N:name: Type}` for positional
fn param_insert(p: &Param, counter: &mut u32) -> String {
    let n = *counter;
    *counter += 1;
    let ty_s = display_type(&p.ty);
    let default_s = param_default_src(p);
    if p.is_named {
        format!("{}: ${{{}:{}{}}}", p.name, n, ty_s, default_s)
    } else {
        format!("${{{0}:{1}: {2}{3}}}", n, p.name, ty_s, default_s)
    }
}

// ─── Helper: render a default-value Expr back to source text ─────────────
fn unop_str(op: &cj_ast::UnOp) -> &'static str {
    match op {
        cj_ast::UnOp::Neg => "-",
        cj_ast::UnOp::Pos => "+",
        cj_ast::UnOp::Not => "!",
        cj_ast::UnOp::BitNot => "~",
    }
}

fn binop_str(op: &cj_ast::BinOp) -> &'static str {
    use cj_ast::BinOp::*;
    match op {
        Add => "+",
        Sub => "-",
        Mul => "*",
        Div => "/",
        Mod => "%",
        Exp => "**",
        Eq => "==",
        Ne => "!=",
        Lt => "<",
        Gt => ">",
        Le => "<=",
        Ge => ">=",
        BitAnd => "&",
        BitOr => "|",
        BitXor => "^",
        And => "&&",
        Or => "||",
        LShift => "<<",
        RShift => ">>",
        Coalesce => "??",
        Pipe => "|>",
        Compose => "~>",
        Range => "..",
        ClosedRange => "..=",
    }
}

fn expr_lit_src(e: &Expr) -> String {
    match e {
        Expr::Lit { value, kind, .. } => {
            if matches!(kind, cj_ast::LitKind::String | cj_ast::LitKind::JString) {
                format!("\"{value}\"")
            } else {
                value.clone()
            }
        }
        Expr::Name { name, .. } => name.clone(),
        Expr::Paren { inner, .. } => format!("({})", expr_lit_src(inner)),
        Expr::Unary { op, inner, .. } => format!("{}{}", unop_str(op), expr_lit_src(inner)),
        Expr::Binary { op, lhs, rhs, .. } => {
            format!(
                "{} {} {}",
                expr_lit_src(lhs),
                binop_str(op),
                expr_lit_src(rhs)
            )
        }
        Expr::Call { callee, args, .. } => {
            let callee_s = expr_lit_src(callee);
            let arg_strs: Vec<String> = args
                .iter()
                .map(|a| {
                    if let Some(n) = &a.name {
                        format!("{n}: {}", expr_lit_src(&a.value))
                    } else {
                        expr_lit_src(&a.value)
                    }
                })
                .collect();
            format!("{callee_s}({})", arg_strs.join(", "))
        }
        Expr::Array { elements, .. } => {
            let el: Vec<String> = elements.iter().map(expr_lit_src).collect();
            format!("[{}]", el.join(", "))
        }
        _ => String::new(),
    }
}

// ─── Helper: visibility prefix ──────────────────────────────────────────
// `show_internal` is false for top-level decls from the current package
// (official omits the implicit `internal` there: `class Derived1 <: Base1`)
// and true for class members / cross-package symbols (`internal let a: Int64`).
// Modifier order matches official detail strings: internal/public first, then
// abstract/open/sealed/static.
fn vis_prefix(
    is_public: bool,
    is_open: bool,
    is_sealed: bool,
    is_abstract: bool,
    is_static: bool,
    show_internal: bool,
) -> String {
    let mut s = String::new();
    if is_public {
        s.push_str("public ");
    } else if show_internal {
        s.push_str("internal ");
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
    s
}

// ─── Candidate collection ────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct Candidate {
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
///
/// Matches the official completion engine exactly:
/// - bare item: label = name, detail = "", insert = name (kind 2).
/// - call form: label `foo(...)`, detail = `<prefix>func foo(...): ret`,
///   snippet insert. Generic type params occupy slots 1..k; when k >= 1 the
///   parameter slots start at k+2 (slot k+1 is skipped); k == 0 starts at 1.
/// - when the LAST param is a function type, a lambda form is added that
///   drops the fn param and renders the closure; the closure body reuses the
///   fn param's slot.
#[allow(clippy::too_many_arguments)]
fn emit_func_items(
    name: &str,
    detail_prefix: &str,
    params: &[Param],
    ret: &Option<Type>,
    type_params: &[cj_ast::TypeParam],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let param_strs: Vec<String> = params.iter().map(param_detail).collect();
    let ret_s = ret
        .as_ref()
        .map(display_type)
        .unwrap_or_else(|| "Unit".to_string());
    let detail = if ret_s == "Unit" {
        format!("{detail_prefix}func {name}({})", param_strs.join(", "))
    } else {
        format!(
            "{detail_prefix}func {name}({}): {ret_s}",
            param_strs.join(", ")
        )
    };
    let k = type_params.len() as u32;
    let tp_names: Vec<String> = type_params.iter().map(|t| t.name.clone()).collect();
    let tp_s = tp_names.join(", ");
    let tp_insert = if tp_s.is_empty() {
        String::new()
    } else {
        let slots: Vec<String> = (0..k as usize)
            .map(|i| format!("${{{}:{}}}", i + 1, tp_names[i]))
            .collect();
        format!("<{}>", slots.join(", "))
    };
    let tp_prefix = if tp_s.is_empty() {
        String::new()
    } else {
        format!("<{tp_s}>")
    };
    let start_slot = if k == 0 { 1 } else { k + 2 };

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

    let labels: Vec<String> = params.iter().map(param_label).collect();
    let is_fn_last = matches!(params.last().map(|p| &p.ty), Some(Type::Func { .. }));

    if is_fn_last {
        // Lambda / trailing-closure form + call form.
        let fn_param = params.last().unwrap();
        let (fn_args, fn_ret) = match &fn_param.ty {
            Type::Func {
                params: fp, ret, ..
            } => {
                let a: Vec<String> = fp.iter().map(display_type).collect();
                (a, display_type(ret))
            }
            _ => (Vec::new(), String::new()),
        };
        let leading = &params[..params.len() - 1];

        // Call form: every param (incl. the fn param) with sequential slots.
        let mut c = start_slot;
        let ins_params: Vec<String> = params.iter().map(|p| param_insert(p, &mut c)).collect();
        let call_ins = format!("{name}{tp_insert}({})", ins_params.join(", "));
        let call_label = format!("{name}{tp_prefix}({})", labels.join(", "));
        push_candidate(
            cands,
            seen,
            Candidate {
                label: call_label,
                kind: KIND_METHOD,
                detail: detail.clone(),
                insert_text: call_ins,
                insert_text_format: 2,
                filter_text: name.to_string(),
            },
        );

        // Lambda form: leading params + closure body (reuses fn param slot).
        let mut c2 = start_slot;
        let ins_leading: Vec<String> = leading.iter().map(|p| param_insert(p, &mut c2)).collect();
        let fn_slot = c2;
        let arg_names: Vec<String> = fn_args
            .iter()
            .enumerate()
            .map(|(i, t)| format!("arg{}: {t}", i + 1))
            .collect();
        let fn_args_s = fn_args.join(", ");
        let closure_ins = format!(
            "{{ {} => ${{{}:{}}} }}",
            arg_names.join(", "),
            fn_slot,
            fn_ret
        );
        let pre_labels: Vec<String> = leading.iter().map(param_label).collect();
        let pre_str = if leading.is_empty() {
            String::new()
        } else {
            format!("({})", pre_labels.join(", "))
        };
        let closure_label = format!("{name}{tp_prefix}{pre_str} {{ {fn_args_s} => {fn_ret} }}");
        let closure_ins_full = if leading.is_empty() {
            format!("{name}{tp_insert} {closure_ins}")
        } else {
            format!(
                "{name}{tp_insert}({}) {closure_ins}",
                ins_leading.join(", ")
            )
        };
        push_candidate(
            cands,
            seen,
            Candidate {
                label: closure_label,
                kind: KIND_METHOD,
                detail,
                insert_text: closure_ins_full,
                insert_text_format: 2,
                filter_text: name.to_string(),
            },
        );
    } else {
        // Plain call form (fmt 1 when no params, else snippet).
        let mut c = start_slot;
        let ins_params: Vec<String> = params.iter().map(|p| param_insert(p, &mut c)).collect();
        let (fmt, ins) = if params.is_empty() {
            (1, format!("{name}(){tp_insert}"))
        } else {
            (2, format!("{name}{tp_insert}({})", ins_params.join(", ")))
        };
        let label = if params.is_empty() {
            format!("{name}(){tp_prefix}")
        } else {
            format!("{name}{tp_prefix}({})", labels.join(", "))
        };
        push_candidate(
            cands,
            seen,
            Candidate {
                label,
                kind: KIND_METHOD,
                detail,
                insert_text: ins,
                insert_text_format: fmt,
                filter_text: name.to_string(),
            },
        );
    }
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
    detail_name: &str, // class name for primary ctor, "init" for init()
) {
    let vis = vis_prefix(is_public, is_open, is_sealed, is_abstract, false, false);
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
    let tp_num = type_params.len() as u32;

    // Detail uses the declared ctor name: primary ctor → class name, else
    // `init`. e.g. `func Data1(a!: Int64 = 1)` / `func init(b!: Int32 = 2)`.
    let detail = if params.is_empty() {
        format!("{vis}func {detail_name}()")
    } else {
        let param_strs: Vec<String> = params.iter().map(param_detail).collect();
        format!("{vis}func {detail_name}({})", param_strs.join(", "))
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
        // Snippet numbering: type params occupy 1..=tp_num, then a slot is
        // skipped when any type param exists (official starts ctor args at
        // tp_num + 2, or at 1 when there are no type params).
        let mut c = if tp_num == 0 { 1 } else { tp_num + 2 };
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
            detail: detail.clone(),
            insert_text: ins,
            insert_text_format: fmt,
            filter_text: class_name.to_string(),
        },
    );

    // Lambda / trailing-closure form: when the LAST param is a function type
    // `fn: (A...) -> B`, official adds a `C(...) { A1, A2 => B }` item that
    // drops the fn param and renders the closure. Only when fn is last.
    if let Some(fn_param) = params.last().filter(|p| matches!(&p.ty, Type::Func { .. })) {
        let (fn_args, fn_ret) = match &fn_param.ty {
            Type::Func {
                params: fp, ret, ..
            } => {
                let a: Vec<String> = fp.iter().map(display_type).collect();
                (a, display_type(ret))
            }
            _ => (Vec::new(), String::new()),
        };
        let leading = &params[..params.len() - 1];
        let mut c = if tp_num == 0 { 1 } else { tp_num + 2 };
        let ins_leading: Vec<String> = leading.iter().map(|p| param_insert(p, &mut c)).collect();
        // closure snippets continue the counter
        let closure_start = c;
        let arg_names: Vec<String> = fn_args
            .iter()
            .enumerate()
            .map(|(i, t)| format!("arg{}: {t}", i + 1))
            .collect();
        let fn_args_s = fn_args.join(", ");
        let closure_ins = format!(
            "{{ {} => ${{{}:{}}} }}",
            arg_names.join(", "),
            closure_start,
            fn_ret
        );
        let leading_label: Vec<String> = leading.iter().map(param_label).collect();
        let leading_str = if leading.is_empty() {
            String::new()
        } else {
            format!("({})", leading_label.join(", "))
        };
        let closure_label = format!("{class_name}{leading_str} {{ {fn_args_s} => {fn_ret} }}");
        // Closure form reuses the constructor's detail (same signature minus fn param)
        // and builds its own closure insert text.
        let closure_ins_full = if leading.is_empty() {
            format!("{class_name}{tp_insert} {closure_ins}")
        } else {
            format!(
                "{class_name}{tp_insert}({}) {closure_ins}",
                ins_leading.join(", ")
            )
        };
        push_candidate(
            cands,
            seen,
            Candidate {
                label: closure_label,
                kind: KIND_FUNCTION,
                detail,
                insert_text: closure_ins_full,
                insert_text_format: 2,
                filter_text: class_name.to_string(),
            },
        );
    }
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
                let vis = vis_prefix(*is_public, *is_open, *is_sealed, *is_abstract, false, false);
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
                                name,
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
                                "init",
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
                let vis = vis_prefix(*is_public, *is_open, false, false, false, false);
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
                                name,
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
                                "init",
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
                let vis = vis_prefix(*is_public, false, false, false, false, false);
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
                let vis = vis_prefix(*is_public, false, false, false, false, false);
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
                let vis = vis_prefix(*is_public, false, false, false, false, false);
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
                let dp = vis_prefix(*is_public, false, false, *is_abstract, false, true);
                emit_func_items(name, &dp, params, ret, type_params, cands, seen);
            }
            Decl::Var {
                name,
                is_public,
                is_mutable,
                ty,
                init,
                ..
            } => {
                let vis = vis_prefix(*is_public, false, false, false, false, false);
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
                let vis = vis_prefix(*is_public, false, false, false, *is_static, false);
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

/// Collect all top-level declaration candidates for a same-package sibling
/// file (classes + ctors, structs + ctors, enums, interfaces, funcs, vars,
/// type aliases, props) — the same shape `collect_file_decls` produces for the
/// opened file, minus a shared `seen` set so cross-file duplicates collapse.
pub fn sibling_candidates(file: &File) -> Vec<Candidate> {
    let mut cands: Vec<Candidate> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    collect_file_decls(file, &mut cands, &mut seen);
    cands
}

// ─── Helper: position of any expression node ─────────────────────────────
// ─── Helper: initializer text of a statement for detail strings ───────────
// Official detail shows the literal initializer source (`let x: T = <init>`),
// not `= ...`. Statements are single-line in the test suite, so slice from the
// first `=` on the statement's line to the end of the statement.
fn stmt_init_source(source: &str, line_1based: u32) -> String {
    let line_text = source_line_text(source, line_1based.saturating_sub(1));
    if let Some(eq) = line_text.find('=') {
        let rest = line_text[eq + 1..].trim();
        let rest = rest.split(';').next().unwrap_or(rest).trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    "...".to_string()
}

/// Collect local variables/params in scope from the file AST.
fn collect_local_scope(
    file: &File,
    source: &str,
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
                collect_lets_in_block(exprs, source, cursor_line, cands, seen);
            }
        }
    }
}

/// Recursively collect let/var statements in a block of expressions.
fn collect_lets_in_block(
    exprs: &[Expr],
    source: &str,
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for e in exprs {
        match e {
            Expr::LetPatternDestructor { patterns, pos, .. } => {
                if pos.line <= cursor_line {
                    for p in patterns {
                        if let Pattern::Var {
                            name,
                            is_mutable,
                            ty,
                            ..
                        } = p
                        {
                            // Official detail: `let x: T = <init>` — the
                            // literal initializer text, not `= ...`.
                            let kw = if *is_mutable { "var" } else { "let" };
                            let init_s = stmt_init_source(source, pos.line);
                            let detail = if let Some(t) = ty {
                                format!("{kw} {name}: {} = {init_s}", display_type(t))
                            } else {
                                format!("{kw} {name} = {init_s}")
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
                    }
                }
            }
            Expr::If { then, els, .. } => {
                collect_expr_block(then, source, cursor_line, cands, seen);
                if let Some(e) = els {
                    collect_expr_block(e, source, cursor_line, cands, seen);
                }
            }
            Expr::Block { stmts, .. } => {
                collect_lets_in_block(stmts, source, cursor_line, cands, seen);
            }
            Expr::ForIn { body, .. } => collect_expr_block(body, source, cursor_line, cands, seen),
            Expr::While { body, .. } => collect_expr_block(body, source, cursor_line, cands, seen),
            Expr::DoWhile { body, .. } => {
                collect_expr_block(body, source, cursor_line, cands, seen)
            }
            Expr::Match { cases, .. } => {
                for mc in cases {
                    collect_expr_block(&mc.body, source, cursor_line, cands, seen);
                }
            }
            Expr::Try {
                body,
                catches,
                finally,
                ..
            } => {
                collect_expr_block(body, source, cursor_line, cands, seen);
                for c in catches {
                    collect_expr_block(&c.body, source, cursor_line, cands, seen);
                }
                if let Some(f) = finally {
                    collect_expr_block(f, source, cursor_line, cands, seen);
                }
            }
            _ => {}
        }
    }
}

fn collect_expr_block(
    e: &Expr,
    source: &str,
    cursor_line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if let Expr::Block { stmts, .. } = e {
        collect_lets_in_block(stmts, source, cursor_line, cands, seen);
    }
}

/// How members are being accessed: through a value (instance members only),
/// through a type name (static members only), or enum cases.
#[derive(Clone, Copy, PartialEq)]
enum AccessKind {
    Instance,
    Static,
    Enum,
}

/// Base name of a type (strips generic args and qualification).
fn type_base_name(t: &Type) -> String {
    match t {
        Type::Ref { name, .. } => name.split('<').next().unwrap_or(name).to_string(),
        Type::Qualified { name, .. } => name.clone(),
        Type::Option { inner, .. } => type_base_name(inner),
        Type::Paren { inner, .. } => type_base_name(inner),
        Type::Constant { inner, .. } => type_base_name(inner),
        _ => String::new(),
    }
}

/// Find a top-level type decl (class/struct/interface/enum) by base name.
fn find_type_decl<'a>(file: &'a File, base: &str) -> Option<&'a Decl> {
    file.decls.iter().find(|d| match d {
        Decl::Class { name, .. }
        | Decl::Struct { name, .. }
        | Decl::Interface { name, .. }
        | Decl::Enum { name, .. } => name == base,
        _ => false,
    })
}

/// The full source line containing byte `offset`.
fn line_at(source: &str, offset: usize) -> &str {
    let offset = offset.min(source.len());
    let start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let rest = &source[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    &rest[..end]
}

/// Source text before byte `offset` on the SAME line (for ctor-param props).
fn line_up_to(source: &str, offset: usize) -> &str {
    let offset = offset.min(source.len());
    let start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    &source[start..offset]
}

/// Whether the declaration line containing `offset` mentions keyword `kw`
/// (used to recover parser-dropped modifiers: mut / private / protected).
fn decl_line_has(source: &str, offset: usize, kw: &str) -> bool {
    let line = line_at(source, offset);
    line.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|t| t == kw)
}

/// Whether a primary-ctor `let`/`var` prefix makes this param a property.
/// Returns Some(true) for `var`, Some(false) for `let`, None otherwise.
fn ctor_param_kind(source: &str, p: &Param) -> Option<bool> {
    let before = line_up_to(source, p.pos.offset);
    let sep = before.rfind([',', '(']).map(|i| i + 1).unwrap_or(0);
    let seg = &before[sep..];
    let words: Vec<&str> = seg
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .collect();
    if words.contains(&"let") {
        Some(false)
    } else if words.contains(&"var") {
        Some(true)
    } else {
        None
    }
}

/// Infer a display type string from an initializer expression.
fn infer_expr_type(file: &File, source: &str, e: &Expr) -> Option<String> {
    match e {
        Expr::Lit { value, .. } => Some(lit_type_str(value)),
        Expr::Name { name, .. } => resolve_var_type(file, source, name, u32::MAX),
        Expr::Paren { inner, .. } => infer_expr_type(file, source, inner),
        Expr::Unary { inner, .. } => infer_expr_type(file, source, inner),
        Expr::Call { callee, .. } => match callee.as_ref() {
            Expr::Name { name, .. } => Some(name.split('<').next().unwrap_or(name).to_string()),
            Expr::Member { name, .. } => Some(name.clone()),
            _ => None,
        },
        Expr::Member { object, .. } => match object.as_ref() {
            // `Type.EnumCase` / `Type.staticMember` keeps the object's type
            Expr::Name { name, .. } => Some(name.clone()),
            _ => infer_expr_type(file, source, object),
        },
        Expr::Array { .. } => Some("Array".to_string()),
        _ => None,
    }
}

/// Display type of a literal value string.
fn lit_type_str(v: &str) -> String {
    if v.starts_with('"') || v.starts_with("b\"") {
        "String".to_string()
    } else if v.starts_with('\'') || v.starts_with("b'") || v.starts_with("r'") {
        "Rune".to_string()
    } else if v == "true" || v == "false" {
        "Bool".to_string()
    } else if v.contains('.') {
        "Float64".to_string()
    } else if v.contains('f') || v.contains('F') {
        "Float32".to_string()
    } else {
        "Int64".to_string()
    }
}

/// Emit members of a class/struct/interface/enum decl into `cands`.
///
/// Walks the type's own members (filtered by `access`), its parent
/// interfaces/base classes, and same-file `extend` blocks (excluding
/// where-constrained extends). `seen` guards against parent cycles.
fn collect_type_members(
    file: &File,
    source: &str,
    decl: &Decl,
    access: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    match decl {
        Decl::Class { members, name, .. } | Decl::Struct { members, name, .. } => {
            for m in members {
                emit_member_decl(file, source, m, access, name, false, false, cands, seen);
            }
            // parents: base class / interfaces (instance members)
            if access == AccessKind::Instance {
                let parents: Vec<Type> = match decl {
                    Decl::Class { parents, .. } => parents.clone(),
                    _ => Vec::new(),
                };
                for p in parents {
                    let pb = type_base_name(&p);
                    if !seen.contains(&pb) {
                        seen.insert(pb.clone());
                        if let Some(pd) = find_type_decl(file, &pb) {
                            collect_type_members(file, source, pd, access, cands, seen);
                        }
                    }
                }
            }
            collect_extend_members(file, source, name, access, cands, seen);
        }
        Decl::Interface { members, name, .. } => {
            for m in members {
                emit_member_decl(file, source, m, access, name, false, true, cands, seen);
            }
            collect_extend_members(file, source, name, access, cands, seen);
        }
        Decl::Enum { name, cases, .. } => {
            collect_enum_members(name, cases, cands, seen);
        }
        _ => {}
    }
}

/// Collect members contributed by `extend T { ... }` blocks for `base` in the
/// same file (skip extends carrying a `where` constraint — official omits them).
fn collect_extend_members(
    file: &File,
    source: &str,
    base: &str,
    access: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for d in &file.decls {
        if let Decl::Extend {
            target,
            members,
            pos,
            ..
        } = d
        {
            if type_base_name(target) != base {
                continue;
            }
            if decl_line_has(source, pos.offset, "where") {
                continue;
            }
            for m in members {
                emit_member_decl(file, source, m, access, base, true, false, cands, seen);
            }
        }
    }
}

/// Emit `let`/`var`-prefixed primary-ctor params as property members.
fn emit_ctor_properties(
    source: &str,
    params: &[Param],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    for p in params
        .iter()
        .filter(|p| ctor_param_kind(source, p).is_some())
    {
        let is_var = ctor_param_kind(source, p).unwrap_or(false);
        let kw = if is_var { "var" } else { "let" };
        let detail = format!("internal {kw} {}: {}", p.name, display_type(&p.ty));
        push_candidate(
            cands,
            seen,
            Candidate {
                label: p.name.clone(),
                kind: KIND_VARIABLE,
                detail,
                insert_text: p.name.clone(),
                insert_text_format: 1,
                filter_text: p.name.clone(),
            },
        );
    }
}

/// Emit one member decl with the official label/detail/insert formats.
#[allow(clippy::too_many_arguments)]
fn emit_member_decl(
    file: &File,
    source: &str,
    m: &Decl,
    access: AccessKind,
    type_name: &str,
    in_extend: bool,
    from_interface: bool,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    // Private members are not visible to completion (incl. `this.` inside
    // extends); the parser drops the `private` modifier, so recover it here.
    match m {
        Decl::Var { pos, .. } | Decl::Func { pos, .. } | Decl::Prop { pos, .. }
            if decl_line_has(source, pos.offset, "private") =>
        {
            return;
        }
        _ => {}
    }
    match m {
        Decl::Var {
            name,
            is_public,
            is_static,
            is_mutable,
            ty,
            init,
            ..
        } => {
            if !static_matches(*is_static, access) {
                return;
            }
            let vis = vis_prefix(*is_public, false, false, false, *is_static, true);
            let ty_s = ty
                .as_ref()
                .map(display_type)
                .or_else(|| init.as_ref().and_then(|i| infer_expr_type(file, source, i)))
                .unwrap_or_default();
            let init_s = init
                .as_ref()
                .map(expr_lit_src)
                .filter(|s| !s.is_empty())
                .map(|s| format!(" = {s}"))
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
            pos,
            ..
        } => {
            if !static_matches(*is_static, access) {
                return;
            }
            // private props are not visible from outside the extending file;
            // recover public/protected/private + mut from source.
            if in_extend && decl_line_has(source, pos.offset, "private") {
                return;
            }
            let mut vis = if decl_line_has(source, pos.offset, "public") {
                "public ".to_string()
            } else if decl_line_has(source, pos.offset, "protected") {
                "protected ".to_string()
            } else {
                vis_prefix(*is_public, false, false, false, *is_static, true)
            };
            let is_mut = decl_line_has(source, pos.offset, "mut");
            if is_mut {
                vis.push_str("mut ");
            }
            let kw = if is_mut { "var" } else { "let" };
            let detail = format!("{vis}{kw} {name}: {}", display_type(ty));
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
            is_static,
            is_abstract,
            params,
            ret,
            type_params,
            body,
            pos,
            ..
        } => {
            // Type-named primary ctor / init / finalizer: not a member method.
            // Primary-ctor `let`/`var` params become properties instead.
            if *name == type_name || name == "init" || name.starts_with('~') {
                if access == AccessKind::Instance {
                    emit_ctor_properties(source, params, cands, seen);
                }
                return;
            }
            if !static_matches(*is_static, access) {
                return;
            }
            let dp = if from_interface {
                // Interface members are implicitly public; abstract when
                // bodyless, open when they carry a default body, plus mut.
                let mut p = String::from("public ");
                if decl_line_has(source, pos.offset, "mut") {
                    p.push_str("mut ");
                }
                if matches!(body, Body::Empty) {
                    p.push_str("abstract ");
                } else {
                    p.push_str("open ");
                }
                if *is_static {
                    p.push_str("static ");
                }
                p
            } else {
                vis_prefix(*is_public, false, false, *is_abstract, *is_static, true)
            };
            emit_func_items(name, &dp, params, ret, type_params, cands, seen);
        }
        Decl::PrimaryCtor { params, .. } if access == AccessKind::Instance => {
            // `let`/`var`-prefixed primary-ctor params become properties.
            emit_ctor_properties(source, params, cands, seen);
        }
        _ => {}
    }
}

/// Whether a member's static-ness fits the access kind.
fn static_matches(is_static: bool, access: AccessKind) -> bool {
    match access {
        AccessKind::Instance => !is_static,
        AccessKind::Static => is_static,
        AccessKind::Enum => false,
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
                    filter_text: ec.name.clone(),
                },
            );
        } else {
            let payload_strs: Vec<String> = ec.payloads.iter().map(display_type).collect();
            let detail = format!("{name}.{}({})", ec.name, payload_strs.join(", "));
            let ins_payload: Vec<String> = (0..payload_strs.len())
                .map(|i| format!("${{{}:{}}}", i + 1, payload_strs[i]))
                .collect();
            let ins = format!("{}({})", ec.name, ins_payload.join(", "));
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: qname.clone(),
                    kind: KIND_METHOD,
                    detail,
                    insert_text: ins,
                    insert_text_format: 2,
                    filter_text: ec.name.clone(),
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
    for sym in STD_CORE {
        if sym.name == "CFunc" {
            // Official renders CFunc as a type-argument snippet.
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: "CFunc<T>".to_string(),
                    kind: KIND_CLASS,
                    detail: sym.detail.to_string(),
                    insert_text: "CFunc<${1:()} -> ${2:Unit}>".to_string(),
                    insert_text_format: 2,
                    filter_text: sym.name.to_string(),
                },
            );
            continue;
        }
        push_candidate(
            cands,
            seen,
            Candidate {
                label: sym.name.to_string(),
                kind: sym.kind,
                detail: sym.detail.to_string(),
                insert_text: sym.name.to_string(),
                insert_text_format: 1,
                filter_text: sym.name.to_string(),
            },
        );
        // Constructor / overload items share the type's filterText.
        for (label, detail, insert, fmt) in sym.ctors {
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: label.to_string(),
                    kind: KIND_FUNCTION,
                    detail: detail.to_string(),
                    insert_text: insert.to_string(),
                    insert_text_format: *fmt,
                    filter_text: sym.name.to_string(),
                },
            );
        }
    }
}

/// Collect keyword candidates.
fn collect_keywords(cands: &mut Vec<Candidate>, seen: &mut HashSet<String>) {
    for (label, detail, fmt, ins) in KEYWORDS {
        // filterText is the leading keyword token (e.g. `func name(){}` filters
        // on `func`), so the template set collapses under a keyword prefix.
        let filter = label.split([' ', '<', '(']).next().unwrap_or(label);
        push_candidate(
            cands,
            seen,
            Candidate {
                label: label.to_string(),
                kind: KIND_KEYWORD,
                detail: detail.to_string(),
                insert_text: ins.to_string(),
                insert_text_format: *fmt,
                filter_text: filter.to_string(),
            },
        );
    }
}

/// Resolve a variable name to its declared type name (from file + params).
/// Resolve a variable name to its declared/inferred type display string.
///
/// When `line` (0-based cursor line) is known, locals of the enclosing
/// top-level function are preferred; otherwise a file-wide fallback scans
/// top-level vars, class members and any function's params.
fn resolve_var_type(file: &File, source: &str, var_name: &str, line: u32) -> Option<String> {
    if line != u32::MAX {
        let cursor = line + 1; // 1-based
        let mut funcs: Vec<&Decl> = file
            .decls
            .iter()
            .filter(|d| matches!(d, Decl::Func { .. }))
            .collect();
        funcs.sort_by_key(|d| decl_pos(d).line);
        for (i, f) in funcs.iter().enumerate() {
            let start = decl_pos(f).line;
            let end = funcs
                .get(i + 1)
                .map(|n| decl_pos(n).line)
                .unwrap_or(u32::MAX);
            if cursor >= start && cursor < end {
                let mut map = HashMap::new();
                collect_func_locals(file, source, f, &mut map);
                if let Some(t) = map.get(var_name) {
                    return Some(t.clone());
                }
                break;
            }
        }
    }
    // fallback: top-level vars, class member vars, any func params
    for d in &file.decls {
        match d {
            Decl::Var { name, ty, init, .. } if name == var_name => {
                return ty
                    .as_ref()
                    .map(display_type)
                    .or_else(|| init.as_ref().and_then(|i| infer_expr_type(file, source, i)));
            }
            Decl::Func { params, .. } => {
                for p in params {
                    if p.name == var_name {
                        return Some(display_type(&p.ty));
                    }
                }
            }
            Decl::Class { members, .. }
            | Decl::Struct { members, .. }
            | Decl::Interface { members, .. } => {
                for m in members {
                    if let Decl::Var { name, ty, init, .. } = m {
                        if name == var_name {
                            return ty.as_ref().map(display_type).or_else(|| {
                                init.as_ref().and_then(|i| infer_expr_type(file, source, i))
                            });
                        }
                    }
                    if let Decl::Prop { name, ty, .. } = m {
                        if name == var_name {
                            return Some(display_type(ty));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Collect params + local lets/vars of a top-level func into `out`.
fn collect_func_locals(file: &File, source: &str, func: &Decl, out: &mut HashMap<String, String>) {
    if let Decl::Func { params, body, .. } = func {
        for p in params {
            out.insert(p.name.clone(), display_type(&p.ty));
        }
        if let Body::Block(exprs) = body {
            collect_let_types(file, source, exprs, out);
        }
    }
}

/// Walk a block collecting `let`/`var` names with their types (explicit or
/// inferred from the initializer).
fn collect_let_types(file: &File, source: &str, exprs: &[Expr], out: &mut HashMap<String, String>) {
    for e in exprs {
        match e {
            Expr::LetPatternDestructor {
                patterns,
                initializer,
                ..
            } => {
                let inferred = infer_expr_type(file, source, initializer);
                for p in patterns {
                    if let Pattern::Var { name, ty, .. } = p {
                        let t = ty.as_ref().map(display_type).or_else(|| inferred.clone());
                        if let Some(t) = t {
                            out.insert(name.clone(), t);
                        }
                    }
                }
            }
            Expr::If { then, els, .. } => {
                collect_block_lets(then, out);
                if let Some(e) = els {
                    collect_block_lets(e, out);
                }
            }
            Expr::Block { stmts, .. } => collect_let_types(file, source, stmts, out),
            Expr::ForIn { body, .. } => collect_block_lets(body, out),
            Expr::While { body, .. } => collect_block_lets(body, out),
            Expr::DoWhile { body, .. } => collect_block_lets(body, out),
            Expr::Match { cases, .. } => {
                for mc in cases {
                    collect_block_lets(&mc.body, out);
                }
            }
            Expr::Try {
                body,
                catches,
                finally,
                ..
            } => {
                collect_block_lets(body, out);
                for c in catches {
                    collect_block_lets(&c.body, out);
                }
                if let Some(f) = finally {
                    collect_block_lets(f, out);
                }
            }
            _ => {}
        }
    }
}
fn collect_block_lets(e: &Expr, out: &mut HashMap<String, String>) {
    if let Expr::Block { stmts, .. } = e {
        for s in stmts {
            if let Expr::LetPatternDestructor {
                patterns,
                initializer,
                ..
            } = s
            {
                for p in patterns {
                    if let Pattern::Var { name, ty, .. } = p {
                        let t = ty.as_ref().map(display_type);
                        if let Some(t) = t {
                            out.insert(name.clone(), t);
                        }
                    }
                }
                let _ = initializer;
            }
        }
    }
}

/// Result of resolving a receiver expression: its type base name, generic
/// args, and how members are accessed.
struct RcvTarget {
    base: String,
    args: Vec<String>,
    access: AccessKind,
}

/// Resolve a receiver expression (the text before the final `.`) to a
/// completion target: type name → static, enum name → enum cases, variable /
/// ctor-call → instance members of the variable's type.
fn resolve_receiver(file: &File, source: &str, recv: &str, line: u32) -> Option<RcvTarget> {
    let mut recv = recv.trim().to_string();
    let is_opt = recv.ends_with('?');
    if is_opt {
        recv.pop();
        recv = recv.trim().to_string();
    }
    if recv.is_empty() {
        return None;
    }
    // Split chained member access on dots outside parens/brackets.
    let segs = split_access_segments(&recv);
    let mut cur: Option<RcvTarget> = None;
    for seg in &segs {
        cur = Some(resolve_access_step(file, source, seg, line, cur)?);
    }
    // Optional chaining: Option<T> instance access unwraps to T.
    if is_opt {
        if let Some(c) = &mut cur {
            if c.base == "Option" && !c.args.is_empty() {
                let inner = c.args[0].clone();
                c.base = inner.split('<').next().unwrap_or(&inner).trim().to_string();
                c.args = type_args_of(&inner);
                c.access = AccessKind::Instance;
            }
        }
    }
    cur
}

/// One step of a chained access: a bare name (var/type/member) or a ctor call.
fn resolve_access_step(
    file: &File,
    source: &str,
    seg: &str,
    line: u32,
    cur: Option<RcvTarget>,
) -> Option<RcvTarget> {
    let seg = seg.trim();
    if seg.ends_with(')') {
        // Ctor call `Type(...)` / `Type<T>(...)` → instance of the type.
        let open = match_paren_open(seg)?;
        let head = &seg[..open];
        let base = head.split('<').next().unwrap_or(head).trim().to_string();
        return Some(RcvTarget {
            base,
            args: type_args_of(head),
            access: AccessKind::Instance,
        });
    }
    let base = seg.split('<').next().unwrap_or(seg).trim().to_string();
    let args = type_args_of(seg);
    match cur {
        None => {
            // First segment: this / type name / enum / var
            if base == "this" {
                let (cb, _) = enclosing_type(file, line)?;
                return Some(RcvTarget {
                    base: cb,
                    args: Vec::new(),
                    access: AccessKind::Instance,
                });
            }
            if let Some(d) = find_type_decl(file, &base) {
                let access = match d {
                    Decl::Enum { .. } => AccessKind::Enum,
                    _ => AccessKind::Static,
                };
                return Some(RcvTarget { base, args, access });
            }
            if is_std_enum(&base) {
                return Some(RcvTarget {
                    base,
                    args,
                    access: AccessKind::Enum,
                });
            }
            if is_std_type(&base) {
                return Some(RcvTarget {
                    base,
                    args,
                    access: AccessKind::Static,
                });
            }
            // type alias → resolve to its target base
            if let Some(t) = resolve_alias_base(file, &base) {
                return Some(RcvTarget {
                    base: t,
                    args,
                    access: AccessKind::Static,
                });
            }
            if let Some(t) = resolve_var_type(file, source, seg, line) {
                let tb = t.split('<').next().unwrap_or(&t).trim().to_string();
                return Some(RcvTarget {
                    base: tb,
                    args: type_args_of(&t),
                    access: AccessKind::Instance,
                });
            }
            None
        }
        Some(cur) => {
            // member access: member `seg` of the current type
            if let Some(mt) = member_type_of(file, source, &cur.base, &base, cur.access) {
                let tb = mt.split('<').next().unwrap_or(&mt).trim().to_string();
                Some(RcvTarget {
                    base: tb,
                    args: type_args_of(&mt),
                    access: AccessKind::Instance,
                })
            } else {
                None
            }
        }
    }
}

/// Split `a.b.c` on dots that are NOT inside parens/brackets.
fn split_access_segments(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                cur.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                cur.push(ch);
            }
            '.' if depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Index of the `(` matching the trailing `)` of `s`.
fn match_paren_open(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, ch) in s.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the top-level generic args of a type string like `Option<OptionC>`.
fn type_args_of(t: &str) -> Vec<String> {
    let Some(open) = t.find('<') else {
        return Vec::new();
    };
    let Some(close) = t.rfind('>') else {
        return Vec::new();
    };
    if close <= open {
        return Vec::new();
    }
    let inner = &t[open + 1..close];
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '<' | '(' => {
                depth += 1;
                cur.push(ch);
            }
            '>' | ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        args.push(cur.trim().to_string());
    }
    args
}

/// Whether `base` is a known std enum (Option).
fn is_std_enum(base: &str) -> bool {
    matches!(base, "Option")
}

/// Whether `base` is a known std class/struct with a member table.
fn is_std_type(base: &str) -> bool {
    matches!(base, "Array" | "String")
}

/// Resolve a type-alias decl to its target's base name.
fn resolve_alias_base(file: &File, base: &str) -> Option<String> {
    for d in &file.decls {
        if let Decl::TypeAlias { name, target, .. } = d {
            if name == base {
                let tb = type_base_name(target);
                if !tb.is_empty() {
                    return Some(tb);
                }
            }
        }
    }
    None
}

/// Enclosing class/struct decl base name for a cursor line (for `this.`).
fn enclosing_type(file: &File, line: u32) -> Option<(String, AccessKind)> {
    let cursor = line + 1; // 1-based
    let mut types: Vec<&Decl> = file
        .decls
        .iter()
        .filter(|d| matches!(d, Decl::Class { .. } | Decl::Struct { .. }))
        .collect();
    types.sort_by_key(|d| decl_pos(d).line);
    for (i, t) in types.iter().enumerate() {
        let start = decl_pos(t).line;
        let end = types
            .get(i + 1)
            .map(|n| decl_pos(n).line)
            .unwrap_or(u32::MAX);
        if cursor >= start && cursor < end {
            let name = match t {
                Decl::Class { name, .. } | Decl::Struct { name, .. } => name.clone(),
                _ => String::new(),
            };
            if !name.is_empty() {
                return Some((name, AccessKind::Instance));
            }
        }
    }
    None
}

/// Resolve member `member` of type `base` to its type display string.
fn member_type_of(
    file: &File,
    source: &str,
    base: &str,
    member: &str,
    access: AccessKind,
) -> Option<String> {
    let d = find_type_decl(file, base)?;
    match d {
        Decl::Class { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Interface { members, .. } => {
            for m in members {
                match m {
                    Decl::Var {
                        name,
                        is_static,
                        ty,
                        init,
                        ..
                    } if name == member && static_matches(*is_static, access) => {
                        return ty.as_ref().map(display_type).or_else(|| {
                            init.as_ref().and_then(|i| infer_expr_type(file, source, i))
                        });
                    }
                    Decl::Prop {
                        name,
                        is_static,
                        ty,
                        ..
                    } if name == member && static_matches(*is_static, access) => {
                        return Some(display_type(ty));
                    }
                    Decl::Func {
                        name,
                        is_static,
                        ret,
                        ..
                    } if name == member && static_matches(*is_static, access) => {
                        return Some(
                            ret.as_ref()
                                .map(display_type)
                                .unwrap_or_else(|| "Unit".to_string()),
                        );
                    }
                    Decl::PrimaryCtor { params, .. } => {
                        for p in params {
                            if p.name == member && ctor_param_kind(source, p).is_some() {
                                return Some(display_type(&p.ty));
                            }
                        }
                    }
                    _ => {}
                }
            }
            // parents
            let parents: Vec<Type> = match d {
                Decl::Class { parents, .. } => parents.clone(),
                _ => Vec::new(),
            };
            for p in parents {
                let pb = type_base_name(&p);
                if let Some(t) = member_type_of(file, source, &pb, member, access) {
                    return Some(t);
                }
            }
            // extends
            for xd in &file.decls {
                if let Decl::Extend {
                    target,
                    members,
                    pos,
                    ..
                } = xd
                {
                    if type_base_name(target) != base || decl_line_has(source, pos.offset, "where")
                    {
                        continue;
                    }
                    for m in members {
                        if let Decl::Var { name, ty, init, .. } = m {
                            if name == member {
                                return ty.as_ref().map(display_type).or_else(|| {
                                    init.as_ref().and_then(|i| infer_expr_type(file, source, i))
                                });
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Member access completion: resolve the receiver expression and add members.
fn collect_member_access(
    file: &File,
    source: &str,
    receiver: &str,
    line: u32,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if let Some(t) = resolve_receiver(file, source, receiver, line) {
        collect_for_target(file, source, &t, cands, seen);
    }
}

/// Emit the candidate set for a resolved target (std tables + file decls).
fn collect_for_target(
    file: &File,
    source: &str,
    t: &RcvTarget,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if t.base == "Option" && !t.args.is_empty() && t.access != AccessKind::Enum {
        // `?.` / value access unwraps to the inner type's members
        let inner = t.args[0].clone();
        let inner_base = inner.split('<').next().unwrap_or(&inner).trim().to_string();
        let it = RcvTarget {
            base: inner_base,
            args: type_args_of(&inner),
            access: AccessKind::Instance,
        };
        collect_for_target(file, source, &it, cands, seen);
        return;
    }
    match t.base.as_str() {
        "Array" => collect_array_members(t.access, cands, seen),
        "String" => collect_string_members(file, source, t.access, cands, seen),
        "Option" => collect_option_members(t.access, cands, seen),
        _ => {
            if let Some(d) = find_type_decl(file, &t.base) {
                collect_type_members(file, source, d, t.access, cands, seen);
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
    sibling_decls: Option<&[Candidate]>,
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
        let receiver = extract_receiver(&before[..dot_idx]);
        if receiver.is_empty() {
            return Value::Null;
        }
        collect_member_access(file, source, &receiver, line, &mut candidates, &mut seen);
    } else {
        // 1. File top-level decls
        collect_file_decls(file, &mut candidates, &mut seen);

        // 2. Same-package sibling decls (from project root scanning)
        if let Some(sibs) = sibling_decls {
            for c in sibs {
                push_candidate(&mut candidates, &mut seen, c.clone());
            }
        }
        let _ = project_root;

        // 3. Local scope (params, let/var in function body)
        collect_local_scope(file, source, line, &mut candidates, &mut seen);

        // 4. Class members in scope (if cursor inside a class/struct body)
        if let Some((cb, _)) = enclosing_type(file, line) {
            if let Some(d) = find_type_decl(file, &cb) {
                collect_type_members(
                    file,
                    source,
                    d,
                    AccessKind::Instance,
                    &mut candidates,
                    &mut seen,
                );
            }
        }

        // 5. Keywords
        collect_keywords(&mut candidates, &mut seen);

        // 6. Std core symbols
        collect_std_symbols(&mut candidates, &mut seen);
    }

    // Official: empty prefix in plain context → null (no trigger).
    if !is_member_access && prefix.is_empty() {
        return Value::Null;
    }

    // Prefix filter — fuzzy subsequence match (case-insensitive).
    // For member access, the client filters, so we don't filter here.
    let items: Vec<Value> = if !is_member_access {
        candidates
            .iter()
            .filter(|c| fuzzy_match(&c.filter_text, &prefix))
            .map(item_json)
            .collect()
    } else {
        candidates.iter().map(item_json).collect()
    };

    // Return null when no candidates (official behavior)
    if items.is_empty() {
        return Value::Null;
    }
    let _ = uri;
    json!(items)
}

/// Extract the receiver expression immediately before the completion dot,
/// handling pipes/arrows/assignment (e.g. `aa.d1 |> aa.` → `aa`) and keeping
/// balanced trailing calls (`Data1(b: 1)`).
fn extract_receiver(s: &str) -> String {
    let s = s.trim_end();
    if s.is_empty() {
        return String::new();
    }
    if s.ends_with(')') {
        // Balanced trailing call: keep the call expression and resolve its
        // callee receiver recursively (e.g. `var x = Data1(b: 1)` → `Data1`).
        if let Some(open) = match_paren_open(s) {
            let head = &s[..open];
            let head_recv = extract_receiver(head);
            if !head_recv.is_empty() {
                return format!("{head_recv}{}", &s[open..]);
            }
        }
        return s.to_string();
    }
    let cut = [
        s.rfind("|>").map(|i| i + 2),
        s.rfind("~>").map(|i| i + 2),
        s.rfind("=>").map(|i| i + 2),
        s.rfind('{').map(|i| i + 1),
        s.rfind('}').map(|i| i + 1),
        s.rfind('(').map(|i| i + 1),
        s.rfind(')').map(|i| i + 1),
        s.rfind(',').map(|i| i + 1),
        s.rfind(';').map(|i| i + 1),
        s.rfind('=').map(|i| i + 1),
        s.rfind('+').map(|i| i + 1),
        s.rfind('-').map(|i| i + 1),
        s.rfind('*').map(|i| i + 1),
        s.rfind('/').map(|i| i + 1),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);
    let tail = s[cut..].trim();
    if tail.is_empty() {
        s.to_string()
    } else {
        tail.to_string()
    }
}

const ARRAY_MEMBERS: &[(&str, u32, &str, &str, u32, &str)] = &[
    ("all", 2, "", "all", 1, "all"),
    ("all { T => Bool }", 2, "public func all(predicate: (T) -> Bool): Bool", "all { arg1: T => ${1:Bool} }", 2, "all"),
    ("all(predicate: (T) -> Bool)", 2, "public func all(predicate: (T) -> Bool): Bool", "all(${1:predicate: (T) -> Bool})", 2, "all"),
    ("any", 2, "", "any", 1, "any"),
    ("any { T => Bool }", 2, "public func any(predicate: (T) -> Bool): Bool", "any { arg1: T => ${1:Bool} }", 2, "any"),
    ("any(predicate: (T) -> Bool)", 2, "public func any(predicate: (T) -> Bool): Bool", "any(${1:predicate: (T) -> Bool})", 2, "any"),
    ("clone", 2, "", "clone", 1, "clone"),
    ("clone()", 2, "public func clone(): Array<T>", "clone()", 1, "clone"),
    ("clone(range: Range<Int64>)", 2, "public func clone(range: Range<Int64>): Array<T>", "clone(${1:range: Range<Int64>})", 2, "clone"),
    ("concat", 2, "", "concat", 1, "concat"),
    ("concat(other: Array<T>)", 2, "public func concat(other: Array<T>): Array<T>", "concat(${1:other: Array<T>})", 2, "concat"),
    ("contains", 2, "", "contains", 1, "contains"),
    ("contains(element: T)", 2, "public func contains(element: T): Bool", "contains(${1:element: T})", 2, "contains"),
    ("copyTo", 2, "", "copyTo", 1, "copyTo"),
    ("copyTo(dst: Array<T>)", 2, "public func copyTo(dst: Array<T>): Unit", "copyTo(${1:dst: Array<T>})", 2, "copyTo"),
    ("copyTo(dst: Array<T>, srcStart: Int64, dstStart: Int64, copyLen: Int64)", 2, "public func copyTo(dst: Array<T>, srcStart: Int64, dstStart: Int64, copyLen: Int64): Unit", "copyTo(${1:dst: Array<T>}, ${2:srcStart: Int64}, ${3:dstStart: Int64}, ${4:copyLen: Int64})", 2, "copyTo"),
    ("enumerate", 2, "", "enumerate", 1, "enumerate"),
    ("enumerate()", 2, "public func enumerate(): Array<(Int64, T)>", "enumerate()", 1, "enumerate"),
    ("fill", 2, "", "fill", 1, "fill"),
    ("fill(value: T)", 2, "public func fill(value: T): Unit", "fill(${1:value: T})", 2, "fill"),
    ("filter", 2, "", "filter", 1, "filter"),
    ("filter { T => Bool }", 2, "public func filter(predicate: (T) -> Bool): Array<T>", "filter { arg1: T => ${1:Bool} }", 2, "filter"),
    ("filter(predicate: (T) -> Bool)", 2, "public func filter(predicate: (T) -> Bool): Array<T>", "filter(${1:predicate: (T) -> Bool})", 2, "filter"),
    ("filterMap", 2, "", "filterMap", 1, "filterMap"),
    ("filterMap<R> { T => Option<R> }", 2, "public func filterMap<R>(transform: (T) -> Option<R>): Array<R>", "filterMap<${1:R}> { arg1: T => ${3:Option<R>} }", 2, "filterMap"),
    ("filterMap<R>(transform: (T) -> Option<R>)", 2, "public func filterMap<R>(transform: (T) -> Option<R>): Array<R>", "filterMap<${1:R}>(${3:transform: (T) -> Option<R>})", 2, "filterMap"),
    ("first", 6, "public let first: Option<T>", "first", 1, "first"),
    ("flatMap", 2, "", "flatMap", 1, "flatMap"),
    ("flatMap<R> { T => Array<R> }", 2, "public func flatMap<R>(transform: (T) -> Array<R>): Array<R>", "flatMap<${1:R}> { arg1: T => ${3:Array<R>} }", 2, "flatMap"),
    ("flatMap<R>(transform: (T) -> Array<R>)", 2, "public func flatMap<R>(transform: (T) -> Array<R>): Array<R>", "flatMap<${1:R}>(${3:transform: (T) -> Array<R>})", 2, "flatMap"),
    ("fold", 2, "", "fold", 1, "fold"),
    ("fold<R>(initial: R) { R, T => R }", 2, "public func fold<R>(initial: R, operation: (R, T) -> R): R", "fold<${1:R}>(${3:initial: R}) { arg1: R, arg2: T => ${4:R} }", 2, "fold"),
    ("fold<R>(initial: R, operation: (R, T) -> R)", 2, "public func fold<R>(initial: R, operation: (R, T) -> R): R", "fold<${1:R}>(${3:initial: R}, ${4:operation: (R, T) -> R})", 2, "fold"),
    ("forEach", 2, "", "forEach", 1, "forEach"),
    ("forEach { T => Unit }", 2, "public func forEach(action: (T) -> Unit): Unit", "forEach { arg1: T => ${1:Unit} }", 2, "forEach"),
    ("forEach(action: (T) -> Unit)", 2, "public func forEach(action: (T) -> Unit): Unit", "forEach(${1:action: (T) -> Unit})", 2, "forEach"),
    ("get", 2, "", "get", 1, "get"),
    ("get(index: Int64)", 2, "public func get(index: Int64): Option<T>", "get(${1:index: Int64})", 2, "get"),
    ("indexOf", 2, "", "indexOf", 1, "indexOf"),
    ("indexOf(element: T)", 2, "public func indexOf(element: T): Option<Int64>", "indexOf(${1:element: T})", 2, "indexOf"),
    ("indexOf(element: T, fromIndex: Int64)", 2, "public func indexOf(element: T, fromIndex: Int64): Option<Int64>", "indexOf(${1:element: T}, ${2:fromIndex: Int64})", 2, "indexOf"),
    ("indexOf(elements: Array<T>)", 2, "public func indexOf(elements: Array<T>): Option<Int64>", "indexOf(${1:elements: Array<T>})", 2, "indexOf"),
    ("indexOf(elements: Array<T>, fromIndex: Int64)", 2, "public func indexOf(elements: Array<T>, fromIndex: Int64): Option<Int64>", "indexOf(${1:elements: Array<T>}, ${2:fromIndex: Int64})", 2, "indexOf"),
    ("intersperse", 2, "", "intersperse", 1, "intersperse"),
    ("intersperse(separator: T)", 2, "public func intersperse(separator: T): Array<T>", "intersperse(${1:separator: T})", 2, "intersperse"),
    ("isEmpty", 2, "", "isEmpty", 1, "isEmpty"),
    ("isEmpty()", 2, "public func isEmpty(): Bool", "isEmpty()", 1, "isEmpty"),
    ("iterator", 2, "", "iterator", 1, "iterator"),
    ("iterator()", 2, "public func iterator(): Iterator<T>", "iterator()", 1, "iterator"),
    ("last", 6, "public let last: Option<T>", "last", 1, "last"),
    ("lastIndexOf", 2, "", "lastIndexOf", 1, "lastIndexOf"),
    ("lastIndexOf(element: T)", 2, "public func lastIndexOf(element: T): Option<Int64>", "lastIndexOf(${1:element: T})", 2, "lastIndexOf"),
    ("lastIndexOf(element: T, fromIndex: Int64)", 2, "public func lastIndexOf(element: T, fromIndex: Int64): Option<Int64>", "lastIndexOf(${1:element: T}, ${2:fromIndex: Int64})", 2, "lastIndexOf"),
    ("lastIndexOf(elements: Array<T>)", 2, "public func lastIndexOf(elements: Array<T>): Option<Int64>", "lastIndexOf(${1:elements: Array<T>})", 2, "lastIndexOf"),
    ("lastIndexOf(elements: Array<T>, fromIndex: Int64)", 2, "public func lastIndexOf(elements: Array<T>, fromIndex: Int64): Option<Int64>", "lastIndexOf(${1:elements: Array<T>}, ${2:fromIndex: Int64})", 2, "lastIndexOf"),
    ("map", 2, "", "map", 1, "map"),
    ("map<R> { T => R }", 2, "public func map<R>(transform: (T) -> R): Array<R>", "map<${1:R}> { arg1: T => ${3:R} }", 2, "map"),
    ("map<R>(transform: (T) -> R)", 2, "public func map<R>(transform: (T) -> R): Array<R>", "map<${1:R}>(${3:transform: (T) -> R})", 2, "map"),
    ("none", 2, "", "none", 1, "none"),
    ("none { T => Bool }", 2, "public func none(predicate: (T) -> Bool): Bool", "none { arg1: T => ${1:Bool} }", 2, "none"),
    ("none(predicate: (T) -> Bool)", 2, "public func none(predicate: (T) -> Bool): Bool", "none(${1:predicate: (T) -> Bool})", 2, "none"),
    ("printSize", 2, "", "printSize", 1, "printSize"),
    ("printSize()", 2, "public func printSize(): Unit", "printSize()", 1, "printSize"),
    ("reduce", 2, "", "reduce", 1, "reduce"),
    ("reduce { T, T => T }", 2, "public func reduce(operation: (T, T) -> T): Option<T>", "reduce { arg1: T, arg2: T => ${1:T} }", 2, "reduce"),
    ("reduce(operation: (T, T) -> T)", 2, "public func reduce(operation: (T, T) -> T): Option<T>", "reduce(${1:operation: (T, T) -> T})", 2, "reduce"),
    ("removePrefix", 2, "", "removePrefix", 1, "removePrefix"),
    ("removePrefix(prefix: Array<T>)", 2, "public func removePrefix(prefix: Array<T>): Array<T>", "removePrefix(${1:prefix: Array<T>})", 2, "removePrefix"),
    ("removeSuffix", 2, "", "removeSuffix", 1, "removeSuffix"),
    ("removeSuffix(suffix: Array<T>)", 2, "public func removeSuffix(suffix: Array<T>): Array<T>", "removeSuffix(${1:suffix: Array<T>})", 2, "removeSuffix"),
    ("repeat", 2, "", "repeat", 1, "repeat"),
    ("repeat(n: Int64)", 2, "public func repeat(n: Int64): Array<T>", "repeat(${1:n: Int64})", 2, "repeat"),
    ("reverse", 2, "", "reverse", 1, "reverse"),
    ("reverse()", 2, "public func reverse(): Unit", "reverse()", 1, "reverse"),
    ("size", 6, "public let size: Int64", "size", 1, "size"),
    ("size1", 2, "", "size1", 1, "size1"),
    ("size1()", 2, "public func size1(): Unit", "size1()", 1, "size1"),
    ("skip", 2, "", "skip", 1, "skip"),
    ("skip(count: Int64)", 2, "public func skip(count: Int64): Array<T>", "skip(${1:count: Int64})", 2, "skip"),
    ("slice", 2, "", "slice", 1, "slice"),
    ("slice(start: Int64, len: Int64)", 2, "public func slice(start: Int64, len: Int64): Array<T>", "slice(${1:start: Int64}, ${2:len: Int64})", 2, "slice"),
    ("splitAt", 2, "", "splitAt", 1, "splitAt"),
    ("splitAt(mid: Int64)", 2, "public func splitAt(mid: Int64): (Array<T>, Array<T>)", "splitAt(${1:mid: Int64})", 2, "splitAt"),
    ("step", 2, "", "step", 1, "step"),
    ("step(count: Int64)", 2, "public func step(count: Int64): Array<T>", "step(${1:count: Int64})", 2, "step"),
    ("swap", 2, "", "swap", 1, "swap"),
    ("swap(index1: Int64, index2: Int64)", 2, "public func swap(index1: Int64, index2: Int64): Unit", "swap(${1:index1: Int64}, ${2:index2: Int64})", 2, "swap"),
    ("take", 2, "", "take", 1, "take"),
    ("take(count: Int64)", 2, "public func take(count: Int64): Array<T>", "take(${1:count: Int64})", 2, "take"),
    ("toArray", 2, "", "toArray", 1, "toArray"),
    ("toArray()", 2, "public func toArray(): Array<T>", "toArray()", 1, "toArray"),
    ("toString", 2, "", "toString", 1, "toString"),
    ("toString()", 2, "public func toString(): String", "toString()", 1, "toString"),
    ("trimEnd", 2, "", "trimEnd", 1, "trimEnd"),
    ("trimEnd { T => Bool }", 2, "public func trimEnd(predicate: (T) -> Bool): Array<T>", "trimEnd { arg1: T => ${1:Bool} }", 2, "trimEnd"),
    ("trimEnd(predicate: (T) -> Bool)", 2, "public func trimEnd(predicate: (T) -> Bool): Array<T>", "trimEnd(${1:predicate: (T) -> Bool})", 2, "trimEnd"),
    ("trimEnd(set: Array<T>)", 2, "public func trimEnd(set: Array<T>): Array<T>", "trimEnd(${1:set: Array<T>})", 2, "trimEnd"),
    ("trimStart", 2, "", "trimStart", 1, "trimStart"),
    ("trimStart { T => Bool }", 2, "public func trimStart(predicate: (T) -> Bool): Array<T>", "trimStart { arg1: T => ${1:Bool} }", 2, "trimStart"),
    ("trimStart(predicate: (T) -> Bool)", 2, "public func trimStart(predicate: (T) -> Bool): Array<T>", "trimStart(${1:predicate: (T) -> Bool})", 2, "trimStart"),
    ("trimStart(set: Array<T>)", 2, "public func trimStart(set: Array<T>): Array<T>", "trimStart(${1:set: Array<T>})", 2, "trimStart"),
    ("zip", 2, "", "zip", 1, "zip"),
    ("zip<R>(other: Array<R>)", 2, "public func zip<R>(other: Array<R>): Array<(T, R)>", "zip<${1:R}>(${3:other: Array<R>})", 2, "zip"),
];

const STRING_MEMBERS: &[(&str, u32, &str, &str, u32, &str)] = &[
    ("clone", 2, "", "clone", 1, "clone"),
    ("clone()", 2, "public func clone(): String", "clone()", 1, "clone"),
    ("compare", 2, "", "compare", 1, "compare"),
    ("compare(other: T)", 2, "public abstract func compare(other: T): Ordering", "compare(${1:other: T})", 2, "compare"),
    ("compare(str: String)", 2, "public func compare(str: String): Ordering", "compare(${1:str: String})", 2, "compare"),
    ("contains", 2, "", "contains", 1, "contains"),
    ("contains(str: String)", 2, "public func contains(str: String): Bool", "contains(${1:str: String})", 2, "contains"),
    ("count", 2, "", "count", 1, "count"),
    ("count(str: String)", 2, "public func count(str: String): Int64", "count(${1:str: String})", 2, "count"),
    ("endsWith", 2, "", "endsWith", 1, "endsWith"),
    ("endsWith(suffix: String)", 2, "public func endsWith(suffix: String): Bool", "endsWith(${1:suffix: String})", 2, "endsWith"),
    ("equalsIgnoreAsciiCase", 2, "", "equalsIgnoreAsciiCase", 1, "equalsIgnoreAsciiCase"),
    ("equalsIgnoreAsciiCase(other: String)", 2, "public func equalsIgnoreAsciiCase(other: String): Bool", "equalsIgnoreAsciiCase(${1:other: String})", 2, "equalsIgnoreAsciiCase"),
    ("get", 2, "", "get", 1, "get"),
    ("get(index: Int64)", 2, "public func get(index: Int64): Option<Byte>", "get(${1:index: Int64})", 2, "get"),
    ("hasNestedDiff", 0, "import std.unittest.diff", "hasNestedDiff", 1, "hasNestedDiff"),
    ("hashCode", 2, "", "hashCode", 1, "hashCode"),
    ("hashCode()", 2, "public func hashCode(): Int64", "hashCode()", 1, "hashCode"),
    ("indexOf", 2, "", "indexOf", 1, "indexOf"),
    ("indexOf(b: Byte)", 2, "public func indexOf(b: Byte): Option<Int64>", "indexOf(${1:b: Byte})", 2, "indexOf"),
    ("indexOf(b: Byte, fromIndex: Int64)", 2, "public func indexOf(b: Byte, fromIndex: Int64): Option<Int64>", "indexOf(${1:b: Byte}, ${2:fromIndex: Int64})", 2, "indexOf"),
    ("indexOf(str: String)", 2, "public func indexOf(str: String): Option<Int64>", "indexOf(${1:str: String})", 2, "indexOf"),
    ("indexOf(str: String, fromIndex: Int64)", 2, "public func indexOf(str: String, fromIndex: Int64): Option<Int64>", "indexOf(${1:str: String}, ${2:fromIndex: Int64})", 2, "indexOf"),
    ("isAscii", 2, "", "isAscii", 1, "isAscii"),
    ("isAscii()", 2, "public func isAscii(): Bool", "isAscii()", 1, "isAscii"),
    ("isAsciiBlank", 2, "", "isAsciiBlank", 1, "isAsciiBlank"),
    ("isAsciiBlank()", 2, "public func isAsciiBlank(): Bool", "isAsciiBlank()", 1, "isAsciiBlank"),
    ("isBlank()", 2, "import std.unicode", "isBlank()", 1, "isBlank"),
    ("isEmpty", 2, "", "isEmpty", 1, "isEmpty"),
    ("isEmpty()", 2, "public func isEmpty(): Bool", "isEmpty()", 1, "isEmpty"),
    ("iterator", 2, "", "iterator", 1, "iterator"),
    ("iterator()", 2, "public func iterator(): Iterator<Byte>", "iterator()", 1, "iterator"),
    ("lastIndexOf", 2, "", "lastIndexOf", 1, "lastIndexOf"),
    ("lastIndexOf(b: Byte)", 2, "public func lastIndexOf(b: Byte): Option<Int64>", "lastIndexOf(${1:b: Byte})", 2, "lastIndexOf"),
    ("lastIndexOf(b: Byte, fromIndex: Int64)", 2, "public func lastIndexOf(b: Byte, fromIndex: Int64): Option<Int64>", "lastIndexOf(${1:b: Byte}, ${2:fromIndex: Int64})", 2, "lastIndexOf"),
    ("lastIndexOf(str: String)", 2, "public func lastIndexOf(str: String): Option<Int64>", "lastIndexOf(${1:str: String})", 2, "lastIndexOf"),
    ("lastIndexOf(str: String, fromIndex: Int64)", 2, "public func lastIndexOf(str: String, fromIndex: Int64): Option<Int64>", "lastIndexOf(${1:str: String}, ${2:fromIndex: Int64})", 2, "lastIndexOf"),
    ("lazySplit", 2, "", "lazySplit", 1, "lazySplit"),
    ("lazySplit(str: String, maxSplits: Int64, removeEmpty!: Bool = false)", 2, "public func lazySplit(str: String, maxSplits: Int64, removeEmpty!: Bool = false): Iterator<String>", "lazySplit(${1:str: String}, ${2:maxSplits: Int64}, removeEmpty: ${3:Bool = false})", 2, "lazySplit"),
    ("lazySplit(str: String, removeEmpty!: Bool = false)", 2, "public func lazySplit(str: String, removeEmpty!: Bool = false): Iterator<String>", "lazySplit(${1:str: String}, removeEmpty: ${2:Bool = false})", 2, "lazySplit"),
    ("lines", 2, "", "lines", 1, "lines"),
    ("lines()", 2, "public func lines(): Iterator<String>", "lines()", 1, "lines"),
    ("padEnd", 2, "", "padEnd", 1, "padEnd"),
    ("padEnd(totalWidth: Int64, padding!: String = \" \")", 2, "public func padEnd(totalWidth: Int64, padding!: String = \" \"): String", "padEnd(${1:totalWidth: Int64}, padding: ${2:String = \" \"})", 2, "padEnd"),
    ("padStart", 2, "", "padStart", 1, "padStart"),
    ("padStart(totalWidth: Int64, padding!: String = \" \")", 2, "public func padStart(totalWidth: Int64, padding!: String = \" \"): String", "padStart(${1:totalWidth: Int64}, padding: ${2:String = \" \"})", 2, "padStart"),
    ("printSize1", 2, "", "printSize1", 1, "printSize1"),
    ("printSize1()", 2, "internal func printSize1(): Unit", "printSize1()", 1, "printSize1"),
    ("rawData", 2, "", "rawData", 1, "rawData"),
    ("rawData()", 2, "public unsafe func rawData(): Array<Byte>", "rawData()", 1, "rawData"),
    ("removePrefix", 2, "", "removePrefix", 1, "removePrefix"),
    ("removePrefix(prefix: String)", 2, "public func removePrefix(prefix: String): String", "removePrefix(${1:prefix: String})", 2, "removePrefix"),
    ("removeSuffix", 2, "", "removeSuffix", 1, "removeSuffix"),
    ("removeSuffix(suffix: String)", 2, "public func removeSuffix(suffix: String): String", "removeSuffix(${1:suffix: String})", 2, "removeSuffix"),
    ("replace", 2, "", "replace", 1, "replace"),
    ("replace(old: String, new: String)", 2, "public func replace(old: String, new: String): String", "replace(${1:old: String}, ${2:new: String})", 2, "replace"),
    ("runes", 2, "", "runes", 1, "runes"),
    ("runes()", 2, "public func runes(): Iterator<Rune>", "runes()", 1, "runes"),
    ("shrink()", 2, "import std.unittest.prop_test", "shrink()", 1, "shrink"),
    ("size", 6, "public let size: Int64", "size", 1, "size"),
    ("split", 2, "", "split", 1, "split"),
    ("split(str: String, maxSplits: Int64, removeEmpty!: Bool = false)", 2, "public func split(str: String, maxSplits: Int64, removeEmpty!: Bool = false): Array<String>", "split(${1:str: String}, ${2:maxSplits: Int64}, removeEmpty: ${3:Bool = false})", 2, "split"),
    ("split(str: String, removeEmpty!: Bool = false)", 2, "public func split(str: String, removeEmpty!: Bool = false): Array<String>", "split(${1:str: String}, removeEmpty: ${2:Bool = false})", 2, "split"),
    ("startsWith", 2, "", "startsWith", 1, "startsWith"),
    ("startsWith(prefix: String)", 2, "public func startsWith(prefix: String): Bool", "startsWith(${1:prefix: String})", 2, "startsWith"),
    ("toArray", 2, "", "toArray", 1, "toArray"),
    ("toArray()", 2, "public func toArray(): Array<Byte>", "toArray()", 1, "toArray"),
    ("toAsciiLower", 2, "", "toAsciiLower", 1, "toAsciiLower"),
    ("toAsciiLower()", 2, "public func toAsciiLower(): String", "toAsciiLower()", 1, "toAsciiLower"),
    ("toAsciiTitle", 2, "", "toAsciiTitle", 1, "toAsciiTitle"),
    ("toAsciiTitle()", 2, "public func toAsciiTitle(): String", "toAsciiTitle()", 1, "toAsciiTitle"),
    ("toAsciiUpper", 2, "", "toAsciiUpper", 1, "toAsciiUpper"),
    ("toAsciiUpper()", 2, "public func toAsciiUpper(): String", "toAsciiUpper()", 1, "toAsciiUpper"),
    ("toLower()", 2, "import std.unicode", "toLower()", 1, "toLower"),
    ("toLower(opt: CasingOption)", 2, "import std.unicode", "toLower(${1:opt: CasingOption})", 2, "toLower"),
    ("toRuneArray", 2, "", "toRuneArray", 1, "toRuneArray"),
    ("toRuneArray()", 2, "public func toRuneArray(): Array<Rune>", "toRuneArray()", 1, "toRuneArray"),
    ("toString", 2, "", "toString", 1, "toString"),
    ("toString()", 2, "public func toString(): String", "toString()", 1, "toString"),
    ("toTitle()", 2, "import std.unicode", "toTitle()", 1, "toTitle"),
    ("toTitle(opt: CasingOption)", 2, "import std.unicode", "toTitle(${1:opt: CasingOption})", 2, "toTitle"),
    ("toTokens()", 2, "import std.ast", "toTokens()", 1, "toTokens"),
    ("toUpper()", 2, "import std.unicode", "toUpper()", 1, "toUpper"),
    ("toUpper(opt: CasingOption)", 2, "import std.unicode", "toUpper(${1:opt: CasingOption})", 2, "toUpper"),
    ("trim()", 2, "import std.unicode", "trim()", 1, "trim"),
    ("trimAscii", 2, "", "trimAscii", 1, "trimAscii"),
    ("trimAscii()", 2, "public func trimAscii(): String", "trimAscii()", 1, "trimAscii"),
    ("trimAsciiEnd", 2, "", "trimAsciiEnd", 1, "trimAsciiEnd"),
    ("trimAsciiEnd()", 2, "public func trimAsciiEnd(): String", "trimAsciiEnd()", 1, "trimAsciiEnd"),
    ("trimAsciiStart", 2, "", "trimAsciiStart", 1, "trimAsciiStart"),
    ("trimAsciiStart()", 2, "public func trimAsciiStart(): String", "trimAsciiStart()", 1, "trimAsciiStart"),
    ("trimEnd", 2, "", "trimEnd", 1, "trimEnd"),
    ("trimEnd { Rune => Bool }", 2, "public func trimEnd(predicate: (Rune) -> Bool): String", "trimEnd { arg1: Rune => ${1:Bool} }", 2, "trimEnd"),
    ("trimEnd()", 2, "import std.unicode", "trimEnd()", 1, "trimEnd"),
    ("trimEnd(predicate: (Rune) -> Bool)", 2, "public func trimEnd(predicate: (Rune) -> Bool): String", "trimEnd(${1:predicate: (Rune) -> Bool})", 2, "trimEnd"),
    ("trimEnd(set: Array<Rune>)", 2, "public func trimEnd(set: Array<Rune>): String", "trimEnd(${1:set: Array<Rune>})", 2, "trimEnd"),
    ("trimEnd(set: String)", 2, "public func trimEnd(set: String): String", "trimEnd(${1:set: String})", 2, "trimEnd"),
    ("trimLeft()", 2, "import std.unicode", "trimLeft()", 1, "trimLeft"),
    ("trimRight()", 2, "import std.unicode", "trimRight()", 1, "trimRight"),
    ("trimStart", 2, "", "trimStart", 1, "trimStart"),
    ("trimStart { Rune => Bool }", 2, "public func trimStart(predicate: (Rune) -> Bool): String", "trimStart { arg1: Rune => ${1:Bool} }", 2, "trimStart"),
    ("trimStart()", 2, "import std.unicode", "trimStart()", 1, "trimStart"),
    ("trimStart(predicate: (Rune) -> Bool)", 2, "public func trimStart(predicate: (Rune) -> Bool): String", "trimStart(${1:predicate: (Rune) -> Bool})", 2, "trimStart"),
    ("trimStart(set: Array<Rune>)", 2, "public func trimStart(set: Array<Rune>): String", "trimStart(${1:set: Array<Rune>})", 2, "trimStart"),
    ("trimStart(set: String)", 2, "public func trimStart(set: String): String", "trimStart(${1:set: String})", 2, "trimStart"),
];

const OPTION_MEMBERS: &[(&str, u32, &str, &str, u32, &str)] = &[
    ("Option<T>.None", 6, "Option<T>.None", "None", 1, "None"),
    (
        "Option<T>.Some(T)",
        2,
        "Option<T>.Some(T)",
        "Some(${1:T})",
        2,
        "Some",
    ),
];

fn collect_array_members(
    access: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if access == AccessKind::Static {
        return;
    }
    for &(label, kind, detail, ins, fmt, filt) in ARRAY_MEMBERS {
        push_candidate(
            cands,
            seen,
            Candidate {
                label: label.to_string(),
                kind,
                detail: detail.to_string(),
                insert_text: ins.to_string(),
                insert_text_format: fmt,
                filter_text: filt.to_string(),
            },
        );
    }
}

fn collect_string_members(
    _file: &File,
    _source: &str,
    access: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if access == AccessKind::Static {
        return;
    }
    for &(label, kind, detail, ins, fmt, filt) in STRING_MEMBERS {
        push_candidate(
            cands,
            seen,
            Candidate {
                label: label.to_string(),
                kind,
                detail: detail.to_string(),
                insert_text: ins.to_string(),
                insert_text_format: fmt,
                filter_text: filt.to_string(),
            },
        );
    }
}

fn collect_option_members(
    access: AccessKind,
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    if access == AccessKind::Enum {
        for &(label, kind, detail, ins, fmt, filt) in OPTION_MEMBERS {
            push_candidate(
                cands,
                seen,
                Candidate {
                    label: label.to_string(),
                    kind,
                    detail: detail.to_string(),
                    insert_text: ins.to_string(),
                    insert_text_format: fmt,
                    filter_text: filt.to_string(),
                },
            );
        }
    }
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
