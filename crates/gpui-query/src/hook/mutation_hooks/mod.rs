//! Mutation hooks and internal retry loops.

mod hooks;
mod internals;

pub use hooks::{
    mutate, mutate_arc, mutate_by_ref, mutate_with_callbacks, use_mutation, use_mutation_state,
};
