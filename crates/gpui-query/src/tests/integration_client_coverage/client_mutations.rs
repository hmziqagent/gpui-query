use gpui::{AppContext as _, BorrowAppContext as _, TestAppContext};

use crate::client::{MutationObserver, ObserverConfig, QueryClient, QueryObserver};
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_mutation_with_key_registration(cx: &mut TestAppContext) {
    setup_test(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
                    .with_key(QueryKey::from("mut_with_key"))
            });
            client.register_mutation::<String, User, QueryError>(&entity, cx);

            let mutations = client.all_mutations::<String, User, QueryError>();
            assert_eq!(mutations.len(), 1);
        });
    });
}

#[gpui::test]
fn test_all_mutations_empty_for_unregistered_type(cx: &mut TestAppContext) {
    setup_test(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let e = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, User, QueryError>(&e, cx);

            let other = client.all_mutations::<u32, User, QueryError>();
            assert!(other.is_empty(), "no u32 mutations registered");
        });
    });
}

#[gpui::test]
fn test_multiple_mutations_same_type(cx: &mut TestAppContext) {
    setup_test(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let m1 = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });
            let m2 =
                cx.new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::new(3)));
            let m3 = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });

            client.register_mutation::<String, User, QueryError>(&m1, cx);
            client.register_mutation::<String, User, QueryError>(&m2, cx);
            client.register_mutation::<String, User, QueryError>(&m3, cx);

            let all = client.all_mutations::<String, User, QueryError>();
            assert_eq!(all.len(), 3, "should have three registered mutations");
        });
    });
}

#[gpui::test]
fn test_mutation_full_lifecycle_with_retries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity =
                cx.new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::new(2)));
            client.register_mutation::<String, User, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("create_user".to_string());
            });
            assert!(entity.read(cx).is_loading());

            entity.update(cx, |m, _| {
                m.complete_failure(QueryError::response("network"));
            });
            assert!(entity.read(cx).is_failure());
            assert_eq!(entity.read(cx).retry_count(), 1);

            entity.update(cx, |m, _| {
                assert!(m.retry());
            });
            assert!(entity.read(cx).is_loading());

            entity.update(cx, |m, _| {
                m.complete_success(User::new(99, "Retry Success"));
            });
            assert!(entity.read(cx).is_success());
            assert_eq!(entity.read(cx).data().unwrap().name, "Retry Success");
        });
    });
}

#[gpui::test]
fn test_mutation_cancel_via_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx.new(|_| {
                MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, User, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("vars".to_string());
            });
            assert!(entity.read(cx).is_loading());

            let signal = entity.read(cx).signal().unwrap().clone();
            assert!(!signal.is_cancelled());

            entity.update(cx, |m, _| {
                m.cancel(QueryError::cancelled("user aborted"));
            });
            assert!(signal.is_cancelled());
            assert!(entity.read(cx).is_failure());
            assert_eq!(entity.read(cx).cancelled_count(), 1);
        });
    });
}

#[gpui::test]
fn test_diagnostics_includes_mutations_with_status(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let idle_mut = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            client.register_mutation::<String, String, QueryError>(&idle_mut, cx);

            let loading_mut = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            loading_mut.update(cx, |m, _| m.begin("vars".to_string()));
            client.register_mutation::<String, String, QueryError>(&loading_mut, cx);

            let success_mut = cx.new(|_| {
                MutationResource::<String, String, QueryError>::new(RetryPolicy::no_retries())
            });
            success_mut.update(cx, |m, _| {
                m.begin("vars".to_string());
                m.complete_success("done".to_string());
            });
            client.register_mutation::<String, String, QueryError>(&success_mut, cx);

            let diag = client.diagnostics(cx);
            assert_eq!(diag.mutation_count, 3);
            assert_eq!(diag.mutations.len(), 3);

            let statuses: Vec<MutationStatus> = diag.mutations.iter().map(|m| m.status).collect();
            assert!(statuses.contains(&MutationStatus::Idle));
            assert!(statuses.contains(&MutationStatus::Loading));
            assert!(statuses.contains(&MutationStatus::Success));
        });
    });
}

#[gpui::test]
fn test_diagnostics_mutation_retry_count(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = cx
                .new(|_| MutationResource::<String, String, QueryError>::new(RetryPolicy::new(3)));
            client.register_mutation::<String, String, QueryError>(&entity, cx);

            entity.update(cx, |m, _| {
                m.begin("vars".to_string());
                m.complete_failure(QueryError::response("fail"));
            });

            let diag = client.diagnostics(cx);
            assert_eq!(diag.mutations.len(), 1);
            assert_eq!(
                diag.mutations[0].retry_count, 1,
                "retry_count should be 1 after one failure"
            );
        });
    });
}

#[gpui::test]
fn test_mutation_observer_observe_returns_subscription(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        let entity = cx
            .new(|_| MutationResource::<String, User, QueryError>::new(RetryPolicy::no_retries()));
        let observer = MutationObserver::<String, User, QueryError>::new(&entity);

        struct DummyView;
        let view = cx.new(|_| DummyView);
        let sub = view.update(cx, |_view, cx| observer.observe(cx));
        assert!(
            sub.is_some(),
            "mutation observe should return Some(Subscription)"
        );
    });
}

#[gpui::test]
fn test_query_observer_with_config_always_notify(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = client.resource::<String, QueryError>("config_always", cx);
            let config = ObserverConfig {
                notify_on_status_change_only: false,
            };
            let observer = QueryObserver::new(&entity).with_config(config);

            struct DummyView;
            let view = cx.new(|_| DummyView);
            let sub = view.update(cx, |_view, cx| observer.observe(cx));
            assert!(sub.is_some());
        });
    });
}
