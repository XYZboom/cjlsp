// AUTO-GENERATED from official cangjie Tokens.inc — do not edit by hand.
// Source: cangjie_compiler/include/cangjie/Lex/Tokens.inc

// Official token identifiers are ALL_CAPS (e.g. INT8, NOT_IN, DOUBLE_COLON), which
// intentionally deviates from Rust's UpperCamelCase convention to stay 1:1 with
// cangjie_compiler/include/cangjie/Lex/Tokens.inc.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    DOT,                  // "." prec=0
    COMMA,                // "," prec=0
    LPAREN,               // "(" prec=0
    RPAREN,               // ")" prec=0
    LSQUARE,              // "[" prec=0
    RSQUARE,              // "]" prec=0
    LCURL,                // "{" prec=0
    RCURL,                // "}" prec=0
    EXP,                  // "**" prec=16
    MUL,                  // "*" prec=15
    MOD,                  // "%" prec=15
    DIV,                  // "/" prec=15
    ADD,                  // "+" prec=14
    SUB,                  // "-" prec=14
    INCR,                 // "++" prec=0
    DECR,                 // "--" prec=0
    AND,                  // "&&" prec=5
    OR,                   // "||" prec=3
    COALESCING,           // "??" prec=2
    PIPELINE,             // "|>" prec=1
    COMPOSITION,          // "~>" prec=1
    NOT,                  // "!" prec=0
    BITAND,               // "&" prec=8
    BITOR,                // "|" prec=6
    BITXOR,               // "^" prec=7
    BITNOT,               // "~" prec=0
    LSHIFT,               // "<<" prec=13
    RSHIFT,               // ">>" prec=13
    COLON,                // ":" prec=0
    SEMI,                 // ";" prec=0
    ASSIGN,               // "=" prec=0
    ADD_ASSIGN,           // "+=" prec=0
    SUB_ASSIGN,           // "-=" prec=0
    MUL_ASSIGN,           // "*=" prec=0
    EXP_ASSIGN,           // "**=" prec=0
    DIV_ASSIGN,           // "/=" prec=0
    MOD_ASSIGN,           // "%=" prec=0
    AND_ASSIGN,           // "&&=" prec=0
    OR_ASSIGN,            // "||=" prec=0
    BITAND_ASSIGN,        // "&=" prec=0
    BITOR_ASSIGN,         // "|=" prec=0
    BITXOR_ASSIGN,        // "^=" prec=0
    LSHIFT_ASSIGN,        // "<<=" prec=0
    RSHIFT_ASSIGN,        // ">>=" prec=0
    ARROW,                // "->" prec=0
    BACKARROW,            // "<-" prec=0
    DOUBLE_ARROW,         // "=>" prec=0
    RANGEOP,              // ".." prec=11
    CLOSEDRANGEOP,        // "..=" prec=11
    ELLIPSIS,             // "..." prec=0
    HASH,                 // "#" prec=0
    AT,                   // "@" prec=0
    QUEST,                // "?" prec=1
    LT,                   // "<" prec=10
    GT,                   // ">" prec=10
    LE,                   // "<=" prec=10
    GE,                   // ">=" prec=10
    IS,                   // "is" prec=10
    AS,                   // "as" prec=10
    NOTEQ,                // "!=" prec=9
    EQUAL,                // "==" prec=9
    WILDCARD,             // "_" prec=0
    INT8,                 // "Int8" prec=0
    INT16,                // "Int16" prec=0
    INT32,                // "Int32" prec=0
    INT64,                // "Int64" prec=0
    INTNATIVE,            // "IntNative" prec=0
    UINT8,                // "UInt8" prec=0
    UINT16,               // "UInt16" prec=0
    UINT32,               // "UInt32" prec=0
    UINT64,               // "UInt64" prec=0
    UINTNATIVE,           // "UIntNative" prec=0
    FLOAT16,              // "Float16" prec=0
    FLOAT32,              // "Float32" prec=0
    FLOAT64,              // "Float64" prec=0
    RUNE,                 // "Rune" prec=0
    BOOLEAN,              // "Bool" prec=0
    NOTHING,              // "Nothing" prec=0
    UNIT,                 // "Unit" prec=0
    STRUCT,               // "struct" prec=0
    ENUM,                 // "enum" prec=0
    VARRAY,               // "VArray" prec=0
    THISTYPE,             // "This" prec=0
    PACKAGE,              // "package" prec=0
    IMPORT,               // "import" prec=0
    CLASS,                // "class" prec=0
    INTERFACE,            // "interface" prec=0
    FUNC,                 // "func" prec=0
    MACRO,                // "macro" prec=0
    QUOTE,                // "quote" prec=0
    DOLLAR,               // "$" prec=0
    LET,                  // "let" prec=0
    VAR,                  // "var" prec=0
    CONST,                // "const" prec=0
    TYPE,                 // "type" prec=0
    INIT,                 // "init" prec=0
    THIS,                 // "this" prec=0
    SUPER,                // "super" prec=0
    IF,                   // "if" prec=0
    ELSE,                 // "else" prec=0
    CASE,                 // "case" prec=0
    TRY,                  // "try" prec=0
    CATCH,                // "catch" prec=0
    FINALLY,              // "finally" prec=0
    FOR,                  // "for" prec=0
    DO,                   // "do" prec=0
    WHILE,                // "while" prec=0
    THROW,                // "throw" prec=0
    RETURN,               // "return" prec=0
    CONTINUE,             // "continue" prec=0
    BREAK,                // "break" prec=0
    IN,                   // "in" prec=0
    NOT_IN,               // "!in" prec=0
    MATCH,                // "match" prec=0
    WHERE,                // "where" prec=0
    EXTEND,               // "extend" prec=0
    WITH,                 // "with" prec=0
    PROP,                 // "prop" prec=0
    STATIC,               // "static" prec=0
    PUBLIC,               // "public" prec=0
    PRIVATE,              // "private" prec=0
    INTERNAL,             // "internal" prec=0
    PROTECTED,            // "protected" prec=0
    OVERRIDE,             // "override" prec=0
    REDEF,                // "redef" prec=0
    ABSTRACT,             // "abstract" prec=0
    SEALED,               // "sealed" prec=0
    OPEN,                 // "open" prec=0
    FOREIGN,              // "foreign" prec=0
    INOUT,                // "inout" prec=0
    MUT,                  // "mut" prec=0
    UNSAFE,               // "unsafe" prec=0
    OPERATOR,             // "operator" prec=0
    SPAWN,                // "spawn" prec=0
    SYNCHRONIZED,         // "synchronized" prec=0
    UPPERBOUND,           // "<:" prec=0
    MAIN,                 // "main" prec=0
    IDENTIFIER,           // "" prec=0
    PACKAGE_IDENTIFIER,   // "" prec=0
    INTEGER_LITERAL,      // "" prec=0
    RUNE_BYTE_LITERAL,    // "" prec=0
    FLOAT_LITERAL,        // "" prec=0
    COMMENT,              // "" prec=0
    NL,                   // "" prec=0
    END,                  // "" prec=0
    SENTINEL,             // "" prec=0
    RUNE_LITERAL,         // "" prec=0
    STRING_LITERAL,       // "" prec=0
    JSTRING_LITERAL,      // "" prec=0
    MULTILINE_STRING,     // "" prec=0
    MULTILINE_RAW_STRING, // "" prec=0
    BOOL_LITERAL,         // "" prec=0
    UNIT_LITERAL,         // "" prec=0
    DOLLAR_IDENTIFIER,    // "" prec=0
    ANNOTATION,           // "" prec=0
    AT_EXCL,              // "@!" prec=0
    COMMON,               // "common" prec=0
    SPECIFIC,             // "specific" prec=0
    PERFORM,              // "perform" prec=0 [EXPERIMENTAL]
    RESUME,               // "resume" prec=0 [EXPERIMENTAL]
    THROWING,             // "throwing" prec=0 [EXPERIMENTAL]
    HANDLE,               // "handle" prec=0 [EXPERIMENTAL]
    ILLEGAL,              // "" prec=0
    DOUBLE_COLON,         // "::" prec=0
    FEATURES,             // "features" prec=0
}

