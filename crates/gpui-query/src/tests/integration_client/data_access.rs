use std::sync::Mutex;

use gpui::{BorrowAppContext as _, TestAppContext};

use crate::client::QueryClient;
use crate::core::*;
use crate::tests::test_support::*;

#[gpui::test]
fn test_cancel_queries_cancels_loading_requests(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("cancel_target");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            let request_id = client
                .next_request_id_for_key::<String, QueryError>(&key)
                .expect("should get request id");
            entity.update(cx, |r, _| {
                let _ = r.begin_request_with_id(Some(request_id), 1_000, QueryFetchMode::Normal);
            });
            assert!(entity.read(cx).is_loading());

            let signal = entity
                .read(cx)
                .signal()
                .expect("signal should exist while loading")
                .clone();
            assert!(!signal.is_cancelled());

            client.cancel_queries(&QueryKeyFilter::Exact(&key), cx);

            assert!(
                signal.is_cancelled(),
                "signal should be cancelled after cancel_queries"
            );
        });
    });
}

#[gpui::test]
fn test_cancel_queries_skips_idle_resources(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("idle_key");
            let _entity = client.resource::<String, QueryError>(key.clone(), cx);

            client.cancel_queries(&QueryKeyFilter::Exact(&key), cx);

            let entity = client.query::<String, QueryError>(&key);
            assert!(entity.is_some(), "resource should still exist");
            assert_eq!(
                entity.unwrap().read(cx).status(),
                QueryStatus::Idle,
                "status should remain Idle"
            );
        });
    });
}

#[gpui::test]
fn test_remove_queries_removes_matching(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let _u1 = client.resource::<String, QueryError>(QueryKey::from(["users", "1"]), cx);
            let _u2 = client.resource::<String, QueryError>(QueryKey::from(["users", "2"]), cx);
            let _p1 = client.resource::<String, QueryError>(QueryKey::from(["posts", "1"]), cx);

            assert_eq!(client.all_queries::<String, QueryError>().len(), 3);

            let prefix = QueryKey::from(["users"]);
            client.remove_queries(&QueryKeyFilter::Prefix(&prefix));

            let remaining = client.all_queries::<String, QueryError>();
            assert_eq!(remaining.len(), 1, "only the post should remain");
            assert!(
                client
                    .query::<String, QueryError>(&QueryKey::from(["posts", "1"]))
                    .is_some(),
                "post should still exist"
            );
            assert!(
                client
                    .query::<String, QueryError>(&QueryKey::from(["users", "1"]))
                    .is_none(),
                "user:1 should be removed"
            );
        });
    });
}

#[gpui::test]
fn test_set_query_data_sets_data_on_existing_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("user:1");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| {
                r.apply_success("Alice".to_string(), 1_000);
            });
            assert_eq!(entity.read(cx).data().unwrap(), "Alice");

            client.set_query_data::<String, QueryError>("user:1", "Bob".to_string(), cx);

            assert_eq!(entity.read(cx).data().unwrap(), "Bob");
            assert_eq!(
                entity.read(cx).previous_data().unwrap(),
                "Alice",
                "previous_data should hold the pre-set value"
            );
        });
    });
}

#[gpui::test]
fn test_set_query_data_creates_resource_if_missing(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("new_key", "hello".to_string(), cx);

            let data = client.get_query_data::<String, QueryError>(&QueryKey::from("new_key"), cx);
            assert_eq!(data, Some("hello".to_string()));
        });
    });
}

#[gpui::test]
fn test_get_query_data_returns_none_for_missing_key(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let data = client.get_query_data::<String, QueryError>(&QueryKey::from("ghost"), cx);
            assert!(data.is_none(), "should return None for nonexistent key");
        });
    });
}

