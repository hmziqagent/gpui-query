//! Query and mutation options; `From<&str>`/`From<String>` let a bare key
//! stand in for a full options value.

use std::sync::Arc;

use crate::core::{CachePolicy, RefetchTrigger, RequestPolicy, RetryPolicy};

/// # Examples
///
/// ```no_run
/// use gpui_query::QueryOptions;
/// use gpui_query::core::{CachePolicy, RetryPolicy};
/// use gpui_query::hook::use_query;
/// # #[derive(Clone)]
/// # struct User;
/// # #[derive(Clone, Debug)]
/// # struct MyError;
/// # fn _doc(cx: &mut gpui::Context<()>) {
///
/// let (_entity, _sub) = use_query("users", |signal| async move {
///     Ok::<Vec<User>, MyError>(vec![])
/// }, cx);
///
/// let (_entity, _sub) = use_query(
///     QueryOptions::new("users")
///         .cache_policy(CachePolicy::Ttl { ttl_ms: 300_000 })
///         .retry_policy(RetryPolicy::new(5)),
///     |signal| async move {
///         Ok::<Vec<User>, MyError>(vec![])
///     },
///     cx,
/// );
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct QueryOptions {
    pub key: crate::core::QueryKey,
    /// Default: `Ttl { ttl_ms: 60_000 }`.
    pub cache_policy: CachePolicy,
    /// Default: `LatestWins`.
    pub request_policy: RequestPolicy,
    /// Default: 3 retries with exponential backoff.
    pub retry_policy: RetryPolicy,
    /// Default 300_000; reserved: GC runs off `QueryClient::with_gc_time`, so this field has no effect today.
    pub gc_time_ms: u64,
    /// Reserved: stored but not consumed; setting it has no effect today.
    pub keep_previous_data: bool,
    /// When `true`, `use_query` bypasses freshness checks (`QueryFetchMode::Force`).
    pub force_fetch: bool,
    /// Reserved: stored but not consumed.
    pub refetch_on_mount: RefetchTrigger,
    /// Reserved: stored but not consumed.
    pub refetch_on_window_focus: RefetchTrigger,
    /// Reserved: stored but not consumed.
    pub refetch_on_reconnect: RefetchTrigger,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            key: crate::core::QueryKey::from("default"),
            cache_policy: CachePolicy::default(),
            request_policy: RequestPolicy::default(),
            retry_policy: RetryPolicy::default(),
            gc_time_ms: 300_000,
            keep_previous_data: false,
            force_fetch: false,
            refetch_on_mount: RefetchTrigger::default(),
            refetch_on_window_focus: RefetchTrigger::default(),
            refetch_on_reconnect: RefetchTrigger::default(),
        }
    }
}

/// Builder methods shared by [`QueryOptions`] and [`InfiniteQueryOptions`] so
/// the two cannot drift.
macro_rules! impl_query_options_builders {
    ($t:ident) => {
        impl $t {
            pub fn cache_policy(mut self, policy: CachePolicy) -> Self {
                self.cache_policy = policy;
                self
            }

            pub fn request_policy(mut self, policy: RequestPolicy) -> Self {
                self.request_policy = policy;
                self
            }

            pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
                self.retry_policy = policy;
                self
            }

            pub fn gc_time(mut self, ms: u64) -> Self {
                self.gc_time_ms = ms;
                self
            }
        }
    };
}

/// `From` key conversions shared by both option types.
macro_rules! impl_key_conversions {
    ($t:ident) => {
        impl From<&str> for $t {
            fn from(key: &str) -> Self {
                Self::new(key)
            }
        }

        impl From<String> for $t {
            fn from(key: String) -> Self {
                Self::new(key)
            }
        }

        impl From<crate::core::QueryKey> for $t {
            fn from(key: crate::core::QueryKey) -> Self {
                Self::new(key)
            }
        }
    };
}

impl QueryOptions {
    pub fn new(key: impl Into<crate::core::QueryKey>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }

    pub fn force(mut self) -> Self {
        self.force_fetch = true;
        self
    }

    pub fn keep_previous(mut self) -> Self {
        self.keep_previous_data = true;
        self
    }
}

impl_query_options_builders!(QueryOptions);

impl_key_conversions!(QueryOptions);

