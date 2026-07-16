#[path = "proof_api.rs"]
mod proof_api;
#[path = "proof_route.rs"]
mod proof_route;
#[path = "proof_types.rs"]
mod proof_types;

pub(crate) use proof_api::{migrate_legacy_queue_namespaces_on_startup, v3, v4};
