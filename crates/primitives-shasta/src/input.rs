//! Shasta input types for guest programs.

use alloy_consensus::Header;
use alloy_primitives::map::B256Map;
use alloy_primitives::{Address, B256};
use raiko2_primitives::{
    ExecutionWitness, RawProof, StatelessInput, WitnessHeader, WitnessStateNode,
};
use raiko2_protocol_shasta::TaikoManifest;
use raiko2_protocol_shasta::shasta::ProofCarryData;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const ANCESTOR_HEADER_WINDOW_LIMIT: usize = 256;

/// Shasta guest program input.
#[derive(Debug, Clone, Default)]
pub struct GuestInput {
    /// The witnesses for each block.
    pub witnesses: Vec<StatelessInput>,
    /// The Taiko manifest.
    pub taiko: TaikoManifest,
    /// Shared ancestor header window for the proposal path.
    pub proposal_ancestor_headers: Vec<WitnessHeader>,
    /// Shared state node pool for the proposal path.
    pub proposal_state_nodes: Vec<WitnessStateNode>,
    /// Carry data required by proposal proof verification.
    pub proof_carry_data: ProofCarryData,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct GuestInputSerde {
    witnesses: Vec<StatelessInput>,
    taiko: TaikoManifest,
    #[serde(default)]
    proposal_ancestor_headers: Vec<WitnessHeader>,
    #[serde(default)]
    proposal_state_nodes: Vec<WitnessStateNode>,
    #[serde(default)]
    proof_carry_data: ProofCarryData,
}

impl GuestInput {
    #[must_use]
    pub fn proposal_ancestor_headers(&self) -> &[WitnessHeader] {
        if self.proposal_ancestor_headers.is_empty() {
            self.witnesses
                .first()
                .map_or(&[], |witness| witness.witness.headers.as_slice())
        } else {
            self.proposal_ancestor_headers.as_slice()
        }
    }

    #[must_use]
    pub fn initial_proposal_ancestor_headers(&self) -> Vec<WitnessHeader> {
        ExecutionWitness::canonicalize_headers(self.proposal_ancestor_headers().to_vec())
    }

    #[must_use]
    pub const fn proposal_state_nodes(&self) -> &[WitnessStateNode] {
        self.proposal_state_nodes.as_slice()
    }

    pub fn compact_proposal_ancestor_headers(&mut self) {
        let headers = self.initial_proposal_ancestor_headers();
        if headers.is_empty() {
            return;
        }

        self.proposal_ancestor_headers = headers;
        for witness in &mut self.witnesses {
            witness.witness.headers.clear();
        }
    }

    #[must_use]
    fn initial_proposal_state_pool(&self) -> (Vec<WitnessStateNode>, Vec<Vec<u32>>) {
        let mut proposal_state_nodes = if self.proposal_state_nodes.is_empty() {
            self.witnesses
                .iter()
                .flat_map(|witness| witness.witness.state.iter().cloned())
                .collect::<Vec<_>>()
        } else {
            self.proposal_state_nodes.clone()
        };
        proposal_state_nodes = ExecutionWitness::canonicalize_state_nodes(proposal_state_nodes);

        if proposal_state_nodes.is_empty() {
            return (proposal_state_nodes, vec![Vec::new(); self.witnesses.len()]);
        }

        let index_by_hash = proposal_state_nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                (
                    node.hash,
                    u32::try_from(index).expect("proposal state pool index exceeds u32"),
                )
            })
            .collect::<B256Map<_>>();

        let witness_state_indices = self
            .witnesses
            .iter()
            .map(|witness| {
                let indices = if witness.witness.state_indices.is_empty() {
                    witness
                        .witness
                        .state
                        .iter()
                        .filter_map(|node| index_by_hash.get(&node.hash).copied())
                        .collect()
                } else {
                    witness.witness.state_indices.clone()
                };
                ExecutionWitness::canonicalize_state_indices(indices)
            })
            .collect();

        (proposal_state_nodes, witness_state_indices)
    }

    pub fn compact_proposal_state_nodes(&mut self) {
        let (proposal_state_nodes, witness_state_indices) = self.initial_proposal_state_pool();
        if proposal_state_nodes.is_empty() {
            return;
        }

        self.proposal_state_nodes = proposal_state_nodes;
        for (witness, state_indices) in self.witnesses.iter_mut().zip(witness_state_indices) {
            witness.witness.state.clear();
            witness.witness.state_indices = state_indices;
        }
    }

    pub fn compact_proposal_witness_data(&mut self) {
        self.compact_proposal_ancestor_headers();
        self.compact_proposal_state_nodes();
    }
}

#[must_use]
pub fn roll_proposal_ancestor_headers(
    current_headers: &[WitnessHeader],
    parent_header: &Header,
) -> Vec<WitnessHeader> {
    let mut next_headers = current_headers.to_vec();
    roll_proposal_ancestor_headers_in_place(
        &mut next_headers,
        parent_header,
        parent_header.hash_slow(),
    );
    next_headers
}

