use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use crate::client::{QueryBucket, QueryClient};
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_gc_preserves_swr_resources_within_stale_window(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 500);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("swr_gc");
            let swr = CachePolicy::StaleWhileRevalidate {
                ttl_ms: 1_000,
                stale_ms: 5_000,
            };
            let entity = client.resource_with_policies::<String, QueryError>(
                key.clone(),
                swr,
                RequestPolicy::LatestWins,
                cx,
            );
            entity.update(cx, |r, _| r.apply_success("data".to_string(), 1_000));

            client.gc_with_time(3_000, cx);
            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "SWR resource within stale window must survive GC"
            );

            client.gc_with_time(8_000, cx);
            assert!(
                client.query::<String, QueryError>(&key).is_none(),
                "SWR resource past total valid window should be evicted"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_swr_resources_within_ttl(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 5_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("swr_fresh");
            let swr = CachePolicy::StaleWhileRevalidate {
                ttl_ms: 5_000,
                stale_ms: 3_000,
            };
            let entity = client.resource_with_policies::<String, QueryError>(
                key.clone(),
                swr,
                RequestPolicy::LatestWins,
                cx,
            );
            entity.update(cx, |r, _| r.apply_success("fresh".to_string(), 1_000));

            client.gc_with_time(3_000, cx);
            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "SWR resource within TTL must survive GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_success_mutation(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("vars".to_string());
                m.complete_success("done".to_string());
            });
            assert!(entity.read(cx).is_success());

            client.gc_with_time(1_000_000, cx);

            let mutations = client.all_mutations::<String, String, QueryError>();
            assert!(
                !mutations.is_empty(),
                "Success mutation should survive GC — only Idle/Failure are evictable"
            );
        });
    });
}

#[gpui::test]
fn test_gc_evicts_idle_infinite_query_with_realistic_timing(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("inf_gc_idle");
            let _entity = client.infinite_resource::<String, QueryError>(key.clone(), cx);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 100_000, cx);

            assert!(
                client.infinite_query::<String, QueryError>(&key).is_none(),
                "idle infinite query should be evicted by GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_loading_infinite_query(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("inf_gc_loading");
            let entity = client.infinite_resource::<String, QueryError>(key.clone(), cx);

            let _rid = client
                .next_request_id_for_infinite_key::<String, QueryError>(&key)
                .expect("request id");
            let mut seq = RequestSequencer::new();
            entity.update(cx, |r, _| {
                r.begin_fetch_next(&mut seq, 1_000);
            });
            assert!(entity.read(cx).status().is_loading());

            client.gc_with_time(1_000_000, cx);

            assert!(
                client.infinite_query::<String, QueryError>(&key).is_some(),
                "loading infinite query must survive GC regardless of age"
            );
        });
    });
}

#[gpui::test]
fn test_gc_evicts_aged_successful_infinite_query(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("inf_gc_success");
            let entity = client.infinite_resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| {
                let mut seq = RequestSequencer::new();
                let id = r.begin_fetch_next(&mut seq, 1_000).expect("begin fetch");
                r.complete_page_success(id, "page0".to_string(), false, true, 1_000);
            });
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);

            client.gc_with_time(3_500, cx);
            assert!(
                client.infinite_query::<String, QueryError>(&key).is_none(),
                "successful infinite query aged past success_threshold must be evicted (#1/#133)"
            );
        });
    });
}

#[gpui::test]
fn test_bucket_default_max_entries_allows_many_resources(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            for i in 0..100 {
                let key = format!("max_{}", i);
                let _entity = client.resource::<String, QueryError>(key, cx);
            }

            let all = client.all_queries::<String, QueryError>();
            assert_eq!(
                all.len(),
                100,
                "all 100 resources should exist within default max_entries(10_000)"
            );
        });
    });
}

