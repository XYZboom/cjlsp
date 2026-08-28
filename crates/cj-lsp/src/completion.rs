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

// insertText form: `name: ${N:T}` for named, `${N:name: T}` for positional
fn param_insert(p: &Param, counter: &mut u32) -> String {
    let n = *counter;
    *counter += 1;
    let ty_s = display_type(&p.ty);
    let default_s = param_default_src(p);
    if p.is_named {
        format!("{}: ${{{}}}{}", p.name, n, ty_s + &default_s)
    } else {
        format!("${{{0}: {1}: {2}}}", n, p.name, ty_s + &default_s)
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
        Expr::Lit { value, .. } => value.clone(),
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
#[allow(clippy::too_many_arguments)]
fn emit_func_items(
    name: &str,
    is_public: bool,
    is_static: bool,
    is_abstract: bool,
    show_internal: bool,
    params: &[Param],
    ret: &Option<Type>,
    type_params: &[cj_ast::TypeParam],
    cands: &mut Vec<Candidate>,
    seen: &mut HashSet<String>,
) {
    let vis = vis_prefix(
        is_public,
        false,
        false,
        is_abstract,
        is_static,
        show_internal,
    );
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
        let ins = if leading.is_empty() {
            format!("{class_name} {closure_ins}")
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
                detail: detail.clone(),
                insert_text: ins,
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
                emit_func_items(
                    name,
                    *is_public,
                    false,
                    *is_abstract,
                    false,
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
                let vis = vis_prefix(*is_public, false, false, false, false, true);
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
                    true,
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
                let vis = vis_prefix(*is_public, false, false, false, *is_static, true);
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
                let vis = vis_prefix(*is_public, false, false, false, false, true);
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
                    true,
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
                let vis = vis_prefix(*is_public, false, false, false, *is_static, true);
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
        let receiver = before[..dot_idx].trim();
        collect_member_access(file, receiver, &mut candidates, &mut seen);
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

        // 4. Class members in scope (if cursor inside a class)
        for d in &file.decls {
            collect_member_scope(d, line, &mut candidates, &mut seen);
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
