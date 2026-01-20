//! Manifest types for encoding block proposals and metadata.

use std::io::{Read, Write};

use alloy_primitives::{Address, U256};
use alloy_rlp::{Decodable, Encodable};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use reth_ethereum_primitives::TransactionSigned;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::constants::{PROPOSAL_MAX_BLOCKS, SHASTA_PAYLOAD_VERSION};

/// Errors that can occur while encoding or decoding Shasta manifests.
#[derive(Debug)]
pub enum ManifestError {
    Io(std::io::Error),
    InvalidPayload(String),
    Compression(String),
    Rlp(alloy_rlp::Error),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io error: {err}"),
            Self::InvalidPayload(msg) => write!(f, "invalid payload: {msg}"),
            Self::Compression(msg) => write!(f, "compression error: {msg}"),
            Self::Rlp(err) => write!(f, "rlp error: {err}"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl From<std::io::Error> for ManifestError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<alloy_rlp::Error> for ManifestError {
    fn from(err: alloy_rlp::Error) -> Self {
        Self::Rlp(err)
    }
}

/// Manifest of a single block proposal, matching `LibManifest.ProtocolBlockManifest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    pub transactions: Vec<TransactionSigned>,
}

/// Manifest for a derivation source, matching `LibManifest.DerivationSourceManifest`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DerivationSourceManifest {
    /// Blocks included in this source.
    pub blocks: Vec<BlockManifest>,
}

impl DerivationSourceManifest {
    /// Encode and compress the derivation source manifest following the Shasta protocol payload
    /// format.
    pub fn encode_and_compress(&self) -> Result<Vec<u8>, ManifestError> {
        encode_manifest_payload(self)
    }

    /// Decompress and decode a derivation source manifest from the Shasta protocol payload bytes.
    pub fn decompress_and_decode(bytes: &[u8], offset: usize) -> Result<Self, ManifestError> {
        let decoded = match decode_manifest_payload(bytes, offset) {
            Ok(data) => data,
            Err(err) => {
                warn!(
                    ?err,
                    "failed to decode manifest payload; returning default manifest"
                );
                return Ok(Self::default());
            }
        };

        let mut decoded_slice = decoded.as_slice();
        let manifest = match <DerivationSourceManifest as Decodable>::decode(&mut decoded_slice) {
            Ok(m) => m,
            Err(err) => {
                warn!(
                    ?err,
                    "failed to decode derivation manifest RLP; returning default manifest"
                );
                return Ok(Self::default());
            }
        };

        if manifest.blocks.len() > PROPOSAL_MAX_BLOCKS {
            warn!(
                blocks = manifest.blocks.len(),
                max = PROPOSAL_MAX_BLOCKS,
                "manifest contains too many blocks; returning default manifest"
            );
            return Ok(Self::default());
        }

        Ok(manifest)
    }
}

fn encode_manifest_payload<T>(manifest: &T) -> Result<Vec<u8>, ManifestError>
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

fn decode_manifest_payload(bytes: &[u8], offset: usize) -> Result<Vec<u8>, ManifestError> {
    if bytes.len() < offset + 64 {
        return Err(ManifestError::InvalidPayload(format!(
            "payload too short for header: expected at least {} bytes, got {}",
            offset + 64,
            bytes.len()
        )));
    }

    let version_raw = U256::from_be_slice(&bytes[offset..offset + 32]);
    let version = u32::try_from(version_raw).map_err(|_| {
        ManifestError::InvalidPayload(format!("version field exceeds u32 range: {version_raw}"))
    })?;
    if version != SHASTA_PAYLOAD_VERSION as u32 {
        return Err(ManifestError::InvalidPayload(format!(
            "unsupported payload version: expected {}, got {version}",
            SHASTA_PAYLOAD_VERSION
        )));
    }

    let size_raw = U256::from_be_slice(&bytes[offset + 32..offset + 64]);
    let size_u64 = u64::try_from(size_raw).map_err(|_| {
        ManifestError::InvalidPayload(format!("size field exceeds u64 range: {size_raw}"))
    })?;
    let size = usize::try_from(size_u64).map_err(|_| {
        ManifestError::InvalidPayload(format!("size field exceeds usize range: {size_u64}"))
    })?;

    if bytes.len() < offset + 64 + size {
        return Err(ManifestError::InvalidPayload(format!(
            "payload too short for compressed data: expected {} bytes, got {}",
            offset + 64 + size,
            bytes.len()
        )));
    }

    let compressed = &bytes[offset + 64..offset + 64 + size];
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|e| ManifestError::Compression(format!("failed to decompress zlib data: {e}")))?;

    Ok(decoded)
}

