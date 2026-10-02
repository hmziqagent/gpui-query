use gpui::{BorrowAppContext as _, TestAppContext};

use crate::client::QueryClient;
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_prepare_fetch_query_uses_force_mode_always_starts(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(
            CachePolicy::Ttl { ttl_ms: 60_000 },
            RequestPolicy::LatestWins,
        ));
    });
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let prepared = client
                .prepare_fetch_query::<String, QueryError>("cached_key", cx)
                .expect("first fetch should start");
            prepared.complete_success("data".to_string(), cx);

            let second = client.prepare_fetch_query::<String, QueryError>("cached_key", cx);
            assert!(
                second.is_some(),
                "prepare_fetch_query uses Force mode, always starts"
            );

            let data =
                client.get_query_data::<String, QueryError>(&QueryKey::from("cached_key"), cx);
            assert_eq!(data, Some("data".to_string()));
        });
    });
}

#[gpui::test]
fn test_prepare_fetch_query_ignored_while_loading_returns_none(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(
            CachePolicy::NoCache,
            RequestPolicy::IgnoreWhileLoading,
        ));
    });
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("pf_ignore_foreign");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            let rid = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("rid");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid), 1_000, QueryFetchMode::Normal);
            });

            let prepared = client.prepare_fetch_query::<String, QueryError>(key.clone(), cx);
            assert!(
                prepared.is_none(),
                "IgnoreWhileLoading with a request in flight must not hand out a prepared fetch"
            );
            assert!(
                entity.read(cx).is_current_request(rid),
                "the original in-flight request must still own the resource"
            );

            let accepted = entity.update(cx, |r, _| {
                r.complete_current_success(rid, "original".to_string(), 2_000)
            });
            assert!(
                accepted,
                "the original fetcher's completion must still be accepted"
            );
            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert_eq!(data, Some("original".to_string()));
        });
    });
}

#[gpui::test]
fn test_prepare_fetch_query_refetch_after_ttl(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(
            CachePolicy::Ttl { ttl_ms: 500 },
            RequestPolicy::LatestWins,
        ));
    });
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let prepared = client
                .prepare_fetch_query::<String, QueryError>("ttl_key", cx)
                .expect("first fetch should start");
            prepared.complete_success("old".to_string(), cx);

            let data = client.get_query_data::<String, QueryError>(&QueryKey::from("ttl_key"), cx);
            assert_eq!(data, Some("old".to_string()));

            let second = client.prepare_fetch_query::<String, QueryError>("ttl_key", cx);
            assert!(
                second.is_some(),
                "prepare_fetch_query should always return Some (Force mode), \
                 even when data was just set"
            );
        });
    });
}

#[gpui::test]
fn test_prepare_prefetch_query_returns_none_for_fresh(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(
            CachePolicy::Ttl { ttl_ms: 60_000 },
            RequestPolicy::LatestWins,
        ));
    });
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("prefresh_fresh");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);
            let now = crate::client::current_time_ms();
            entity.update(cx, |r, _| r.apply_success("fresh_data".to_string(), now));

            let age_ms = crate::client::current_time_ms().saturating_sub(now);
            assert!(
                age_ms < 60_000,
                "precondition: age {}ms must be < 60s TTL for deterministic result",
                age_ms,
            );

            let result = client.prepare_prefetch_query::<String, QueryError>(
                key.clone(),
                CachePolicy::Ttl { ttl_ms: 60_000 },
                RequestPolicy::LatestWins,
                cx,
            );
            assert!(
                result.is_none(),
                "prefetch should return None for fresh data \
                 (age ~0ms, well within 60s TTL)"
            );
        });
    });
}

