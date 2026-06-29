//! Shasta-specific primitives for Raiko V2.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod anchor;
mod blob;
mod fixture;
mod input;
pub mod instance;
mod proof;

pub use anchor::{
    anchor_max_offset_for_chain, should_bypass_stalled_anchor_linkage, validate_anchor_progression,
    MAINNET_WINDOW_CHAIN_IDS,
};
pub use blob::verify_proposal_mode_blob_usage;
pub use fixture::{
    DEFAULT_GUEST_INPUT_ROOT, guest_input_proposal_path, guest_input_proposals_dir,
    guest_input_suite_path, guest_input_suites_dir, parse_guest_input_proposal_id,
    validate_fixture_key,
};
pub use input::{
    ANCESTOR_HEADER_WINDOW_LIMIT, GuestInput, ShastaRawAggregationGuestInput,
    ShastaRisc0AggregationGuestInput, ShastaZkAggregationGuestInput,
    roll_proposal_ancestor_headers, roll_proposal_ancestor_headers_in_place,
};
pub use proof::{
    build_proof_carry_data, decode_proof_carry_data, decode_proof_carry_data_opt,
    encode_proof_carry_data, proof_carry_from_proof,
};
