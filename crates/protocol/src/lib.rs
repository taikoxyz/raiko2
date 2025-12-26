#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko V2 protocol core types.
//!
//! This crate provides protocol-agnostic types shared across hardforks.
//! Hardfork-specific codecs and data live in fork-specific crates.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use raiko2_protocol::{TaikoManifest, TaikoProverData, InputDataSource};
//!
//! let manifest: TaikoManifest<(), ()> = TaikoManifest::default();
//! let prover_data: TaikoProverData<()> = TaikoProverData::default();
//! let _ = InputDataSource::default();
//! ```

mod manifest;

// Re-export core manifest types
pub use manifest::{
    BlobProofType, InputDataSource, ManifestChainSpec, TaikoManifest, TaikoProverData,
};