impl TokenKind {
    /// Human-readable value string, matching the official `VALUE` column.
    pub fn value_str(self) -> &'static str {
        match self {
            TokenKind::DOT => "dot",
            TokenKind::COMMA => "comma",
            TokenKind::LPAREN => "l_paren",
            TokenKind::RPAREN => "r_paren",
            TokenKind::LSQUARE => "l_square",
            TokenKind::RSQUARE => "r_square",
            TokenKind::LCURL => "l_curl",
            TokenKind::RCURL => "r_curl",
            TokenKind::EXP => "exp",
            TokenKind::MUL => "mul",
            TokenKind::MOD => "mod",
            TokenKind::DIV => "div",
            TokenKind::ADD => "add",
            TokenKind::SUB => "sub",
            TokenKind::INCR => "incr",
            TokenKind::DECR => "decr",
            TokenKind::AND => "and",
            TokenKind::OR => "or",
            TokenKind::COALESCING => "coalescing",
            TokenKind::PIPELINE => "pipeline",
            TokenKind::COMPOSITION => "composition",
            TokenKind::NOT => "not",
            TokenKind::BITAND => "bit_and",
            TokenKind::BITOR => "bit_or",
            TokenKind::BITXOR => "bit_xor",
            TokenKind::BITNOT => "bit_not",
            TokenKind::LSHIFT => "lshift",
            TokenKind::RSHIFT => "rshift",
            TokenKind::COLON => "colon",
            TokenKind::SEMI => "semi",
            TokenKind::ASSIGN => "assign",
            TokenKind::ADD_ASSIGN => "add_assign",
            TokenKind::SUB_ASSIGN => "sub_assign",
            TokenKind::MUL_ASSIGN => "mul_assign",
            TokenKind::EXP_ASSIGN => "exp_assign",
            TokenKind::DIV_ASSIGN => "div_assign",
            TokenKind::MOD_ASSIGN => "mod_assign",
            TokenKind::AND_ASSIGN => "and_assign",
            TokenKind::OR_ASSIGN => "or_assign",
            TokenKind::BITAND_ASSIGN => "bit_and_assign",
            TokenKind::BITOR_ASSIGN => "bit_or_assign",
            TokenKind::BITXOR_ASSIGN => "bit_xor_assign",
            TokenKind::LSHIFT_ASSIGN => "lshift_assign",
            TokenKind::RSHIFT_ASSIGN => "rshift_assign",
            TokenKind::ARROW => "arrow",
            TokenKind::BACKARROW => "backarrow",
            TokenKind::DOUBLE_ARROW => "double_arrow",
            TokenKind::RANGEOP => "range_op",
            TokenKind::CLOSEDRANGEOP => "closed_range_op",
            TokenKind::ELLIPSIS => "ellipsis",
            TokenKind::HASH => "hash",
            TokenKind::AT => "at",
            TokenKind::QUEST => "quest",
            TokenKind::LT => "less",
            TokenKind::GT => "greater",
            TokenKind::LE => "less_equal",
            TokenKind::GE => "greater_equal",
            TokenKind::IS => "is",
            TokenKind::AS => "as",
            TokenKind::NOTEQ => "not_equal",
            TokenKind::EQUAL => "equal",
            TokenKind::WILDCARD => "wildcard",
            TokenKind::INT8 => "Int8",
            TokenKind::INT16 => "Int16",
            TokenKind::INT32 => "Int32",
            TokenKind::INT64 => "Int64",
            TokenKind::INTNATIVE => "IntNative",
            TokenKind::UINT8 => "UInt8",
            TokenKind::UINT16 => "UInt16",
            TokenKind::UINT32 => "UInt32",
            TokenKind::UINT64 => "UInt64",
            TokenKind::UINTNATIVE => "UIntNative",
            TokenKind::FLOAT16 => "Float16",
            TokenKind::FLOAT32 => "Float32",
            TokenKind::FLOAT64 => "Float64",
            TokenKind::RUNE => "Rune",
            TokenKind::BOOLEAN => "Bool",
            TokenKind::NOTHING => "Nothing",
            TokenKind::UNIT => "Unit",
            TokenKind::STRUCT => "struct",
            TokenKind::ENUM => "enum",
            TokenKind::VARRAY => "VArray",
            TokenKind::THISTYPE => "This",
            TokenKind::PACKAGE => "package",
            TokenKind::IMPORT => "import",
            TokenKind::CLASS => "class",
            TokenKind::INTERFACE => "interface",
            TokenKind::FUNC => "func",
            TokenKind::MACRO => "macro",
            TokenKind::QUOTE => "quote",
            TokenKind::DOLLAR => "dollar",
            TokenKind::LET => "let",
            TokenKind::VAR => "var",
            TokenKind::CONST => "const",
            TokenKind::TYPE => "type",
            TokenKind::INIT => "init",
            TokenKind::THIS => "this",
            TokenKind::SUPER => "super",
            TokenKind::IF => "if",
            TokenKind::ELSE => "else",
            TokenKind::CASE => "case",
            TokenKind::TRY => "try",
            TokenKind::CATCH => "catch",
            TokenKind::FINALLY => "finally",
            TokenKind::FOR => "for",
            TokenKind::DO => "do",
            TokenKind::WHILE => "while",
            TokenKind::THROW => "throw",
            TokenKind::RETURN => "return",
            TokenKind::CONTINUE => "continue",
            TokenKind::BREAK => "break",
            TokenKind::IN => "in",
            TokenKind::NOT_IN => "not_in",
            TokenKind::MATCH => "match",
            TokenKind::WHERE => "where",
            TokenKind::EXTEND => "extend",
            TokenKind::WITH => "with",
            TokenKind::PROP => "prop",
            TokenKind::STATIC => "static",
            TokenKind::PUBLIC => "public",
            TokenKind::PRIVATE => "private",
            TokenKind::INTERNAL => "internal",
            TokenKind::PROTECTED => "protected",
            TokenKind::OVERRIDE => "override",
            TokenKind::REDEF => "redef",
            TokenKind::ABSTRACT => "abstract",
            TokenKind::SEALED => "sealed",
            TokenKind::OPEN => "open",
            TokenKind::FOREIGN => "foreign",
            TokenKind::INOUT => "inout",
            TokenKind::MUT => "mut",
            TokenKind::UNSAFE => "unsafe",
            TokenKind::OPERATOR => "operator",
            TokenKind::SPAWN => "spawn",
            TokenKind::SYNCHRONIZED => "synchronized",
            TokenKind::UPPERBOUND => "upperbound",
            TokenKind::MAIN => "main",
            TokenKind::IDENTIFIER => "identifier",
            TokenKind::PACKAGE_IDENTIFIER => "package_identifier",
            TokenKind::INTEGER_LITERAL => "integer_literal",
            TokenKind::RUNE_BYTE_LITERAL => "rune_byte_literal",
            TokenKind::FLOAT_LITERAL => "float_literal",
            TokenKind::COMMENT => "comment",
            TokenKind::NL => "newline",
            TokenKind::END => "end",
            TokenKind::SENTINEL => "sentinel",
            TokenKind::RUNE_LITERAL => "char_literal",
            TokenKind::STRING_LITERAL => "string_literal",
            TokenKind::JSTRING_LITERAL => "jstring_literal",
            TokenKind::MULTILINE_STRING => "multiline_string",
            TokenKind::MULTILINE_RAW_STRING => "multiline_raw_string",
            TokenKind::BOOL_LITERAL => "bool_literal",
            TokenKind::UNIT_LITERAL => "unit_literal",
            TokenKind::DOLLAR_IDENTIFIER => "dollar_identifier",
            TokenKind::ANNOTATION => "annotation",
            TokenKind::AT_EXCL => "at_exclamation",
            TokenKind::COMMON => "common",
            TokenKind::SPECIFIC => "specific",
            TokenKind::PERFORM => "perform",
            TokenKind::RESUME => "resume",
            TokenKind::THROWING => "throwing",
            TokenKind::HANDLE => "handle",
            TokenKind::ILLEGAL => "illegal",
            TokenKind::DOUBLE_COLON => "double_colon",
            TokenKind::FEATURES => "features",
        }
    }

    /// Literal text ("" for non-literal tokens).
    pub fn literal(self) -> &'static str {
        match self {
            TokenKind::DOT => ".",
            TokenKind::COMMA => ",",
            TokenKind::LPAREN => "(",
            TokenKind::RPAREN => ")",
            TokenKind::LSQUARE => "[",
            TokenKind::RSQUARE => "]",
            TokenKind::LCURL => "{",
            TokenKind::RCURL => "}",
            TokenKind::EXP => "**",
            TokenKind::MUL => "*",
            TokenKind::MOD => "%",
            TokenKind::DIV => "/",
            TokenKind::ADD => "+",
            TokenKind::SUB => "-",
            TokenKind::INCR => "++",
            TokenKind::DECR => "--",
            TokenKind::AND => "&&",
            TokenKind::OR => "||",
            TokenKind::COALESCING => "??",
            TokenKind::PIPELINE => "|>",
            TokenKind::COMPOSITION => "~>",
            TokenKind::NOT => "!",
            TokenKind::BITAND => "&",
            TokenKind::BITOR => "|",
            TokenKind::BITXOR => "^",
            TokenKind::BITNOT => "~",
            TokenKind::LSHIFT => "<<",
            TokenKind::RSHIFT => ">>",
            TokenKind::COLON => ":",
            TokenKind::SEMI => ";",
            TokenKind::ASSIGN => "=",
            TokenKind::ADD_ASSIGN => "+=",
            TokenKind::SUB_ASSIGN => "-=",
            TokenKind::MUL_ASSIGN => "*=",
            TokenKind::EXP_ASSIGN => "**=",
            TokenKind::DIV_ASSIGN => "/=",
            TokenKind::MOD_ASSIGN => "%=",
            TokenKind::AND_ASSIGN => "&&=",
            TokenKind::OR_ASSIGN => "||=",
            TokenKind::BITAND_ASSIGN => "&=",
            TokenKind::BITOR_ASSIGN => "|=",
            TokenKind::BITXOR_ASSIGN => "^=",
            TokenKind::LSHIFT_ASSIGN => "<<=",
            TokenKind::RSHIFT_ASSIGN => ">>=",
            TokenKind::ARROW => "->",
            TokenKind::BACKARROW => "<-",
            TokenKind::DOUBLE_ARROW => "=>",
            TokenKind::RANGEOP => "..",
            TokenKind::CLOSEDRANGEOP => "..=",
            TokenKind::ELLIPSIS => "...",
            TokenKind::HASH => "#",
            TokenKind::AT => "@",
            TokenKind::QUEST => "?",
            TokenKind::LT => "<",
            TokenKind::GT => ">",
            TokenKind::LE => "<=",
            TokenKind::GE => ">=",
            TokenKind::IS => "is",
            TokenKind::AS => "as",
            TokenKind::NOTEQ => "!=",
            TokenKind::EQUAL => "==",
            TokenKind::WILDCARD => "_",
            TokenKind::INT8 => "Int8",
            TokenKind::INT16 => "Int16",
            TokenKind::INT32 => "Int32",
            TokenKind::INT64 => "Int64",
            TokenKind::INTNATIVE => "IntNative",
            TokenKind::UINT8 => "UInt8",
            TokenKind::UINT16 => "UInt16",
            TokenKind::UINT32 => "UInt32",
            TokenKind::UINT64 => "UInt64",
            TokenKind::UINTNATIVE => "UIntNative",
            TokenKind::FLOAT16 => "Float16",
            TokenKind::FLOAT32 => "Float32",
            TokenKind::FLOAT64 => "Float64",
            TokenKind::RUNE => "Rune",
            TokenKind::BOOLEAN => "Bool",
            TokenKind::NOTHING => "Nothing",
            TokenKind::UNIT => "Unit",
            TokenKind::STRUCT => "struct",
            TokenKind::ENUM => "enum",
            TokenKind::VARRAY => "VArray",
            TokenKind::THISTYPE => "This",
            TokenKind::PACKAGE => "package",
            TokenKind::IMPORT => "import",
            TokenKind::CLASS => "class",
            TokenKind::INTERFACE => "interface",
            TokenKind::FUNC => "func",
            TokenKind::MACRO => "macro",
            TokenKind::QUOTE => "quote",
            TokenKind::DOLLAR => "$",
            TokenKind::LET => "let",
            TokenKind::VAR => "var",
            TokenKind::CONST => "const",
            TokenKind::TYPE => "type",
            TokenKind::INIT => "init",
            TokenKind::THIS => "this",
            TokenKind::SUPER => "super",
            TokenKind::IF => "if",
            TokenKind::ELSE => "else",
            TokenKind::CASE => "case",
            TokenKind::TRY => "try",
            TokenKind::CATCH => "catch",
            TokenKind::FINALLY => "finally",
            TokenKind::FOR => "for",
            TokenKind::DO => "do",
            TokenKind::WHILE => "while",
            TokenKind::THROW => "throw",
            TokenKind::RETURN => "return",
            TokenKind::CONTINUE => "continue",
            TokenKind::BREAK => "break",
            TokenKind::IN => "in",
            TokenKind::NOT_IN => "!in",
            TokenKind::MATCH => "match",
            TokenKind::WHERE => "where",
            TokenKind::EXTEND => "extend",
            TokenKind::WITH => "with",
            TokenKind::PROP => "prop",
            TokenKind::STATIC => "static",
            TokenKind::PUBLIC => "public",
            TokenKind::PRIVATE => "private",
            TokenKind::INTERNAL => "internal",
            TokenKind::PROTECTED => "protected",
            TokenKind::OVERRIDE => "override",
            TokenKind::REDEF => "redef",
            TokenKind::ABSTRACT => "abstract",
            TokenKind::SEALED => "sealed",
            TokenKind::OPEN => "open",
            TokenKind::FOREIGN => "foreign",
            TokenKind::INOUT => "inout",
            TokenKind::MUT => "mut",
            TokenKind::UNSAFE => "unsafe",
            TokenKind::OPERATOR => "operator",
            TokenKind::SPAWN => "spawn",
            TokenKind::SYNCHRONIZED => "synchronized",
            TokenKind::UPPERBOUND => "<:",
            TokenKind::MAIN => "main",
            TokenKind::IDENTIFIER => "",
            TokenKind::PACKAGE_IDENTIFIER => "",
            TokenKind::INTEGER_LITERAL => "",
            TokenKind::RUNE_BYTE_LITERAL => "",
            TokenKind::FLOAT_LITERAL => "",
            TokenKind::COMMENT => "",
            TokenKind::NL => "",
            TokenKind::END => "",
            TokenKind::SENTINEL => "",
            TokenKind::RUNE_LITERAL => "",
            TokenKind::STRING_LITERAL => "",
            TokenKind::JSTRING_LITERAL => "",
            TokenKind::MULTILINE_STRING => "",
            TokenKind::MULTILINE_RAW_STRING => "",
            TokenKind::BOOL_LITERAL => "",
            TokenKind::UNIT_LITERAL => "",
            TokenKind::DOLLAR_IDENTIFIER => "",
            TokenKind::ANNOTATION => "",
            TokenKind::AT_EXCL => "@!",
            TokenKind::COMMON => "common",
            TokenKind::SPECIFIC => "specific",
            TokenKind::PERFORM => "perform",
            TokenKind::RESUME => "resume",
            TokenKind::THROWING => "throwing",
            TokenKind::HANDLE => "handle",
            TokenKind::ILLEGAL => "",
            TokenKind::DOUBLE_COLON => "::",
            TokenKind::FEATURES => "features",
        }
    }

    /// Operator precedence (0 = not an operator).
    pub fn precedence(self) -> u8 {
        match self {
            TokenKind::DOT => 0,
            TokenKind::COMMA => 0,
            TokenKind::LPAREN => 0,
            TokenKind::RPAREN => 0,
            TokenKind::LSQUARE => 0,
            TokenKind::RSQUARE => 0,
            TokenKind::LCURL => 0,
            TokenKind::RCURL => 0,
            TokenKind::EXP => 16,
            TokenKind::MUL => 15,
            TokenKind::MOD => 15,
            TokenKind::DIV => 15,
            TokenKind::ADD => 14,
            TokenKind::SUB => 14,
            TokenKind::INCR => 0,
            TokenKind::DECR => 0,
            TokenKind::AND => 5,
            TokenKind::OR => 3,
            TokenKind::COALESCING => 2,
            TokenKind::PIPELINE => 1,
            TokenKind::COMPOSITION => 1,
            TokenKind::NOT => 0,
            TokenKind::BITAND => 8,
            TokenKind::BITOR => 6,
            TokenKind::BITXOR => 7,
            TokenKind::BITNOT => 0,
            TokenKind::LSHIFT => 13,
            TokenKind::RSHIFT => 13,
            TokenKind::COLON => 0,
            TokenKind::SEMI => 0,
            TokenKind::ASSIGN => 0,
            TokenKind::ADD_ASSIGN => 0,
            TokenKind::SUB_ASSIGN => 0,
            TokenKind::MUL_ASSIGN => 0,
            TokenKind::EXP_ASSIGN => 0,
            TokenKind::DIV_ASSIGN => 0,
            TokenKind::MOD_ASSIGN => 0,
            TokenKind::AND_ASSIGN => 0,
            TokenKind::OR_ASSIGN => 0,
            TokenKind::BITAND_ASSIGN => 0,
            TokenKind::BITOR_ASSIGN => 0,
            TokenKind::BITXOR_ASSIGN => 0,
            TokenKind::LSHIFT_ASSIGN => 0,
            TokenKind::RSHIFT_ASSIGN => 0,
            TokenKind::ARROW => 0,
            TokenKind::BACKARROW => 0,
            TokenKind::DOUBLE_ARROW => 0,
            TokenKind::RANGEOP => 11,
            TokenKind::CLOSEDRANGEOP => 11,
            TokenKind::ELLIPSIS => 0,
            TokenKind::HASH => 0,
            TokenKind::AT => 0,
            TokenKind::QUEST => 1,
            TokenKind::LT => 10,
            TokenKind::GT => 10,
            TokenKind::LE => 10,
            TokenKind::GE => 10,
            TokenKind::IS => 10,
            TokenKind::AS => 10,
            TokenKind::NOTEQ => 9,
            TokenKind::EQUAL => 9,
            TokenKind::WILDCARD => 0,
            TokenKind::INT8 => 0,
            TokenKind::INT16 => 0,
            TokenKind::INT32 => 0,
            TokenKind::INT64 => 0,
            TokenKind::INTNATIVE => 0,
            TokenKind::UINT8 => 0,
            TokenKind::UINT16 => 0,
            TokenKind::UINT32 => 0,
            TokenKind::UINT64 => 0,
            TokenKind::UINTNATIVE => 0,
            TokenKind::FLOAT16 => 0,
            TokenKind::FLOAT32 => 0,
            TokenKind::FLOAT64 => 0,
            TokenKind::RUNE => 0,
            TokenKind::BOOLEAN => 0,
            TokenKind::NOTHING => 0,
            TokenKind::UNIT => 0,
            TokenKind::STRUCT => 0,
            TokenKind::ENUM => 0,
            TokenKind::VARRAY => 0,
            TokenKind::THISTYPE => 0,
            TokenKind::PACKAGE => 0,
            TokenKind::IMPORT => 0,
            TokenKind::CLASS => 0,
            TokenKind::INTERFACE => 0,
            TokenKind::FUNC => 0,
            TokenKind::MACRO => 0,
            TokenKind::QUOTE => 0,
            TokenKind::DOLLAR => 0,
            TokenKind::LET => 0,
            TokenKind::VAR => 0,
            TokenKind::CONST => 0,
            TokenKind::TYPE => 0,
            TokenKind::INIT => 0,
            TokenKind::THIS => 0,
            TokenKind::SUPER => 0,
            TokenKind::IF => 0,
            TokenKind::ELSE => 0,
            TokenKind::CASE => 0,
            TokenKind::TRY => 0,
            TokenKind::CATCH => 0,
            TokenKind::FINALLY => 0,
            TokenKind::FOR => 0,
            TokenKind::DO => 0,
            TokenKind::WHILE => 0,
            TokenKind::THROW => 0,
            TokenKind::RETURN => 0,
            TokenKind::CONTINUE => 0,
            TokenKind::BREAK => 0,
            TokenKind::IN => 0,
            TokenKind::NOT_IN => 0,
            TokenKind::MATCH => 0,
            TokenKind::WHERE => 0,
            TokenKind::EXTEND => 0,
            TokenKind::WITH => 0,
            TokenKind::PROP => 0,
            TokenKind::STATIC => 0,
            TokenKind::PUBLIC => 0,
            TokenKind::PRIVATE => 0,
            TokenKind::INTERNAL => 0,
            TokenKind::PROTECTED => 0,
            TokenKind::OVERRIDE => 0,
            TokenKind::REDEF => 0,
            TokenKind::ABSTRACT => 0,
            TokenKind::SEALED => 0,
            TokenKind::OPEN => 0,
            TokenKind::FOREIGN => 0,
            TokenKind::INOUT => 0,
            TokenKind::MUT => 0,
            TokenKind::UNSAFE => 0,
            TokenKind::OPERATOR => 0,
            TokenKind::SPAWN => 0,
            TokenKind::SYNCHRONIZED => 0,
            TokenKind::UPPERBOUND => 0,
            TokenKind::MAIN => 0,
            TokenKind::IDENTIFIER => 0,
            TokenKind::PACKAGE_IDENTIFIER => 0,
            TokenKind::INTEGER_LITERAL => 0,
            TokenKind::RUNE_BYTE_LITERAL => 0,
            TokenKind::FLOAT_LITERAL => 0,
            TokenKind::COMMENT => 0,
            TokenKind::NL => 0,
            TokenKind::END => 0,
            TokenKind::SENTINEL => 0,
            TokenKind::RUNE_LITERAL => 0,
            TokenKind::STRING_LITERAL => 0,
            TokenKind::JSTRING_LITERAL => 0,
            TokenKind::MULTILINE_STRING => 0,
            TokenKind::MULTILINE_RAW_STRING => 0,
            TokenKind::BOOL_LITERAL => 0,
            TokenKind::UNIT_LITERAL => 0,
            TokenKind::DOLLAR_IDENTIFIER => 0,
            TokenKind::ANNOTATION => 0,
            TokenKind::AT_EXCL => 0,
            TokenKind::COMMON => 0,
            TokenKind::SPECIFIC => 0,
            TokenKind::PERFORM => 0,
            TokenKind::RESUME => 0,
            TokenKind::THROWING => 0,
            TokenKind::HANDLE => 0,
            TokenKind::ILLEGAL => 0,
            TokenKind::DOUBLE_COLON => 0,
            TokenKind::FEATURES => 0,
        }
    }
}

