//! `shared` holds the machinery common to [`QueryBucket`] and
//! [`InfiniteQueryBucket`](crate::client::InfiniteQueryBucket): weak-entity
//! entries, sequencers, eviction, GC, and bulk key-filter operations.

mod erased_ops;
mod ops;
pub(crate) mod shared;
pub(crate) mod types;

pub use ops::QueryBucket;