#[gpui::test]
fn test_prepare_prefetch_query_returns_none_while_revalidate_in_flight(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("prefetch_stale_active");
            let cache_policy = CachePolicy::StaleWhileRevalidate {
                ttl_ms: 1_000,
                stale_ms: 60_000,
            };
            let now = crate::client::current_time_ms();
            let cached_at = now.saturating_sub(31_000);
            let entity = client.resource_with_policies::<String, QueryError>(
                key.clone(),
                cache_policy,
                RequestPolicy::IgnoreWhileLoading,
                cx,
            );
            entity.update(cx, |r, _| r.apply_success("stale".to_string(), cached_at));

            let rid = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("rid");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid), now, QueryFetchMode::Normal);
            });
            assert!(
                entity.read(cx).is_current_request(rid),
                "precondition: a revalidate must be in flight over the stale data"
            );

            let prepared = client.prepare_prefetch_query::<String, QueryError>(
                key.clone(),
                cache_policy,
                RequestPolicy::IgnoreWhileLoading,
                cx,
            );
            assert!(
                prepared.is_none(),
                "IgnoreWhileLoading handing back the still-active id means a \
                 revalidate is already running; prefetch must return None"
            );
            assert!(
                entity.read(cx).is_current_request(rid),
                "the in-flight revalidate must still own the resource"
            );
            assert!(
                !entity.read(cx).signal().unwrap().is_cancelled(),
                "the duplicate prefetch must not cancel the active revalidate"
            );
        });
    });
}

#[gpui::test]
fn test_prepared_fetch_complete_failure_stores_error(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("pf_fail");
            let prepared = client
                .prepare_fetch_query::<String, QueryError>(key.clone(), cx)
                .expect("should start");

            let error = QueryError::response("server unavailable");
            prepared.complete_failure(error.clone(), cx);

            let entity = client.query::<String, QueryError>(&key).unwrap();
            assert_eq!(entity.read(cx).status(), QueryStatus::Failure);
            assert!(entity.read(cx).data().is_none());
            assert!(entity.read(cx).error().is_some());
        });
    });
}

#[gpui::test]
fn test_prepared_fetch_signal_properties(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let prepared = client
                .prepare_fetch_query::<String, QueryError>("signal_test", cx)
                .expect("should start");

            assert!(
                !prepared.signal.is_cancelled(),
                "signal should start uncancelled"
            );
            assert!(
                prepared.request_id.value() > 0,
                "request_id should have a positive value"
            );

            prepared.complete_success("data".to_string(), cx);
        });
    });
}

#[gpui::test]
fn test_cancel_queries_across_type_buckets(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key_s = QueryKey::from("target");
            let entity_s = client.resource::<String, QueryError>(key_s.clone(), cx);
            let rid_s = client
                .next_request_id_for_key::<String, QueryError>(&key_s)
                .expect("rid");
            entity_s.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid_s), 1_000, QueryFetchMode::Normal);
            });

            let key_u = QueryKey::from("target");
            let entity_u = client.resource::<u32, QueryError>(key_u.clone(), cx);
            let rid_u = client
                .next_request_id_for_key::<u32, QueryError>(&key_u)
                .expect("rid");
            entity_u.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid_u), 1_000, QueryFetchMode::Normal);
            });

            let sig_s = entity_s.read(cx).signal().unwrap().clone();
            let sig_u = entity_u.read(cx).signal().unwrap().clone();

            client.cancel_queries(&QueryKeyFilter::Exact(&key_s), cx);

            assert!(sig_s.is_cancelled(), "String query should be cancelled");
            assert!(sig_u.is_cancelled(), "u32 query should be cancelled");
        });
    });
}

#[gpui::test]
fn test_cancel_queries_all_filter(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key1 = QueryKey::from("a1");
            let key2 = QueryKey::from("a2");
            let e1 = client.resource::<String, QueryError>(key1.clone(), cx);
            let e2 = client.resource::<String, QueryError>(key2.clone(), cx);

            let rid1 = client
                .next_request_id_for_key::<String, QueryError>(&key1)
                .unwrap();
            let rid2 = client
                .next_request_id_for_key::<String, QueryError>(&key2)
                .unwrap();

            e1.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid1), 1_000, QueryFetchMode::Normal);
            });
            e2.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid2), 1_000, QueryFetchMode::Normal);
            });

            client.cancel_queries(&QueryKeyFilter::All, cx);

            let sig1 = e1.read(cx).signal().unwrap().clone();
            let sig2 = e2.read(cx).signal().unwrap().clone();
            assert!(sig1.is_cancelled());
            assert!(sig2.is_cancelled());
        });
    });
}

#[gpui::test]
fn test_cancel_queries_skips_idle_infinite_queries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("inf_idle");
            let _entity = client.infinite_resource::<String, QueryError>(key.clone(), cx);

            client.cancel_queries(&QueryKeyFilter::Exact(&key), cx);

            let retrieved = client.infinite_query::<String, QueryError>(&key);
            assert!(
                retrieved.is_some(),
                "idle infinite query should still exist"
            );
        });
    });
}
