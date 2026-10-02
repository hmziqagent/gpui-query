//! gpui-query: async state management for GPUI, inspired by TanStack Query.
//! Layers, strictly additive: `core` (serde-only state machine), `client`
//! (GPUI registry), `hook` (`use_query` & friends), `persist` (disk cache).

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "core")]
#[cfg_attr(docsrs, doc(cfg(feature = "core")))]
pub mod core;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;

#[cfg(feature = "hook")]
#[cfg_attr(docsrs, doc(cfg(feature = "hook")))]
pub mod hook;

// current_time_ms is defined identically in client and hook; the duplicate glob re-export is harmless.
#[cfg(feature = "core")]
#[cfg_attr(docsrs, doc(cfg(feature = "core")))]
pub use core::*;

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
#[allow(ambiguous_glob_reexports)]
pub use client::*;

#[cfg(feature = "hook")]
#[cfg_attr(docsrs, doc(cfg(feature = "hook")))]
pub use hook::*;

#[cfg(test)]
mod tests;

#[cfg(all(doctest, feature = "hook"))]
mod readme_doctests {
    #[doc = include_str!("../README.md")]
    struct Readme;
}

#[cfg(all(doctest, feature = "persist"))]
mod repo_readme_doctests {
    #[doc = include_str!("../../../README.md")]
    struct RepoReadme;
}
