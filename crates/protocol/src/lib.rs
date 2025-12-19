#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko V2 Protocol Types
//!
//! This crate provides Taiko Shasta protocol types and codecs.
//! These types are compatible with taiko-client-rs and used for:
//!
//! - Decoding Shasta inbox events (Proposed, Proved)
//! - Encoding/decoding derivation source manifests
//! - Block manifest structures for batch proposals
//! - Taiko batch manifest types for zkVM guest programs
//!
//! ## Usage
//!
//! ```rust,ignore
//! use raiko2_protocol::{
//!     ShastaEventData,
//!     TaikoManifest, TaikoProverData, InputDataSource,
//! };
//!
//! // Decode a proposed event
//! let event_data = ShastaEventData::from_event_data(&bytes)?;
//!
//! // Create a manifest for proof generation
//! let manifest = TaikoManifest::default();
//! ```

#![allow(missing_docs)]

mod libhash;
mod manifest;
mod shasta;

// Re-export shasta types
pub use libhash::*;
pub use shasta::*;

// Re-export manifest types
pub use manifest::{
    BlobProofType, InputDataSource, ManifestChainSpec, TaikoManifest, TaikoProverData,
};