pub fn roll_proposal_ancestor_headers_in_place(
    current_headers: &mut Vec<WitnessHeader>,
    parent_header: &Header,
    parent_hash: B256,
) {
    if current_headers.len() == ANCESTOR_HEADER_WINDOW_LIMIT {
        current_headers.rotate_left(1);
        current_headers.pop();
    }
    current_headers.push(WitnessHeader::from_header_with_hash(
        parent_header.clone(),
        parent_hash,
    ));
}

impl From<GuestInputSerde> for GuestInput {
    fn from(value: GuestInputSerde) -> Self {
        let mut input = Self {
            witnesses: value.witnesses,
            taiko: value.taiko,
            proposal_ancestor_headers: value.proposal_ancestor_headers,
            proposal_state_nodes: value.proposal_state_nodes,
            proof_carry_data: value.proof_carry_data,
        };
        if !input.proposal_ancestor_headers.is_empty() {
            input.proposal_ancestor_headers =
                ExecutionWitness::canonicalize_headers(input.proposal_ancestor_headers);
        }
        if !input.proposal_state_nodes.is_empty() {
            input.proposal_state_nodes =
                ExecutionWitness::canonicalize_state_nodes(input.proposal_state_nodes);
        }
        input
    }
}

impl Serialize for GuestInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut guest_input = GuestInputSerde {
            witnesses: self.witnesses.clone(),
            taiko: self.taiko.clone(),
            proposal_ancestor_headers: self.initial_proposal_ancestor_headers(),
            proposal_state_nodes: Vec::new(),
            proof_carry_data: self.proof_carry_data.clone(),
        };
        let (proposal_state_nodes, witness_state_indices) = self.initial_proposal_state_pool();
        guest_input.proposal_state_nodes = proposal_state_nodes;

        if !guest_input.proposal_ancestor_headers.is_empty() {
            for witness in &mut guest_input.witnesses {
                witness.witness.headers.clear();
            }
        }
        if !guest_input.proposal_state_nodes.is_empty() {
            for (witness, state_indices) in
                guest_input.witnesses.iter_mut().zip(witness_state_indices)
            {
                witness.witness.state.clear();
                witness.witness.state_indices = state_indices;
            }
        }

        guest_input.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for GuestInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        GuestInputSerde::deserialize(deserializer).map(Into::into)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaRawAggregationGuestInput {
    /// All block proofs to prove
    pub proofs: Vec<RawProof>,
    pub proof_carry_data_vec: Vec<ProofCarryData>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaZkAggregationGuestInput {
    /// Verifier image id for the SP1 proofs being aggregated
    pub image_id: [u32; 8],
    /// Public inputs associated with each underlying proof
    pub block_inputs: Vec<B256>,
    /// Proof carry data associated with each underlying proof
    pub proof_carry_data_vec: Vec<ProofCarryData>,
    /// Address representing the prover/aggregator (zero for zk provers today)
    pub prover_address: Address,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ShastaBoundlessAggregationGuestInput {
    /// Verifier image id for the RISC0 proposal proofs being aggregated.
    pub image_id: [u32; 8],
    /// Proof carry data associated with each underlying proof.
    pub proof_carry_data_vec: Vec<ProofCarryData>,
    /// Bincode-encoded RISC0 receipts for the underlying proofs.
    pub receipts: Vec<Vec<u8>>,
    /// Address representing the prover/aggregator (zero for zk provers today).
    pub prover_address: Address,
}

#[cfg(test)]
mod tests {
    use super::{GuestInput, roll_proposal_ancestor_headers};
    use alloy_consensus::Header;
    use alloy_primitives::{B256, Bytes};
    use raiko2_primitives::{ExecutionWitness, StatelessInput, WitnessHeader, WitnessStateNode};

    fn sample_header(number: u64, parent_hash: B256) -> Header {
        Header {
            number,
            parent_hash,
            timestamp: 1_700_000_000 + number,
            gas_limit: 30_000_000,
            state_root: B256::repeat_byte(number as u8),
            ..Default::default()
        }
    }

    fn sample_state_node(byte: u8) -> WitnessStateNode {
        WitnessStateNode::from_bytes(Bytes::from(vec![byte; 4]))
    }

    #[test]
    fn json_serialize_moves_shared_ancestor_headers() {
        let parent = sample_header(10, B256::ZERO);
        let mut input = GuestInput::default();
        input.witnesses.push(StatelessInput::default());
        input.witnesses[0].witness.headers = vec![WitnessHeader::from_header(parent.clone())];

        let value = serde_json::to_value(&input).expect("serialize guest input");
        let witnesses = value["witnesses"]
            .as_array()
            .expect("serialized witnesses array");
        let proposal_headers = value["proposal_ancestor_headers"]
            .as_array()
            .expect("serialized proposal headers");

        assert_eq!(proposal_headers.len(), 1);
        assert_eq!(
            witnesses[0]["witness"]["headers"]
                .as_array()
                .expect("serialized witness headers")
                .len(),
            0
        );
    }

    #[test]
    fn bincode_roundtrip_preserves_shared_ancestor_headers() {
        let parent = sample_header(10, B256::ZERO);
        let mut input = GuestInput::default();
        input.witnesses.push(StatelessInput::default());
        input.witnesses[0].witness.headers = vec![WitnessHeader::from_header(parent.clone())];

        let bytes = bincode::serialize(&input).expect("serialize guest input");
        let decoded: GuestInput = bincode::deserialize(&bytes).expect("deserialize guest input");

        assert_eq!(decoded.proposal_ancestor_headers.len(), 1);
        assert!(decoded.witnesses[0].witness.headers.is_empty());
        assert_eq!(
            decoded.proposal_ancestor_headers[0].full_header(),
            Some(&parent)
        );
    }

    #[test]
    fn json_serialize_compacts_shared_state_nodes() {
        let first = sample_state_node(0x11);
        let second = sample_state_node(0x22);
        let canonical_pool =
            ExecutionWitness::canonicalize_state_nodes(vec![second.clone(), first.clone()]);

        let mut input = GuestInput::default();
        input.witnesses.push(StatelessInput::default());
        input.witnesses.push(StatelessInput::default());
        input.witnesses[0].witness.state = vec![second.clone(), first.clone(), first.clone()];
        input.witnesses[1].witness.state = vec![second.clone()];

        let value = serde_json::to_value(&input).expect("serialize guest input");
        let witnesses = value["witnesses"]
            .as_array()
            .expect("serialized witnesses array");
        let proposal_state_nodes = value["proposal_state_nodes"]
            .as_array()
            .expect("serialized proposal state nodes");

        assert_eq!(proposal_state_nodes.len(), canonical_pool.len());
        for (serialized, expected) in proposal_state_nodes.iter().zip(canonical_pool.iter()) {
            assert_eq!(
                *serialized,
                serde_json::to_value(expected).expect("serialize state node")
            );
        }

        let first_index = canonical_pool
            .iter()
            .position(|node| node.hash == first.hash)
            .expect("first node in canonical pool") as u64;
        let second_index = canonical_pool
            .iter()
            .position(|node| node.hash == second.hash)
            .expect("second node in canonical pool") as u64;

        assert_eq!(
            witnesses[0]["witness"]["state"]
                .as_array()
                .expect("serialized witness state")
                .len(),
            0
        );
        assert_eq!(
            witnesses[1]["witness"]["state"]
                .as_array()
                .expect("serialized witness state")
                .len(),
            0
        );
        assert_eq!(
            witnesses[0]["witness"]["state_indices"],
            serde_json::json!([first_index, second_index])
        );
        assert_eq!(
            witnesses[1]["witness"]["state_indices"],
            serde_json::json!([second_index])
        );
    }

    #[test]
    fn bincode_roundtrip_preserves_compacted_shared_state_nodes() {
        let first = sample_state_node(0x11);
        let second = sample_state_node(0x22);
        let canonical_pool =
            ExecutionWitness::canonicalize_state_nodes(vec![second.clone(), first.clone()]);

        let mut input = GuestInput::default();
        input.witnesses.push(StatelessInput::default());
        input.witnesses.push(StatelessInput::default());
        input.witnesses[0].witness.state = vec![second.clone(), first.clone(), first.clone()];
        input.witnesses[1].witness.state = vec![second.clone()];

        let bytes = bincode::serialize(&input).expect("serialize guest input");
        let decoded: GuestInput = bincode::deserialize(&bytes).expect("deserialize guest input");

        let first_index = canonical_pool
            .iter()
            .position(|node| node.hash == first.hash)
            .expect("first node in canonical pool") as u32;
        let second_index = canonical_pool
            .iter()
            .position(|node| node.hash == second.hash)
            .expect("second node in canonical pool") as u32;

        assert_eq!(decoded.proposal_state_nodes, canonical_pool);
        assert!(decoded.witnesses[0].witness.state.is_empty());
        assert!(decoded.witnesses[1].witness.state.is_empty());
        assert_eq!(
            decoded.witnesses[0].witness.state_indices,
            vec![first_index, second_index]
        );
        assert_eq!(
            decoded.witnesses[1].witness.state_indices,
            vec![second_index]
        );
    }

    #[test]
    fn rolling_window_keeps_headers_full() {
        let first = WitnessHeader::from_header(sample_header(10, B256::ZERO));
        let second_header = sample_header(11, first.hash);

        let rolled = roll_proposal_ancestor_headers(&[first.clone()], &second_header);

        assert_eq!(rolled.len(), 2);
        assert_eq!(rolled[0].full_header(), first.full_header());
        assert_eq!(rolled[1].full_header(), Some(&second_header));
    }
}
