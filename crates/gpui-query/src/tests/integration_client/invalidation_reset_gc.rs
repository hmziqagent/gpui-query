use gpui::{BorrowAppContext as _, TestAppContext};

use crate::client::QueryClient;
use crate::core::*;
use crate::tests::test_support::*;

fn create_success_at_time(
    client: &mut QueryClient,
    cx: &mut gpui::App,
    key: &str,
    data: &str,
    success_time_ms: u64,
) {
    let entity = client.resource_with_policies::<String, QueryError>(
        QueryKey::from(key),
        CachePolicy::Ttl { ttl_ms: 60_000 },
        RequestPolicy::LatestWins,
        cx,
    );
    entity.update(cx, |r, _| {
        r.apply_success(data.to_string(), success_time_ms)
    });
}

#[gpui::test]
fn test_invalidate_queries_exact_filter(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key1 = QueryKey::from(["users", "1"]);
            let key2 = QueryKey::from(["users", "2"]);

            let e1 = client.resource::<String, QueryError>(key1.clone(), cx);
            let e2 = client.resource::<String, QueryError>(key2.clone(), cx);

            e1.update(cx, |r, _| r.apply_success("Alice".to_string(), 1_000));
            e2.update(cx, |r, _| r.apply_success("Bob".to_string(), 1_000));

            assert!(
                e1.read(cx).is_cache_fresh(1_500),
                "should be fresh before invalidate"
            );
            assert!(e2.read(cx).is_cache_fresh(1_500));

            client.invalidate_queries(&QueryKeyFilter::Exact(&key1), cx);

            assert!(
                !e1.read(cx).is_cache_fresh(1_500),
                "e1 should be stale after exact invalidate"
            );
            assert!(
                e1.read(cx).data().is_some(),
                "data should survive invalidation"
            );
            assert!(e2.read(cx).is_cache_fresh(1_500), "e2 should remain fresh");
        });
    });
}

#[gpui::test]
fn test_invalidate_queries_prefix_filter_across_types(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let u1 = client.resource::<String, QueryError>(QueryKey::from(["users", "1"]), cx);
            let u2 = client.resource::<String, QueryError>(QueryKey::from(["users", "2"]), cx);
            let p1 = client.resource::<String, QueryError>(QueryKey::from(["posts", "1"]), cx);

            u1.update(cx, |r, _| r.apply_success("user1".to_string(), 1_000));
            u2.update(cx, |r, _| r.apply_success("user2".to_string(), 1_000));
            p1.update(cx, |r, _| r.apply_success("post1".to_string(), 1_000));

            let prefix = QueryKey::from(["users"]);
            client.invalidate_queries(&QueryKeyFilter::Prefix(&prefix), cx);

            assert!(
                !u1.read(cx).is_cache_fresh(1_500),
                "users/1 should be stale"
            );
            assert!(
                !u2.read(cx).is_cache_fresh(1_500),
                "users/2 should be stale"
            );
            assert!(
                p1.read(cx).is_cache_fresh(1_500),
                "posts/1 should still be fresh"
            );
        });
    });
}

#[gpui::test]
fn test_invalidate_queries_all_filter(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let u = client.resource::<String, QueryError>("user", cx);
            let p = client.resource::<String, QueryError>("post", cx);

            u.update(cx, |r, _| r.apply_success("u".to_string(), 1_000));
            p.update(cx, |r, _| r.apply_success("p".to_string(), 1_000));

            client.invalidate_queries(&QueryKeyFilter::All, cx);

            assert!(!u.read(cx).is_cache_fresh(1_500));
            assert!(!p.read(cx).is_cache_fresh(1_500));
        });
    });
}

#[gpui::test]
fn test_reset_queries_clears_data_and_status(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let e1 = client.resource::<String, QueryError>("key1", cx);
            let e2 = client.resource::<String, QueryError>("key2", cx);

            e1.update(cx, |r, _| r.apply_success("data1".to_string(), 1_000));
            e2.update(cx, |r, _| r.apply_success("data2".to_string(), 1_000));

            assert!(e1.read(cx).data().is_some());
            assert!(e2.read(cx).data().is_some());

            client.reset_queries(&QueryKeyFilter::All, cx);

            assert!(
                e1.read(cx).data().is_none(),
                "data should be cleared after reset"
            );
            assert_eq!(e1.read(cx).status(), QueryStatus::Idle);
            assert!(e2.read(cx).data().is_none());
            assert_eq!(e2.read(cx).status(), QueryStatus::Idle);
        });
    });
}

