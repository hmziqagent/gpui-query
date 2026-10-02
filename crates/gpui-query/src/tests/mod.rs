mod core_cache;
mod core_error;
mod core_infinite_query;
mod core_lifecycle;
mod core_mutation;
mod core_policy_types;
mod core_request;
mod core_resource_advanced;
mod core_select;
#[cfg(feature = "hook")]
mod hook_tests;
#[cfg(feature = "client")]
mod integration_client;
#[cfg(feature = "hook")]
mod integration_client_coverage;
mod property_tests;
mod test_support;
