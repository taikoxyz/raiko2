//! Raiko2 primitives - core types for the Raiko V2 prover.
//!
//! This crate provides the foundational types used throughout the Raiko V2 system,
//! including input/output types for guest programs, proof types, and error handling.

#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

pub mod blob;
pub mod chain_spec;
mod context;
mod error;
mod input;
mod output;
pub mod proof;
pub mod proof_type;
mod serde_bincode;
mod stateless;

pub use chain_spec::{ChainSpec, SupportedChainSpecs};
pub use context::{L2BlockRange, ProofContext, ProofRequest, ShastaRequest};
pub use error::{RaikoError, RaikoResult};
pub use input::{AggregationGuestInput, RawAggregationGuestInput, RawProof, StatelessInput};
pub use output::{AggregationGuestOutput, GuestOutput, GuestProposalOutput};
pub use proof::{IdStore, IdWrite, Proof, ProofKey, ProverConfig};
pub use proof_type::ProofType;
pub use stateless::{ExecutionWitness, StatelessValidationError};
