use std::future::Future;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Poll, Waker};
use std::time::Duration;

use gpui::{AppContext as _, BorrowAppContext as _, Entity, TestAppContext};

use crate::client::{
    CacheMutation, NoopPersister, PersistError, PersistFilter, PersistHandle, PersistOptions,
    PersistSnapshot, PersistedEntry, Persister, QueryClient, hydrate,
};
use crate::core::{
    InfiniteQueryResource, MutationResource, QueryError, QueryKey, QueryKeyFilter, QueryResource,
    QueryStatus,
};
use crate::hook::{
    InfiniteQueryOptions, fetch_query, mutate, use_infinite_query, use_mutation, use_query_manual,
};
use crate::tests::test_support::*;

#[derive(Default, Clone)]
struct MemPersister {
    last_saved: Arc<StdMutex<Option<PersistSnapshot>>>,
    load_value: Arc<StdMutex<Option<PersistSnapshot>>>,
    save_count: Arc<StdMutex<u32>>,
}

impl Persister for MemPersister {
    async fn load(&self) -> Result<PersistSnapshot, PersistError> {
        Ok(self
            .load_value
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| PersistSnapshot {
                entries: Default::default(),
                version: crate::client::PERSIST_VERSION,
            }))
    }

    async fn save(&self, snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        *self.save_count.lock().unwrap() += 1;
        *self.last_saved.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

fn ser_string(s: &String) -> serde_json::Value {
    serde_json::to_value(s).expect("serialize")
}

fn zero_debounce() -> PersistOptions {
    PersistOptions {
        debounce: Duration::ZERO,
        ..PersistOptions::default()
    }
}

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[gpui::test]
fn test_set_query_data_bumps_cache_mutation(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        let _handle = cx.update_global::<QueryClient, _>(|client, cx| {
            client.persist_with(NoopPersister, PersistOptions::default(), cx)
        });
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("k1", "v1".to_string(), cx);
        });
        assert!(cx.has_global::<CacheMutation>());
    });
}

#[gpui::test]
fn test_collect_persist_snapshot_uses_registered_serializer(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);

            let e = client.resource::<String, QueryError>(QueryKey::from("snap_k"), cx);
            e.update(cx, |r, _| {
                r.apply_success("payload".to_string(), crate::client::current_time_ms())
            });

            let snap = client.collect_persist_snapshot(&PersistFilter::All, DAY, cx);
            assert_eq!(snap.entries.len(), 1);
            let entry = snap.entries.get("snap_k").expect("entry present");
            assert_eq!(entry.value, serde_json::json!("payload"));
        });
    });
}

#[gpui::test]
fn test_collect_persist_snapshot_skips_unregistered_types(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let e = client.resource::<String, QueryError>(QueryKey::from("unreg"), cx);
            e.update(cx, |r, _| {
                r.apply_success("data".to_string(), crate::client::current_time_ms())
            });

            let snap =
                client.collect_persist_snapshot(&PersistFilter::All, Duration::from_secs(3600), cx);
            assert!(
                snap.entries.is_empty(),
                "unregistered type -> no value-carrying entry"
            );
        });
    });
}

#[gpui::test]
fn test_collect_persist_snapshot_filter_and_max_age(cx: &mut TestAppContext) {
    setup_query_client(cx);
    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let now = crate::client::current_time_ms();
            for parts in [["users", "1"], ["users", "2"]] {
                let e = client.resource::<String, QueryError>(QueryKey::from(parts), cx);
                e.update(cx, |r, _| r.apply_success("v".to_string(), now));
            }
            let e = client.resource::<String, QueryError>(QueryKey::from(["posts", "9"]), cx);
            e.update(cx, |r, _| {
                r.apply_success("old".to_string(), now.saturating_sub(10_000_000))
            });

            let snap = client.collect_persist_snapshot(
                &PersistFilter::Prefix(QueryKey::from(["users"])),
                Duration::from_secs(3600),
                cx,
            );
            assert_eq!(snap.entries.len(), 2);

            let snap_all =
                client.collect_persist_snapshot(&PersistFilter::All, Duration::from_secs(1), cx);
            assert_eq!(snap_all.entries.len(), 2);
            assert!(!snap_all.entries.contains_key("posts::9"));
        });
    });
}

#[gpui::test]
fn test_persist_with_saves_on_mutation(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (_handle, entity) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let entity = client.resource::<String, QueryError>(QueryKey::from("persisted"), cx);
            entity.update(cx, |r, _| {
                r.apply_success("data".to_string(), crate::client::current_time_ms())
            });
            (handle, entity)
        });
        H {
            _entity: entity,
            _handle,
        }
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("trigger", "x".to_string(), cx);
        });
    });
    cx.run_until_parked();

    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("persist_with should have saved after the mutation");
    assert!(
        saved.entries.contains_key("persisted"),
        "saved snapshot should include the retained Success entry: {:?}",
        saved.entries.keys().collect::<Vec<_>>()
    );
    let _ = harness;
}

