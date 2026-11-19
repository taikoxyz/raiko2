//! Canonical ABI encoding of Etna landing inputs and Appendix A commitments.
//!
//! Before deriving the proof statement, callers authenticate the deployment,
//! generation, L1 reference, origin, committee, execution results and data
//! availability. Callers also enforce the range, applicable uint48 bounds and
//! input budgets before allocation, and derive exactly one authenticated cut
//! leaf per interior height in ascending order.

use alloy_primitives::{Address, B256, Keccak256, U256, keccak256};
use alloy_sol_types::{SolValue, sol};

sol! {
    /// The eight static fields of the proposed Etna Inbox landing input.
    #[derive(Debug, PartialEq, Eq)]
    struct LandInput {
        /// Height of the proof origin.
        uint256 parent_height;
        /// Terminal height of the proof.
        uint256 last_height;
        /// Terminal execution block hash.
        bytes32 last_block_hash;
        /// Terminal `CometBFT` block hash.
        bytes32 last_comet_hash;
        /// Terminal execution state root.
        bytes32 last_state_root;
        /// Reference L1 block number.
        uint256 l1_reference_block_number;
        /// Forced inclusion cursor after the terminal block.
        uint256 forced_inclusion_head;
        /// Selection snapshot of the committee signing the terminal block.
        uint256 active_committee_snapshot;
    }
}

/// Inputs to the Etna landing statement for the complete proof range.
pub struct LandStatement<'a> {
    /// L1 chain ID of the deployment.
    pub l1_chain_id: U256,
    /// L2 chain ID of the deployment.
    pub l2_chain_id: U256,
    /// L1 Etna Inbox address.
    pub inbox: Address,
    /// L1 Etna Registry address.
    pub registry: Address,
    /// Generation of the proof range.
    pub generation: U256,
    /// Execution block hash at the proof origin.
    pub parent_hash: B256,
    /// `CometBFT` block hash at the proof origin.
    pub parent_comet_hash: B256,
    /// Hash in authenticated Inbox slot 269, with its configuration opened
    /// under ENC-A-36.
    pub committee_config_hash: B256,
    /// The corresponding Etna Inbox landing input.
    pub input: LandInput,
    /// Hash of the reference L1 block.
    pub l1_reference_block_hash: B256,
    /// Versioned blob hashes in commitment order.
    pub blob_hashes: &'a [B256],
    /// Forced inclusion cursor at the proof origin.
    pub forced_inclusion_head_before: U256,
    /// Cut leaves in height order for every strictly interior height.
    pub cut_leaves: &'a [B256],
}

impl LandStatement<'_> {
    /// Encodes the 21 static ABI words hashed by ENC-A-35.
    #[must_use]
    pub fn preimage(&self) -> [u8; 672] {
        let input = &self.input;
        let encoded = (
            B256::right_padding_from(b"TAIKO_ETNA_LAND_V1"),
            self.l1_chain_id,
            self.l2_chain_id,
            self.inbox,
            self.registry,
            self.generation,
            input.parent_height,
            self.parent_hash,
            self.parent_comet_hash,
            self.committee_config_hash,
            input.last_height,
            input.last_block_hash,
            input.last_comet_hash,
            input.last_state_root,
            input.l1_reference_block_number,
            self.l1_reference_block_hash,
            hash_packed_words(self.blob_hashes),
            self.forced_inclusion_head_before,
            input.forced_inclusion_head,
            input.active_committee_snapshot,
            cut_hash(
                self.generation,
                input.parent_height,
                input.last_height,
                self.cut_leaves,
            ),
        )
            .abi_encode_params();
        let mut preimage = [0; 672];
        preimage.copy_from_slice(&encoded);
        preimage
    }

    /// Computes the bytes32 digest passed to the Etna Inbox verifier.
    ///
    /// Backend public-input framing is applied by the backend integration.
    #[must_use]
    pub fn digest(&self) -> B256 {
        keccak256(self.preimage())
    }
}

/// Computes the four-word cut leaf for one height under ENC-A-29.
#[must_use]
pub fn cut_leaf(height: U256, block_hash: B256, comet_hash: B256, forced_head: U256) -> B256 {
    keccak256((height, block_hash, comet_hash, forced_head).abi_encode_params())
}

/// Computes the ENC-A-30 commitment to the supplied leaves in slice order.
///
/// Callers enforce exactly one authenticated leaf per strictly interior height
/// in increasing order. A range of one block uses an empty slice.
#[must_use]
pub fn cut_hash(generation: U256, parent_height: U256, last_height: U256, leaves: &[B256]) -> B256 {
    keccak256(
        (
            B256::right_padding_from(b"ETNA_LAND_CUTS_V1"),
            generation,
            parent_height,
            last_height,
            hash_packed_words(leaves),
        )
            .abi_encode_params(),
    )
}

fn hash_packed_words(words: &[B256]) -> B256 {
    let mut hasher = Keccak256::new();
    for word in words {
        hasher.update(word.as_slice());
    }
    hasher.finalize()
}
