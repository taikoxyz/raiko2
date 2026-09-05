use alloy_consensus::Header;
use alloy_primitives::{B256, Bytes, keccak256};
use alloy_rlp::{Decodable, Encodable};
use reth_consensus::ConsensusError;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as DeError};

/// Canonical header witness representation used throughout the prover pipeline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WitnessHeader {
    /// Block number for ancestor linkage validation.
    pub number: u64,
    /// Parent hash for ancestor linkage validation.
    pub parent_hash: B256,
    /// Timestamp used by consensus validation.
    pub timestamp: u64,
    /// Precomputed block hash for the header.
    ///
    /// Recomputed from the header whenever a full header is present; for compact headers it is
    /// host-trusted and must never feed consensus decisions.
    pub hash: B256,
    /// Full header, kept only when later validation needs it.
    pub header: Option<Header>,
}

impl WitnessHeader {
    /// Build a witness header from a decoded header.
    #[must_use]
    pub fn from_header(header: Header) -> Self {
        let hash = header.hash_slow();
        Self::from_header_with_hash(header, hash)
    }

    /// Build a witness header from a decoded header and a precomputed block hash.
    #[must_use]
    pub const fn from_header_with_hash(header: Header, hash: B256) -> Self {
        Self {
            number: header.number,
            parent_hash: header.parent_hash,
            timestamp: header.timestamp,
            hash,
            header: Some(header),
        }
    }

    /// Build a compact witness header that keeps only the metadata needed by stateless validation.
    #[must_use]
    pub fn from_compact_header(header: &Header) -> Self {
        Self {
            number: header.number,
            parent_hash: header.parent_hash,
            timestamp: header.timestamp,
            hash: header.hash_slow(),
            header: None,
        }
    }

    /// Return a compact version of this witness header.
    #[must_use]
    pub fn into_compact(self) -> Self {
        Self {
            number: self.number,
            parent_hash: self.parent_hash,
            timestamp: self.timestamp,
            hash: self.hash,
            header: None,
        }
    }

    /// Drop the full header payload while preserving compact metadata in place.
    pub fn compact_in_place(&mut self) {
        self.header = None;
    }

    /// Build a witness header from RLP-encoded bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes do not decode into a valid header.
    pub fn from_rlp(bytes: &[u8]) -> Result<Self, alloy_rlp::Error> {
        let mut slice = bytes;
        Header::decode(&mut slice).map(Self::from_header)
    }

    /// Build a witness header from any RLP-encodable header representation.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded header does not decode into a canonical header.
    pub fn from_encoded_header<T>(header: &T) -> Result<Self, alloy_rlp::Error>
    where
        T: Encodable,
    {
        let encoded = alloy_rlp::encode(header);
        Self::from_rlp(encoded.as_ref())
    }

