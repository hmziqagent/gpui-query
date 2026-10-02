use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gpui_query::client::{PersistSnapshot, PersistedEntry, Persister};
use gpui_query::core::CachePolicy;
use gpui_query_persist::FilePersister;

const ENTRY_COUNT: usize = 1000;

fn thousand_entry_snapshot() -> PersistSnapshot {
    let mut snapshot = PersistSnapshot::new();
    for i in 0..ENTRY_COUNT {
        snapshot.entries.insert(
            format!("tenant::acme::todos::{i}"),
            PersistedEntry {
                value: serde_json::json!({ "id": i, "title": "task item" }),
                cached_at: 1_700_000_000_000,
                cache_policy: CachePolicy::Ttl { ttl_ms: 60_000 },
                meta: None,
            },
        );
    }
    snapshot
}

fn bench_file_persister_round_trip(c: &mut Criterion) {
    let snapshot = thousand_entry_snapshot();
    let dir = tempfile::tempdir().expect("temp dir for bench");
    let persister = FilePersister::json(dir.path().join("gpui-query-bench-cache.json"));
    c.bench_function("file_persister_json_round_trip_1000_entries", |b| {
        b.iter(|| {
            pollster::block_on(persister.save(black_box(&snapshot))).expect("save snapshot");
            let loaded = pollster::block_on(persister.load()).expect("load snapshot");
            std::fs::remove_file(persister.path()).expect("cleanup cache file");
            black_box(loaded.entries.len())
        })
    });
}

criterion_group!(benches, bench_file_persister_round_trip);
criterion_main!(benches);