/// Static keyword identifier -> TokenKind lookup (empty literal tokens excluded).
pub fn lookup_keyword(s: &str) -> Option<TokenKind> {
    // true/false are lexed as BOOL_LITERAL tokens (official Lexer.cpp LookupKeyword map).
    if s == "true" || s == "false" {
        return Some(TokenKind::BOOL_LITERAL);
    }
    Some(match s {
        "is" => TokenKind::IS,
        "as" => TokenKind::AS,
        "_" => TokenKind::WILDCARD,
        "Int8" => TokenKind::INT8,
        "Int16" => TokenKind::INT16,
        "Int32" => TokenKind::INT32,
        "Int64" => TokenKind::INT64,
        "IntNative" => TokenKind::INTNATIVE,
        "UInt8" => TokenKind::UINT8,
        "UInt16" => TokenKind::UINT16,
        "UInt32" => TokenKind::UINT32,
        "UInt64" => TokenKind::UINT64,
        "UIntNative" => TokenKind::UINTNATIVE,
        "Float16" => TokenKind::FLOAT16,
        "Float32" => TokenKind::FLOAT32,
        "Float64" => TokenKind::FLOAT64,
        "Rune" => TokenKind::RUNE,
        "Bool" => TokenKind::BOOLEAN,
        "Nothing" => TokenKind::NOTHING,
        "Unit" => TokenKind::UNIT,
        "struct" => TokenKind::STRUCT,
        "enum" => TokenKind::ENUM,
        "VArray" => TokenKind::VARRAY,
        "This" => TokenKind::THISTYPE,
        "package" => TokenKind::PACKAGE,
        "import" => TokenKind::IMPORT,
        "class" => TokenKind::CLASS,
        "interface" => TokenKind::INTERFACE,
        "func" => TokenKind::FUNC,
        "macro" => TokenKind::MACRO,
        "quote" => TokenKind::QUOTE,
        "let" => TokenKind::LET,
        "var" => TokenKind::VAR,
        "const" => TokenKind::CONST,
        "type" => TokenKind::TYPE,
        "init" => TokenKind::INIT,
        "this" => TokenKind::THIS,
        "super" => TokenKind::SUPER,
        "if" => TokenKind::IF,
        "else" => TokenKind::ELSE,
        "case" => TokenKind::CASE,
        "try" => TokenKind::TRY,
        "catch" => TokenKind::CATCH,
        "finally" => TokenKind::FINALLY,
        "for" => TokenKind::FOR,
        "do" => TokenKind::DO,
        "while" => TokenKind::WHILE,
        "throw" => TokenKind::THROW,
        "return" => TokenKind::RETURN,
        "continue" => TokenKind::CONTINUE,
        "break" => TokenKind::BREAK,
        "in" => TokenKind::IN,
        "match" => TokenKind::MATCH,
        "where" => TokenKind::WHERE,
        "extend" => TokenKind::EXTEND,
        "with" => TokenKind::WITH,
        "prop" => TokenKind::PROP,
        "static" => TokenKind::STATIC,
        "public" => TokenKind::PUBLIC,
        "private" => TokenKind::PRIVATE,
        "internal" => TokenKind::INTERNAL,
        "protected" => TokenKind::PROTECTED,
        "override" => TokenKind::OVERRIDE,
        "redef" => TokenKind::REDEF,
        "abstract" => TokenKind::ABSTRACT,
        "sealed" => TokenKind::SEALED,
        "open" => TokenKind::OPEN,
        "foreign" => TokenKind::FOREIGN,
        "inout" => TokenKind::INOUT,
        "mut" => TokenKind::MUT,
        "unsafe" => TokenKind::UNSAFE,
        "operator" => TokenKind::OPERATOR,
        "spawn" => TokenKind::SPAWN,
        "synchronized" => TokenKind::SYNCHRONIZED,
        "main" => TokenKind::MAIN,
        "common" => TokenKind::COMMON,
        "specific" => TokenKind::SPECIFIC,
        "perform" => TokenKind::PERFORM,
        "resume" => TokenKind::RESUME,
        "throwing" => TokenKind::THROWING,
        "handle" => TokenKind::HANDLE,
        "features" => TokenKind::FEATURES,
        _ => return None,
    })
}