    /// Returns the full header when present.
    #[must_use]
    pub const fn full_header(&self) -> Option<&Header> {
        self.header.as_ref()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WitnessHeaderSerde {
    header: Header,
    hash: B256,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WitnessHeaderCompactSerde {
    number: u64,
    parent_hash: B256,
    timestamp: u64,
    hash: B256,
}

#[derive(Debug, Serialize, Deserialize)]
struct WitnessHeaderBincode<'a> {
    number: u64,
    parent_hash: B256,
    timestamp: u64,
    hash: B256,
    header: Option<alloy_consensus::serde_bincode_compat::Header<'a>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WitnessHeaderHuman {
    Legacy(Bytes),
    Full(Box<WitnessHeaderSerde>),
    Compact(WitnessHeaderCompactSerde),
}

impl From<WitnessHeaderSerde> for WitnessHeader {
    fn from(value: WitnessHeaderSerde) -> Self {
        let _host_hash = value.hash;
        Self::from_header(value.header)
    }
}

impl From<WitnessHeaderCompactSerde> for WitnessHeader {
    /// Host-trusted metadata: no full header is available to recompute `hash`, so consensus
    /// paths reject compact headers via `ensure_full_ancestor_headers`.
    fn from(value: WitnessHeaderCompactSerde) -> Self {
        Self {
            number: value.number,
            parent_hash: value.parent_hash,
            timestamp: value.timestamp,
            hash: value.hash,
            header: None,
        }
    }
}

impl From<&WitnessHeader> for WitnessHeaderSerde {
    fn from(value: &WitnessHeader) -> Self {
        Self {
            header: value
                .full_header()
                .cloned()
                .expect("full header required for full serde form"),
            hash: value.hash,
        }
    }
}

impl From<&WitnessHeader> for WitnessHeaderCompactSerde {
    fn from(value: &WitnessHeader) -> Self {
        Self {
            number: value.number,
            parent_hash: value.parent_hash,
            timestamp: value.timestamp,
            hash: value.hash,
        }
    }
}

impl<'a> From<&'a WitnessHeader> for WitnessHeaderBincode<'a> {
    fn from(value: &'a WitnessHeader) -> Self {
        Self {
            number: value.number,
            parent_hash: value.parent_hash,
            timestamp: value.timestamp,
            hash: value.hash,
            header: value.full_header().map(Into::into),
        }
    }
}

impl From<WitnessHeaderBincode<'_>> for WitnessHeader {
    fn from(value: WitnessHeaderBincode<'_>) -> Self {
        if let Some(header) = value.header {
            return Self::from_header(header.into());
        }

        // Compact arm: host-trusted metadata (nothing to recompute `hash` from); consensus paths
        // reject compact headers via `ensure_full_ancestor_headers`.
        Self {
            number: value.number,
            parent_hash: value.parent_hash,
            timestamp: value.timestamp,
            hash: value.hash,
            header: None,
        }
    }
}

impl Serialize for WitnessHeader {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            if self.full_header().is_some() {
                WitnessHeaderSerde::from(self).serialize(serializer)
            } else {
                WitnessHeaderCompactSerde::from(self).serialize(serializer)
            }
        } else {
            WitnessHeaderBincode::from(self).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for WitnessHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return match WitnessHeaderHuman::deserialize(deserializer)? {
                WitnessHeaderHuman::Legacy(bytes) => {
                    Self::from_rlp(bytes.as_ref()).map_err(|err| D::Error::custom(err.to_string()))
                }
                WitnessHeaderHuman::Full(value) => Ok((*value).into()),
                WitnessHeaderHuman::Compact(value) => Ok(value.into()),
            };
        }

        WitnessHeaderBincode::deserialize(deserializer).map(Into::into)
    }
}

/// Canonical RLP witness node used to materialize sparse tries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WitnessStateNode {
    /// Precomputed Keccak hash of the RLP bytes.
    pub hash: B256,
    /// Raw RLP bytes for the trie node.
    pub bytes: Bytes,
}

