//! Raiko2 primitives - core types for the Raiko V2 prover.
//!
//! This crate provides the foundational types used throughout the Raiko V2 system,
//! including input/output types for guest programs, proof types, and error handling.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

mod chain_spec;
mod context;
mod error;
pub mod guest;
mod input;
pub mod instance;
mod output;
mod proof;
pub(crate) mod proof_type;

pub use chain_spec::{ChainSpec, SupportedChainSpecs};
pub use context::{ProofContext, ProofRequest};
pub use error::{RaikoError, RaikoResult};
pub use input::{
    AggregationGuestInput, BlobProofType, GuestInput, RawAggregationGuestInput, RawProof,
    ShastaRawAggregationGuestInput, ShastaZkAggregationGuestInput, StatelessInput, TaikoManifest,
    TaikoProverData,
};
pub use output::{AggregationGuestOutput, GuestBatchOutput, GuestOutput};
pub use proof::{IdStore, IdWrite, Proof, ProofKey, ProverConfig, ProverError, ProverResult};
