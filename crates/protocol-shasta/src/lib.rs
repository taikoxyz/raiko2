#![allow(missing_docs)]
#![allow(unreachable_pub)]
#![allow(clippy::redundant_pub_crate)]

//! Raiko V2 Shasta protocol types and codecs.
//!
//! This crate provides Taiko Shasta protocol types and codecs.
//! These types are compatible with taiko-client-rs and used for:
//!
//! - Decoding Shasta inbox events (Proposed, Proved)
//! - Encoding/decoding derivation source manifests
//! - Block manifest structures for proposals
//! - Taiko proposal manifest types for zkVM guest programs

pub mod libhash;
pub mod shasta;

/// Shasta-specialized prover data (checkpoint-aware).
pub type TaikoProverData = raiko2_protocol::TaikoProverData<shasta::Checkpoint>;

/// Shasta-specialized manifest.
pub type TaikoManifest =
    raiko2_protocol::TaikoManifest<shasta::ShastaEventData, shasta::Checkpoint>;
