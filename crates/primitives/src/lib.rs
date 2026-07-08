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
mod opcode_lab;
mod output;
mod precompile_lab;
pub mod proof;
pub mod proof_type;
mod serde_bincode;
mod stateless;

pub use chain_spec::{
    ChainSpec, SHASTA_CHECKPOINT_VERSION, SupportedChainSpecs,
    shasta_checkpoint_storage_slot_candidates, shasta_checkpoint_storage_slots,
    shasta_checkpoint_storage_slots_nested, storage_slot_key,
};
pub use context::{
    L2BlockRange, PreflightOptions, PreflightRpcClientConfig, PreflightRpcRetryConfig,
    ProofContext, ProofRequest, ShastaCheckpoint, ShastaRequest,
};
pub use error::{RaikoError, RaikoResult};
pub use input::{AggregationGuestInput, RawAggregationGuestInput, RawProof, StatelessInput};
pub use opcode_lab::OpcodeLabInput;
pub use output::{AggregationGuestOutput, GuestOutput, GuestProposalOutput};
pub use precompile_lab::PrecompileLabInput;
pub use proof::{IdStore, IdWrite, Proof, ProofKey, ProverConfig};
pub use proof_type::ProofType;
pub use serde_bincode::EthereumBlock;
pub use stateless::{ExecutionWitness, StatelessValidationError, WitnessHeader, WitnessStateNode};
