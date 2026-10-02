use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use crate::client::{InfiniteQueryObserver, QueryClient};
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_gc_with_zero_time_clamped_evicts_idle(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 0);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _entity = client.resource::<String, QueryError>("gc_zero", cx);
            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 2_000, cx);
            assert_eq!(
                client.all_queries::<String, QueryError>().len(),
                0,
                "Idle resource with no snapshot timestamp should be evicted once \
                 its age (2000ms) exceeds the clamped gc_threshold (1000ms)"
            );
        });
    });
}

#[gpui::test]
fn test_gc_with_time_explicit_time_value(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _e1 = client.resource::<String, QueryError>("gc_evict", cx);
            let _e2 = client.resource::<String, QueryError>("gc_evict2", cx);
            assert_eq!(client.all_queries::<String, QueryError>().len(), 2);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 500, cx);
            assert_eq!(
                client.all_queries::<String, QueryError>().len(),
                2,
                "never-fetched resources should survive gc_with_time(now+500): \
                 their age (500ms) is below the clamped gc_threshold (1000ms)"
            );

            client.gc_with_time(now + 100_000, cx);
            assert_eq!(
                client.all_queries::<String, QueryError>().len(),
                0,
                "Idle resources should be evicted at gc_with_time(now+100_000)"
            );

            let _e3 = client.resource::<String, QueryError>("gc_big_time", cx);
            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);

            client.gc_with_time(now + 100_000, cx);
            assert_eq!(
                client.all_queries::<String, QueryError>().len(),
                0,
                "Idle resource should also be evicted at gc_with_time(now+100_000)"
            );

            let diag = client.diagnostics(cx);
            assert_eq!(
                diag.query_count, 0,
                "diagnostics should report 0 queries after all were evicted"
            );
        });
    });
}

#[gpui::test]
fn test_gc_with_time_before_entry_baseline_keeps_never_fetched_resource(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("gc_past_now");
            let _e = client.resource::<String, QueryError>(key.clone(), cx);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now.saturating_sub(60_000), cx);

            assert!(
                client.query::<String, QueryError>(&key).is_some(),
                "a gc time older than the entry baseline must not evict a never-fetched \
                 resource: age saturates to 0, below the gc_threshold"
            );
        });
    });
}

#[gpui::test]
fn test_gc_runs_across_all_bucket_types(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 1_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _q = client.resource::<String, QueryError>("q_gc", cx);

            let _iq = client.infinite_resource::<String, QueryError>("iq_gc", cx);

            let loading_mut = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });
            loading_mut.update(cx, |m, _| m.begin("vars".to_string()));
            client.register_mutation::<String, User, QueryError>(&loading_mut, cx);

            let idle_mut = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, User, QueryError>(&idle_mut, cx);

            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);
            assert_eq!(client.all_infinite_queries::<String, QueryError>().len(), 1);
            assert_eq!(client.all_mutations::<String, User, QueryError>().len(), 2);

            let now = crate::client::current_time_ms();
            client.gc_with_time(now + 100_000, cx);

            assert!(
                client.all_queries::<String, QueryError>().is_empty(),
                "idle query with no snapshot should be evicted by GC"
            );
            assert!(
                client
                    .all_infinite_queries::<String, QueryError>()
                    .is_empty(),
                "idle infinite query with no snapshot should be evicted by GC"
            );

            let mutations = client.all_mutations::<String, User, QueryError>();
            assert_eq!(
                mutations.len(),
                1,
                "only the loading mutation should survive GC; the idle one aged past gc_threshold"
            );
        });
    });
}

#[gpui::test]
fn test_remove_queries_affects_infinite_queries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _iq = client
                .infinite_resource::<String, QueryError>(QueryKey::from(["users", "inf"]), cx);
            let _q = client.resource::<String, QueryError>(QueryKey::from(["users", "reg"]), cx);

            assert_eq!(client.all_infinite_queries::<String, QueryError>().len(), 1);
            assert_eq!(client.all_queries::<String, QueryError>().len(), 1);

            let prefix = QueryKey::from(["users"]);
            client.remove_queries(&QueryKeyFilter::Prefix(&prefix));

            assert!(
                client
                    .all_infinite_queries::<String, QueryError>()
                    .is_empty(),
                "infinite query should be removed"
            );
            assert!(
                client.all_queries::<String, QueryError>().is_empty(),
                "regular query should be removed"
            );
        });
    });
}

#[gpui::test]
fn test_invalidate_queries_affects_infinite_queries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _iq = client.infinite_resource::<String, QueryError>("inf_inv", cx);
            client.invalidate_queries(&QueryKeyFilter::All, cx);

            let retrieved = client.infinite_query::<String, QueryError>(&QueryKey::from("inf_inv"));
            assert!(
                retrieved.is_some(),
                "infinite query should still exist after invalidate"
            );
        });
    });
}

