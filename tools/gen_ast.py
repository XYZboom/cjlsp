#!/usr/bin/env python3
"""Generate cj-ast Rust definitions from the official Cangjie AST spec.

Sources:
  - ASTKind.inc            : authoritative node inventory (kind, value, node, size)
  - Node.h (manually curated field map below) : per-node semantic fields

The generator emits Rust enum-based AST that mirrors the official hierarchy
(Decl/Expr/Type/Pattern + auxiliary nodes). Compiler-internal fields
(hash/mangledName/desugarExpr/checkFlag/...) are intentionally excluded — the
frontend + LSP do not need them; keeping them would defeat Rust's value types.

Usage: python3 tools/gen_ast.py > crates/cj-ast/src/generated.rs
"""
from __future__ import annotations
import re, sys, os

HERE = os.path.dirname(os.path.abspath(__file__))
# tools/ is inside cj-lang/, official sources are one level up under /root/Code/cangjie/
CANGJIE_DIR = os.path.dirname(HERE)


def find_repo(start: str) -> str:
    """Walk up from `start` until a dir containing the official compiler repo
    (cangjie_compiler/include/cangjie) is found. Works from the main tree and
    from git worktrees (where `..` lands on .worktrees/)."""
    cur = os.path.abspath(start)
    for _ in range(8):
        if os.path.isdir(os.path.join(cur, "cangjie_compiler", "include", "cangjie")):
            return cur
        cur = os.path.dirname(cur)
    return os.path.dirname(CANGJIE_DIR)


REPO = find_repo(CANGJIE_DIR)
ASTKIND_INC = os.path.join(REPO, "cangjie_compiler", "include", "cangjie", "AST", "ASTKind.inc")
# Worktree layout: tools/ lives in .worktrees/<name>/, whose parent is NOT the
# repo root. Fall back to the canonical location of the official sources.
if not os.path.exists(ASTKIND_INC):
    ASTKIND_INC = os.path.join(
        REPO, "cangjie", "cangjie_compiler", "include", "cangjie", "AST", "ASTKind.inc"
    )
if not os.path.exists(ASTKIND_INC):
    ASTKIND_INC = "/root/Code/cangjie/cangjie_compiler/include/cangjie/AST/ASTKind.inc"

# ---------------------------------------------------------------------------
# 1. Parse ASTKind.inc
# ---------------------------------------------------------------------------
def parse_astkind(path: str) -> list[dict]:
    nodes = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r'\s*ASTKIND\((\w+),\s*"([^"]*)",\s*(\w+),\s*(\d+)\)', line)
            if m:
                nodes.append({
                    "kind": m.group(1),
                    "value": m.group(2),
                    "node": m.group(3),
                    "size": int(m.group(4)),
                })
    return nodes

# ---------------------------------------------------------------------------
# 2. Node hierarchy (from ASTKind.inc VALUE markers + node naming)
# ---------------------------------------------------------------------------
# Value strings mark group boundaries: "*decl", "*expr", "*type", "*pattern".
# We classify each node into one of: Decl / Expr / Type / Pattern / Other.
# Group-marker entries (value starts with '*') are NOT concrete nodes.
def classify(nodes: list[dict]) -> dict[str, str]:
    cls = {}
    for n in nodes:
        v = n["value"]
        if v.startswith("*"):
            cls[n["kind"]] = "MARKER"
        else:
            group = n["value"].split("_")[0]  # e.g. main_decl -> main
            # Use the node name's suffix heuristic instead: real nodes belong to
            # the group of the nearest preceding marker. Track current group.
    # redo with order tracking
    cls = {}
    current = None
    # Auxiliary nodes (independent structs, not Expr/Decl/Type/Pattern subtypes).
    AUX = {"GENERIC", "GENERIC_CONSTRAINT", "MATCH_CASE", "MATCH_CASE_OTHER", "FUNC_ARG",
           "FUNC_PARAM_LIST", "FUNC_BODY", "STRUCT_BODY", "CLASS_BODY", "INTERFACE_BODY",
           "DUMMY_BODY", "IMPORT_CONTENT", "IMPORT_SPEC", "PACKAGE_SPEC", "PACKAGE",
           "FEATURE_ID", "FEATURES_SET", "FEATURES_DIRECTIVE", "FILE", "NODE",
           "MACRO_EXPAND_PARAM", "ANNOTATION"}
    # Abstract intermediate base classes (not concrete nodes).
    ABSTRACT = {"DECL", "PATTERN", "TYPE", "EXPR", "CLASS_LIKE_DECL"}
    for n in nodes:
        v = n["value"]
        if v.startswith("*"):
            current = {
                "*decl": "Decl", "*expr": "Expr",
                "*type": "Type", "*pattern": "Pattern",
            }.get(v, "Other")
            cls[n["kind"]] = "MARKER"
        elif n["kind"] in ABSTRACT:
            cls[n["kind"]] = "MARKER"
        elif n["kind"] in AUX:
            cls[n["kind"]] = "Aux"
        else:
            cls[n["kind"]] = current if current else "Other"
    return cls

