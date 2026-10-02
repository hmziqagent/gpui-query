//! Layer 0: transport-agnostic query lifecycle primitives, serde-only.
//! Fetch protocol: `begin_request` → `accept_current_request` (single-use
//! `RequestGuard`) → `complete_success`/`complete_failure`.

pub(crate) mod error;
mod fetched;
mod infinite_query;
mod key;
mod key_filter;
mod mutation;
mod network_mode;
mod policy;
mod refetch;
mod request;
mod resource;
mod retry;
mod select;
mod signal;
mod status;

pub use error::{QueryError, QueryErrorKind};
pub use fetched::Fetched;
pub use infinite_query::{FetchDirection, InfiniteQueryResource};
pub use key::QueryKey;
pub use key_filter::QueryKeyFilter;
pub use mutation::{MutationResource, MutationStatus};
pub use network_mode::NetworkMode;
pub use policy::{CachePolicy, QueryBeginResult, QueryFetchMode, RequestPolicy};
pub use refetch::RefetchTrigger;
pub use request::{QueryTimestamp, RequestGuard, RequestId, RequestSequencer};
pub use resource::QueryResource;
pub use retry::RetryPolicy;
pub use select::{MappedQueryResource, SelectTransform};
pub use signal::QuerySignal;
pub use status::QueryStatus;

#[cfg(feature = "client")]
mod current_task {
    //! `gpui::Task<T>` is Debug but not Clone/PartialEq/Eq, and several resource
    //! structs derive those: Clone yields an empty handle, all instances compare
    //! equal, and Drop aborts the task (gpui semantics), so `set` replaces.
    use gpui::Task;

    #[derive(Debug, Default)]
    pub(crate) struct CurrentTask(Option<Task<()>>);

    impl Clone for CurrentTask {
        fn clone(&self) -> Self {
            Self(None)
        }
    }

    impl PartialEq for CurrentTask {
        fn eq(&self, _other: &Self) -> bool {
            true
        }
    }

    impl Eq for CurrentTask {}

    impl CurrentTask {
        pub(crate) fn set(&mut self, task: Task<()>) {
            self.0 = Some(task);
        }
    }
}
