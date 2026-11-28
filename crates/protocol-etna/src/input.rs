//! Owned, provisional native transport for the existing Etna range verifier.
//!
//! Every field is an untrusted request. The caller supplies authenticated range/domain context,
//! phase-one profile, L1 reference authority, history limits and EVM configuration separately.
//! Context, Inbox proofs, timing/rules and committee/QC obligations remain with the composing
//! caller. This carrier does not authorize signing or define an on-chain ABI.
//!
//! Cap encoded input bytes before deserializing: serde itself does not bound allocations.
//! New carrier fields reject unknown JSON keys. Reused upstream execution and Comet headers
//! retain their existing serde formats and unknown-field behavior.

use alloc::vec::Vec;
use alloy_consensus::Header;
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};
use tendermint::block::Header as CometHeader;

use crate::{
    l1_history::{
        AnchorHeaderWitness, BeaconHeader, BeaconHistoryWitness, HistoricalPeriodWitness,
        L1HistoryError, L1HistoryLimits, L1HistoryProof, SszProofNode,
    },
    statement::{LandInput, LandStatement},
};

/// Owned fields of the complete 19-word landing statement.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandStatementInput {
    /// Requested L1 chain ID.
    pub l1_chain_id: U256,
    /// Requested L2 chain ID.
    pub l2_chain_id: U256,
    /// Requested Inbox address.
    pub inbox: Address,
    /// Requested generation.
    pub generation: U256,
    /// Execution block hash at the excluded origin.
    pub parent_hash: B256,
    /// Comet block hash at the excluded origin.
    pub parent_comet_hash: B256,
    /// Requested deployment configuration commitment.
    pub config_hash: B256,
    /// Seven-field landing input.
    pub input: LandInput,
    /// Hash of the explicit L1 reference Q.
    pub l1_reference_block_hash: B256,
    /// Original ordered blob versioned hashes.
    pub blob_hashes: Vec<B256>,
    /// Incoming forced-inclusion cursor.
    pub forced_inclusion_head_before: U256,
    /// Original cut leaves in height order.
    pub cut_leaves: Vec<B256>,
}

impl LandStatementInput {
    /// Borrows the exact supplied values; this does not authenticate them.
    #[must_use]
    pub fn as_statement(&self) -> LandStatement<'_> {
        LandStatement {
            l1_chain_id: self.l1_chain_id,
            l2_chain_id: self.l2_chain_id,
            inbox: self.inbox,
            generation: self.generation,
            parent_hash: self.parent_hash,
            parent_comet_hash: self.parent_comet_hash,
            config_hash: self.config_hash,
            input: self.input.clone(),
            l1_reference_block_hash: self.l1_reference_block_hash,
            blob_hashes: &self.blob_hashes,
            forced_inclusion_head_before: self.forced_inclusion_head_before,
            cut_leaves: &self.cut_leaves,
        }
    }
}

impl From<&LandStatement<'_>> for LandStatementInput {
    fn from(value: &LandStatement<'_>) -> Self {
        Self {
            l1_chain_id: value.l1_chain_id,
            l2_chain_id: value.l2_chain_id,
            inbox: value.inbox,
            generation: value.generation,
            parent_hash: value.parent_hash,
            parent_comet_hash: value.parent_comet_hash,
            config_hash: value.config_hash,
            input: value.input.clone(),
            l1_reference_block_hash: value.l1_reference_block_hash,
            blob_hashes: value.blob_hashes.to_vec(),
            forced_inclusion_head_before: value.forced_inclusion_head_before,
            cut_leaves: value.cut_leaves.to_vec(),
        }
    }
}

/// Exact original execution-header bytes and their existing beacon opening.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorHeaderInput {
    /// Original raw execution header, without normalization.
    pub raw_header: Vec<u8>,
    /// Containing beacon header for a pre-reference anchor.
    pub beacon_header: Option<BeaconHeader>,
    /// Original leaf-to-root payload branch.
    pub payload_siblings: Vec<B256>,
}

/// One original completed-period witness.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPeriodInput {
    /// Original zero-based period.
    pub period: u64,
    /// Block-roots commitment.
    pub block_roots: B256,
    /// Original other half of a legacy `HistoricalBatch`.
    pub legacy_state_roots: Option<B256>,
    /// Original root-scoped helper nodes in supplied order.
    pub helpers: Vec<SszProofNode>,
}

