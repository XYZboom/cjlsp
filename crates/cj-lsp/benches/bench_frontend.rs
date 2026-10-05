//! Frontend performance benchmarks.
//!
//! Run the complete release-mode diagnostics benchmark with:
//! `cargo bench -p cj-lsp --bench bench_frontend -- pipeline/full_diagnostics`

use criterion::{black_box, criterion_group, criterion_main, Criterion};

const NORMAL_SOURCE: &str = include_str!("../../../benchmarks/corpus/large_valid.cj");
const DENSE_SOURCE: &str = include_str!("../../../benchmarks/corpus/diagnostic_dense.cj");

fn bench_lexer(c: &mut Criterion) {
    let mut group = c.benchmark_group("lexer");
    group.sample_size(20);
    group.bench_function("tokenize_large_valid", |b| {
        b.iter(|| {
            let mut lexer = cj_lexer::Lexer::new(black_box(NORMAL_SOURCE));
            black_box(lexer.tokenize())
        })
    });
    group.finish();
}

fn bench_parser(c: &mut Criterion) {
    let mut lexer = cj_lexer::Lexer::new(NORMAL_SOURCE);
    let tokens = lexer.tokenize();
    let mut group = c.benchmark_group("parser");
    group.sample_size(20);
    group.bench_function("parse_large_valid", |b| {
        b.iter(|| {
            let mut parser = cj_parser::Parser::new(black_box(NORMAL_SOURCE), tokens.clone());
            black_box(parser.parse_file())
        })
    });
    group.finish();
}

/// Lexer + parser + every source-local sema pass + SCAN text formatting.
fn bench_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");
    group.sample_size(20);
    group.bench_function("full_diagnostics_large_valid", |b| {
        b.iter(|| {
            black_box(cj_frontend::analyze_source(
                black_box(NORMAL_SOURCE),
                "large_valid.cj",
            ))
        })
    });
    group.bench_function("full_diagnostics_dense", |b| {
        b.iter(|| {
            black_box(cj_frontend::analyze_source(
                black_box(DENSE_SOURCE),
                "diagnostic_dense.cj",
            ))
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