#[gpui::test]
fn test_loading_mutation_survives_gc(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("vars".to_string());
            });
            assert!(entity.read(cx).is_loading());

            client.gc_with_time(1_000_000, cx);

            let mutations = client.all_mutations::<String, String, QueryError>();
            assert_eq!(
                mutations.len(),
                1,
                "loading mutation must survive GC regardless of age"
            );
        });
    });
}

#[gpui::test]
fn test_idle_mutation_is_evicted_by_gc_after_age_exceeds_threshold(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&entity, cx);

            assert!(
                entity.read(cx).is_idle(),
                "mutation should be idle before any operation"
            );
            assert_eq!(
                client.all_mutations::<String, String, QueryError>().len(),
                1,
                "mutation should exist before GC"
            );

            let far_future = crate::client::current_time_ms() + 3_600_000;
            client.gc_with_time(far_future, cx);

            assert_eq!(
                client.all_mutations::<String, String, QueryError>().len(),
                0,
                "idle mutation should be evicted when age exceeds gc_threshold"
            );
        });
    });
}

#[gpui::test]
fn test_observer_status_dedup_default_config_is_status_change_only(_cx: &mut TestAppContext) {
    let config = crate::client::ObserverConfig::default();
    assert!(
        config.notify_on_status_change_only,
        "default ObserverConfig should notify on status change only"
    );

    let always_notify = crate::client::ObserverConfig {
        notify_on_status_change_only: false,
    };
    assert!(
        !always_notify.notify_on_status_change_only,
        "explicit always-notify config should be false"
    );
}

#[gpui::test]
fn test_mutation_bucket_evict_oldest_keeps_count_bounded(cx: &mut TestAppContext) {
    const MAX_ENTRIES: usize = 10_000;

    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let mut live: Vec<gpui::Entity<MutationResource<String, String, QueryError>>> =
                Vec::with_capacity(MAX_ENTRIES + 2);

            for _ in 0..(MAX_ENTRIES + 2) {
                let entity = cx.new(|_| {
                    MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
                });
                client.register_mutation::<String, String, QueryError>(&entity, cx);
                live.push(entity);
            }

            let mutations = client.all_mutations::<String, String, QueryError>();
            assert_eq!(
                mutations.len(),
                MAX_ENTRIES,
                "MutationBucket entry count must stay bounded at DEFAULT_MAX_ENTRIES \
                 ({}); evict_oldest should have triggered on every insert past the cap",
                MAX_ENTRIES
            );

            let diag = client.diagnostics(cx);
            assert_eq!(
                diag.mutation_count, MAX_ENTRIES,
                "diagnostics.mutation_count must match the bounded bucket size"
            );

            drop(live);
        });
    });
}

fn create_evict_entry(
    bucket: &mut QueryBucket<String, QueryError>,
    key: &str,
    cx: &mut gpui::App,
) -> gpui::Entity<QueryResource<String, QueryError>> {
    bucket.get_or_create(
        QueryKey::from(key),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::LatestWins,
        cx,
    )
}

fn stamp_and_refresh(
    bucket: &mut QueryBucket<String, QueryError>,
    entity: &gpui::Entity<QueryResource<String, QueryError>>,
    key: &str,
    updated_at_ms: u64,
    cx: &mut gpui::App,
) {
    entity.update(cx, |r, _| r.apply_success(key.to_string(), updated_at_ms));
    bucket.get_or_create(
        QueryKey::from(key),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::LatestWins,
        cx,
    );
}

#[gpui::test]
fn test_evict_oldest_removes_oldest_live_entry_at_capacity(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let mut bucket = QueryBucket::<String, QueryError>::new();
        bucket.inner.max_entries = 3;

        let a = create_evict_entry(&mut bucket, "evict_a", cx);
        stamp_and_refresh(&mut bucket, &a, "evict_a", 1_000, cx);
        let b = create_evict_entry(&mut bucket, "evict_b", cx);
        stamp_and_refresh(&mut bucket, &b, "evict_b", 2_000, cx);
        let c = create_evict_entry(&mut bucket, "evict_c", cx);
        stamp_and_refresh(&mut bucket, &c, "evict_c", 3_000, cx);

        let _d = create_evict_entry(&mut bucket, "evict_d", cx);

        assert!(
            !bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_a")),
            "oldest live entry should be evicted at capacity"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_b"))
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_c"))
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_d"))
        );
    });
}