#[gpui::test]
fn test_hydrate_primes_via_deserializer_registry(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();

    let mut snap = PersistSnapshot {
        entries: Default::default(),
        version: crate::client::PERSIST_VERSION,
    };
    snap.entries.insert(
        "hydrate_k".to_string(),
        PersistedEntry {
            value: serde_json::json!("hydrated-value"),
            cached_at: crate::client::current_time_ms(),
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    *persister.load_value.lock().unwrap() = Some(snap);

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
    }
    let harness = cx.new(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
        let entity = cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource::<String, QueryError>(QueryKey::from("hydrate_k"), cx)
        });
        H { _entity: entity }
    });

    let filter = PersistFilter::All;
    let max_age = DAY;
    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &filter, max_age, cx))
        })
    });

    assert!(
        outcome.is_ok(),
        "hydrate should succeed: {:?}",
        outcome.err()
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let data =
                client.get_query_data::<String, QueryError>(&QueryKey::from("hydrate_k"), cx);
            assert_eq!(
                data,
                Some("hydrated-value".to_string()),
                "hydrate should have primed the value via the deserializer registry"
            );
        });
    });
    let _ = harness;
}

#[gpui::test]
fn test_hydrate_rebuilds_multi_segment_keys(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();

    let mut snap = PersistSnapshot {
        entries: Default::default(),
        version: crate::client::PERSIST_VERSION,
    };
    snap.entries.insert(
        "users::42::posts".to_string(),
        PersistedEntry {
            value: serde_json::json!("post-data"),
            cached_at: crate::client::current_time_ms(),
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    *persister.load_value.lock().unwrap() = Some(snap);

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
    }
    let harness = cx.new(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
        let entity = cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource::<String, QueryError>(QueryKey::from(["users", "42", "posts"]), cx)
        });
        H { _entity: entity }
    });

    let key = QueryKey::from(["users", "42", "posts"]);
    let prefix = PersistFilter::Prefix(QueryKey::from(["users"]));
    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &prefix, DAY, cx))
        })
    });
    assert!(
        outcome.is_ok(),
        "hydrate should succeed: {:?}",
        outcome.err()
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert_eq!(
                data,
                Some("post-data".to_string()),
                "Prefix must match the reconstructed segments"
            );
        });
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>(key.clone(), "sentinel".to_string(), cx);
        });
    });
    let exact = PersistFilter::Exact(key.clone());
    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &exact, DAY, cx))
        })
    });
    assert!(
        outcome.is_ok(),
        "hydrate should succeed: {:?}",
        outcome.err()
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let data = client.get_query_data::<String, QueryError>(&key, cx);
            assert_eq!(
                data,
                Some("post-data".to_string()),
                "Exact must match the reconstructed segments"
            );
        });
    });
    let _ = harness;
}

#[gpui::test]
fn test_hydrate_rejects_version_mismatch(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    *persister.load_value.lock().unwrap() = Some(PersistSnapshot {
        entries: Default::default(),
        version: 9999,
    });

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
    });

    let filter = PersistFilter::All;
    let max_age = DAY;
    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &filter, max_age, cx))
        })
    });

    match outcome {
        Err(PersistError::VersionMismatch { expected, found }) => {
            assert_eq!(expected, crate::client::PERSIST_VERSION);
            assert_eq!(found, 9999);
        }
        other => panic!("expected VersionMismatch, got {other:?}"),
    }
}

#[gpui::test]
fn test_hydrate_hostile_entries_skip_without_panicking(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();

    let now = crate::client::current_time_ms();
    let mut snap = PersistSnapshot {
        entries: Default::default(),
        version: crate::client::PERSIST_VERSION,
    };
    snap.entries.insert(
        "hostile_shape".to_string(),
        PersistedEntry {
            value: serde_json::json!({"evil": [1, null, "x"]}),
            cached_at: now,
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    snap.entries.insert(
        "future_cached_at".to_string(),
        PersistedEntry {
            value: serde_json::json!("future"),
            cached_at: u64::MAX,
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    snap.entries.insert(
        "ancient_cached_at".to_string(),
        PersistedEntry {
            value: serde_json::json!("ancient"),
            cached_at: 0,
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    *persister.load_value.lock().unwrap() = Some(snap);

    struct H {
        hostile: Entity<QueryResource<String, QueryError>>,
        future: Entity<QueryResource<String, QueryError>>,
        ancient: Entity<QueryResource<String, QueryError>>,
    }
    let harness = cx.new(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
        let hostile = cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource::<String, QueryError>(QueryKey::from("hostile_shape"), cx)
        });
        let future = cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource::<String, QueryError>(QueryKey::from("future_cached_at"), cx)
        });
        let ancient = cx.update_global::<QueryClient, _>(|client, cx| {
            client.resource::<String, QueryError>(QueryKey::from("ancient_cached_at"), cx)
        });
        H {
            hostile,
            future,
            ancient,
        }
    });

    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(
                client,
                &persister,
                &PersistFilter::All,
                Duration::from_secs(60),
                cx,
            ))
        })
    });
    assert!(outcome.is_ok(), "hostile entries must not fail hydrate");

    cx.update(|cx| {
        assert!(
            harness.read(cx).hostile.read(cx).data().is_none(),
            "a value with a hostile JSON shape must be skipped, not primed or panicked on"
        );
        assert!(
            harness.read(cx).ancient.read(cx).data().is_none(),
            "an entry older than max_age must be filtered out"
        );
        assert_eq!(
            harness.read(cx).future.read(cx).data(),
            Some(&"future".to_string()),
            "u64::MAX cached_at must saturate to age 0 and stay hydratable"
        );
    });
}

