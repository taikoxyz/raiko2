//! Shasta-specific primitives for Raiko V2.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod anchor;
mod blob;
mod input;
pub mod instance;
mod proof;

pub use anchor::{anchor_max_offset_for_chain, validate_anchor_progression};
pub use blob::verify_proposal_mode_blob_usage;
pub use input::{
    ANCESTOR_HEADER_WINDOW_LIMIT, GuestInput, ShastaRawAggregationGuestInput,
    ShastaZkAggregationGuestInput, roll_proposal_ancestor_headers,
    roll_proposal_ancestor_headers_in_place,
};
pub use proof::{
    build_proof_carry_data, decode_proof_carry_data, decode_proof_carry_data_opt,
    encode_proof_carry_data, proof_carry_from_proof,
};
