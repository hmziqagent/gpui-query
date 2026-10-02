use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gpui_query::core::{RequestId, RequestSequencer};

const WINDOW: usize = 1024;

fn bench_request_id_mint(c: &mut Criterion) {
    let mut sequencer = RequestSequencer::new();
    c.bench_function("request_sequencer_mint_loop", |b| {
        b.iter(|| {
            let mut checksum: u64 = 0;
            for _ in 0..WINDOW {
                checksum ^= black_box(&mut sequencer).next_request().value();
            }
            black_box(checksum)
        })
    });
}

fn bench_request_id_accept_equality(c: &mut Criterion) {
    let mut sequencer = RequestSequencer::new();
    let current = sequencer.next_request();
    let incoming: Vec<RequestId> = (0..WINDOW).map(|_| sequencer.next_request()).collect();
    c.bench_function("request_id_accept_equality_loop", |b| {
        b.iter(|| {
            let mut matches: usize = 0;
            for id in &incoming {
                if black_box(*id) == black_box(current) {
                    matches += 1;
                }
            }
            black_box(matches)
        })
    });
}

criterion_group!(
    benches,
    bench_request_id_mint,
    bench_request_id_accept_equality
);
criterion_main!(benches);