/// Shared beacon history rooted at Q's parent beacon block.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeaconHistoryInput {
    /// Original reference beacon header.
    pub reference: BeaconHeader,
    /// Original post-state multiproof helpers.
    pub state_helpers: Vec<SszProofNode>,
    /// Original historical-summary length metadata.
    pub summary_length: Option<u64>,
    /// Original legacy historical-roots length metadata.
    pub legacy_length: Option<u64>,
    /// Original completed-period groups in supplied order.
    pub periods: Vec<HistoricalPeriodInput>,
}

/// Untrusted owned L1 blocks anchored to one exact execution reference Q.
///
/// Carries Q's raw header, distinct anchor headers `a_i`, and one shared beacon-history witness.
/// Verification authenticates every pre-Q anchor relative to Q's `parentBeaconBlockRoot`;
/// a direct-Q opening must match Q's exact identity. The composing range checks which anchors
/// it requires. This material does not define an advancing L2 synchronization boundary.
/// Resource limits are supplied separately by the caller.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchoredL1Blocks {
    /// Original raw execution header for the requested reference Q.
    pub reference_header: Vec<u8>,
    /// Distinct anchor openings `a_i`, supplied in increasing execution-height order.
    pub anchors: Vec<AnchorHeaderInput>,
    /// Shared history for every pre-Q anchor; absent when opening only Q directly.
    pub beacon: Option<BeaconHistoryInput>,
}

impl AnchoredL1Blocks {
    /// Builds temporary borrowed views after enforcing caller-supplied count budgets.
    ///
    /// This conversion does not verify headers or membership. Authentication belongs to
    /// [`crate::l1_history::verify_l1_history`], with exact anchor coverage checked by the
    /// composing range. The callback cannot retain references to the temporary view arrays.
    /// Outer encoded-byte limits must already have been enforced before serde allocation.
    ///
    /// # Errors
    /// Returns [`L1HistoryError::Budget`] if header or helper counts exceed trusted limits.
    pub fn with_proof<T>(
        &self,
        limits: L1HistoryLimits,
        callback: impl for<'a> FnOnce(L1HistoryProof<'a>) -> T,
    ) -> Result<T, L1HistoryError> {
        // Budget temporary views before allocating them. The existing verifier checks these
        // budgets again, and remains authoritative for witness semantics and byte limits.
        if self.anchors.len() >= limits.max_headers {
            return Err(L1HistoryError::Budget);
        }
        let mut remaining = limits.max_proof_nodes;
        let mut charge = |count| -> Result<(), L1HistoryError> {
            remaining = remaining.checked_sub(count).ok_or(L1HistoryError::Budget)?;
            Ok(())
        };
        for anchor in &self.anchors {
            charge(anchor.payload_siblings.len())?;
        }
        if let Some(beacon) = &self.beacon {
            if beacon.periods.len() > self.anchors.len() {
                return Err(L1HistoryError::Budget);
            }
            charge(beacon.state_helpers.len())?;
            for period in &beacon.periods {
                charge(period.helpers.len())?;
                charge(usize::from(period.legacy_state_roots.is_some()))?;
            }
        }
        let anchors: Vec<_> = self
            .anchors
            .iter()
            .map(|anchor| AnchorHeaderWitness {
                raw_header: &anchor.raw_header,
                beacon_header: anchor.beacon_header,
                payload_siblings: &anchor.payload_siblings,
            })
            .collect();
        let periods: Vec<_> = self
            .beacon
            .iter()
            .flat_map(|beacon| &beacon.periods)
            .map(|period| HistoricalPeriodWitness {
                period: period.period,
                block_roots: period.block_roots,
                legacy_state_roots: period.legacy_state_roots,
                helpers: &period.helpers,
            })
            .collect();
        let beacon = self.beacon.as_ref().map(|beacon| BeaconHistoryWitness {
            reference: beacon.reference,
            state_helpers: &beacon.state_helpers,
            summary_length: beacon.summary_length,
            legacy_length: beacon.legacy_length,
            periods: &periods,
        });
        Ok(callback(L1HistoryProof {
            reference_header: &self.reference_header,
            anchors: &anchors,
            beacon,
            limits,
        }))
    }
}

