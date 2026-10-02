use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use crate::client::QueryClient;
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_mutation_lifecycle_through_client(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });

            client.register_mutation::<String, User, QueryError>(&entity, cx);

            let mutations = client.all_mutations::<String, User, QueryError>();
            assert_eq!(mutations.len(), 1, "should have one registered mutation");

            entity.update(cx, |m, _| {
                m.begin("new_name".to_string());
            });
            assert!(entity.read(cx).is_loading());
            assert_eq!(entity.read(cx).variables(), Some(&"new_name".to_string()));

            entity.update(cx, |m, _| {
                m.complete_success(User::new(1, "Alice Updated"));
            });
            assert!(entity.read(cx).is_success());
            assert_eq!(entity.read(cx).data().unwrap().name, "Alice Updated");
        });
    });
}

#[gpui::test]
fn test_mutation_failure_and_retry(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity =
                cx.new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::new(2)));
            client.register_mutation::<String, User, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("vars".to_string());
                m.complete_failure(QueryError::response("timeout"));
            });
            assert!(entity.read(cx).is_failure());
            assert_eq!(entity.read(cx).retry_count(), 1);

            entity.update(cx, |m, _| {
                assert!(m.retry());
            });
            assert!(entity.read(cx).is_loading());

            entity.update(cx, |m, _| {
                m.complete_failure(QueryError::response("timeout again"));
            });
            assert_eq!(entity.read(cx).retry_count(), 2);
            assert!(
                !entity.read(cx).should_retry(),
                "retries should be exhausted"
            );
        });
    });
}

#[gpui::test]
fn test_mutation_reset_clears_state(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        let entity = cx
            .new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries()));
        entity.update(cx, |m, _| {
            m.begin("vars".to_string());
            m.complete_success(User::default());
        });
        assert!(entity.read(cx).is_success());

        entity.update(cx, |m, _| m.reset());
        assert!(entity.read(cx).is_idle());
        assert!(entity.read(cx).data().is_none());
    });
}

#[gpui::test]
fn test_full_lifecycle_idle_to_loading_to_success_to_gc(cx: &mut TestAppContext) {
    setup_query_client_with_gc(cx, 5_000);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from(["users", "42"]);
            let _prepared = client
                .prepare_fetch_query::<String, QueryError>(key.clone(), cx)
                .expect("should start fetch");

            let entity = client
                .query::<String, QueryError>(&key)
                .expect("entity should exist");
            assert!(entity.read(cx).is_loading());

            entity.update(cx, |r, _| r.apply_success("Carol".to_string(), 1_000));
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);
            assert_eq!(entity.read(cx).data().unwrap(), "Carol");

            client.gc_with_time(2_800, cx);

            let surviving = client.query::<String, QueryError>(&key).expect(
                "success resource should survive GC (age 1800ms < success_threshold 10000ms)",
            );
            assert_eq!(
                surviving.read(cx).data().unwrap(),
                "Carol",
                "data should be intact after GC"
            );
        });
    });
}

#[gpui::test]
fn test_full_lifecycle_failure_recovery(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("flaky");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            let rid1 = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("request id");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid1), 1_000, QueryFetchMode::Normal);
            });
            entity.update(cx, |r, _| {
                r.complete_current_failure(rid1, QueryError::response("fail"), 1_100)
            });
            assert_eq!(entity.read(cx).status(), QueryStatus::Failure);
            assert!(entity.read(cx).data().is_none());

            let rid2 = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("request id 2");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid2), 1_200, QueryFetchMode::Normal);
            });
            entity.update(cx, |r, _| {
                r.complete_current_success(rid2, "recovered".to_string(), 1_300)
            });
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);
            assert_eq!(entity.read(cx).data().unwrap(), "recovered");
        });
    });
}

#[gpui::test]
fn test_optimistic_update_and_rollback_lifecycle(cx: &mut TestAppContext) {
    setup_query_client_with_policies(cx, CachePolicy::NoCache, RequestPolicy::LatestWins);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from(["users", "42"]);
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| r.apply_success("Carol".to_string(), 1_000));

            client.set_query_data::<String, QueryError>(
                key.clone(),
                "Carol (saving...)".to_string(),
                cx,
            );
            assert_eq!(entity.read(cx).data().unwrap(), "Carol (saving...)");
            assert_eq!(entity.read(cx).previous_data().unwrap(), "Carol");

            let rid = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("request id");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid), 1_100, QueryFetchMode::Force);
            });
            assert!(entity.read(cx).is_loading());

            entity.update(cx, |r, _| {
                r.complete_current_success(rid, "Carol (saved)".to_string(), 1_200)
            });
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);
            assert_eq!(entity.read(cx).data().unwrap(), "Carol (saved)");
        });
    });
}

#[gpui::test]
fn test_optimistic_update_rollback_on_failure(cx: &mut TestAppContext) {
    setup_query_client_with_policies(cx, CachePolicy::NoCache, RequestPolicy::LatestWins);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from(["users", "42"]);
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| r.apply_success("Carol".to_string(), 1_000));

            client.set_query_data::<String, QueryError>(
                key.clone(),
                "Carol (saving...)".to_string(),
                cx,
            );

            let rid = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("request id");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(rid), 1_100, QueryFetchMode::Force);
            });
            entity.update(cx, |r, _| {
                r.complete_current_failure(rid, QueryError::response("network error"), 1_200)
            });

            assert_eq!(entity.read(cx).status(), QueryStatus::Failure);

            let rolled_back = entity.update(cx, |r, _| r.rollback_to_previous());
            assert!(rolled_back);
            assert_eq!(entity.read(cx).data().unwrap(), "Carol");
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);
        });
    });
}
