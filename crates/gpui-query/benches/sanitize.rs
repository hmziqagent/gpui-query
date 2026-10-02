use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gpui_query::core::QueryError;

const KIB: usize = 1024;

fn no_pattern_fill(size: usize) -> String {
    let unit = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor ";
    let mut out = unit.repeat(size / unit.len() + 1);
    out.truncate(size);
    out
}

fn paths_and_schemes_fill(size: usize) -> String {
    let unit = "postgres://db.internal:5432/app /home/alice/report.pdf mysql://db2.internal:3306/shop /etc/config.toml redis://cache:6379/0 /var/log/app.log mongodb://cluster.internal:27017/grid /users/bob/notes.txt ";
    let mut out = unit.repeat(size / unit.len() + 1);
    out.truncate(size);
    out
}

fn token_fill(size: usize) -> String {
    let unit =
        "login token=abc123 rejected; session token expired; bearer xyz failed for token refresh ";
    let mut out = unit.repeat(size / unit.len() + 1);
    out.truncate(size);
    out
}

fn bench_sanitize(c: &mut Criterion) {
    let small = QueryError::unknown(no_pattern_fill(4 * KIB));
    let large = QueryError::unknown(no_pattern_fill(64 * KIB));
    let dense = QueryError::unknown(paths_and_schemes_fill(32 * KIB));
    let tokened = QueryError::unknown(token_fill(4 * KIB));

    c.bench_function("sanitize_no_pattern_4kib", |b| {
        b.iter(|| black_box(&small).sanitized())
    });
    c.bench_function("sanitize_no_pattern_64kib", |b| {
        b.iter(|| black_box(&large).sanitized())
    });
    c.bench_function("sanitize_paths_and_schemes_32kib", |b| {
        b.iter(|| black_box(&dense).sanitized())
    });
    c.bench_function("sanitize_token_4kib", |b| {
        b.iter(|| black_box(&tokened).sanitized())
    });
}

criterion_group!(benches, bench_sanitize);
criterion_main!(benches);