#[gpui::test]
fn test_with_query_data_reads_without_clone(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("len_key", "hello".to_string(), cx);

            let len = client.with_query_data::<String, QueryError, usize>(
                &QueryKey::from("len_key"),
                cx,
                |s| s.len(),
            );
            assert_eq!(len, Some(5));

            let missing = client.with_query_data::<String, QueryError, usize>(
                &QueryKey::from("absent"),
                cx,
                |s| s.len(),
            );
            assert!(missing.is_none());
        });
    });
}

#[gpui::test]
fn test_rollback_query_data_via_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("user:1");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| r.apply_success("Alice".to_string(), 1_000));
            client.set_query_data::<String, QueryError>(
                "user:1",
                "Alice (optimistic)".to_string(),
                cx,
            );
            assert_eq!(entity.read(cx).data().unwrap(), "Alice (optimistic)");

            let rolled_back = entity.update(cx, |r, _| r.rollback_to_previous());
            assert!(rolled_back);
            assert_eq!(entity.read(cx).data().unwrap(), "Alice");
            assert_eq!(entity.read(cx).status(), QueryStatus::Success);
        });
    });
}

#[cfg(feature = "persist")]
#[gpui::test]
fn test_dehydrate_collects_success_entries(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let e1 = client.resource::<String, QueryError>(QueryKey::from("ok"), cx);
            e1.update(cx, |r, _| r.apply_success("data".to_string(), 1_000));

            let _e2 = client.resource::<String, QueryError>(QueryKey::from("idle"), cx);

            let state = client.dehydrate(cx);
            assert_eq!(
                state.entries.len(),
                1,
                "only Success resources should be dehydrated"
            );
            assert_eq!(state.entries[0].key, "ok");
            assert_eq!(state.entries[0].kind, "query");
        });
    });
}

#[cfg(feature = "persist")]
#[gpui::test]
fn test_dehydrate_skips_non_success_resources(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let e1 = client.resource::<String, QueryError>("fail", cx);
            e1.update(cx, |r, _| {
                r.apply_failure(QueryError::response("err"), 1_000)
            });

            let _e2 = client.resource::<String, QueryError>("idle2", cx);

            let state = client.dehydrate(cx);
            assert!(
                state.entries.is_empty(),
                "non-Success resources should not be dehydrated"
            );
        });
    });
}

#[gpui::test]
fn test_next_request_id_is_monotonically_increasing(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("seq_test");
            let _entity = client.resource::<String, QueryError>(key.clone(), cx);

            let id1 = client.next_request_id_for_key::<String, QueryError>(&key);
            let id2 = client.next_request_id_for_key::<String, QueryError>(&key);
            let id3 = client.next_request_id_for_key::<String, QueryError>(&key);

            assert!(id1.is_some());
            assert!(id2.is_some());
            assert!(id3.is_some());

            let id1 = id1.unwrap();
            let id2 = id2.unwrap();
            let id3 = id3.unwrap();

            assert!(id1.value() < id2.value(), "sequence should be increasing");
            assert!(id2.value() < id3.value(), "sequence should be increasing");
            assert_eq!(id1.scope_id(), id2.scope_id(), "same scope for same key");
        });
    });
}

#[gpui::test]
fn test_next_request_id_returns_none_for_missing_key(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            let id = client.next_request_id_for_key::<String, QueryError>(&QueryKey::from("ghost"));
            assert!(id.is_none(), "should return None for nonexistent key");
        });
    });
}

#[gpui::test]
fn test_prepare_fetch_query_success_lifecycle(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("fetch_key");
            let prepared = client.prepare_fetch_query::<String, QueryError>(key.clone(), cx);

            assert!(prepared.is_some(), "should return Some for new resource");
            let prepared = prepared.unwrap();
            assert!(
                !prepared.signal.is_cancelled(),
                "signal should start uncancelled"
            );

            prepared.complete_success("fetched_data".to_string(), cx);

            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert_eq!(data, Some("fetched_data".to_string()));
        });
    });
}

