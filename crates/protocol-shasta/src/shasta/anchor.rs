//! Shasta anchor transaction construction from taiko-client-rs.

pub use taiko_client_protocol::shasta::{
    AnchorTransactionValidationError,
    AnchorTxConstructor,
    AnchorTxConstructorError,
    AnchorV4Input,
    // Structural / golden-touch check only — not the consensus-level validator from
    // `alethia_reth_consensus::validation::validate_anchor_transaction` used by the guest.
    validate_anchor_transaction as validate_anchor_transaction_structure,
};