impl Encodable for BlockManifest {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        let payload_length = self.timestamp.length()
            + self.coinbase.length()
            + self.anchor_block_number.length()
            + self.gas_limit.length()
            + self.transactions.length();

        let header = alloy_rlp::Header {
            list: true,
            payload_length,
        };
        header.encode(out);

        self.timestamp.encode(out);
        self.coinbase.encode(out);
        self.anchor_block_number.encode(out);
        self.gas_limit.encode(out);
        self.transactions.encode(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.timestamp.length()
            + self.coinbase.length()
            + self.anchor_block_number.length()
            + self.gas_limit.length()
            + self.transactions.length();
        payload_length + alloy_rlp::length_of_length(payload_length)
    }
}

impl Decodable for BlockManifest {
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let header = alloy_rlp::Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::Custom(
                "BlockManifest must be encoded as a list",
            ));
        }

        Ok(Self {
            timestamp: u64::decode(buf)?,
            coinbase: Address::decode(buf)?,
            anchor_block_number: u64::decode(buf)?,
            gas_limit: u64::decode(buf)?,
            transactions: Vec::<TransactionSigned>::decode(buf)?,
        })
    }
}

impl Encodable for DerivationSourceManifest {
    fn encode(&self, out: &mut dyn alloy_rlp::BufMut) {
        let payload_length = self.blocks.length();
        let header = alloy_rlp::Header {
            list: true,
            payload_length,
        };
        header.encode(out);
        self.blocks.encode(out);
    }

    fn length(&self) -> usize {
        let payload_length = self.blocks.length();
        payload_length + alloy_rlp::length_of_length(payload_length)
    }
}

impl Decodable for DerivationSourceManifest {
    fn decode(buf: &mut &[u8]) -> Result<Self, alloy_rlp::Error> {
        let header = alloy_rlp::Header::decode(buf)?;
        if !header.list {
            return Err(alloy_rlp::Error::Custom(
                "DerivationSourceManifest must be encoded as a list",
            ));
        }

        let blocks = Vec::<BlockManifest>::decode(buf)?;
        if blocks.len() > PROPOSAL_MAX_BLOCKS {
            return Err(alloy_rlp::Error::Custom(
                "DerivationSourceManifest blocks length exceeds PROPOSAL_MAX_BLOCKS",
            ));
        }

        Ok(Self { blocks })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_manifest() -> DerivationSourceManifest {
        let block = BlockManifest {
            timestamp: 1_234_567_890,
            coinbase: Address::from([1u8; 20]),
            anchor_block_number: 100,
            gas_limit: 30_000_000,
            transactions: vec![],
        };

        DerivationSourceManifest {
            blocks: vec![block],
        }
    }

    #[test]
    fn manifest_payload_roundtrip() {
        let original = create_test_manifest();
        let encoded = original.encode_and_compress().unwrap();

        assert!(encoded.len() >= 64);
        assert_eq!(encoded[31], SHASTA_PAYLOAD_VERSION);

        let decoded = DerivationSourceManifest::decompress_and_decode(&encoded, 0).unwrap();
        assert_eq!(decoded.blocks.len(), original.blocks.len());
    }

    #[test]
    fn decode_manifest_payload_too_short() {
        let payload = vec![0u8; 32];
        let result = super::decode_manifest_payload(&payload, 0);
        assert!(result.is_err());
    }

    #[test]
    fn decode_manifest_payload_version_mismatch() {
        let mut payload = vec![0u8; 64];
        payload[31] = SHASTA_PAYLOAD_VERSION + 1;

        let result = super::decode_manifest_payload(&payload, 0);
        assert!(result.is_err());
    }
}