#[gpui::test]
fn test_prepare_fetch_query_failure_lifecycle(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("fail_fetch");
            let prepared = client.prepare_fetch_query::<String, QueryError>(key.clone(), cx);
            assert!(prepared.is_some());

            let prepared = prepared.unwrap();
            prepared.complete_failure(QueryError::response("server error"), cx);

            let entity = client.query::<String, QueryError>(&key).unwrap();
            assert_eq!(entity.read(cx).status(), QueryStatus::Failure);
        });
    });
}

#[gpui::test]
fn test_prepare_prefetch_query_starts_for_stale_resource(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("prefetch_key");
            let entity = client.resource::<String, QueryError>(key.clone(), cx);

            entity.update(cx, |r, _| r.apply_success("old_data".to_string(), 100));

            let prepared = client.prepare_prefetch_query::<String, QueryError>(
                key.clone(),
                CachePolicy::Ttl { ttl_ms: 1_000 },
                RequestPolicy::LatestWins,
                cx,
            );
            assert!(
                prepared.is_some(),
                "prefetch should start for stale resource"
            );

            let prepared = prepared.unwrap();
            prepared.complete_success("fresh_data".to_string(), cx);

            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert_eq!(data, Some("fresh_data".to_string()));
        });
    });
}

#[gpui::test]
fn remove_queries_with_fetch_in_flight_keeps_bucket_and_fallback_ids_disjoint(
    cx: &mut TestAppContext,
) {
    setup_query_client(cx);
    let (in_flight_id, entity) = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let key = QueryKey::from("request_id_alias");
            let prepared = client
                .prepare_fetch_query::<String, QueryError>(key, cx)
                .expect("prepare_fetch_query should start");
            let in_flight_id = prepared.request_id;
            let entity = prepared.entity.clone();
            client.remove_queries(&QueryKeyFilter::All);
            (in_flight_id, entity)
        })
    });

    let (fallback_id, stale_rejected, fallback_accepted, ignored) = cx.update(|cx| {
        entity.update(cx, |resource, _| {
            let fallback_id =
                match resource.begin_request_with_id(None, 1_000, QueryFetchMode::Force) {
                    QueryBeginResult::Started { request_id, .. } => request_id,
                    other => panic!("expected Started after remove_queries, got {other:?}"),
                };
            let stale_rejected = resource.accept_current_request(in_flight_id).is_none();
            let fallback_accepted = resource.accept_current_request(fallback_id).is_some();
            (
                fallback_id,
                stale_rejected,
                fallback_accepted,
                resource.ignored_results(),
            )
        })
    });

    assert_ne!(
        in_flight_id, fallback_id,
        "bucket-sequencer id and fallback id aliased: the stale in-flight \
         result would be accepted as fresh"
    );
    assert!(
        stale_rejected,
        "the stale in-flight completion must be discarded as ignored"
    );
    assert_eq!(ignored, 1);
    assert!(
        fallback_accepted,
        "the fallback request must still be accepted"
    );
}

#[cfg(feature = "persist")]
#[gpui::test]
fn test_persist_and_restore_cycle(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let entity = client.resource::<String, QueryError>("persist_me", cx);
            entity.update(cx, |r, _| r.apply_success("value".to_string(), 1_000));

            struct MemPersister {
                entries: Mutex<Vec<crate::client::DehydratedEntry>>,
            }
            impl crate::client::QueryPersister for MemPersister {
                fn load(&self) -> Vec<crate::client::DehydratedEntry> {
                    self.entries.lock().unwrap().clone()
                }
                fn save(&self, entries: Vec<crate::client::DehydratedEntry>) {
                    *self.entries.lock().unwrap() = entries;
                }
            }

            let persister = MemPersister {
                entries: Mutex::new(Vec::new()),
            };

            client.persist(&persister, cx);

            let loaded = QueryClient::restore(&persister);
            assert_eq!(loaded.len(), 1, "should have one persisted entry");
            assert_eq!(loaded[0].key, "persist_me");
        });
    });
}
