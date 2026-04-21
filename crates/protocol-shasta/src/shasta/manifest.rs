//! Shasta derivation source manifest types and codecs.
//!
//! This module follows the Shasta protocol payload format described in Taiko's protocol docs.

use std::convert::TryFrom;
use std::io::{Read, Write};

use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, U256};
use alloy_rlp::{Decodable, Encodable, RlpDecodable, RlpEncodable};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize};

/// The maximum number of blocks allowed in a proposal.
pub const PROPOSAL_MAX_BLOCKS: usize = 192;

/// The current version of the Shasta protocol payload format.
pub const SHASTA_PAYLOAD_VERSION: u8 = 0x1;

/// Errors returned while encoding/decoding a Shasta manifest payload.
#[derive(Debug)]
pub enum ProtocolError {
    /// Invalid payload header or format.
    InvalidPayload(String),
    /// Zlib compression/decompression failure.
    Compression(String),
    /// IO failure during encoding/decoding.
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPayload(message) => write!(f, "invalid payload: {message}"),
            Self::Compression(message) => write!(f, "compression: {message}"),
            Self::Io(error) => write!(f, "io: {error}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<std::io::Error> for ProtocolError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Manifest of a single block proposal, matching `LibManifest.ProtocolBlockManifest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, RlpEncodable, RlpDecodable)]
#[serde(rename_all = "camelCase")]
pub struct BlockManifest {
    /// The timestamp of the block.
    pub timestamp: u64,
    /// The block coinbase.
    pub coinbase: Address,
    /// The anchor block number used for derivation.
    pub anchor_block_number: u64,
    /// The block gas limit.
    pub gas_limit: u64,
    /// Transactions that make up the block.
    #[serde(default)]
    pub transactions: Vec<TxEnvelope>,
}

/// Manifest for a derivation source, matching `LibManifest.DerivationSourceManifest`.
#[derive(Debug, Clone, Serialize, Deserialize, RlpEncodable, RlpDecodable)]
#[serde(rename_all = "camelCase")]
pub struct DerivationSourceManifest {
    /// Blocks included in this source.
    pub blocks: Vec<BlockManifest>,
}

impl Default for DerivationSourceManifest {
    fn default() -> Self {
        Self {
            blocks: vec![BlockManifest::default()],
        }
    }
}

impl DerivationSourceManifest {
    /// Encode and compress the derivation source manifest following the Shasta protocol payload
    /// format.
    ///
    /// # Errors
    ///
    /// Returns an error if the manifest cannot be RLP-encoded or if the payload cannot be
    /// compressed.
    pub fn encode_and_compress(&self) -> Result<Vec<u8>> {
        encode_manifest_payload(self)
    }

    /// Decompress and decode a derivation source manifest from the Shasta protocol payload bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload header is invalid or the payload cannot be decompressed.
    /// Returns an error if the decoded bytes are not a valid derivation source manifest or if the
    /// manifest exceeds the protocol block limit.
    pub fn decompress_and_decode(bytes: &[u8], offset: usize) -> Result<Self> {
        let decoded = decode_manifest_payload(bytes, offset)?;
        let mut decoded_slice = decoded.as_slice();
        let manifest = <Self as Decodable>::decode(&mut decoded_slice).map_err(|err| {
            ProtocolError::InvalidPayload(format!(
                "failed to decode derivation manifest RLP: {err}"
            ))
        })?;

        if manifest.blocks.len() > PROPOSAL_MAX_BLOCKS {
            return Err(ProtocolError::InvalidPayload(format!(
                "manifest contains {} blocks, max {PROPOSAL_MAX_BLOCKS}",
                manifest.blocks.len()
            )));
        }

        Ok(manifest)
    }
}

fn encode_manifest_payload<T>(manifest: &T) -> Result<Vec<u8>>
where
    T: Encodable,
{
    let rlp_encoded = alloy_rlp::encode(manifest);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&rlp_encoded)?;
    let compressed = encoder.finish()?;

    let mut output = Vec::with_capacity(64 + compressed.len());

    let mut version_bytes = [0u8; 32];
    version_bytes[31] = SHASTA_PAYLOAD_VERSION;
    output.extend_from_slice(&version_bytes);

    let len_bytes = U256::from(compressed.len()).to_be_bytes::<32>();
    output.extend_from_slice(&len_bytes);
    output.extend_from_slice(&compressed);

    Ok(output)
}

fn decode_manifest_payload(bytes: &[u8], offset: usize) -> Result<Vec<u8>> {
    if bytes.len() < offset + 64 {
        return Err(ProtocolError::InvalidPayload(format!(
            "payload too short for header: expected at least {} bytes, got {}",
            offset + 64,
            bytes.len()
        )));
    }

    let version_raw = U256::from_be_slice(&bytes[offset..offset + 32]);
    let version = u32::try_from(version_raw).map_err(|_| {
        ProtocolError::InvalidPayload(format!("version field exceeds u32 range: {version_raw}"))
    })?;
    if version != u32::from(SHASTA_PAYLOAD_VERSION) {
        return Err(ProtocolError::InvalidPayload(format!(
            "unsupported payload version: expected {SHASTA_PAYLOAD_VERSION}, got {version}"
        )));
    }

    let size_raw = U256::from_be_slice(&bytes[offset + 32..offset + 64]);
    let size_u64 = u64::try_from(size_raw).map_err(|_| {
        ProtocolError::InvalidPayload(format!("size field exceeds u64 range: {size_raw}"))
    })?;
    let size = usize::try_from(size_u64).map_err(|_| {
        ProtocolError::InvalidPayload(format!("size field exceeds usize range: {size_u64}"))
    })?;

    if bytes.len() < offset + 64 + size {
        return Err(ProtocolError::InvalidPayload(format!(
            "payload too short for compressed data: expected {} bytes, got {}",
            offset + 64 + size,
            bytes.len()
        )));
    }

    let compressed = &bytes[offset + 64..offset + 64 + size];
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| ProtocolError::Compression(format!("failed to decompress zlib data: {e}")))?;

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decompress_and_decode_rejects_invalid_payload_header() {
        let err =
            DerivationSourceManifest::decompress_and_decode(&[0u8; 64], 0).expect_err("reject");

        assert!(err.to_string().contains("unsupported payload version"));
    }

    #[test]
    fn decompress_and_decode_rejects_oversized_manifest() {
        let manifest = DerivationSourceManifest {
            blocks: vec![BlockManifest::default(); PROPOSAL_MAX_BLOCKS + 1],
        };
        let payload = manifest.encode_and_compress().expect("encode manifest");
        let err = DerivationSourceManifest::decompress_and_decode(&payload, 0).expect_err("reject");

        assert!(err.to_string().contains("manifest contains 193 blocks"));
    }
}
