//! HTTP request handlers.

mod admin;
mod errors;
mod health;
mod metrics;
mod proof;
mod ready;
#[cfg(feature = "tdx")]
mod tdx;

pub(crate) use admin::{get_ballot, set_ballot};
pub use health::health;
pub use metrics::metrics;
pub use proof::{
    cancel_task, get_task, list_proofs, prune_proofs, report_proofs, request_aggregation_proof,
    request_batch_shasta_proof,
};
pub use ready::ready;
#[cfg(feature = "tdx")]
pub use tdx::bootstrap as tdx_bootstrap;
