use alloy_primitives::B256;
use raiko2_primitives::Proof;
use risc0_zkvm::SessionInfo;
use serde::{Deserialize, Serialize};

/// RISC0 prover configuration parameters.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Risc0Config {
    /// Whether to use Bonsai proving service.
    pub bonsai: bool,
    /// Whether to generate SNARK proof.
    pub snark: bool,
    /// Whether to use dev mode and return a fake receipt instead of a real proof.
    #[serde(default)]
    pub mock: bool,
    /// Whether to enable profiling.
    pub profile: bool,
    /// Execution power of 2 (for cycle limit).
    pub execution_po2: u32,
    /// Whether to verify the proof after generation.
    #[serde(default = "default_true")]
    pub verify: bool,
}

const fn default_true() -> bool {
    true
}

impl Default for Risc0Config {
    fn default() -> Self {
        Self {
            bonsai: true,
            snark: true,
            mock: false,
            profile: false,
            execution_po2: 20,
            verify: true,
        }
    }
}

/// RISC0 proof response.
#[derive(Clone, Serialize, Deserialize)]
pub struct Risc0Response {
    /// Hex-encoded RISC0 proof payload (seal when available, otherwise journal bytes).
    pub proof: String,
    /// JSON-serialized receipt
    pub receipt: String,
    /// Image ID of the guest program
    pub image_id: String,
    /// Public input hash
    pub input: B256,
    /// Additional zkVM execution metadata.
    pub extra_data: Option<serde_json::Value>,
}

/// Serializable segment-level execution metadata for RISC0.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Risc0SegmentMetadata {
    /// Segment index in execution order.
    pub index: usize,
    /// Proving cycle limit in powers of two.
    pub po2: u32,
    /// User cycles consumed by this segment.
    pub cycles: u32,
}

/// Serializable execution metadata exposed by mock RISC0 runs.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Risc0ExecutionMetadata {
    /// zkVM backend name.
    pub zkvm: String,
    /// Execution mode, currently `mock` for fake-receipt runs.
    pub mode: String,
    /// Whether the receipt is a fake dev-mode receipt.
    pub fake_receipt: bool,
    /// Hex-encoded guest image id.
    pub image_id: String,
    /// Hex-encoded public input hash derived from the journal.
    pub input_hash: String,
    /// Number of journal bytes committed by the guest.
    pub journal_bytes: usize,
    /// Exit status reported by the guest session.
    pub exit_code: String,
    /// Total user cycles across all segments.
    pub total_cycles: u64,
    /// Number of executed segments.
    pub segment_count: usize,
    /// Per-segment cycle metadata.
    pub segments: Vec<Risc0SegmentMetadata>,
}

impl Risc0ExecutionMetadata {
    /// Build a serializable metadata snapshot from a completed RISC0 session.
    #[must_use]
    pub fn from_session(
        session: &SessionInfo,
        image_id: String,
        input_hash: B256,
        journal_bytes: usize,
        mode: &str,
        fake_receipt: bool,
    ) -> Self {
        let segments = session
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| Risc0SegmentMetadata {
                index,
                po2: segment.po2,
                cycles: segment.cycles,
            })
            .collect::<Vec<_>>();

        Self {
            zkvm: "risc0".to_string(),
            mode: mode.to_string(),
            fake_receipt,
            image_id,
            input_hash: alloy_primitives::hex::encode_prefixed(input_hash.as_slice()),
            journal_bytes,
            exit_code: format!("{:?}", session.exit_code),
            total_cycles: session.cycles(),
            segment_count: segments.len(),
            segments,
        }
    }
}

impl From<Risc0Response> for Proof {
    fn from(value: Risc0Response) -> Self {
        Self {
            proof: Some(value.proof),
            quote: Some(value.receipt),
            input: Some(value.input),
            uuid: Some(value.image_id),
            kzg_proof: None,
            extra_data: value.extra_data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Risc0Response;
    use alloy_primitives::B256;
    use serde_json::json;

    #[test]
    fn risc0_response_preserves_extra_data() {
        let response = Risc0Response {
            proof: "0x1234".to_string(),
            receipt: "{\"receipt\":true}".to_string(),
            image_id: "0xdeadbeef".to_string(),
            input: B256::ZERO,
            extra_data: Some(json!({
                "zkvm": "risc0",
                "mode": "mock",
                "total_cycles": 42
            })),
        };

        let proof: raiko2_primitives::Proof = response.into();
        assert_eq!(
            proof.extra_data,
            Some(json!({
                "zkvm": "risc0",
                "mode": "mock",
                "total_cycles": 42
            }))
        );
    }
}