#[gpui::test]
fn test_evict_oldest_prefers_dead_entry_over_live_at_capacity(cx: &mut TestAppContext) {
    let mut bucket = QueryBucket::<String, QueryError>::new();
    bucket.inner.max_entries = 2;

    let live = cx.update(|cx| {
        let dead = create_evict_entry(&mut bucket, "evict_dead", cx);
        stamp_and_refresh(&mut bucket, &dead, "evict_dead", 1_000, cx);
        drop(dead);

        let live = create_evict_entry(&mut bucket, "evict_live", cx);
        stamp_and_refresh(&mut bucket, &live, "evict_live", 2_000, cx);
        live
    });

    cx.update(|cx| {
        create_evict_entry(&mut bucket, "evict_new", cx);

        assert!(
            !bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_dead")),
            "collected entry should be evicted before any live entry"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_live")),
            "older live entry should be kept while a collected entry can go"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_new"))
        );
    });
    drop(live);
}

#[gpui::test]
fn test_evict_oldest_with_only_dead_entries_keeps_bucket_bounded(cx: &mut TestAppContext) {
    let mut bucket = QueryBucket::<String, QueryError>::new();
    bucket.inner.max_entries = 3;

    cx.update(|cx| {
        for i in 0..3 {
            create_evict_entry(&mut bucket, &format!("evict_dead_{i}"), cx);
        }
        assert_eq!(bucket.inner.entries.len(), 3);
    });

    cx.update(|cx| {
        create_evict_entry(&mut bucket, "evict_live_key", cx);

        assert_eq!(
            bucket.inner.entries.len(),
            3,
            "a bucket of collected entries must still evict on insert instead of \
             growing past max_entries"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_live_key"))
        );
    });
}

#[gpui::test]
fn test_evict_oldest_prefers_dead_entry_with_newest_mirror_at_capacity(cx: &mut TestAppContext) {
    let mut bucket = QueryBucket::<String, QueryError>::new();
    bucket.inner.max_entries = 2;

    let live = cx.update(|cx| {
        let live = create_evict_entry(&mut bucket, "evict_live_older", cx);
        stamp_and_refresh(&mut bucket, &live, "evict_live_older", 1_000, cx);
        live
    });

    cx.update(|cx| {
        let dead = create_evict_entry(&mut bucket, "evict_dead_newest", cx);
        stamp_and_refresh(&mut bucket, &dead, "evict_dead_newest", 2_000, cx);
        drop(dead);
    });

    cx.update(|cx| {
        create_evict_entry(&mut bucket, "evict_after_dead", cx);

        assert!(
            !bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_dead_newest")),
            "collected entry must be evicted first even when its mirror age is \
             the newest in the bucket"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_live_older")),
            "older live entry must survive while a collected entry can go"
        );
        assert!(
            bucket
                .inner
                .entries
                .contains_key(&QueryKey::from("evict_after_dead"))
        );
    });
    drop(live);
}

#[gpui::test]
fn test_gc_drops_dead_mutation_with_stale_loading_mirror(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&entity, cx);

            entity.update(cx, |m, _| m.begin("vars".to_string()));
            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 1_000_000, cx);

            assert_eq!(
                client.diagnostics(cx).mutation_count,
                1,
                "loading mutation must survive GC while its entity is alive"
            );
            drop(entity);
        });
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 1_000_000, cx);

            assert_eq!(
                client.diagnostics(cx).mutation_count,
                0,
                "a dead mutation with a stale loading mirror must be dropped by GC"
            );
        });
    });
}