impl WitnessStateNode {
    /// Build a witness state node from raw RLP bytes.
    #[must_use]
    pub fn from_bytes(bytes: Bytes) -> Self {
        Self {
            hash: keccak256(&bytes),
            bytes,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct WitnessStateNodeSerde {
    hash: B256,
    bytes: Bytes,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WitnessStateNodeHuman {
    Legacy(Bytes),
    Structured(WitnessStateNodeSerde),
}

impl From<WitnessStateNodeSerde> for WitnessStateNode {
    fn from(value: WitnessStateNodeSerde) -> Self {
        let _host_hash = value.hash;
        Self::from_bytes(value.bytes)
    }
}

impl From<&WitnessStateNode> for WitnessStateNodeSerde {
    fn from(value: &WitnessStateNode) -> Self {
        Self {
            hash: value.hash,
            bytes: value.bytes.clone(),
        }
    }
}

impl Serialize for WitnessStateNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            self.bytes.serialize(serializer)
        } else {
            WitnessStateNodeSerde::from(self).serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for WitnessStateNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return match WitnessStateNodeHuman::deserialize(deserializer)? {
                WitnessStateNodeHuman::Legacy(bytes) => Ok(Self::from_bytes(bytes)),
                WitnessStateNodeHuman::Structured(value) => Ok(value.into()),
            };
        }

        WitnessStateNodeSerde::deserialize(deserializer).map(Into::into)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ExecutionWitnessSerde {
    state: Vec<WitnessStateNode>,
    #[serde(default)]
    state_indices: Vec<u32>,
    codes: Vec<Bytes>,
    keys: Vec<Bytes>,
    headers: Vec<WitnessHeader>,
}

/// Canonical execution witness representation used by raiko2.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionWitness {
    /// List of all hashed trie node preimages needed for execution and post-state recomputation.
    pub state: Vec<WitnessStateNode>,
    /// Indices into a proposal-level shared state node pool.
    pub state_indices: Vec<u32>,
    /// List of all code preimages needed for execution and post-state recomputation.
    pub codes: Vec<Bytes>,
    /// List of unhashed account/storage key preimages required by the witness.
    pub keys: Vec<Bytes>,
    /// Ancestor headers with precomputed hashes.
    pub headers: Vec<WitnessHeader>,
}

impl ExecutionWitness {
    /// Sort and deduplicate state nodes by hash.
    #[must_use]
    pub fn canonicalize_state_nodes(mut state: Vec<WitnessStateNode>) -> Vec<WitnessStateNode> {
        state.sort_by_key(|node| node.hash);
        state.dedup_by_key(|node| node.hash);
        state
    }

    /// Sort and deduplicate shared state pool indices.
    #[must_use]
    pub fn canonicalize_state_indices(mut state_indices: Vec<u32>) -> Vec<u32> {
        state_indices.sort_unstable();
        state_indices.dedup();
        state_indices
    }

    /// Sort headers by block number while preserving full header payloads.
    #[must_use]
    pub fn canonicalize_headers(mut headers: Vec<WitnessHeader>) -> Vec<WitnessHeader> {
        headers.sort_by_key(|header| header.number);
        headers
    }

    #[must_use]
    fn from_canonicalized_serde(value: ExecutionWitnessSerde) -> Self {
        Self {
            state: Self::canonicalize_state_nodes(value.state),
            state_indices: Self::canonicalize_state_indices(value.state_indices),
            codes: value.codes,
            keys: value.keys,
            headers: Self::canonicalize_headers(value.headers),
        }
    }
}

impl From<ExecutionWitnessSerde> for ExecutionWitness {
    fn from(value: ExecutionWitnessSerde) -> Self {
        Self {
            state: value.state,
            state_indices: value.state_indices,
            codes: value.codes,
            keys: value.keys,
            headers: value.headers,
        }
    }
}

impl From<&ExecutionWitness> for ExecutionWitnessSerde {
    fn from(value: &ExecutionWitness) -> Self {
        Self {
            state: value.state.clone(),
            state_indices: value.state_indices.clone(),
            codes: value.codes.clone(),
            keys: value.keys.clone(),
            headers: value.headers.clone(),
        }
    }
}

impl Serialize for ExecutionWitness {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ExecutionWitnessSerde::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExecutionWitness {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            return ExecutionWitnessSerde::deserialize(deserializer)
                .map(Self::from_canonicalized_serde);
        }

        let witness = ExecutionWitnessSerde::deserialize(deserializer)?;
        Ok(Self {
            state: witness.state,
            state_indices: witness.state_indices,
            codes: witness.codes,
            keys: witness.keys,
            headers: witness.headers,
        })
    }
}

impl TryFrom<alloy_rpc_types_debug::ExecutionWitness> for ExecutionWitness {
    type Error = alloy_rlp::Error;

    fn try_from(value: alloy_rpc_types_debug::ExecutionWitness) -> Result<Self, Self::Error> {
        let headers = value
            .headers
            .into_iter()
            .map(|bytes| WitnessHeader::from_rlp(bytes.as_ref()))
            .collect::<Result<_, _>>()?;
        let state = value
            .state
            .into_iter()
            .map(WitnessStateNode::from_bytes)
            .collect();

        Ok(Self {
            state,
            state_indices: Vec::new(),
            codes: value.codes,
            keys: value.keys,
            headers: Self::canonicalize_headers(headers),
        }
        .with_canonical_state())
    }
}

impl ExecutionWitness {
    #[must_use]
    fn with_canonical_state(mut self) -> Self {
        self.state = Self::canonicalize_state_nodes(self.state);
        self.state_indices = Self::canonicalize_state_indices(self.state_indices);
        self
    }
}

/// Errors that can occur during stateless validation.
#[derive(Debug, thiserror::Error)]
pub enum StatelessValidationError {
    #[error("ancestor header count ({count}) exceeds limit ({limit})")]
    AncestorHeaderLimitExceeded { count: usize, limit: usize },

