// cj-frontend: Cangjie frontend CLI (cjc-frontend compatible)
//
// M1: supports `cj-frontend <file.cj>` to lex a file and print the token stream.
// Pipeline (parse/sema/dump-ast) lands in M2+.
use std::env;
use std::fs;
use std::process::ExitCode;

use cj_lexer::{lex_all, Lexer};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: cj-frontend <file.cj>");
        return ExitCode::from(2);
    }

    let path = &args[1];
    // Read as raw bytes: Cangjie sources may contain intentionally invalid UTF-8
    // (lexer must diagnose them, not crash — see LLT Lexer/Unicode/illegal.cj).
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let (src, had_invalid) = lossy_utf8(&bytes);
    if had_invalid {
        eprintln!(
            "// note: source contains invalid UTF-8 bytes ({} replaced)",
            src.len()
        );
    }

    // Full token stream including NL/comments/END (what a real pipeline consumes).
    let mut lexer = Lexer::new(&src);
    let tokens = lexer.tokenize();
    for t in tokens.iter().filter(|t| {
        t.kind != cj_lexer::TokenKind::COMMENT
            && t.kind != cj_lexer::TokenKind::NL
            && t.kind != cj_lexer::TokenKind::END
    }) {
        if t.text.is_empty() {
            println!("{}:{}:{}", t.begin.line, t.begin.column, t.kind);
        } else {
            println!(
                "{}:{}:{} {:?}",
                t.begin.line, t.begin.column, t.kind, t.text
            );
        }
    }

    for e in &lexer.errors {
        eprintln!("error: {} at {}:{}", e.message, e.pos.line, e.pos.column);
    }

    // Sanity: lexing should not fail on any input; count tokens.
    eprintln!("// {} tokens (excluding trivia)", lex_all(&src).len());

    ExitCode::SUCCESS
}

/// Decode bytes lossily (invalid UTF-8 -> U+FFFD), returning whether any bytes
/// were replaced. Cangjie sources may intentionally contain invalid UTF-8 for
/// lexer diagnostics; we must lex them rather than fail.
fn lossy_utf8(bytes: &[u8]) -> (String, bool) {
    let was_valid = std::str::from_utf8(bytes).is_ok();
    (String::from_utf8_lossy(bytes).into_owned(), !was_valid)
}
