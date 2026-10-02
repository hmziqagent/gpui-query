use std::sync::Mutex;

use gpui::{BorrowAppContext as _, TestAppContext};

use crate::client::{DehydratedEntry, DehydratedState, QueryClient};
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_diagnostics_query_status_accuracy(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _idle = client.resource::<String, QueryError>("diag_idle", cx);

            let prepared = client
                .prepare_fetch_query::<String, QueryError>("diag_success", cx)
                .expect("should start");
            prepared.complete_success("data".to_string(), cx);

            let diag = client.diagnostics(cx);
            let idle_diag = diag
                .queries
                .iter()
                .find(|q| q.key == "diag_idle")
                .expect("should find idle entry");
            assert_eq!(idle_diag.status, QueryStatus::Idle);

            let success_diag = diag
                .queries
                .iter()
                .find(|q| q.key == "diag_success")
                .expect("should find success entry");
            assert_eq!(success_diag.status, QueryStatus::Success);
            assert!(
                success_diag.cache_age_ms.is_some(),
                "success should have cache age"
            );
        });
    });
}

#[gpui::test]
fn test_diagnostics_cache_policy_label(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(QueryClient::with_policies(
            CachePolicy::StaleWhileRevalidate {
                ttl_ms: 1_000,
                stale_ms: 2_000,
            },
            RequestPolicy::LatestWins,
        ));
    });
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _entity = client.resource::<String, QueryError>("swr_key", cx);
            let diag = client.diagnostics(cx);
            let q = diag.queries.iter().find(|q| q.key == "swr_key").unwrap();
            assert!(q.cache_policy.contains("Stale-while-revalidate"));
        });
    });
}

#[gpui::test]
fn test_dehydrate_includes_infinite_query_success(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let q = client.resource::<String, QueryError>("q1", cx);
            q.update(cx, |r, _| r.apply_success("data".to_string(), 1_000));

            let _iq = client.infinite_resource::<String, QueryError>("iq1", cx);

            let state = client.dehydrate(cx);
            let regular_entries: Vec<_> =
                state.entries.iter().filter(|e| e.kind == "query").collect();
            assert_eq!(regular_entries.len(), 1);
            assert_eq!(regular_entries[0].key, "q1");

            let inf_entries: Vec<_> = state
                .entries
                .iter()
                .filter(|e| e.kind == "infinite")
                .collect();
            assert!(inf_entries.is_empty(), "idle infinite query not dehydrated");
        });
    });
}

#[gpui::test]
fn test_dehydrated_state_default_and_construction(_cx: &mut TestAppContext) {
    let state = DehydratedState::default();
    assert!(state.entries.is_empty());

    let entry = DehydratedEntry {
        key: "users".to_string(),
        type_id: std::any::TypeId::of::<(String, QueryError)>(),
        kind: "query",
    };
    let state = DehydratedState {
        entries: vec![entry],
    };
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].key, "users");
}

#[gpui::test]
fn test_hydrate_is_noop(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let state = DehydratedState {
                entries: vec![DehydratedEntry {
                    key: "test".to_string(),
                    type_id: std::any::TypeId::of::<(String, QueryError)>(),
                    kind: "query",
                }],
            };
            client.hydrate(state, cx);

            let data = client.get_query_data::<String, QueryError>(&QueryKey::from("test"), cx);
            assert!(data.is_none(), "hydrate is a no-op, no data injected");
        });
    });
}

#[gpui::test]
fn test_persister_empty_restore(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            struct EmptyPersister;
            impl crate::client::QueryPersister for EmptyPersister {
                fn load(&self) -> Vec<DehydratedEntry> {
                    Vec::new()
                }
                fn save(&self, _entries: Vec<DehydratedEntry>) {}
            }

            client.persist(&EmptyPersister, cx);
            let loaded = QueryClient::restore(&EmptyPersister);
            assert!(loaded.is_empty());
        });
    });
}

#[gpui::test]
fn test_persister_records_multiple_entries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            for i in 0..5 {
                let key = format!("persist_{i}");
                let e = client.resource::<String, QueryError>(key.clone(), cx);
                e.update(cx, |r, _| r.apply_success(format!("val_{i}"), 1_000));
            }
            let _idle = client.resource::<String, QueryError>("idle_persist", cx);

            struct CapturePersister {
                entries: Mutex<Vec<DehydratedEntry>>,
            }
            impl crate::client::QueryPersister for CapturePersister {
                fn load(&self) -> Vec<DehydratedEntry> {
                    self.entries.lock().unwrap().clone()
                }
                fn save(&self, entries: Vec<DehydratedEntry>) {
                    *self.entries.lock().unwrap() = entries;
                }
            }

            let persister = CapturePersister {
                entries: Mutex::new(Vec::new()),
            };

            client.persist(&persister, cx);
            let saved = persister.entries.lock().unwrap().clone();
            assert_eq!(saved.len(), 5, "only success entries should be persisted");
            for entry in &saved {
                assert!(entry.key.starts_with("persist_"));
                assert_eq!(entry.kind, "query");
            }
        });
    });
}