use std::fmt;
impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value_str())
    }
}

impl TokenKind {
    /// True for identifier-like tokens (IDENTIFIER or backquoted identifier).
    /// Keywords that can double as identifiers in Cangjie (is/as/main/etc.)
    /// are intentionally NOT included — the lexer already classifies them.
    pub fn is_identifier_like(self) -> bool {
        matches!(self, TokenKind::IDENTIFIER)
    }

    /// True for operator-overload names usable after `operator func`
    /// (e.g. `+`, `==`, `[]`, `()`).
    pub fn operator_like(self) -> bool {
        matches!(
            self,
            TokenKind::ADD
                | TokenKind::SUB
                | TokenKind::MUL
                | TokenKind::DIV
                | TokenKind::MOD
                | TokenKind::EXP
                | TokenKind::EQUAL
                | TokenKind::NOTEQ
                | TokenKind::LT
                | TokenKind::GT
                | TokenKind::LE
                | TokenKind::GE
                | TokenKind::AND
                | TokenKind::OR
                | TokenKind::BITAND
                | TokenKind::BITOR
                | TokenKind::BITXOR
                | TokenKind::LSHIFT
                | TokenKind::RSHIFT
                | TokenKind::LSQUARE
                | TokenKind::RSQUARE
                | TokenKind::LPAREN
                | TokenKind::RPAREN
                | TokenKind::ASSIGN
                | TokenKind::ADD_ASSIGN
                | TokenKind::SUB_ASSIGN
                | TokenKind::MUL_ASSIGN
                | TokenKind::DIV_ASSIGN
                | TokenKind::MOD_ASSIGN
                | TokenKind::EXP_ASSIGN
                | TokenKind::BITAND_ASSIGN
                | TokenKind::BITOR_ASSIGN
                | TokenKind::BITXOR_ASSIGN
                | TokenKind::LSHIFT_ASSIGN
                | TokenKind::RSHIFT_ASSIGN
                | TokenKind::INCR
                | TokenKind::DECR
        )
    }