#[gpui::test]
fn test_persist_with_driven_by_real_fetch_completion(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        entity: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let _handle = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            client.persist_with(persister.clone(), zero_debounce(), cx)
        });
        let (entity, _sub) = use_query_manual::<String, QueryError, _>(
            QueryKey::from("fetched"),
            crate::core::CachePolicy::NoCache,
            crate::core::RequestPolicy::LatestWins,
            cx,
        );
        H { entity, _handle }
    });

    harness.update(cx, |this, cx| {
        fetch_query(
            &this.entity,
            || async { Ok::<_, QueryError>("fetched-value".to_string()) },
            cx,
        );
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let resource = harness.read(cx).entity.read(cx);
        assert_eq!(
            resource.status(),
            QueryStatus::Success,
            "the real fetch must resolve to Success before asserting on the save"
        );
        assert_eq!(resource.data(), Some(&"fetched-value".to_string()));
    });

    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("persist_with should have saved after the real fetch completed");
    let entry = saved
        .entries
        .get("fetched")
        .expect("saved snapshot should include the fetched entry");
    assert_eq!(
        entry.value,
        serde_json::json!("fetched-value"),
        "the snapshot value should be the fetched result, serialized"
    );
    let _ = harness;
}

#[gpui::test]
fn test_persist_with_driven_by_real_mutation_completion(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        _query: Entity<QueryResource<String, QueryError>>,
        mutation: Entity<MutationResource<String, String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let _handle = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            client.persist_with(persister.clone(), zero_debounce(), cx)
        });
        let query = cx.update_global::<QueryClient, _>(|client, cx| {
            let e = client.resource::<String, QueryError>(QueryKey::from("retained"), cx);
            e.update(cx, |r, _| {
                r.apply_success("data".to_string(), crate::client::current_time_ms())
            });
            e
        });
        let (mutation, _msub) = use_mutation::<String, String, QueryError, _>((), cx);
        H {
            _query: query,
            mutation,
            _handle,
        }
    });

    harness.update(cx, |this, cx| {
        mutate(
            &this.mutation,
            "vars".to_string(),
            |_vars| async move { Ok::<_, QueryError>("mutation-done".to_string()) },
            cx,
        );
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let m = harness.read(cx).mutation.read(cx);
        assert_eq!(
            m.data().cloned(),
            Some("mutation-done".to_string()),
            "the real mutation must resolve to Success before asserting on the save"
        );
    });

    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("persist_with should have saved after the real mutation completed");
    assert!(
        saved.entries.contains_key("retained"),
        "the mutation-completion bump should have saved the retained Success entry: {:?}",
        saved.entries.keys().collect::<Vec<_>>()
    );
    let _ = harness;
}

#[gpui::test]
fn test_persist_with_driven_by_real_infinite_completion(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        infinite: Entity<InfiniteQueryResource<Vec<String>, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let _handle = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<Vec<String>, QueryError>(|v| {
                serde_json::to_value(v).expect("serialize")
            });
            client.persist_with(persister.clone(), zero_debounce(), cx)
        });
        let (infinite, _isub) = use_infinite_query(
            InfiniteQueryOptions::new("infinite-feed")
                .cache_policy(crate::core::CachePolicy::Ttl { ttl_ms: 0 }),
            |_last_page| async move { Ok::<_, QueryError>((vec!["page-0".to_string()], true)) },
            cx,
        );
        H { infinite, _handle }
    });

    cx.run_until_parked();

    cx.update(|cx| {
        let r = harness.read(cx).infinite.read(cx);
        assert_eq!(
            r.status(),
            QueryStatus::Success,
            "the real infinite fetch must resolve to Success before asserting on the save"
        );
    });

    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("persist_with should have saved after the real infinite fetch completed");
    let entry = saved
        .entries
        .get("infinite-feed")
        .expect("saved snapshot should include the infinite first-page entry");
    assert_eq!(
        entry.value,
        serde_json::json!(["page-0"]),
        "the snapshot value should be the fetched first page, serialized"
    );
    let _ = harness;
}

