use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use gpui_query::core::{CachePolicy, RetryPolicy};

const MAX_ATTEMPT: u32 = 64;

fn bench_retry_backoff(c: &mut Criterion) {
    let exponential = RetryPolicy::new(3).with_exponential_backoff();
    let flat = RetryPolicy::new(3);
    c.bench_function("retry_policy_backoff_loop", |b| {
        b.iter(|| {
            let mut total: u64 = 0;
            for attempt in 0..MAX_ATTEMPT {
                total += black_box(&exponential).delay_for_attempt(attempt);
                total += black_box(&flat).delay_for_attempt(attempt);
            }
            black_box(total)
        })
    });
}

fn bench_cache_policy_staleness(c: &mut Criterion) {
    let policies = [
        CachePolicy::NoCache,
        CachePolicy::Ttl { ttl_ms: 60_000 },
        CachePolicy::StaleWhileRevalidate {
            ttl_ms: 60_000,
            stale_ms: 300_000,
        },
    ];
    let ages = [0, 10_000, 90_000, 500_000, 3_600_000];
    c.bench_function("cache_policy_staleness_loop", |b| {
        b.iter(|| {
            let mut fresh_count: u32 = 0;
            for policy in policies {
                for &age in &ages {
                    let age = black_box(age);
                    if policy.is_fresh(age) {
                        fresh_count += 1;
                    }
                    if policy.is_stale_but_serveable(age) {
                        fresh_count += 1;
                    }
                    if policy.is_expired(age) {
                        fresh_count += 1;
                    }
                }
            }
            black_box(fresh_count)
        })
    });
}

criterion_group!(benches, bench_retry_backoff, bench_cache_policy_staleness);
criterion_main!(benches);
