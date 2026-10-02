//! `use_infinite_query` plus the public `fetch_*_page_infinite` helpers.

mod fetch_helpers;
mod fetch_runners;
mod hook;

pub use fetch_helpers::{fetch_next_page_infinite, fetch_previous_page_infinite};
pub use hook::use_infinite_query;
