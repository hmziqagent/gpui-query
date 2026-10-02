//! `use_query`, `use_mutation`, and `use_infinite_query` hooks; each returns
//! its resource entity plus the observer `Subscription` that keeps it live.

mod fetch_retry;
mod gpui_compat;
mod mutation_hooks;
mod options;
mod query_hooks;
mod use_infinite_query;
mod use_query_select;

pub(crate) use gpui_compat::read_entity;

pub use options::{InfiniteQueryOptions, MutationCallbacks, MutationOptions, QueryOptions};

pub use query_hooks::{
    fetch_query, fetch_query_with_policy, fetch_query_with_signal, use_query, use_query_manual,
    use_query_manual_opts, use_query_unsignalled, use_query_unsignalled_opts,
    use_query_with_policy,
};

pub use use_infinite_query::{
    fetch_next_page_infinite, fetch_previous_page_infinite, use_infinite_query,
};

pub use use_query_select::use_query_select;

pub use mutation_hooks::{
    mutate, mutate_arc, mutate_by_ref, mutate_with_callbacks, use_mutation, use_mutation_state,
};

/// Milliseconds since the UNIX epoch; pre-epoch clocks clamp to `0` (treated as stale).
#[inline]
pub fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Lets `use_mutation((), cx)` use the default options.
impl From<()> for MutationOptions {
    fn from((): ()) -> Self {
        Self::default()
    }
}
