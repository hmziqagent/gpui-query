//! Retry-aware page-fetch runner with two-phase completion.

use std::sync::Arc;

use crate::core::{InfiniteQueryResource, RequestId};

use crate::hook::{current_time_ms, read_entity};

/// The runner is direction-agnostic; this carries the cursor page and the
/// `is_next` flag passed to
/// [`InfiniteQueryResource::complete_success_with_guard`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PageDirection {
    Next,
    Previous,
}

impl PageDirection {
    fn is_next(self) -> bool {
        matches!(self, PageDirection::Next)
    }

    /// Last page for `Next`, first for `Previous`; the `Arc<T>` accessor is a refcount bump, no page clone.
    fn cursor_page_arc<T: Clone + Send + Sync + 'static, E>(
        self,
        resource: &InfiniteQueryResource<T, E>,
    ) -> Option<Arc<T>> {
        match self {
            PageDirection::Next => resource.last_page_arc(),
            PageDirection::Previous => resource.first_page_arc(),
        }
    }
}

/// Two-phase completion so a superseded request never writes; a cancelled or
/// superseded fetch stops retrying after the delay; the cursor is read once,
/// which stays accurate across retries because hook-driven page changes end
/// this request, though a manual `append_page`/`prepend_page` between
/// attempts is not covered and the next retry fetches on the old cursor.
pub(super) async fn run_fetch_page_with_id<T, E, F, Fut>(
    entity: &gpui::WeakEntity<InfiniteQueryResource<T, E>>,
    fetcher: &F,
    request_id: RequestId,
    retry_policy: &crate::core::RetryPolicy,
    cx: &mut gpui::AsyncApp,
    direction: PageDirection,
) where
    T: Clone + Send + Sync + 'static,
    E: Clone + Send + Sync + std::fmt::Debug + 'static,
    F: Fn(Option<&T>) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<(T, bool), E>> + Send + 'static,
{
    let cursor_page_arc: Option<Arc<T>> = {
        let Some(e) = entity.upgrade() else { return };
        read_entity(&e, cx, |r, _| direction.cursor_page_arc(r)).flatten()
    };

    let mut attempt: u32 = 0;

    loop {
        let result = fetcher(cursor_page_arc.as_ref().map(|a| a.as_ref())).await;

        let now_ms = current_time_ms();

        let Some(e) = entity.upgrade() else { return };

        match result {
            Ok((page, has_more)) => {
                let _ = e.update(cx, |resource, cx| {
                    if let Some(guard) = resource.accept_current_request(request_id) {
                        resource.reset_retry_count();
                        resource.complete_success_with_guard(
                            guard,
                            page,
                            has_more,
                            direction.is_next(),
                            now_ms,
                        );
                        cx.notify();
                        #[cfg(feature = "persist")]
                        cx.default_global::<crate::client::CacheMutation>();
                    }
                });
                return;
            }
            Err(error) => {
                if retry_policy.should_retry(attempt) {
                    let delay_ms = retry_policy.delay_for_attempt(attempt);
                    attempt += 1;

                    if delay_ms > 0 {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(delay_ms))
                            .await;
                    }

                    let Some(e) = entity.upgrade() else { return };
                    let (cancelled, still_current) = read_entity(&e, cx, |r, _| {
                        (
                            r.signal().map(|s| s.is_cancelled()).unwrap_or(false),
                            r.is_current_request(request_id),
                        )
                    })
                    .unwrap_or((true, false));
                    if cancelled || !still_current {
                        return;
                    }
                    let _ = e.update(cx, |resource, _cx| {
                        resource.increment_retry();
                    });
                } else {
                    let _ = e.update(cx, |resource, cx| {
                        if let Some(guard) = resource.accept_current_request(request_id) {
                            resource.reset_retry_count();
                            resource.complete_failure_with_guard(guard, error);
                            cx.notify();
                            #[cfg(feature = "persist")]
                            cx.default_global::<crate::client::CacheMutation>();
                        }
                    });
                    return;
                }
            }
        }
    }
}