    #[error("invalid ancestor chain")]
    InvalidAncestorChain,

    #[error("failed to reveal witness data for pre-state root {pre_state_root}")]
    WitnessRevealFailed { pre_state_root: B256 },

    #[error("shared witness state index {index} out of bounds for pool length {len}")]
    SharedWitnessStateIndexOutOfBounds { index: u32, len: usize },

    #[error("stateless block execution failed: {0}")]
    StatelessExecutionFailed(String),

    #[error("consensus validation failed: {0}")]
    ConsensusValidationFailed(#[from] ConsensusError),

    #[error("stateless state root calculation failed")]
    StatelessStateRootCalculationFailed,

    #[error("stateless pre-state root calculation failed")]
    StatelessPreStateRootCalculationFailed,

    #[error("missing required ancestor headers")]
    MissingAncestorHeader,

    #[error("could not deserialize ancestor headers")]
    HeaderDeserializationFailed,

    #[error("compact ancestor headers are not accepted in host-untrusted validation")]
    CompactAncestorHeaderUnsupported,

    #[error("mismatched post-state root: {got}\n {expected}")]
    PostStateRootMismatch { got: B256, expected: B256 },

    #[error("mismatched pre-state root: {got}\n {expected}")]
    PreStateRootMismatch { got: B256, expected: B256 },

    #[error("signer recovery failed")]
    SignerRecovery,

    #[error("signature s value not normalized for homestead block")]
    HomesteadSignatureNotNormalized,

    /// The parent-state Shasta `CheckpointStore` record could not be read: no known layout holds a
    /// record, the record is malformed, or the witness cannot prove a probed slot.
    #[error("{0}")]
    ParentCheckpointUnavailable(String),

    #[error("{0}")]
    Custom(&'static str),
}

#[cfg(test)]
mod tests {
    use super::{ExecutionWitness, WitnessHeader, WitnessStateNode, WitnessStateNodeSerde};
    use alloy_consensus::Header;
    use alloy_primitives::{Address, B64, B256, Bloom, Bytes, U256, keccak256};

    fn sample_header(number: u64) -> Header {
        Header {
            parent_hash: B256::repeat_byte(0x11),
            ommers_hash: B256::repeat_byte(0x22),
            beneficiary: Address::default(),
            state_root: B256::repeat_byte(0x33),
            transactions_root: B256::repeat_byte(0x44),
            receipts_root: B256::repeat_byte(0x55),
            logs_bloom: Bloom::default(),
            difficulty: U256::default(),
            number,
            gas_limit: 30_000_000,
            gas_used: 21_000,
            timestamp: 1_700_000_000 + number,
            extra_data: Bytes::from_static(b"raiko2"),
            mix_hash: B256::repeat_byte(0x66),
            nonce: B64::default(),
            base_fee_per_gas: Some(1_000_000_000),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            ..Default::default()
        }
    }

    #[test]
    fn witness_header_deserializes_legacy_json_bytes() {
        let header = sample_header(42);
        let legacy_json = serde_json::to_string(&Bytes::from(alloy_rlp::encode(&header)))
            .expect("serialize legacy header bytes");

        let witness_header: WitnessHeader =
            serde_json::from_str(&legacy_json).expect("deserialize legacy witness header");

        assert_eq!(witness_header.full_header(), Some(&header));
        assert_eq!(witness_header.hash, header.hash_slow());
    }

    #[test]
    fn witness_header_deserialization_ignores_supplied_hash_when_full() {
        let header = sample_header(42);
        let wrong_hash = B256::repeat_byte(0x99);
        let json = serde_json::json!({
            "header": header,
            "hash": wrong_hash,
        });

        let witness_header: WitnessHeader =
            serde_json::from_value(json).expect("deserialize structured witness header");

        assert_eq!(witness_header.hash, header.hash_slow());
        assert_ne!(witness_header.hash, wrong_hash);
    }

    #[test]
    fn witness_header_bincode_deserialization_ignores_supplied_hash_when_full() {
        let header = sample_header(42);
        let witness_header = WitnessHeader {
            hash: B256::repeat_byte(0x99),
            ..WitnessHeader::from_header(header.clone())
        };

        let encoded = bincode::serialize(&witness_header).expect("serialize witness header");
        let decoded: WitnessHeader =
            bincode::deserialize(&encoded).expect("deserialize witness header");

        assert_eq!(decoded.hash, header.hash_slow());
    }

    #[test]
    fn execution_witness_bincode_roundtrip_preserves_headers() {
        let header = WitnessHeader::from_header(sample_header(7));
        let witness = ExecutionWitness {
            state: vec![WitnessStateNode::from_bytes(Bytes::from_static(b"state"))],
            state_indices: Vec::new(),
            codes: vec![Bytes::from_static(b"code")],
            keys: vec![Bytes::from_static(b"key")],
            headers: vec![header],
        };

        let encoded = bincode::serialize(&witness).expect("serialize witness");
        let decoded: ExecutionWitness =
            bincode::deserialize(&encoded).expect("deserialize witness");

        assert_eq!(decoded, witness);
    }

    #[test]
    fn execution_witness_json_deserialization_canonicalizes_headers() {
        let oldest = sample_header(1);
        let parent = sample_header(2);
        let json = serde_json::json!({
            "state": [],
            "codes": [],
            "keys": [],
            "headers": [
                Bytes::from(alloy_rlp::encode(&parent)),
                Bytes::from(alloy_rlp::encode(&oldest)),
            ],
        });

        let witness: ExecutionWitness =
            serde_json::from_value(json).expect("deserialize witness from legacy json");

        assert_eq!(witness.headers.len(), 2);
        assert_eq!(witness.headers[0].number, 1);
        assert_eq!(witness.headers[0].full_header(), Some(&oldest));
        assert_eq!(witness.headers[1].number, 2);
        assert_eq!(witness.headers[1].full_header(), Some(&parent));
    }

    #[test]
    fn execution_witness_json_deserialization_canonicalizes_state_nodes() {
        let first = Bytes::from_static(b"node-a");
        let second = Bytes::from_static(b"node-b");
        let json = serde_json::json!({
            "state": [second.clone(), first.clone(), first.clone()],
            "codes": [],
            "keys": [],
            "headers": [],
        });

        let witness: ExecutionWitness =
            serde_json::from_value(json).expect("deserialize witness from legacy json");

        assert_eq!(witness.state.len(), 2);
        assert_eq!(witness.state[0].hash, keccak256(&first));
        assert_eq!(witness.state[0].bytes, first);
        assert_eq!(witness.state[1].hash, keccak256(&second));
        assert_eq!(witness.state[1].bytes, second);
    }

    #[test]
    fn witness_state_node_deserialization_ignores_supplied_hash() {
        let bytes = Bytes::from_static(b"node-a");
        let wrong_hash = B256::repeat_byte(0x99);
        let json = serde_json::json!({
            "hash": wrong_hash,
            "bytes": bytes,
        });

        let node: WitnessStateNode =
            serde_json::from_value(json).expect("deserialize structured state node");

        assert_eq!(node.hash, keccak256(&bytes));
        assert_ne!(node.hash, wrong_hash);
    }

    #[test]
    fn witness_state_node_bincode_deserialization_ignores_supplied_hash() {
        let bytes = Bytes::from_static(b"node-a");
        let encoded = bincode::serialize(&WitnessStateNodeSerde {
            hash: B256::repeat_byte(0x99),
            bytes: bytes.clone(),
        })
        .expect("serialize state node");

        let node: WitnessStateNode =
            bincode::deserialize(&encoded).expect("deserialize state node");

        assert_eq!(node.hash, keccak256(&bytes));
    }

    #[test]
    fn canonicalize_headers_preserves_full_headers() {
        let oldest = WitnessHeader::from_header(sample_header(1));
        let parent = WitnessHeader::from_header(sample_header(2));
        let headers = ExecutionWitness::canonicalize_headers(vec![parent.clone(), oldest.clone()]);

        assert_eq!(headers.len(), 2);
        assert_eq!(headers[0].number, 1);
        assert_eq!(headers[0].full_header(), oldest.full_header());
        assert_eq!(headers[1].number, 2);
        assert_eq!(headers[1].full_header(), parent.full_header());
    }
}
