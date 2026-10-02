use std::collections::HashMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gpui_query::core::QueryKey;

const SNAPSHOT_KEYS: usize = 1000;

fn bench_to_path(c: &mut Criterion) {
    let key = QueryKey::from(["users", "42", "posts", "comments"]);
    c.bench_function("query_key_to_path_4_segments", |b| {
        b.iter(|| black_box(&key).to_path())
    });
}

fn bench_snapshot_keyed_by_to_path(c: &mut Criterion) {
    let keys: Vec<QueryKey> = (0..SNAPSHOT_KEYS)
        .map(|i| {
            let id = i.to_string();
            QueryKey::from(["tenant", "acme", "todos", id.as_str()])
        })
        .collect();
    c.bench_function("snapshot_1000_keys_keyed_by_to_path", |b| {
        b.iter(|| {
            let snapshot: HashMap<String, QueryKey> = keys
                .iter()
                .map(|key| (key.to_path(), key.clone()))
                .collect();
            black_box(snapshot.len())
        })
    });
}

criterion_group!(benches, bench_to_path, bench_snapshot_keyed_by_to_path);
criterion_main!(benches);
