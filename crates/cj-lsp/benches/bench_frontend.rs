//! Frontend performance benchmarks (T27): lexer / parser / LSP diagnostics.
//!
//! Run: `cargo bench -p cj-lsp`  (criterion; writes tools/bench_baseline.txt via
//! the reporter harness if run from the repo root).

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::path::PathBuf;

// ─── Sample sources ──────────────────────────────────────────────────────────

/// A moderately large real-world Cangjie file (from the LLT suite).
fn real_world_source() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/samples/large.cj");
    match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(_) => synthesized_source(), // fall back if samples missing
    }
}

/// A synthesized Cangjie file with many declarations / expressions.
fn synthesized_source() -> String {
    let mut s = String::from("package bench\n\n");
    for i in 0..200 {
        s.push_str(&format!(
            "public class C{i}<: Any {{\n    let x: Int64 = {i}\n    let y: Float64 = {i}.5\n    func add(a: Int64, b: Int64): Int64 {{\n        let t = a + b\n        if t > 1 {{ return t }} else {{ return {i} }}\n    }}\n}}\n\n"
        ));
    }
    for i in 0..100 {
        s.push_str(&format!(
            "public func f{i}(a: Int64, b: String): String {{\n    let arr = [1, 2, 3, {i}]\n    let m = match a {{ 0 => \"zero\", _ => \"other\" }}\n    return b + m\n}}\n\n"
        ));
    }
    s
}

// ─── Benchmarks ──────────────────────────────────────────────────────────────

fn bench_lexer(c: &mut Criterion) {
    let src = real_world_source();
    let mut group = c.benchmark_group("lexer");
    group.sample_size(20);
    group.bench_function("tokenize_real", |b| {
        b.iter(|| {
            let mut lexer = cj_lexer::Lexer::new(black_box(&src));
            black_box(lexer.tokenize())
        })
    });
    group.finish();
}

fn bench_parser(c: &mut Criterion) {
    let src = real_world_source();
    let mut group = c.benchmark_group("parser");
    group.sample_size(20);
    // Pre-tokenize once (lexer is bench_lexer's concern) to isolate parser cost.
    let mut lexer = cj_lexer::Lexer::new(&src);
    let tokens = lexer.tokenize();
    group.bench_function("parse_real", |b| {
        b.iter(|| {
            let mut p = cj_parser::Parser::new(black_box(&src), tokens.clone());
            black_box(p.parse_file())
        })
    });
    group.bench_function("parse_synthesized", |b| {
        let s = synthesized_source();
        let mut lx = cj_lexer::Lexer::new(&s);
        let toks = lx.tokenize();
        b.iter(|| {
            let mut p = cj_parser::Parser::new(black_box(&s), toks.clone());
            black_box(p.parse_file())
        })
    });
    group.finish();
}

/// Full frontend pipeline: tokenize + parse + sema collector (what the LSP
/// runs per didOpen). Uses the same entry the benchmark contract cares about.
fn bench_pipeline(c: &mut Criterion) {
    let src = real_world_source();
    let mut group = c.benchmark_group("pipeline");
    group.sample_size(20);
    group.bench_function("tokenize_parse", |b| {
        b.iter(|| {
            let mut lexer = cj_lexer::Lexer::new(black_box(&src));
            let tokens = lexer.tokenize();
            let mut p = cj_parser::Parser::new(&src, tokens);
            black_box(p.parse_file())
        })
    });
    group.finish();
}

criterion_group! {
    name = frontend;
    config = Criterion::default().warm_up_time(std::time::Duration::from_secs(1));
    targets = bench_lexer, bench_parser, bench_pipeline
}
criterion_main!(frontend);