#[gpui::test]
fn test_persist_with_driven_by_imperative_prepared_fetch(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        query: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let _handle = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            client.persist_with(persister.clone(), zero_debounce(), cx)
        });
        let (query, _qsub) = use_query_manual::<String, QueryError, _>(
            QueryKey::from("imperative"),
            crate::core::CachePolicy::NoCache,
            crate::core::RequestPolicy::LatestWins,
            cx,
        );
        H { query, _handle }
    });

    harness.update(cx, |_this, cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let prepared = client
                .prepare_fetch_query::<String, QueryError>(QueryKey::from("imperative"), cx)
                .expect("imperative fetch should start (resource is not fresh)");
            prepared.complete_success("imperative-value".to_string(), cx);
        });
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let r = harness.read(cx).query.read(cx);
        assert_eq!(
            r.data(),
            Some(&"imperative-value".to_string()),
            "the imperative fetch must have completed before asserting on the save"
        );
    });

    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("persist_with should have saved after the imperative completion");
    let entry = saved
        .entries
        .get("imperative")
        .expect("saved snapshot should include the imperative Success entry");
    assert_eq!(
        entry.value,
        serde_json::json!("imperative-value"),
        "the snapshot value should be the imperative fetch result, serialized"
    );
    let _ = harness;
}

#[gpui::test]
fn test_persist_with_debounce_coalesces(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();
    let save_count = persister.save_count.clone();

    let debounce = Duration::from_millis(50);

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (_handle, entity) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(
                persister.clone(),
                PersistOptions {
                    debounce,
                    ..PersistOptions::default()
                },
                cx,
            );
            let entity = client.resource::<String, QueryError>(QueryKey::from("coalesced"), cx);
            entity.update(cx, |r, _| {
                r.apply_success("seed".to_string(), crate::client::current_time_ms())
            });
            (handle, entity)
        });
        H {
            _entity: entity,
            _handle,
        }
    });

    for i in 0..5_u32 {
        cx.update(|cx| {
            cx.update_global::<QueryClient, _>(|client, cx| {
                client.set_query_data::<String, QueryError>("trigger", format!("v{i}"), cx);
            });
        });
    }
    cx.run_until_parked();
    assert_eq!(
        *save_count.lock().unwrap(),
        0,
        "no save should fire before the debounce window elapses"
    );

    cx.background_executor
        .advance_clock(debounce + Duration::from_millis(1));
    cx.run_until_parked();

    assert_eq!(
        *save_count.lock().unwrap(),
        1,
        "a rapid burst of mutations should coalesce into exactly one save"
    );
    let saved = captured
        .lock()
        .unwrap()
        .clone()
        .expect("the single coalesced save should have produced a snapshot");
    assert!(
        saved.entries.contains_key("coalesced"),
        "the coalesced save should include the retained Success entry: {:?}",
        saved.entries.keys().collect::<Vec<_>>()
    );
    let _ = harness;
}

#[derive(Clone, Default)]
struct FlushGate {
    inner: Arc<StdMutex<FlushGateInner>>,
}

#[derive(Default)]
struct FlushGateInner {
    released: bool,
    wakers: Vec<Waker>,
}

impl FlushGate {
    fn release(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.released = true;
        for waker in inner.wakers.drain(..) {
            waker.wake();
        }
    }

    fn wait(&self) -> impl Future<Output = ()> + Send {
        let inner = self.inner.clone();
        async move {
            std::future::poll_fn(move |cx| {
                let mut guard = inner.lock().unwrap();
                if guard.released {
                    return Poll::Ready(());
                }
                if !guard.wakers.iter().any(|w| w.will_wake(cx.waker())) {
                    guard.wakers.push(cx.waker().clone());
                }
                Poll::Pending
            })
            .await
        }
    }
}

#[derive(Clone)]
struct GatedPersister {
    gate: FlushGate,
    events: Arc<StdMutex<Vec<String>>>,
}

impl Persister for GatedPersister {
    async fn load(&self) -> Result<PersistSnapshot, PersistError> {
        Ok(PersistSnapshot::new())
    }

    async fn save(&self, snapshot: &PersistSnapshot) -> Result<(), PersistError> {
        let value = snapshot
            .entries
            .get("gated")
            .and_then(|e| e.value.as_str())
            .unwrap_or("?")
            .to_string();
        self.events.lock().unwrap().push(format!("start:{value}"));
        self.gate.wait().await;
        self.events.lock().unwrap().push(format!("end:{value}"));
        Ok(())
    }
}