    /// True for tokens that can be used as a NAME anywhere (identifier or
    /// keyword-as-name, matching official `ParseIdentifierFromToken`).
    /// Excludes pure punctuation/operators/literals.
    pub fn is_name_like(self) -> bool {
        matches!(
            self,
            TokenKind::IDENTIFIER
                | TokenKind::MAIN
                | TokenKind::IS
                | TokenKind::AS
                | TokenKind::IN
                | TokenKind::NOT_IN
                | TokenKind::MATCH
                | TokenKind::WHERE
                | TokenKind::EXTEND
                | TokenKind::WITH
                | TokenKind::PROP
                | TokenKind::STATIC
                | TokenKind::PUBLIC
                | TokenKind::PRIVATE
                | TokenKind::INTERNAL
                | TokenKind::PROTECTED
                | TokenKind::OVERRIDE
                | TokenKind::REDEF
                | TokenKind::ABSTRACT
                | TokenKind::SEALED
                | TokenKind::OPEN
                | TokenKind::FOREIGN
                | TokenKind::INOUT
                | TokenKind::MUT
                | TokenKind::UNSAFE
                | TokenKind::OPERATOR
                | TokenKind::SPAWN
                | TokenKind::SYNCHRONIZED
                | TokenKind::UPPERBOUND
                | TokenKind::COMMON
                | TokenKind::SPECIFIC
                | TokenKind::FEATURES
        )
    }
}