#[gpui::test]
fn test_reset_queries_prefix_preserves_non_matching(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let u1 = client.resource::<String, QueryError>(QueryKey::from(["users", "1"]), cx);
            let p1 = client.resource::<String, QueryError>(QueryKey::from(["posts", "1"]), cx);

            u1.update(cx, |r, _| r.apply_success("user".to_string(), 1_000));
            p1.update(cx, |r, _| r.apply_success("post".to_string(), 1_000));

            let prefix = QueryKey::from(["users"]);
            client.reset_queries(&QueryKeyFilter::Prefix(&prefix), cx);

            assert!(u1.read(cx).data().is_none(), "users/1 should be reset");
            assert_eq!(u1.read(cx).status(), QueryStatus::Idle);
            assert!(p1.read(cx).data().is_some(), "posts/1 should keep its data");
        });
    });
}

#[gpui::test]
fn test_gc_evicts_idle_resources_with_no_snapshot(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _entity = client.resource::<String, QueryError>("idle_key", cx);
            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 1_500, cx);

            let queries = client.all_queries::<String, QueryError>();
            assert!(
                queries.is_empty(),
                "idle resource with no snapshot should be evicted once older than gc_time"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_live_never_fetched_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc/live_never_fetched");
            let e1 = client.resource::<String, QueryError>(key.clone(), cx);

            client.gc(cx);

            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "GC must keep the bucket entry of a live never-fetched resource"
            );
            let e2 = client.resource::<String, QueryError>(key, cx);
            assert_eq!(e1, e2, "resource() after GC must return the same entity");
        });
    });
}

#[gpui::test]
fn test_gc_preserves_set_query_data_primed_entry(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc/primed");
            client.set_query_data::<String, QueryError>(key.clone(), "v".to_string(), cx);

            client.gc(cx);

            assert_eq!(
                client
                    .get_query_data::<String, QueryError>(&key, cx)
                    .as_deref(),
                Some("v"),
                "set_query_data-primed data must survive GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_completed_entry(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc/completed");
            let prepared = client
                .prepare_fetch_query::<String, QueryError>(key.clone(), cx)
                .expect("prepare_fetch_query should start");
            prepared.complete_success("v".to_string(), cx);

            client.gc(cx);

            assert_eq!(
                client
                    .get_query_data::<String, QueryError>(&key, cx)
                    .as_deref(),
                Some("v"),
                "completed entry must survive GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_evicts_failure_resources_after_gc_time(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("fail_key");

            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| {
                r.apply_failure(QueryError::response("broken"), 1_000)
            });

            client.gc_with_time(2_500, cx);

            assert!(
                client.query::<String, QueryError>(&key).is_none(),
                "failure resource should be evicted when age (1500ms) exceeds gc_time (1000ms)"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_failure_resources_before_gc_time(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("fail_key_early");

            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| {
                r.apply_failure(QueryError::response("broken"), 2_000)
            });

            client.gc_with_time(2_500, cx);

            let entity = client
                .query::<String, QueryError>(&key)
                .expect("failure resource should survive when age (500ms) < gc_time (1000ms)");
            assert_eq!(
                entity.read(cx).status(),
                QueryStatus::Failure,
                "entity should still be in Failure state"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_loading_resources_regardless_of_age(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("loading_key");

            let prepared = client
                .prepare_fetch_query::<String, QueryError>(key.clone(), cx)
                .expect("should start");

            client.gc_with_time(1_000_000, cx);

            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "loading resource must survive GC regardless of age"
            );

            prepared.complete_success("data".to_string(), cx);

            let entity = client
                .query::<String, QueryError>(&key)
                .expect("entity should still exist after completion");
            assert_eq!(
                entity.read(cx).status(),
                QueryStatus::Success,
                "entity should be Success after completing the fetch"
            );
            assert_eq!(
                entity.read(cx).data().unwrap(),
                "data",
                "data should match what was completed"
            );
        });
    });
}