#[gpui::test]
fn test_reset_queries_affects_infinite_queries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _iq = client.infinite_resource::<String, QueryError>("inf_reset", cx);
            client.reset_queries(&QueryKeyFilter::All, cx);

            let retrieved =
                client.infinite_query::<String, QueryError>(&QueryKey::from("inf_reset"));
            assert!(
                retrieved.is_some(),
                "infinite query should exist after reset"
            );
            assert_eq!(retrieved.unwrap().read(cx).status(), QueryStatus::Idle);
        });
    });
}

#[gpui::test]
fn test_infinite_query_observer_creation_and_observe(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = client.infinite_resource::<String, QueryError>("inf_obs", cx);
            let observer = InfiniteQueryObserver::new(&entity);

            struct DummyView;
            let view = cx.new(|_| DummyView);
            let sub = view.update(cx, |_view, cx| observer.observe(cx));
            assert!(sub.is_some(), "observe should return Some(Subscription)");
        });
    });
}

#[gpui::test]
fn test_current_time_ms_is_reasonable(_cx: &mut TestAppContext) {
    let now = crate::client::current_time_ms();
    assert!(
        now > 1_700_000_000_000,
        "current_time_ms should be post-2023"
    );
    assert!(
        now < 4_000_000_000_000,
        "current_time_ms should be pre-2128"
    );
}

#[gpui::test]
fn test_multiple_resources_same_type_share_bucket(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            for i in 0..10 {
                let key = format!("multi_{i}");
                let _e = client.resource::<String, QueryError>(key, cx);
            }

            let all = client.all_queries::<String, QueryError>();
            assert_eq!(all.len(), 10, "should have 10 resources in the same bucket");

            let diag = client.diagnostics(cx);
            assert_eq!(diag.query_count, 10);
            assert_eq!(diag.queries.len(), 10);
        });
    });
}

#[gpui::test]
fn test_invalidate_then_refetch_lifecycle(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("inv_refetch");

            let p1 = client
                .prepare_fetch_query::<String, QueryError>(key.clone(), cx)
                .expect("first fetch");
            p1.complete_success("v1".to_string(), cx);

            assert_eq!(
                client.get_query_data::<String, QueryError>(&key, cx),
                Some("v1".to_string())
            );

            client.invalidate_queries(&QueryKeyFilter::Exact(&key), cx);

            let p2 = client.prepare_fetch_query::<String, QueryError>(key.clone(), cx);
            assert!(
                p2.is_some(),
                "prepare_fetch_query must return Some after invalidation — \
                 Force mode always starts a new request regardless of cache state"
            );

            p2.unwrap().complete_success("v2".to_string(), cx);
            assert_eq!(
                client.get_query_data::<String, QueryError>(&key, cx),
                Some("v2".to_string())
            );
        });
    });
}

#[gpui::test]
fn test_reset_then_set_query_data(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("reset_set");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| r.apply_success("original".to_string(), 1_000));

            client.reset_queries(&QueryKeyFilter::Exact(&key), cx);
            assert!(entity.read(cx).data().is_none());
            assert_eq!(entity.read(cx).status(), QueryStatus::Idle);

            client.set_query_data::<String, QueryError>(key, "restored".to_string(), cx);
            assert_eq!(entity.read(cx).data().unwrap(), "restored");
        });
    });
}

#[gpui::test]
fn test_multi_segment_key_in_client_operations(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from(["org", "team", "user", "42"]);
            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            entity.update(cx, |r, _| {
                r.apply_success("deep_key_data".to_string(), 1_000)
            });

            let found = client.query::<String, QueryError>(&key);
            assert!(found.is_some());

            let prefix = QueryKey::from(["org", "team"]);
            client.invalidate_queries(&QueryKeyFilter::Prefix(&prefix), cx);
            assert!(
                !entity.read(cx).is_cache_fresh(1_500),
                "should be stale after prefix invalidate"
            );

            assert_eq!(entity.read(cx).data().unwrap(), "deep_key_data");

            client.reset_queries(&QueryKeyFilter::Prefix(&prefix), cx);
            assert!(
                entity.read(cx).data().is_none(),
                "data cleared after prefix reset"
            );
        });
    });
}

#[gpui::test]
fn test_set_query_data_different_types_no_conflict(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("shared_key", "string_val".to_string(), cx);
            client.set_query_data::<u32, QueryError>("shared_key", 42_u32, cx);

            let s = client.get_query_data::<String, QueryError>(&QueryKey::from("shared_key"), cx);
            let n = client.get_query_data::<u32, QueryError>(&QueryKey::from("shared_key"), cx);

            assert_eq!(s, Some("string_val".to_string()));
            assert_eq!(n, Some(42_u32));
        });
    });
}

#[gpui::test]
fn test_clear_data_via_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("clear_me");
            client.set_query_data::<String, QueryError>(key.clone(), "data".to_string(), cx);

            let entity = client.query::<String, QueryError>(&key).unwrap();
            assert_eq!(entity.read(cx).data().unwrap(), "data");

            entity.update(cx, |r, _| r.clear_data());
            assert!(entity.read(cx).data().is_none(), "data should be cleared");
            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert!(data.is_none());
        });
    });
}
