//! Shasta-specific primitives for Raiko V2.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod blob;
mod input;
pub mod instance;
mod proof;

pub use blob::verify_proposal_mode_blob_usage;
pub use input::{GuestInput, ShastaRawAggregationGuestInput, ShastaZkAggregationGuestInput};
pub use proof::{decode_proof_carry_data, decode_proof_carry_data_opt, encode_proof_carry_data};
