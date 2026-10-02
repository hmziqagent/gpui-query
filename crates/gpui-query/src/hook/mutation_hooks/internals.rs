//! The retry loop's `Fn(&V)` mutator borrows the variables from the stored
//! `Arc<V>` (no `V::clone` per retry); the `Fn(V)` public entrypoints adapt
//! at the call site.

use std::sync::Arc;

use crate::core::{MutationResource, RetryPolicy};

use super::super::options::MutationCallbacks;

use crate::hook::read_entity;

/// Retries via `increment_retry()` + `prepare_retry()` so observers never see
/// a transient Failure between attempts; only exhausted retries produce a
/// terminal `complete_failure()`. Stops once the mutation leaves Loading
/// (cancelled or reset); intermediate calls don't notify (observer dedupes).
pub(super) async fn run_mutation_loop<V, T, E, F, Fut>(
    weak: &gpui::WeakEntity<MutationResource<V, T, E>>,
    variables: Arc<V>,
    mutator: F,
    retry_policy: &RetryPolicy,
    callbacks: Option<MutationCallbacks<T, E>>,
    cx: &mut gpui::AsyncApp,
) where
    V: Send + Sync + 'static,
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    F: Fn(&V) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let needs_data =
        |cb: &MutationCallbacks<T, E>| cb.on_success.is_some() || cb.on_settled.is_some();
    let mut attempt: u32 = 0;

    loop {
        let result = mutator(&*variables).await;

        match result {
            Ok(data) => {
                let Some(entity) = weak.upgrade() else {
                    if let Some(ref cb) = callbacks
                        && let Some(ref f) = cb.on_settled
                    {
                        f(None, None);
                    }
                    return;
                };

                // A cancel/reset while the mutator was awaited leaves a
                // terminal state a late Ok must not overwrite.
                if !read_entity(&entity, cx, |r, _| r.is_loading()).unwrap_or(false) {
                    let terminal_error =
                        read_entity(&entity, cx, |r, _| r.error().cloned()).flatten();
                    fire_error_callbacks(&callbacks, &terminal_error);
                    return;
                }

                let data_for_callback = callbacks
                    .as_ref()
                    .is_some_and(needs_data)
                    .then(|| data.clone());
                entity.update(cx, |resource, cx| {
                    resource.complete_success(data);
                    resource.reset_retry_count();
                    cx.notify();
                    #[cfg(feature = "persist")]
                    cx.default_global::<crate::client::CacheMutation>();
                });

                // Fire outside the entity borrow so callbacks can call entity.update().
                if let Some(ref cb) = callbacks {
                    if let Some(ref d) = data_for_callback
                        && let Some(ref f) = cb.on_success
                    {
                        f(d);
                    }
                    if let Some(ref f) = cb.on_settled {
                        f(data_for_callback.as_ref(), None);
                    }
                }

                return;
            }
            Err(error) => {
                let error_for_callback = callbacks
                    .as_ref()
                    .is_some_and(|cb| cb.on_error.is_some() || cb.on_settled.is_some())
                    .then(|| error.clone());

                if retry_policy.should_retry(attempt) {
                    let delay_ms = retry_policy.delay_for_attempt(attempt);

                    let Some(entity) = weak.upgrade() else {
                        fire_error_callbacks(&callbacks, &error_for_callback);
                        return;
                    };
                    entity.update(cx, |resource, _cx| {
                        resource.increment_retry();
                    });

                    if delay_ms > 0 {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(delay_ms))
                            .await;
                    }

                    let Some(entity) = weak.upgrade() else {
                        fire_error_callbacks(&callbacks, &error_for_callback);
                        return;
                    };
                    if !read_entity(&entity, cx, |r, _| r.is_loading()).unwrap_or(false) {
                        fire_error_callbacks(&callbacks, &error_for_callback);
                        return;
                    }

                    entity.update(cx, |resource, _cx| {
                        resource.prepare_retry();
                    });

                    attempt += 1;
                } else {
                    if let Some(entity) = weak.upgrade() {
                        entity.update(cx, |resource, cx| {
                            resource.complete_failure(error);
                            resource.reset_retry_count();
                            cx.notify();
                            #[cfg(feature = "persist")]
                            cx.default_global::<crate::client::CacheMutation>();
                        });
                    }

                    fire_error_callbacks(&callbacks, &error_for_callback);
                    return;
                }
            }
        }
    }
}

/// Fires `on_error` / `on_settled` whether or not the entity survived.
fn fire_error_callbacks<T, E>(
    callbacks: &Option<MutationCallbacks<T, E>>,
    error_for_callback: &Option<E>,
) {
    if let Some(cb) = callbacks {
        if let Some(ec) = error_for_callback
            && let Some(ref f) = cb.on_error
        {
            f(ec);
        }
        if let Some(ref f) = cb.on_settled {
            f(None, error_for_callback.as_ref());
        }
    }
}