#[gpui::test]
fn flush_while_save_in_flight_queues_and_saves_in_order(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let gate = FlushGate::default();
    let persister = GatedPersister {
        gate: gate.clone(),
        events: Arc::new(StdMutex::new(Vec::new())),
    };
    let events = persister.events.clone();

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, entity) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(persister, zero_debounce(), cx);
            let entity = client.resource::<String, QueryError>(QueryKey::from("gated"), cx);
            (handle, entity)
        });
        H {
            _entity: entity,
            _handle: handle,
        }
    });

    cx.update(|cx| {
        let entity = harness.read_with(cx, |h, _| h._entity.clone());
        cx.update_global::<QueryClient, _>(|_client, cx| {
            entity.update(cx, |r, _| {
                r.apply_success("v1".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["start:v1"],
        "the first save must be started and blocked on the gate"
    );

    cx.update(|cx| {
        let entity = harness.read_with(cx, |h, _| h._entity.clone());
        cx.update_global::<QueryClient, _>(|_client, cx| {
            entity.update(cx, |r, _| {
                r.apply_success("v2".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["start:v1"],
        "a flush while a save is in flight must queue, not start a second save"
    );

    gate.release();
    cx.run_until_parked();
    assert_eq!(
        events.lock().unwrap().as_slice(),
        ["start:v1", "end:v1", "start:v2", "end:v2"],
        "the queued flush must save the newer snapshot after the in-flight save completes"
    );
    let _ = harness;
}

fn block_on_ready<R>(fut: impl std::future::Future<Output = R>) -> R {
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll, Waker};

    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = Box::pin(fut);
    let mut pinned: Pin<&mut dyn Future<Output = R>> = Pin::as_mut(&mut fut);
    loop {
        match pinned.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

#[gpui::test]
fn hydrate_primed_value_reaches_mounted_use_query_observer(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();

    let mut snap = PersistSnapshot {
        entries: Default::default(),
        version: crate::client::PERSIST_VERSION,
    };
    snap.entries.insert(
        "hydrate-notify".to_string(),
        PersistedEntry {
            value: serde_json::json!("hydrated"),
            cached_at: crate::client::current_time_ms(),
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    *persister.load_value.lock().unwrap() = Some(snap);

    struct H {
        _entity: Entity<QueryResource<String, QueryError>>,
        _sub: gpui::Subscription,
    }
    let harness = cx.new(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
        let (entity, sub) = use_query_manual::<String, QueryError, _>(
            QueryKey::from("hydrate-notify"),
            crate::core::CachePolicy::NoCache,
            crate::core::RequestPolicy::LatestWins,
            cx,
        );
        H {
            _entity: entity,
            _sub: sub,
        }
    });

    let hits = Arc::new(AtomicUsize::new(0));
    let hits_for_observer = hits.clone();
    let _notified = harness.update(cx, |_, cx| {
        cx.observe_self(move |_, _| {
            hits_for_observer.fetch_add(1, Ordering::SeqCst);
        })
    });

    let outcome = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &PersistFilter::All, DAY, cx))
        })
    });
    assert!(
        outcome.is_ok(),
        "hydrate should succeed: {:?}",
        outcome.err()
    );
    cx.run_until_parked();

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            let data =
                client.get_query_data::<String, QueryError>(&QueryKey::from("hydrate-notify"), cx);
            assert_eq!(
                data,
                Some("hydrated".to_string()),
                "hydrate should have primed the value"
            );
        });
    });
    assert!(
        hits.load(Ordering::SeqCst) >= 1,
        "hydrate priming via set_query_data must re-render a mounted use_query consumer"
    );
}

#[gpui::test]
fn flush_serializes_only_dirty_entries(cx: &mut TestAppContext) {
    static SERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

    fn counting_serialize(value: &String) -> serde_json::Value {
        SERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        serde_json::to_value(value).expect("serialize")
    }

    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        dirty_a: Entity<QueryResource<String, QueryError>>,
        dirty_b: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, dirty_a, dirty_b) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(counting_serialize);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let dirty_a = client.resource::<String, QueryError>(QueryKey::from("dirty_a"), cx);
            let dirty_b = client.resource::<String, QueryError>(QueryKey::from("dirty_b"), cx);
            (handle, dirty_a, dirty_b)
        });
        H {
            dirty_a,
            dirty_b,
            _handle: handle,
        }
    });
    cx.update(|cx| {
        let (dirty_a, dirty_b) =
            harness.read_with(cx, |h, _| (h.dirty_a.clone(), h.dirty_b.clone()));
        cx.update_global::<QueryClient, _>(|_client, cx| {
            for entity in [dirty_a, dirty_b] {
                entity.update(cx, |r, _| {
                    r.apply_success("v1".to_string(), crate::client::current_time_ms())
                });
            }
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        SERIALIZATIONS.load(Ordering::SeqCst),
        2,
        "the first flush serializes both live entries"
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("dirty_b", "v2".to_string(), cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(
        SERIALIZATIONS.load(Ordering::SeqCst),
        3,
        "unchanged dirty_a must not be re-serialized"
    );
    let saved = captured.lock().unwrap().clone().expect("flush 2 saved");
    assert_eq!(
        saved.entries.get("dirty_b").map(|e| &e.value),
        Some(&serde_json::json!("v2"))
    );
    assert_eq!(
        saved.entries.get("dirty_a").map(|e| &e.value),
        Some(&serde_json::json!("v1")),
        "the unchanged entry stays in the saved store"
    );
    let _ = harness;
}

#[gpui::test]
fn unchanged_cache_flushes_nothing(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let save_count = persister.save_count.clone();

    struct H {
        steady: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, steady) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let steady = client.resource::<String, QueryError>(QueryKey::from("steady"), cx);
            (handle, steady)
        });
        H {
            steady,
            _handle: handle,
        }
    });
    cx.update(|cx| {
        let entity = harness.read(cx).steady.clone();
        cx.update_global::<QueryClient, _>(|_client, cx| {
            entity.update(cx, |r, _| {
                r.apply_success("v".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        *save_count.lock().unwrap(),
        1,
        "the data write flushed exactly once"
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|_client, cx| {
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        *save_count.lock().unwrap(),
        1,
        "a bump without a data write must not save"
    );
    let _ = harness;
}

#[gpui::test]
fn set_query_data_write_flushes_without_touching_timestamp(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();
    let save_count = persister.save_count.clone();

    struct H {
        epoch_key: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, epoch_key) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let epoch_key = client.resource::<String, QueryError>(QueryKey::from("epoch_key"), cx);
            (handle, epoch_key)
        });
        H {
            epoch_key,
            _handle: handle,
        }
    });
    cx.update(|cx| {
        let entity = harness.read(cx).epoch_key.clone();
        cx.update_global::<QueryClient, _>(|_client, cx| {
            entity.update(cx, |r, _| {
                r.apply_success("v1".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    let first_cached_at = captured
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|s| s.entries.get("epoch_key"))
        .map(|e| e.cached_at)
        .expect("the first flush stored epoch_key");

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.set_query_data::<String, QueryError>("epoch_key", "v2".to_string(), cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(*save_count.lock().unwrap(), 2);
    let saved = captured.lock().unwrap().clone().expect("flush 2 saved");
    let entry = saved.entries.get("epoch_key").expect("epoch_key persisted");
    assert_eq!(entry.value, serde_json::json!("v2"));
    assert_eq!(
        entry.cached_at, first_cached_at,
        "set_data leaves last_updated_at untouched; the data epoch is the flush signal"
    );
    let _ = harness;
}

#[gpui::test]
fn removed_key_is_pruned_from_the_saved_store(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();

    struct H {
        pruned_out: Entity<QueryResource<String, QueryError>>,
        pruned_stay: Entity<QueryResource<String, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, pruned_out, pruned_stay) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let pruned_out =
                client.resource::<String, QueryError>(QueryKey::from("pruned_out"), cx);
            let pruned_stay =
                client.resource::<String, QueryError>(QueryKey::from("pruned_stay"), cx);
            (handle, pruned_out, pruned_stay)
        });
        H {
            pruned_out,
            pruned_stay,
            _handle: handle,
        }
    });
    cx.update(|cx| {
        let (out, stay) =
            harness.read_with(cx, |h, _| (h.pruned_out.clone(), h.pruned_stay.clone()));
        cx.update_global::<QueryClient, _>(|_client, cx| {
            for entity in [out, stay] {
                entity.update(cx, |r, _| {
                    r.apply_success("v1".to_string(), crate::client::current_time_ms())
                });
            }
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    let first = captured.lock().unwrap().clone().expect("flush 1 saved");
    assert!(first.entries.contains_key("pruned_out"));

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.remove_queries(&QueryKeyFilter::Exact(&QueryKey::from("pruned_out")));
            client.set_query_data::<String, QueryError>("pruned_stay", "v2".to_string(), cx);
        });
    });
    cx.run_until_parked();

    let saved = captured.lock().unwrap().clone().expect("flush 2 saved");
    assert!(
        !saved.entries.contains_key("pruned_out"),
        "a removed key must not resurrect in the saved store: {:?}",
        saved.entries.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        saved.entries.get("pruned_stay").map(|e| &e.value),
        Some(&serde_json::json!("v2"))
    );
    let _ = harness;
}

#[gpui::test]
fn colon_collision_keys_persist_distinct_and_hydrate_exact(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let key_a = QueryKey::from(["a::", ""]);
    let key_b = QueryKey::from(["a", "::"]);

    let snapshot = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            for (key, value) in [(&key_a, "alpha"), (&key_b, "beta")] {
                let e = client.resource::<String, QueryError>(key.clone(), cx);
                e.update(cx, |r, _| {
                    r.apply_success(value.to_string(), crate::client::current_time_ms())
                });
            }
            client.collect_persist_snapshot(&PersistFilter::All, DAY, cx)
        })
    });
    assert_ne!(key_a.to_path(), key_b.to_path());
    assert_eq!(
        snapshot.entries.len(),
        2,
        "the collision pair must map to distinct paths: {:?}",
        snapshot.entries.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        snapshot.entries.get(&key_a.to_path()).map(|e| &e.value),
        Some(&serde_json::json!("alpha"))
    );
    assert_eq!(
        snapshot.entries.get(&key_b.to_path()).map(|e| &e.value),
        Some(&serde_json::json!("beta"))
    );
    *persister.load_value.lock().unwrap() = Some(snapshot);

    for (filter_key, expected, other) in [
        (key_a.clone(), "alpha", &key_b),
        (key_b.clone(), "beta", &key_a),
    ] {
        let mut fresh = QueryClient::new();
        fresh.register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        let held = cx.update(|cx| {
            vec![
                fresh.resource::<String, QueryError>(key_a.clone(), cx),
                fresh.resource::<String, QueryError>(key_b.clone(), cx),
            ]
        });
        let outcome = cx.update(|cx| {
            block_on_ready(hydrate(
                &mut fresh,
                &persister,
                &PersistFilter::Exact(filter_key.clone()),
                DAY,
                cx,
            ))
        });
        assert!(
            outcome.is_ok(),
            "hydrate should succeed: {:?}",
            outcome.err()
        );
        cx.update(|cx| {
            assert_eq!(
                fresh.get_query_data::<String, QueryError>(&filter_key, cx),
                Some(expected.to_string()),
                "Exact must match the reconstructed segments"
            );
            assert_eq!(
                fresh.get_query_data::<String, QueryError>(other, cx),
                None,
                "the other collision key must not be primed by this Exact filter"
            );
        });
        drop(held);
    }
}

#[gpui::test]
fn hydrate_discards_previous_format_version(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let mut stale = PersistSnapshot {
        entries: Default::default(),
        version: crate::client::PERSIST_VERSION - 1,
    };
    stale.entries.insert(
        "old_format_key".to_string(),
        PersistedEntry {
            value: serde_json::json!("stale"),
            cached_at: crate::client::current_time_ms(),
            cache_policy: crate::core::CachePolicy::default(),
            meta: None,
        },
    );
    *persister.load_value.lock().unwrap() = Some(stale);

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, _cx| {
            client
                .register_deserializer::<String, QueryError>(|v| v.as_str().map(|s| s.to_string()));
        });
        let outcome = cx.update_global::<QueryClient, _>(|client, cx| {
            block_on_ready(hydrate(client, &persister, &PersistFilter::All, DAY, cx))
        });
        match outcome {
            Err(PersistError::VersionMismatch { expected, found }) => {
                assert_eq!(expected, crate::client::PERSIST_VERSION);
                assert_eq!(found, crate::client::PERSIST_VERSION - 1);
            }
            other => panic!("expected VersionMismatch, got {other:?}"),
        }
        cx.update_global::<QueryClient, _>(|client, cx| {
            assert_eq!(
                client.get_query_data::<String, QueryError>(&QueryKey::from("old_format_key"), cx),
                None,
                "a previous-format snapshot must not prime any value"
            );
        });
    });
}