#[gpui::test]
fn test_gc_evicts_success_resources_after_success_threshold(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("success_old");

            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| r.apply_success("data".to_string(), 1_000));

            client.gc_with_time(3_500, cx);

            assert!(
                client.query::<String, QueryError>(&key).is_none(),
                "success resource should be evicted when age (2500ms) exceeds success_threshold (2000ms)"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_success_resources_within_success_threshold(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("success_fresh");

            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| r.apply_success("data".to_string(), 2_000));

            client.gc_with_time(3_500, cx);

            let entity = client.query::<String, QueryError>(&key).expect(
                "success resource should survive when age (1500ms) < success_threshold (2000ms)",
            );
            assert_eq!(
                entity.read(cx).data().unwrap(),
                "data",
                "data should be intact after GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_across_multiple_type_buckets(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _s = client.resource::<String, QueryError>("s", cx);
            let _n = client.resource::<u32, QueryError>("n", cx);

            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);
            assert_eq!(client.all_queries::<u32, QueryError>().len(), 1);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 3_000, cx);

            assert!(
                client.all_queries::<String, QueryError>().is_empty(),
                "idle String resource evicted"
            );
            assert!(
                client.all_queries::<u32, QueryError>().is_empty(),
                "idle u32 resource evicted"
            );
        });
    });
}

#[gpui::test]
fn test_gc_mixed_states_precise_eviction(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let prepared = client
                .prepare_fetch_query::<String, QueryError>("loading", cx)
                .expect("should start");

            create_success_at_time(client, cx, "success_fresh", "data", 1_000);

            create_success_at_time(client, cx, "success_old", "data", 0);

            assert_eq!(client.all_queries::<String, QueryError>().len(), 3);

            client.gc_with_time(2_500, cx);

            let remaining = client.all_queries::<String, QueryError>();
            assert_eq!(
                remaining.len(),
                2,
                "exactly 1 of 3 resources should be evicted"
            );

            let remaining_keys: Vec<String> = remaining
                .iter()
                .map(|e| e.read(cx).key().to_path())
                .collect();
            assert!(
                remaining_keys.contains(&"loading".to_string()),
                "loading should survive: {:?}",
                remaining_keys
            );
            assert!(
                remaining_keys.contains(&"success_fresh".to_string()),
                "success_fresh should survive: {:?}",
                remaining_keys
            );

            prepared.complete_success("data".to_string(), cx);
        });
    });
}

#[gpui::test]
fn test_gc_boundary_success_threshold_exact(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("boundary");
            create_success_at_time(client, cx, "boundary", "data", 1_000);

            client.gc_with_time(3_000, cx);
            assert!(
                client.query::<String, QueryError>(&key).is_none(),
                "age=2000ms == success_threshold=2000ms => must be evicted (>= boundary)"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_invalidated_entry_with_fresh_baseline(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc/invalidated_baseline");
            let held = client.resource::<String, QueryError>(key.clone(), cx);
            held.update(cx, |r, _| r.apply_success("v".to_string(), 1_000));

            let type_id = std::any::TypeId::of::<(String, QueryError)>();
            let bucket = client.buckets.get_mut(&type_id).unwrap();
            let typed = bucket
                .as_any_mut()
                .downcast_mut::<crate::client::QueryBucket<String, QueryError>>()
                .unwrap();
            typed.inner.entries.get_mut(&key).unwrap().updated_at = 1_000;

            client.invalidate_queries(&QueryKeyFilter::Exact(&key), cx);

            client.gc_with_time(3_000, cx);

            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "invalidated entry held by a live component must survive GC on a fresh baseline"
            );
            let again = client.resource::<String, QueryError>(key, cx);
            assert_eq!(
                again, held,
                "resource() must not mint a second entity after GC"
            );
        });
    });
}

#[gpui::test]
fn test_gc_preserves_reset_entry_with_fresh_baseline(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc/reset_baseline");
            let held = client.resource::<String, QueryError>(key.clone(), cx);
            held.update(cx, |r, _| r.apply_success("v".to_string(), 1_000));

            let type_id = std::any::TypeId::of::<(String, QueryError)>();
            let bucket = client.buckets.get_mut(&type_id).unwrap();
            let typed = bucket
                .as_any_mut()
                .downcast_mut::<crate::client::QueryBucket<String, QueryError>>()
                .unwrap();
            typed.inner.entries.get_mut(&key).unwrap().updated_at = 1_000;

            client.reset_queries(&QueryKeyFilter::Exact(&key), cx);

            client.gc_with_time(3_000, cx);

            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "reset entry held by a live component must survive GC on a fresh baseline"
            );
            assert_eq!(held.read(cx).status(), QueryStatus::Idle);
        });
    });
}