impl From<L1HistoryProof<'_>> for AnchoredL1Blocks {
    fn from(value: L1HistoryProof<'_>) -> Self {
        Self {
            reference_header: value.reference_header.to_vec(),
            anchors: value
                .anchors
                .iter()
                .map(|anchor| AnchorHeaderInput {
                    raw_header: anchor.raw_header.to_vec(),
                    beacon_header: anchor.beacon_header,
                    payload_siblings: anchor.payload_siblings.to_vec(),
                })
                .collect(),
            beacon: value.beacon.map(|beacon| BeaconHistoryInput {
                reference: beacon.reference,
                state_helpers: beacon.state_helpers.to_vec(),
                summary_length: beacon.summary_length,
                legacy_length: beacon.legacy_length,
                periods: beacon
                    .periods
                    .iter()
                    .map(|period| HistoricalPeriodInput {
                        period: period.period,
                        block_roots: period.block_roots,
                        legacy_state_roots: period.legacy_state_roots,
                        helpers: period.helpers.to_vec(),
                    })
                    .collect(),
            }),
        }
    }
}

/// Original Comet/execution material for one block, before authentication.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeBlockInput {
    /// Supplied Comet header using its existing transport representation.
    pub comet_header: CometHeader,
    /// Original Comet transaction bytes in supplied order.
    pub comet_transactions: Vec<Vec<u8>>,
    /// Supplied execution header using its existing transport representation.
    pub execution_header: Header,
    /// Exact original EIP2718 execution transactions in supplied order.
    pub execution_transactions: Vec<Vec<u8>>,
}

/// Provisional owned native input, excluding all caller authority and execution resources.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeInput {
    /// Complete supplied range in original order.
    pub blocks: Vec<RangeBlockInput>,
    /// Complete requested landing statement.
    pub statement: LandStatementInput,
    /// Supplied cursor after every block, without count truncation.
    pub post_block_cursors: Vec<u64>,
    /// Supplied excluded execution parent, authenticated by the caller's context.
    pub parent_execution_header: Header,
    /// Q-relative L1 anchor headers and their shared authentication material.
    pub l1_history: AnchoredL1Blocks,
}

/// Owned proof resources for one distinct L1 anchor; its root is derived by the verifier.
///
/// `W` is the caller's existing account/storage witness type. The generic carrier avoids a
/// second proof representation and does not make a deserialized witness authoritative.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct L1CommitteeInput<W> {
    /// Number matched against an authenticated L1 header, never an independent root.
    pub anchor_number: u64,
    /// Existing account/storage proof resources for Inbox slots 278 and 279.
    pub witness: W,
    /// Preimage that must match the proven anchor fallback-key hash.
    pub fallback_keys: etna_core::fallback::FallbackKeys,
}

/// Owned parent consensus bootstrap, required only when p exceeds the generation origin.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentConsensusInput {
    /// Existing Comet header representation, hashed against the authenticated parent identity.
    pub header: CometHeader,
    /// Native keys and powers that must open the parent's `validators_hash`.
    pub active: Vec<(etna_core::key::ConsensusKey, u64)>,
    /// Native keys and powers that must open the parent's `next_validators_hash`.
    pub next: Vec<(etna_core::key::ConsensusKey, u64)>,
    /// Exact raw p-anchor header used only by the parent-anchor exception.
    pub anchor_header: Vec<u8>,
}

/// Owned consensus inputs consumed by the state-authenticated native range verifier.
///
/// Original native blocks and certificates remain canonical protobuf bytes. Only the parent
/// bootstrap supplies sets directly, and its keys/powers are checked against the actual parent
/// header. Every later set is derived from authenticated state by the shared committee core.
/// No context, resource limit, signer authorization, or host-verified flag is carried here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusInput<W> {
    /// Original canonical native Comet block bytes for every covered height.
    pub original_blocks: Vec<Vec<u8>>,
    /// Exactly the required standalone native certificates, in ascending height order.
    pub required_commits: Vec<Vec<u8>>,
    /// Preimage of the generation origin's fixed fallback hash.
    pub origin_fallback_keys: etna_core::fallback::FallbackKeys,
    /// Existing Inbox account/storage resources under the authenticated L1 reference root.
    pub reference_state: W,
    /// Existing resources for each distinct covered or parent anchor, in increasing number order.
    pub anchors: Vec<L1CommitteeInput<W>>,
    /// Parent bootstrap required exactly when p is above the generation origin.
    pub parent: Option<ParentConsensusInput>,
}

/// Owned range plus its native committee, original-body and certificate proof resources.
///
/// Existing block execution witnesses and shared-node pools remain separate caller arguments.
/// The guest instantiates `W` with the existing execution-witness type; serialization conveys
/// no authority. Encoded input bytes must be capped before deserializing this transport.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusRangeInput<W> {
    /// Existing owned DA/range/SSZ request.
    pub range: RangeInput,
    /// Original native consensus bytes and existing state proof resources.
    pub consensus: ConsensusInput<W>,
}
