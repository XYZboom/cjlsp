// cj-frontend: Cangjie frontend CLI (cjc-frontend compatible)
//
// Supports:
//   cj-frontend <file.cj>              — lex + parse, print diagnostics (default)
//   cj-frontend --dump-parse <file>    — token dump (official -frontend --dump-parse)
//   cj-frontend --dump-ast <file>      — AST dump
use std::env;
use std::fs;
use std::process::ExitCode;

use cj_lexer::Lexer;
use cj_parser::Parser;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let mut mode = "parse";
    let mut path: Option<&str> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--dump-parse" | "--dump-tokens" => {
                mode = "tokens";
                i += 1;
            }
            "--dump-ast" => {
                mode = "ast";
                i += 1;
            }
            "-frontend" => {
                i += 1;
            }
            s if s.starts_with('-') && s != "-" => {
                // ignore unknown flags for now (e.g. -o, --diagnostic-format)
                i += 1;
            }
            s => {
                path = Some(s);
                i += 1;
            }
        }
    }

    let Some(path) = path else {
        eprintln!("usage: cj-frontend [--dump-parse|--dump-ast] <file.cj>");
        return ExitCode::from(2);
    };

    // Read as raw bytes (sources may contain invalid UTF-8 for diagnostics).
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let src = String::from_utf8_lossy(&bytes).into_owned();

    match mode {
        "tokens" => dump_tokens(&src, path),
        "ast" => dump_ast(&src),
        _ => parse_and_report(&src, path),
    }
}

/// `--dump-parse`: print every token as `<file:line:col> value kind`
/// (official DumpTokens format).
fn dump_tokens(src: &str, path: &str) -> ExitCode {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize();
    for t in tokens {
        if t.kind == cj_lexer::TokenKind::END {
            break;
        }
        let position = format!("<{path}:{}:{}>", t.begin.line, t.begin.column);
        if t.kind == cj_lexer::TokenKind::COMMENT {
            println!("{} {}", t.kind.value_str(), position);
        } else if t.kind == cj_lexer::TokenKind::NL {
            // official: Println(tok == "\n" ? "\\n" : "\\r\\n", KIND, position)
            let value = if t.text == "\n" { "\\n" } else { "\\r\\n" };
            println!("{value} {} {}", t.kind.value_str(), position);
        } else {
            println!("{} {} {}", t.text, t.kind.value_str(), position);
        }
    }
    ExitCode::SUCCESS
}

/// `--dump-ast`: parse and print the AST (M2c refines the exact format).
fn dump_ast(src: &str) -> ExitCode {
    let mut parser = Parser::new(src, Lexer::new(src).tokenize());
    let file = parser.run();
    print_file(&file, 0);
    for d in &parser.diags {
        eprintln!("error: {} at {}:{}", d.message, d.line, d.col);
    }
    ExitCode::SUCCESS
}

/// Default: parse + sema and report diagnostics (SCAN-format text).
fn parse_and_report(src: &str, path: &str) -> ExitCode {
    let mut parser = Parser::new(src, Lexer::new(src).tokenize());
    let file = parser.run();
    // semantic analysis (symbol collection / redefinition detection)
    let collector = cj_sema::Collector::new();
    let sema_result = collector.collect_file(&file);
    let source_lines: Vec<String> = src.lines().map(String::from).collect();
    let fmt = cj_diag::TextFormatter {
        file_name: path,
        source_lines: &source_lines,
    };
    let mut errors = 0;
    let mut warnings = 0;
    for d in parser.diags.iter().chain(sema_result.diags.iter()) {
        match d.severity {
            cj_diag::Severity::Warning => warnings += 1,
            _ => errors += 1,
        }
        eprint!("{}", fmt.render(d));
    }
    if errors > 0 {
        eprint!("{}", cj_diag::render_summary(errors, warnings));
    } else if parser.diags.is_empty() && sema_result.diags.is_empty() {
        eprintln!("// parse OK");
    } else {
        eprintln!("// warnings only");
    }
    ExitCode::SUCCESS
}

fn print_file(file: &cj_ast::File, _indent: usize) {
    if let Some(pkg) = &file.package {
        println!("package {pkg}");
    }
    for imp in &file.imports {
        println!("import {}", imp.path.join("."));
    }
    for d in &file.decls {
        println!("{d:?}");
    }
}
