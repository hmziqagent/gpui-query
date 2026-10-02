use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use crate::client::{MutationObserver, QueryClient, QueryObserver};
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_resource_creates_and_deduplicates(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("user:1");

            let e1 = client.resource::<String, QueryError>(key.clone(), cx);
            assert_eq!(e1.read(cx).status(), QueryStatus::Idle);

            let e2 = client.resource::<String, QueryError>(key.clone(), cx);
            assert_eq!(
                e1.entity_id(),
                e2.entity_id(),
                "same key should return same entity"
            );

            let e3 = client.resource::<String, QueryError>("user:2", cx);
            assert_ne!(
                e1.entity_id(),
                e3.entity_id(),
                "different key should return different entity"
            );
        });
    });
}

#[gpui::test]
fn test_resource_with_explicit_policies(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = client.resource_with_policies::<String, QueryError>(
                "post:1",
                CachePolicy::StaleWhileRevalidate {
                    ttl_ms: 1_000,
                    stale_ms: 2_000,
                },
                RequestPolicy::IgnoreWhileLoading,
                cx,
            );
            entity.read_with(cx, |r, _| {
                assert_eq!(
                    r.cache_policy(),
                    CachePolicy::StaleWhileRevalidate {
                        ttl_ms: 1_000,
                        stale_ms: 2_000,
                    }
                );
                assert_eq!(r.request_policy(), RequestPolicy::IgnoreWhileLoading);
            });
        });
    });
}

#[gpui::test]
fn test_query_retrieves_existing_entity(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("item:1");

            let created = client.resource::<String, QueryError>(key.clone(), cx);

            let retrieved = client.query::<String, QueryError>(&key);
            assert!(retrieved.is_some(), "should find existing key");
            assert_eq!(created.entity_id(), retrieved.unwrap().entity_id());

            let missing = client.query::<String, QueryError>(&QueryKey::from("nope"));
            assert!(missing.is_none(), "should not find nonexistent key");
        });
    });
}

#[gpui::test]
fn test_diagnostics_with_resources(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _e1 = client.resource::<String, QueryError>(QueryKey::from(["users", "1"]), cx);

            let prepared = client
                .prepare_fetch_query::<String, QueryError>(QueryKey::from(["users", "2"]), cx)
                .expect("should start");
            prepared.complete_success("Bob".to_string(), cx);

            let mutation = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&mutation, cx);

            let diag = client.diagnostics(cx);
            assert_eq!(diag.query_count, 2, "should have two query entries");
            assert_eq!(diag.mutation_count, 1, "should have one mutation");
            assert_eq!(
                diag.queries.len(),
                2,
                "diagnostics should have two query records"
            );

            let users_diags: Vec<_> = diag
                .queries
                .iter()
                .filter(|q| q.key.contains("users"))
                .collect();
            assert_eq!(users_diags.len(), 2, "should have two user queries");

            let success_diag = users_diags
                .iter()
                .find(|q| q.status == QueryStatus::Success);
            assert!(
                success_diag.is_some(),
                "at least one query should show Success (the one completed via PreparedFetch)"
            );
        });
    });
}

#[gpui::test]
fn test_query_observer_observe_returns_subscription(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = client.resource::<String, QueryError>("sub_key", cx);

            let mut observer = QueryObserver::new(&entity);
            let subscription = observe_with_dummy_view(cx, &mut observer);
            assert!(
                subscription.is_some(),
                "observe should return Some(Subscription)"
            );

            drop(subscription);
        });
    });
}

#[gpui::test]
fn test_mutation_observer_creation(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        let entity = cx
            .new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries()));
        let _observer = MutationObserver::<String, User, QueryError>::new(&entity);
    });
}