# ---------------------------------------------------------------------------
# 3. Semantic fields per node (curated from Node.h).
#    Maps official node kind -> list of (rust_name, rust_type, doc).
#    Compiler-internal fields are excluded (see header comment).
# ---------------------------------------------------------------------------
FIELDS: dict[str, list[tuple[str, str, str]]] = {
    # ---- Decls ----
    "MAIN_DECL": [("body", "Body", "function body")],
    "FUNC_DECL": [
        ("name", "String", "function name"),
        ("name_pos", "CodePos", "position of the function name token (start of the identifier)"),
        ("is_public", "bool", "public modifier"),
        ("is_static", "bool", "static modifier"),
        ("is_abstract", "bool", "abstract modifier"),
        ("type_params", "Vec<TypeParam>", "generic parameters"),
        ("params", "Vec<Param>", "parameters"),
        ("ret", "Option<Type>", "return type"),
        ("body", "Body", "function body"),
    ],
    "MACRO_DECL": [
        ("name", "String", "macro name"),
        ("is_public", "bool", "public modifier"),
        ("params", "Vec<Param>", "parameters"),
        ("body", "Body", "macro body"),
    ],
    "CLASS_DECL": [
        ("name", "String", "class name"),
        ("name_pos", "CodePos", "position of the class name token"),
        ("is_public", "bool", "public modifier"),
        ("is_abstract", "bool", "abstract modifier"),
        ("is_open", "bool", "open modifier"),
        ("is_sealed", "bool", "sealed modifier"),
        ("type_params", "Vec<TypeParam>", "generic parameters"),
        ("parents", "Vec<Type>", "parent types (<: A, B)"),
        ("members", "Vec<Decl>", "class members"),
    ],
    "INTERFACE_DECL": [
        ("name", "String", "interface name"),
        ("name_pos", "CodePos", "position of the interface name token"),
        ("is_public", "bool", "public modifier"),
        ("type_params", "Vec<TypeParam>", "generic parameters"),
        ("parents", "Vec<Type>", "parent interfaces"),
        ("members", "Vec<Decl>", "interface members"),
    ],
    "EXTEND_DECL": [
        ("is_public", "bool", "public modifier"),
        ("target", "Type", "extended type"),
        ("members", "Vec<Decl>", "extension members"),
    ],
    "ENUM_DECL": [
        ("name", "String", "enum name"),
        ("name_pos", "CodePos", "position of the enum name token"),
        ("is_public", "bool", "public modifier"),
        ("type_params", "Vec<TypeParam>", "generic parameters"),
        ("cases", "Vec<EnumCase>", "enum cases"),
    ],
    "STRUCT_DECL": [
        ("name", "String", "struct name"),
        ("name_pos", "CodePos", "position of the struct name token"),
        ("is_public", "bool", "public modifier"),
        ("is_open", "bool", "open modifier"),
        ("type_params", "Vec<TypeParam>", "generic parameters"),
        ("members", "Vec<Decl>", "struct members"),
    ],
    "TYPE_ALIAS_DECL": [
        ("name", "String", "alias name"),
        ("is_public", "bool", "public modifier"),
        ("target", "Type", "aliased type"),
    ],
    "PRIMARY_CTOR_DECL": [
        ("is_public", "bool", "public modifier"),
        ("params", "Vec<Param>", "constructor parameters"),
        ("body", "Body", "constructor body"),
    ],
    "BUILTIN_DECL": [("name", "String", "builtin decl name")],
    "VAR_DECL": [
        ("name", "String", "variable name"),
        ("name_pos", "CodePos", "position of the variable name token"),
        ("is_mutable", "bool", "var vs let"),
        ("is_public", "bool", "public modifier"),
        ("is_static", "bool", "static modifier"),
        ("ty", "Option<Type>", "declared type"),
        ("init", "Option<Expr>", "initializer"),
    ],
    "PROP_DECL": [
        ("name", "String", "property name"),
        ("is_public", "bool", "public modifier"),
        ("is_static", "bool", "static modifier"),
        ("ty", "Type", "property type"),
    ],
    "FUNC_PARAM": [
        ("name", "String", "parameter name"),
        ("is_named", "bool", "named-call parameter (a!:)"),
        ("ty", "Type", "parameter type"),
        ("default", "Option<Expr>", "default value"),
    ],
    "VAR_WITH_PATTERN_DECL": [
        ("pattern", "Pattern", "destructuring pattern"),
        ("ty", "Option<Type>", "declared type"),
        ("init", "Option<Expr>", "initializer"),
    ],
    "GENERIC_PARAM_DECL": [
        ("name", "String", "type parameter name"),
        ("bounds", "Vec<Type>", "constraints (where T <: X)"),
    ],
    "PACKAGE_DECL": [("name", "String", "package name")],
    "MACRO_EXPAND_DECL": [
        ("name", "String", "macro name"),
        ("args", "Vec<Tokenish>", "macro arguments"),
    ],
    "INVALID_DECL": [],

    # ---- Patterns ----
    "VAR_PATTERN": [
        ("name", "String", "binding name"),
        ("name_pos", "CodePos", "position of the binding name token"),
        ("is_mutable", "bool", "mut modifier"),
        ("ty", "Option<Type>", "optional type annotation (e.g. `let x: Int64 = ...`)"),
    ],
    "CONST_PATTERN": [("literal", "Option<Box<Expr>>", "literal constant (ConstPattern.literal)")],
    "TUPLE_PATTERN": [("elements", "Vec<Pattern>", "tuple elements")],
    "ENUM_PATTERN": [
        ("name", "String", "constructor name"),
        ("args", "Vec<Pattern>", "constructor args"),
    ],
    "VAR_OR_ENUM_PATTERN": [
        ("name", "String", "name (var or enum ctor)"),
        ("args", "Vec<Pattern>", "args if enum ctor"),
    ],
    "TYPE_PATTERN": [("ty", "Type", "type annotation")],
    "EXCEPT_TYPE_PATTERN": [("ty", "Type", "excluded type")],
    "COMMAND_TYPE_PATTERN": [("ty", "Type", "effect type")],
    "WILDCARD_PATTERN": [],
    "INVALID_PATTERN": [],

    # ---- Types ----
    "REF_TYPE": [
        ("name", "String", "type name"),
        ("args", "Vec<Type>", "generic args"),
    ],
    "QUALIFIED_TYPE": [("name", "String", "qualified name (a.b.C)")],
    "OPTION_TYPE": [("inner", "Box<Type>", "T? inner type")],
    "CONSTANT_TYPE": [("inner", "Box<Type>", "const T inner type")],
    "VARRAY_TYPE": [("inner", "Box<Type>", "VArray<T> inner type")],
    "PRIMITIVE_TYPE": [("kind", "PrimitiveKind", "primitive kind")],
    "PAREN_TYPE": [("inner", "Box<Type>", "parenthesized type")],
    "FUNC_TYPE": [
        ("params", "Vec<Type>", "parameter types"),
        ("ret", "Box<Type>", "return type"),
    ],
    "TUPLE_TYPE": [("elements", "Vec<Type>", "tuple element types")],
    "THIS_TYPE": [],
    "INVALID_TYPE": [],

    # ---- Exprs ----
    "WILDCARD_EXPR": [],
    "CALL_EXPR": [
        ("callee", "Box<Expr>", "called expression"),
        ("args", "Vec<FuncArg>", "arguments"),
        ("type_args", "Vec<Type>", "explicit type args"),
    ],
    "PAREN_EXPR": [("inner", "Box<Expr>", "parenthesized expression")],
    "MEMBER_ACCESS": [
        ("object", "Box<Expr>", "receiver"),
        ("name", "String", "member name"),
    ],
    "REF_EXPR": [
        ("name", "String", "referenced name"),
        ("type_args", "Vec<Type>", "explicit type args"),
    ],
    "OPTIONAL_EXPR": [("inner", "Box<Expr>", "postfix ? expression")],
    "OPTIONAL_CHAIN_EXPR": [("inner", "Box<Expr>", "?. chain root")],
    "PRIMITIVE_TYPE_EXPR": [("kind", "PrimitiveKind", "primitive type name")],
    "RETURN_EXPR": [("value", "Option<Box<Expr>>", "returned value")],
    "LIT_CONST_EXPR": [
        ("kind", "LitKind", "literal kind"),
        ("value", "String", "raw literal value"),
    ],
    "INTERPOLATION_EXPR": [("parts", "Vec<InterpPart>", "interpolation parts")],
    "STR_INTERPOLATION_EXPR": [
        ("parts", "Vec<InterpPart>", "string interpolation parts"),
    ],
    "ASSIGN_EXPR": [
        ("op", "AssignOp", "assignment operator"),
        ("lhs", "Box<Expr>", "left-hand side"),
        ("rhs", "Box<Expr>", "right-hand side"),
    ],
    "UNARY_EXPR": [
        ("op", "UnOp", "unary operator"),
        ("inner", "Box<Expr>", "operand"),
    ],
    "BINARY_EXPR": [
        ("op", "BinOp", "binary operator"),
        ("lhs", "Box<Expr>", "left operand"),
        ("rhs", "Box<Expr>", "right operand"),
    ],
    "INC_OR_DEC_EXPR": [
        ("is_inc", "bool", "++ vs --"),
        ("is_prefix", "bool", "prefix vs postfix"),
        ("inner", "Box<Expr>", "operand"),
    ],
    "SUBSCRIPT_EXPR": [
        ("object", "Box<Expr>", "indexed object"),
        ("index", "Box<Expr>", "index expression"),
    ],
    "IS_EXPR": [
        ("inner", "Box<Expr>", "tested expression"),
        ("ty", "Type", "type"),
    ],
    "AS_EXPR": [
        ("inner", "Box<Expr>", "casted expression"),
        ("ty", "Type", "target type"),
    ],
    "RANGE_EXPR": [
        ("start", "Box<Expr>", "range start"),
        ("end", "Box<Expr>", "range end"),
        ("inclusive", "bool", ".. vs ..="),
    ],
    "ARRAY_LIT": [("elements", "Vec<Expr>", "array elements")],
    "ARRAY_EXPR": [("elements", "Vec<Expr>", "array expression elements")],
    "POINTER_EXPR": [("inner", "Box<Expr>", "pointer dereference")],
    "TUPLE_LIT": [("elements", "Vec<Expr>", "tuple elements")],
    "MATCH_EXPR": [
        ("scrutinee", "Box<Expr>", "matched expression"),
        ("cases", "Vec<MatchCase>", "match cases"),
    ],
    "BLOCK": [("stmts", "Vec<Expr>", "statements")],
    "IF_EXPR": [
        ("cond", "Box<Expr>", "condition"),
        ("then", "Box<Expr>", "then branch"),
        ("els", "Option<Box<Expr>>", "else branch"),
    ],
    "LET_PATTERN_DESTRUCTOR": [
        ("patterns", "Vec<Pattern>", "patterns to be destructed"),
        ("initializer", "Box<Expr>", "initializer expression"),
    ],
    "TOKEN_PART": [("text", "String", "token text")],
    "QUOTE_EXPR": [("parts", "Vec<Expr>", "quote parts")],
    "TRY_EXPR": [
        ("body", "Box<Expr>", "try body"),
        ("catches", "Vec<CatchClause>", "catch clauses"),
        ("finally", "Option<Box<Expr>>", "finally block"),
    ],
    "WHILE_EXPR": [
        ("cond", "Box<Expr>", "condition"),
        ("body", "Box<Expr>", "loop body"),
    ],
    "JUMP_EXPR": [
        ("is_break", "bool", "break vs continue"),
    ],
    "LAMBDA_EXPR": [
        ("params", "Vec<Param>", "lambda parameters"),
        ("body", "Box<Expr>", "lambda body"),
    ],
    "TRAIL_CLOSURE_EXPR": [
        ("call", "Box<Expr>", "call receiving closure"),
        ("closure", "Box<Expr>", "trailing closure"),
    ],
    "FOR_IN_EXPR": [
        ("pattern", "Pattern", "loop pattern"),
        ("iter", "Box<Expr>", "iterated expression"),
        ("body", "Box<Expr>", "loop body"),
    ],
    "DO_WHILE_EXPR": [
        ("cond", "Box<Expr>", "condition"),
        ("body", "Box<Expr>", "loop body"),
    ],
    "TYPE_CONV_EXPR": [
        ("ty", "Type", "target type"),
        ("inner", "Box<Expr>", "converted expression"),
    ],
    "THROW_EXPR": [("inner", "Box<Expr>", "thrown expression")],
    "PERFORM_EXPR": [("inner", "Box<Expr>", "performed expression")],
    "RESUME_EXPR": [("inner", "Box<Expr>", "resumed expression")],
    "SPAWN_EXPR": [("inner", "Box<Expr>", "spawned block")],
    "SYNCHRONIZED_EXPR": [("inner", "Box<Expr>", "synchronized block")],
    "MACRO_EXPAND_EXPR": [
        ("name", "String", "macro name"),
        ("args", "Vec<Tokenish>", "macro args"),
    ],
    "IF_AVAILABLE_EXPR": [
        ("features", "Vec<String>", "feature list"),
        ("then", "Box<Expr>", "then branch"),
        ("els", "Option<Box<Expr>>", "else branch"),
    ],
    "INVALID_EXPR": [],

    # ---- auxiliary nodes ----
    "GENERIC": [("params", "Vec<TypeParam>", "generic parameters")],
    "GENERIC_CONSTRAINT": [("name", "String", "constrained name")],
    "MATCH_CASE": [
        ("pattern", "Pattern", "case pattern"),
        ("guard", "Option<Expr>", "if guard"),
        ("body", "Expr", "case body"),
    ],
    "MATCH_CASE_OTHER": [("body", "Expr", "else body")],
    "FUNC_ARG": [
        ("name", "Option<String>", "named arg name"),
        ("value", "Expr", "argument value"),
    ],
    "FUNC_PARAM_LIST": [("params", "Vec<Param>", "parameters")],
    "FUNC_BODY": [
        ("params", "Vec<Param>", "parameters"),
        ("ret", "Option<Type>", "return type"),
        ("body", "Body", "body"),
    ],
    "STRUCT_BODY": [("members", "Vec<Decl>", "members")],
    "CLASS_BODY": [("members", "Vec<Decl>", "members")],
    "INTERFACE_BODY": [("members", "Vec<Decl>", "members")],
    "DUMMY_BODY": [],
    "IMPORT_CONTENT": [("path", "Vec<String>", "import path parts")],
    "IMPORT_SPEC": [
        ("path", "Vec<String>", "import path"),
        ("glob", "bool", ".* import"),
        ("selected", "Vec<String>", "selected symbols"),
    ],
    "PACKAGE_SPEC": [("name", "String", "package name")],
    "PACKAGE": [
        ("name", "String", "package name"),
        ("files", "Vec<File>", "source files"),
    ],
    "FEATURE_ID": [("name", "String", "feature name")],
    "FEATURES_SET": [("features", "Vec<String>", "features")],
    "FEATURES_DIRECTIVE": [("features", "Vec<String>", "features")],
    "FILE": [
        ("package", "Option<String>", "package name"),
        ("package_pos", "Option<CodePos>", "package name position"),
        ("imports", "Vec<ImportSpec>", "imports"),
        ("decls", "Vec<Decl>", "declarations"),
    ],
    "NODE": [],
}

