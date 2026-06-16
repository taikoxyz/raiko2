//! HTTP request handlers.

mod admin;
mod errors;
mod health;
mod metrics;
mod proof;
mod ready;

pub(crate) use admin::{get_ballot, set_ballot};
pub use health::health;
pub use metrics::metrics;
pub use proof::{
    cancel_task, clear_prover, get_prover_status, get_task, list_proofs, prune_proofs,
    report_proofs, request_aggregation_proof, request_batch_shasta_proof,
};
pub use ready::ready;