impl From<(crate::core::QueryKey, CachePolicy, RequestPolicy)> for QueryOptions {
    fn from(
        (key, cache_policy, request_policy): (crate::core::QueryKey, CachePolicy, RequestPolicy),
    ) -> Self {
        Self {
            key,
            cache_policy,
            request_policy,
            ..Default::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct MutationOptions {
    /// Default: no retries.
    pub retry_policy: RetryPolicy,
    /// Default: 300_000.
    pub gc_time_ms: u64,
}

impl Default for MutationOptions {
    fn default() -> Self {
        Self {
            retry_policy: RetryPolicy::no_retries(),
            gc_time_ms: 300_000,
        }
    }
}

impl MutationOptions {
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    pub fn gc_time(mut self, ms: u64) -> Self {
        self.gc_time_ms = ms;
        self
    }
}

pub type MutationSuccessCallback<T> = Option<Arc<dyn Fn(&T) + Send + Sync>>;

pub type MutationErrorCallback<E> = Option<Arc<dyn Fn(&E) + Send + Sync>>;

pub type MutationSettledCallback<T, E> = Option<Arc<dyn Fn(Option<&T>, Option<&E>) + Send + Sync>>;

/// Manual `Clone` (field `Arc`s, no `T: Clone` / `E: Clone` bound); shared across concurrent invocations.
pub struct MutationCallbacks<T, E> {
    /// Fired on terminal success.
    pub on_success: MutationSuccessCallback<T>,
    /// Fired on terminal failure (retries exhausted or cancelled).
    pub on_error: MutationErrorCallback<E>,
    /// Fired on every terminal outcome.
    pub on_settled: MutationSettledCallback<T, E>,
}

impl<T, E> Clone for MutationCallbacks<T, E> {
    fn clone(&self) -> Self {
        Self {
            on_success: self.on_success.clone(),
            on_error: self.on_error.clone(),
            on_settled: self.on_settled.clone(),
        }
    }
}

impl<T, E> Default for MutationCallbacks<T, E> {
    fn default() -> Self {
        Self {
            on_success: None,
            on_error: None,
            on_settled: None,
        }
    }
}

impl<T, E> MutationCallbacks<T, E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on_success(mut self, f: impl Fn(&T) + Send + Sync + 'static) -> Self {
        self.on_success = Some(Arc::new(f));
        self
    }

    pub fn on_error(mut self, f: impl Fn(&E) + Send + Sync + 'static) -> Self {
        self.on_error = Some(Arc::new(f));
        self
    }

    pub fn on_settled(
        mut self,
        f: impl Fn(Option<&T>, Option<&E>) + Send + Sync + 'static,
    ) -> Self {
        self.on_settled = Some(Arc::new(f));
        self
    }
}

#[derive(Clone, Debug)]
pub struct InfiniteQueryOptions {
    pub key: crate::core::QueryKey,
    pub cache_policy: CachePolicy,
    pub request_policy: RequestPolicy,
    /// Default: 50; oldest pages are evicted beyond it.
    pub max_pages: Option<usize>,
    pub retry_policy: RetryPolicy,
    /// Default: 300_000.
    pub gc_time_ms: u64,
}

impl Default for InfiniteQueryOptions {
    fn default() -> Self {
        Self {
            key: crate::core::QueryKey::from("default"),
            cache_policy: CachePolicy::default(),
            request_policy: RequestPolicy::default(),
            max_pages: Some(50),
            retry_policy: RetryPolicy::default(),
            gc_time_ms: 300_000,
        }
    }
}

impl InfiniteQueryOptions {
    pub fn new(key: impl Into<crate::core::QueryKey>) -> Self {
        Self {
            key: key.into(),
            ..Self::default()
        }
    }

    /// Retained pages before old ones are evicted; see
    /// [`InfiniteQueryOptions::unbounded_pages`] for no limit.
    pub fn max_pages(mut self, max: usize) -> Self {
        self.max_pages = Some(max);
        self
    }

    /// No limit: page storage grows without bound if the user scrolls far enough.
    pub fn unbounded_pages(mut self) -> Self {
        self.max_pages = None;
        self
    }
}

impl_query_options_builders!(InfiniteQueryOptions);

impl_key_conversions!(InfiniteQueryOptions);