# ---------------------------------------------------------------------------
# 4. Group hierarchy for generated enums.
# ---------------------------------------------------------------------------
def build() -> str:
    nodes = parse_astkind(ASTKIND_INC)
    cls = classify(nodes)
    lines: list[str] = []
    w = lines.append

    w("// AUTO-GENERATED from official cangjie ASTKind.inc + Node.h — do not edit.")
    w("// Generated by tools/gen_ast.py. Node inventory mirrors the official")
    w("// compiler; compiler-internal fields are excluded (frontend/LSP only).")
    w("")
    w("use crate::program::CodePos;")
    w("")
    # ---- primitive kind enum ----
    w("/// Primitive/builtin type kinds (official `BuiltInType`).")
    w("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    w("pub enum PrimitiveKind {")
    for p in ["Int8", "Int16", "Int32", "Int64", "IntNative", "UInt8", "UInt16",
              "UInt32", "UInt64", "UIntNative", "Float16", "Float32", "Float64",
              "Rune", "Bool", "Nothing", "Unit", "VArray", "String"]:
        w(f"    {p},")
    w("}")
    w("")

    # ---- literal kinds ----
    w("/// Literal kind (official `LitConstKind`).")
    w("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    w("pub enum LitKind { Integer, RuneByte, Float, Rune, String, JString, Bool, Unit, None }")
    w("")

    # ---- operator enums ----
    w("/// Binary operator.")
    w("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    w("pub enum BinOp { Add, Sub, Mul, Div, Mod, Exp, And, Or, BitAnd, BitOr, BitXor,")
    w("    LShift, RShift, Eq, Ne, Lt, Gt, Le, Ge, Coalesce, Pipe, Range, ClosedRange }")
    w("")
    w("/// Unary operator.")
    w("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    w("pub enum UnOp { Neg, Pos, Not, BitNot }")
    w("")
    w("/// Assignment operator.")
    w("#[derive(Debug, Clone, Copy, PartialEq, Eq)]")
    w("pub enum AssignOp { Assign, AddAssign, SubAssign, MulAssign, DivAssign, ModAssign,")
    w("    ExpAssign, AndAssign, OrAssign, BitAndAssign, BitOrAssign, BitXorAssign,")
    w("    LShiftAssign, RShiftAssign }")
    w("")

    # ---- TypeParam / Param / body ----
    w("/// Generic type parameter.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct TypeParam { pub name: String, pub bounds: Vec<Type>, pub pos: CodePos }")
    w("")
    w("/// Function/method parameter.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct Param { pub name: String, pub is_named: bool, pub ty: Type,")
    w("    pub default: Option<Expr>, pub pos: CodePos }")
    w("")
    w("/// Function body.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum Body { Block(Vec<Expr>), Empty }")
    w("")
    w("/// Call argument.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct FuncArg { pub name: Option<String>, pub value: Expr, pub pos: CodePos }")
    w("")
    w("/// Interpolation part: literal text or embedded expression.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum InterpPart { Text(String), Expr(Box<Expr>) }")
    w("")
    w("/// Catch clause.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct CatchClause { pub name: Option<String>, pub ty: Option<Type>,")
    w("    pub body: Expr, pub pos: CodePos }")
    w("")
    w("/// Match case.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct MatchCase { pub pattern: Pattern, pub guard: Option<Expr>,")
    w("    pub body: Expr, pub pos: CodePos }")
    w("")
    w("/// Enum case.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct EnumCase { pub name: String, pub payloads: Vec<Type>, pub pos: CodePos }")
    w("")
    w("/// Import spec.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct ImportSpec { pub path: Vec<String>, pub glob: bool,")
    w("    pub selected: Vec<String>, pub pos: CodePos,")
    w("    /// Span of the imported package name (first..last path segment).")
    w("    pub name_pos: CodePos }")
    w("")
    w("/// File (package + imports + decls).")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct File { pub package: Option<String>,")
    w("    /// Position of the declared package name (start of first segment).")
    w("    pub package_pos: Option<CodePos>,")
    w("    pub imports: Vec<ImportSpec>,")
    w("    pub decls: Vec<Decl>, pub pos: CodePos }")
    w("")

    # ---- Type enum ----
    type_kinds = [n["kind"] for n in nodes if cls.get(n["kind"]) == "Type"]
    w("/// Type node.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum Type {")
    for k in type_kinds:
        fields = FIELDS.get(k, [])
        name = k.split("_")[0].title() + "Type" if k != "REF_TYPE" else "Ref"
        # map kind to a readable variant name
        variant = {
            "REF_TYPE": "Ref", "QUALIFIED_TYPE": "Qualified", "OPTION_TYPE": "Option",
            "CONSTANT_TYPE": "Constant", "VARRAY_TYPE": "VArray", "PRIMITIVE_TYPE": "Primitive",
            "PAREN_TYPE": "Paren", "FUNC_TYPE": "Func", "TUPLE_TYPE": "Tuple",
            "THIS_TYPE": "This", "INVALID_TYPE": "Invalid",
        }[k]
        if not fields:
            w(f"    /// {k}")
            w(f"    {variant}(CodePos),")
        else:
            w(f"    /// {k}")
            w(f"    {variant} {{")
            for (rn, rt, doc) in fields:
                w(f"        /// {doc}")
                w(f"        {rn}: {rt},")
            w(f"        pos: CodePos,")
            w(f"    }},")
    w("}")
    w("")

    # ---- Pattern enum ----
    pat_kinds = [n["kind"] for n in nodes if cls.get(n["kind"]) == "Pattern"]
    w("/// Pattern node.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum Pattern {")
    variant = {
        "VAR_PATTERN": "Var", "CONST_PATTERN": "Const", "TUPLE_PATTERN": "Tuple",
        "ENUM_PATTERN": "Enum", "VAR_OR_ENUM_PATTERN": "VarOrEnum",
        "TYPE_PATTERN": "Typed", "EXCEPT_TYPE_PATTERN": "ExceptType",
        "COMMAND_TYPE_PATTERN": "CommandType", "WILDCARD_PATTERN": "Wildcard",
        "INVALID_PATTERN": "Invalid",
    }
    for k in pat_kinds:
        fields = FIELDS.get(k, [])
        v = variant.get(k, k.title())
        if not fields:
            w(f"    /// {k}")
            w(f"    {v}(CodePos),")
        else:
            w(f"    /// {k}")
            w(f"    {v} {{")
            for (rn, rt, doc) in fields:
                w(f"        /// {doc}")
                w(f"        {rn}: {rt},")
            w(f"        pos: CodePos,")
            w(f"    }},")
    w("}")
    w("")

    # ---- Expr enum ----
    expr_kinds = [n["kind"] for n in nodes if cls.get(n["kind"]) == "Expr"]
    w("/// Expression node.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum Expr {")
    for k in expr_kinds:
        fields = FIELDS.get(k, [])
        v = {
            "WILDCARD_EXPR": "Wildcard", "CALL_EXPR": "Call", "PAREN_EXPR": "Paren",
            "MEMBER_ACCESS": "Member", "REF_EXPR": "Name", "OPTIONAL_EXPR": "Optional",
            "OPTIONAL_CHAIN_EXPR": "OptionalChain", "PRIMITIVE_TYPE_EXPR": "PrimitiveType",
            "RETURN_EXPR": "Return", "LIT_CONST_EXPR": "Lit", "INTERPOLATION_EXPR": "Interpolation",
            "STR_INTERPOLATION_EXPR": "StrInterpolation", "ASSIGN_EXPR": "Assign",
            "UNARY_EXPR": "Unary", "BINARY_EXPR": "Binary", "INC_OR_DEC_EXPR": "IncOrDec",
            "SUBSCRIPT_EXPR": "Subscript", "IS_EXPR": "Is", "AS_EXPR": "As",
            "RANGE_EXPR": "Range", "ARRAY_LIT": "ArrayLit", "ARRAY_EXPR": "Array",
            "POINTER_EXPR": "Pointer", "TUPLE_LIT": "Tuple", "MATCH_EXPR": "Match",
            "BLOCK": "Block", "IF_EXPR": "If", "LET_PATTERN_DESTRUCTOR": "LetPatternDestructor",
            "TOKEN_PART": "TokenPart", "QUOTE_EXPR": "Quote", "TRY_EXPR": "Try",
            "WHILE_EXPR": "While", "JUMP_EXPR": "Jump", "LAMBDA_EXPR": "Lambda",
            "TRAIL_CLOSURE_EXPR": "TrailingClosure", "FOR_IN_EXPR": "ForIn",
            "DO_WHILE_EXPR": "DoWhile", "TYPE_CONV_EXPR": "TypeConv", "THROW_EXPR": "Throw",
            "PERFORM_EXPR": "Perform", "RESUME_EXPR": "Resume", "SPAWN_EXPR": "Spawn",
            "SYNCHRONIZED_EXPR": "Synchronized", "MACRO_EXPAND_EXPR": "MacroExpand",
            "IF_AVAILABLE_EXPR": "IfAvailable", "INVALID_EXPR": "Invalid",
        }[k]
        if not fields:
            w(f"    /// {k}")
            w(f"    {v}(CodePos),")
        else:
            w(f"    /// {k}")
            w(f"    {v} {{")
            for (rn, rt, doc) in fields:
                w(f"        /// {doc}")
                w(f"        {rn}: {rt},")
            w(f"        pos: CodePos,")
            w(f"    }},")
    w("}")
    w("")

    # ---- Decl enum ----
    decl_kinds = [n["kind"] for n in nodes if cls.get(n["kind"]) == "Decl"]
    w("/// Declaration node.")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub enum Decl {")
    for k in decl_kinds:
        fields = FIELDS.get(k, [])
        v = {
            "MAIN_DECL": "Main", "FUNC_DECL": "Func", "MACRO_DECL": "Macro",
            "CLASS_DECL": "Class", "INTERFACE_DECL": "Interface", "EXTEND_DECL": "Extend",
            "ENUM_DECL": "Enum", "STRUCT_DECL": "Struct", "TYPE_ALIAS_DECL": "TypeAlias",
            "PRIMARY_CTOR_DECL": "PrimaryCtor", "BUILTIN_DECL": "Builtin",
            "VAR_DECL": "Var", "PROP_DECL": "Prop", "FUNC_PARAM": "FuncParam",
            "VAR_WITH_PATTERN_DECL": "VarWithPattern", "GENERIC_PARAM_DECL": "GenericParam",
            "PACKAGE_DECL": "Package", "MACRO_EXPAND_DECL": "MacroExpand",
            "INVALID_DECL": "Invalid",
        }[k]
        if not fields:
            w(f"    /// {k}")
            w(f"    {v}(CodePos),")
        else:
            w(f"    /// {k}")
            w(f"    {v} {{")
            for (rn, rt, doc) in fields:
                w(f"        /// {doc}")
                w(f"        {rn}: {rt},")
            w(f"        pos: CodePos,")
            w(f"    }},")
    w("}")
    w("")

    # ---- Tokenish (macro arg carrier) ----
    w("/// Placeholder for macro arguments (token-level; refined with macro support).")
    w("#[derive(Debug, Clone, PartialEq, Eq)]")
    w("pub struct Tokenish { pub text: String, pub pos: CodePos }")
    w("")

    return "\n".join(lines)

if __name__ == "__main__":
    print(build())