#[gpui::test]
fn infinite_first_page_reuses_flushed_payload_until_pages_change(cx: &mut TestAppContext) {
    static SERIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

    fn counting_serialize(value: &Vec<String>) -> serde_json::Value {
        SERIALIZATIONS.fetch_add(1, Ordering::SeqCst);
        serde_json::to_value(value).expect("serialize")
    }

    setup_query_client(cx);
    let persister = MemPersister::default();
    let save_count = persister.save_count.clone();

    struct H {
        feed: Entity<InfiniteQueryResource<Vec<String>, QueryError>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (handle, feed) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<Vec<String>, QueryError>(counting_serialize);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            let feed = client
                .infinite_resource::<Vec<String>, QueryError>(QueryKey::from("inf-flush"), cx);
            (handle, feed)
        });
        H {
            feed,
            _handle: handle,
        }
    });
    cx.update(|cx| {
        let feed = harness.read_with(cx, |h, _| h.feed.clone());
        cx.update_global::<QueryClient, _>(|_client, cx| {
            feed.update(cx, |r, _| {
                let mut seq = crate::core::RequestSequencer::new();
                let now = crate::client::current_time_ms();
                let id = r.begin_fetch_next(&mut seq, now).expect("fetch starts");
                assert!(r.complete_page_success(id, vec!["page-0".to_string()], true, true, now));
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        SERIALIZATIONS.load(Ordering::SeqCst),
        1,
        "the first flush serializes the fresh first page"
    );

    cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|_client, cx| {
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        SERIALIZATIONS.load(Ordering::SeqCst),
        1,
        "an unchanged first page must not be re-serialized"
    );
    assert_eq!(
        *save_count.lock().unwrap(),
        1,
        "nothing dirty means no save"
    );

    cx.update(|cx| {
        let feed = harness.read_with(cx, |h, _| h.feed.clone());
        cx.update_global::<QueryClient, _>(|_client, cx| {
            feed.update(cx, |r, _| {
                let mut seq = crate::core::RequestSequencer::new();
                let now = crate::client::current_time_ms();
                r.set_has_previous_page(true);
                let id = r
                    .begin_fetch_previous(&mut seq, now)
                    .expect("previous fetch starts");
                assert!(r.complete_page_success(id, vec!["page-1".to_string()], false, false, now));
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        SERIALIZATIONS.load(Ordering::SeqCst),
        2,
        "a page write must re-serialize"
    );
    assert_eq!(*save_count.lock().unwrap(), 2);
    let saved = persister
        .last_saved
        .lock()
        .unwrap()
        .clone()
        .expect("flush 2 saved");
    assert_eq!(
        saved.entries.get("inf-flush").map(|e| &e.value),
        Some(&serde_json::json!(["page-1"])),
        "the re-serialized payload must carry the new first page"
    );
    let _ = harness;
}

#[gpui::test]
#[ignore = "probe: quantitative, run with --ignored"]
fn integration_persist_collect_delta_vs_full_cost(cx: &mut TestAppContext) {
    use std::collections::HashMap;
    use std::time::Instant;

    setup_query_client(cx);
    let held: Vec<Entity<QueryResource<String, QueryError>>> = cx.update(|cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            (0..2_000usize)
                .map(|i| {
                    let e = client
                        .resource::<String, QueryError>(QueryKey::from(format!("probe/{i}")), cx);
                    e.update(cx, |r, _| {
                        r.apply_success(format!("value-{i}"), crate::client::current_time_ms())
                    });
                    e
                })
                .collect()
        })
    });

    let (full, delta, reused, flushed) = cx.update(|cx| {
        let filter = PersistFilter::All;
        let day = DAY;
        let mut flushed: HashMap<String, (gpui::EntityId, u64)> = HashMap::new();
        cx.update_global::<QueryClient, _>(|client, cx| {
            let first = client.collect_persist_delta(&filter, day, &flushed, cx);
            for c in &first.fresh {
                flushed.insert(c.path.clone(), (c.entity_id, c.epoch));
            }
            let full_start = Instant::now();
            for _ in 0..100 {
                std::hint::black_box(client.collect_persist_snapshot(&filter, day, cx));
            }
            let full = full_start.elapsed() / 100;
            let delta_start = Instant::now();
            let mut reused_count = 0usize;
            for _ in 0..100 {
                let out = client.collect_persist_delta(&filter, day, &flushed, cx);
                reused_count = out.reused.len();
                std::hint::black_box(out);
            }
            let delta = delta_start.elapsed() / 100;
            (full, delta, reused_count, flushed.len())
        })
    });
    println!(
        "probe persist-collect over 2000 live Success entries, 100 sweeps: full-collect {full:?}/sweep, delta-collect {delta:?}/sweep (reused={reused}, flushed={flushed})"
    );
    let _ = held;
}

#[gpui::test]
fn evicted_and_recreated_entry_reserializes_at_matching_write_count(cx: &mut TestAppContext) {
    setup_query_client(cx);
    let persister = MemPersister::default();
    let captured = persister.last_saved.clone();
    let save_count = persister.save_count.clone();

    struct H {
        _first: Entity<QueryResource<String, QueryError>>,
        second: Option<Entity<QueryResource<String, QueryError>>>,
        _handle: PersistHandle,
    }
    let harness = cx.new(|cx| {
        let (first, handle) = cx.update_global::<QueryClient, _>(|client, cx| {
            client.register_serializer::<String, QueryError>(ser_string);
            let first = client.resource::<String, QueryError>(QueryKey::from("recreated"), cx);
            let handle = client.persist_with(persister.clone(), zero_debounce(), cx);
            (first, handle)
        });
        H {
            _first: first,
            second: None,
            _handle: handle,
        }
    });

    cx.update(|cx| {
        let first = harness.read_with(cx, |h, _| h._first.clone());
        cx.update_global::<QueryClient, _>(|_client, cx| {
            first.update(cx, |r, _| {
                r.apply_success("v1".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
        });
    });
    cx.run_until_parked();
    assert_eq!(*save_count.lock().unwrap(), 1);

    harness.update(cx, |h, cx| {
        cx.update_global::<QueryClient, _>(|client, cx| {
            client.remove_queries(&QueryKeyFilter::Exact(&QueryKey::from("recreated")));
            let second = client.resource::<String, QueryError>(QueryKey::from("recreated"), cx);
            second.update(cx, |r, _| {
                r.apply_success("v2".to_string(), crate::client::current_time_ms())
            });
            cx.default_global::<CacheMutation>();
            h.second = Some(second);
        });
    });
    cx.run_until_parked();

    let saved = captured.lock().unwrap().clone().expect("flush 2 saved");
    assert_eq!(
        saved.entries.get("recreated").map(|e| &e.value),
        Some(&serde_json::json!("v2")),
        "a recreated entry whose write count matches its pre-eviction flushed \
         epoch must still re-serialize; the flushed gate needs entity identity"
    );
    let _ = harness;
}
