//! Sparse Ethereum execution-header authentication relative to a landing statement.

mod merkle;

use alloc::{collections::BTreeMap, vec::Vec};
use alloy_primitives::{B256, U256};
use etna_core::l1_header::{L1HeaderView, MAX_HEADER_BYTES};
use merkle::{Leaves, header_root, open, pair, scalar};

/// Policy supplied by the authenticated deployment profile.
#[derive(Clone, Copy, Debug)]
pub struct L1HistoryLimits {
    /// Total raw execution header occurrences, including the reference.
    pub max_headers: usize,
    /// Total auxiliary SHA-256 nodes across all body, state and period proofs.
    pub max_proof_nodes: usize,
}

/// One auxiliary node, indexed relative to its own tree root.
#[derive(Clone, Copy, Debug)]
pub struct SszProofNode {
    /// Positive generalized index. Helpers are supplied in ascending order.
    pub index: u64,
    /// SSZ subtree root.
    pub root: B256,
}

/// The five consensus-layer `BeaconBlockHeader` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeaconHeader {
    /// Produced beacon slot.
    pub slot: u64,
    /// Proposing validator.
    pub proposer_index: u64,
    /// Previous produced beacon header root.
    pub parent_root: B256,
    /// Post-block beacon state root.
    pub state_root: B256,
    /// Beacon block body root.
    pub body_root: B256,
}

/// Membership opening for a distinct execution anchor.
#[derive(Clone, Copy, Debug)]
pub struct AnchorHeaderWitness<'a> {
    /// Original raw execution header.
    pub raw_header: &'a [u8],
    /// Containing beacon header for a pre-reference anchor.
    pub beacon_header: Option<BeaconHeader>,
    /// Execution block hash to beacon body root, ordered leaf to root.
    pub payload_siblings: &'a [B256],
}

/// Completed vector shared by all targets in the same 8192-slot period.
#[derive(Clone, Copy, Debug)]
pub struct HistoricalPeriodWitness<'a> {
    /// Zero-based period since genesis.
    pub period: u64,
    /// Block roots vector commitment.
    pub block_roots: B256,
    /// Other half of a legacy `HistoricalBatch`, present only for legacy periods.
    pub legacy_state_roots: Option<B256>,
    /// Helpers local to the block roots vector.
    pub helpers: &'a [SszProofNode],
}

/// Consensus history rooted at Q's parent beacon block.
#[derive(Clone, Copy, Debug)]
pub struct BeaconHistoryWitness<'a> {
    /// Parent beacon block committed by Q.
    pub reference: BeaconHeader,
    /// One multiproof scoped to the reference post-state.
    pub state_helpers: &'a [SszProofNode],
    /// Historical summary count when a post-Capella completed period is queried.
    pub summary_length: Option<u64>,
    /// Historical roots count when a legacy period is queried.
    pub legacy_length: Option<u64>,
    /// Completed period groups in increasing period order.
    pub periods: &'a [HistoricalPeriodWitness<'a>],
}

/// Native borrowed witness for one explicit landing reference.
#[derive(Clone, Copy, Debug)]
pub struct L1HistoryProof<'a> {
    /// Exact raw execution reference header.
    pub reference_header: &'a [u8],
    /// All anchor openings sorted by strictly increasing execution height.
    pub anchors: &'a [AnchorHeaderWitness<'a>],
    /// Shared beacon history, required exactly for a pre-reference anchor.
    pub beacon: Option<BeaconHistoryWitness<'a>>,
    /// Caller-authenticated resource policy.
    pub limits: L1HistoryLimits,
}

/// Failure authenticating the supplied L1 history.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum L1HistoryError {
    /// The input exceeds its configured resource allowance.
    #[error("L1 history count or byte budget exceeded")]
    Budget,
    /// Invalid framing, required header field or commitment.
    #[error("invalid L1 history header")]
    Header,
    /// Statement reference or supplied execution heights disagree.
    #[error("L1 history reference or anchor identity mismatch")]
    Identity,
    /// Unknown network, fork, or inconsistent slot.
    #[error("unsupported L1 history chain, slot or fork")]
    Schema,
    /// Required root, opening position or canonical helper set disagrees.
    #[error("invalid L1 history Merkle opening")]
    Merkle,
    /// Missing, unused, or inconsistent history material.
    #[error("empty, missing or inconsistent L1 history witness")]
    Shape,
}

const WINDOW: u64 = 8192;
const LIST_LIMIT: u64 = 1 << 24;

struct Network {
    genesis: u64,
    epochs: [u64; 5],
    versions: [u32; 5],
    unsupported_epoch: Option<u64>,
}

impl Network {
    // Official eth-clients/{mainnet,sepolia,hoodi} config revisions are pinned in the fixture README.
    fn from_id(id: U256) -> Result<Self, L1HistoryError> {
        match u64::try_from(id).ok() {
            Some(1) => Ok(Self {
                genesis: 1_606_824_023,
                epochs: [144_896, 194_048, 269_568, 364_032, 411_392],
                versions: [
                    0x0200_0000,
                    0x0300_0000,
                    0x0400_0000,
                    0x0500_0000,
                    0x0600_0000,
                ],
                unsupported_epoch: None,
            }),
            Some(11_155_111) => Ok(Self {
                genesis: 1_655_733_600,
                epochs: [100, 56_832, 132_608, 222_464, 272_640],
                versions: [
                    0x9000_0071,
                    0x9000_0072,
                    0x9000_0073,
                    0x9000_0074,
                    0x9000_0075,
                ],
                unsupported_epoch: Some(353_024),
            }),
            Some(560_048) => Ok(Self {
                genesis: 1_742_213_400,
                epochs: [0, 0, 0, 2048, 50_688],
                versions: [
                    0x3000_0910,
                    0x4000_0910,
                    0x5000_0910,
                    0x6000_0910,
                    0x7000_0910,
                ],
                unsupported_epoch: None,
            }),
            _ => Err(L1HistoryError::Schema),
        }
    }

    fn slot(&self, timestamp: u64) -> Result<u64, L1HistoryError> {
        let elapsed = timestamp
            .checked_sub(self.genesis)
            .ok_or(L1HistoryError::Schema)?;
        if elapsed % 12 != 0 {
            return Err(L1HistoryError::Schema);
        }
        Ok(elapsed / 12)
    }

    fn schema(&self, slot: u64) -> Result<Schema, L1HistoryError> {
        let epoch = slot / 32;
        if self.unsupported_epoch.is_some_and(|first| epoch >= first) {
            return Err(L1HistoryError::Schema);
        }
        let fork = self
            .epochs
            .iter()
            .rposition(|start| epoch >= *start)
            .ok_or(L1HistoryError::Schema)?;
        Ok(Schema {
            state_base: if fork < 3 { 32 } else { 64 },
            payload_base: if fork < 2 { 16 } else { 32 },
            version: self.versions[fork].to_be_bytes(),
            summaries: fork >= 1,
        })
    }
}

#[derive(Clone, Copy)]
struct Schema {
    state_base: u64,
    payload_base: u64,
    version: [u8; 4],
    summaries: bool,
}

// L1HeaderView has already validated complete RLP item boundaries, the raw byte limit,
// and Q's hash. Leave all fields except the stable EIP-4788 field opaque.
fn parent_beacon_root(mut raw: &[u8]) -> Result<B256, L1HistoryError> {
    let mut fields =
        alloy_rlp::Header::decode_bytes(&mut raw, true).map_err(|_| L1HistoryError::Header)?;
    for _ in 0..19 {
        let header = alloy_rlp::Header::decode(&mut fields).map_err(|_| L1HistoryError::Header)?;
        fields = fields
            .get(header.payload_length..)
            .ok_or(L1HistoryError::Header)?;
    }
    let bytes =
        alloy_rlp::Header::decode_bytes(&mut fields, false).map_err(|_| L1HistoryError::Header)?;
    B256::try_from(bytes).map_err(|_| L1HistoryError::Header)
}

fn check_budget(proof: &L1HistoryProof<'_>) -> Result<(), L1HistoryError> {
    if proof.anchors.len() >= proof.limits.max_headers
        || proof.reference_header.len() > MAX_HEADER_BYTES
    {
        return Err(L1HistoryError::Budget);
    }
    let mut remaining = proof.limits.max_proof_nodes;
    let mut charge = |count| -> Result<(), L1HistoryError> {
        remaining = remaining.checked_sub(count).ok_or(L1HistoryError::Budget)?;
        Ok(())
    };
    if let Some(beacon) = proof.beacon {
        if beacon.periods.len() > proof.anchors.len() {
            return Err(L1HistoryError::Budget);
        }
        charge(beacon.state_helpers.len())?;
        for period in beacon.periods {
            charge(period.helpers.len())?;
            charge(usize::from(period.legacy_state_roots.is_some()))?;
        }
    }
    for anchor in proof.anchors {
        if anchor.raw_header.len() > MAX_HEADER_BYTES {
            return Err(L1HistoryError::Budget);
        }
        charge(anchor.payload_siblings.len())?;
    }
    Ok(())
}

struct HistoryState {
    leaves: Leaves,
    periods: BTreeMap<u64, Leaves>,
    schema: Schema,
}

impl HistoryState {
    fn new(reference: &BeaconHeader, schema: Schema) -> Result<Self, L1HistoryError> {
        // BeaconState.slot is field 2. BeaconState.fork is field 3, and
        // Fork.current_version is field 1 of its four-leaf container.
        let mut leaves = Leaves::new();
        open(&mut leaves, schema.state_base + 2, scalar(reference.slot))?;
        let mut version = B256::ZERO;
        version[..4].copy_from_slice(&schema.version);
        open(&mut leaves, (schema.state_base + 3) * 4 + 1, version)?;
        Ok(Self {
            leaves,
            periods: BTreeMap::new(),
            schema,
        })
    }

    fn add_target(
        &mut self,
        reference: &BeaconHeader,
        target: &BeaconHeader,
    ) -> Result<(), L1HistoryError> {
        let age = reference
            .slot
            .checked_sub(target.slot)
            .ok_or(L1HistoryError::Schema)?;
        let offset = target.slot % WINDOW;
        match age {
            0 if target == reference => Ok(()),
            0 => Err(L1HistoryError::Identity),
            1..=WINDOW => open(
                &mut self.leaves,
                (self.schema.state_base + 5) * WINDOW + offset,
                header_root(target),
            ),
            _ => open(
                self.periods.entry(target.slot / WINDOW).or_default(),
                WINDOW + offset,
                header_root(target),
            ),
        }
    }

    fn verify_periods(
        &mut self,
        network: &Network,
        beacon: &BeaconHistoryWitness<'_>,
    ) -> Result<(), L1HistoryError> {
        if beacon.periods.len() != self.periods.len() {
            return Err(L1HistoryError::Shape);
        }
        let completed = beacon.reference.slot / WINDOW;
        // BeaconState fields 27 and 7 contain lists with a 24-level contents tree
        // under child 2 of the list root and a length chunk at child 3.
        let summary_field = self.schema.state_base + 27;
        let legacy_field = self.schema.state_base + 7;
        // At an aligned Capella transition the previous fork closed the last legacy period.
        let summary_start = if self.schema.summaries && !self.periods.is_empty() {
            let length = beacon
                .summary_length
                .filter(|n| *n <= LIST_LIMIT)
                .ok_or(L1HistoryError::Shape)?;
            let start = completed.checked_sub(length).ok_or(L1HistoryError::Shape)?;
            if start != network.epochs[1] * 32 / WINDOW {
                return Err(L1HistoryError::Shape);
            }
            open(&mut self.leaves, summary_field * 2 + 1, scalar(length))?;
            start
        } else {
            if beacon.summary_length.is_some() {
                return Err(L1HistoryError::Shape);
            }
            completed
        };
        let needs_legacy = self
            .periods
            .keys()
            .next()
            .is_some_and(|p| *p < summary_start);
        if needs_legacy {
            let length = beacon
                .legacy_length
                .filter(|n| *n <= LIST_LIMIT && *n == summary_start)
                .ok_or(L1HistoryError::Shape)?;
            open(&mut self.leaves, legacy_field * 2 + 1, scalar(length))?;
        } else if beacon.legacy_length.is_some() {
            return Err(L1HistoryError::Shape);
        }

        for ((period, leaves), group) in core::mem::take(&mut self.periods)
            .into_iter()
            .zip(beacon.periods)
        {
            if period != group.period || period >= completed {
                return Err(L1HistoryError::Shape);
            }
            merkle::verify(leaves, group.helpers, group.block_roots)?;
            if period < summary_start {
                let state_roots = group.legacy_state_roots.ok_or(L1HistoryError::Shape)?;
                open(
                    &mut self.leaves,
                    legacy_field * 2 * LIST_LIMIT + period,
                    pair(group.block_roots, state_roots),
                )?;
            } else {
                if group.legacy_state_roots.is_some() {
                    return Err(L1HistoryError::Shape);
                }
                let entry = summary_field * 2 * LIST_LIMIT + (period - summary_start);
                open(&mut self.leaves, entry * 2, group.block_roots)?;
            }
        }
        Ok(())
    }
}

/// Authenticates every opened original execution header relative to the given statement reference.
///
/// # Errors
///
/// Returns an error if any raw header, consensus opening or deployment bound fails.
pub fn verify_l1_history<'a>(
    chain_id: U256,
    reference_number: u64,
    reference_hash: B256,
    proof: &L1HistoryProof<'a>,
) -> Result<Vec<L1HeaderView<'a>>, L1HistoryError> {
    check_budget(proof)?;
    if proof.anchors.is_empty() {
        return Err(L1HistoryError::Shape);
    }
    let network = Network::from_id(chain_id)?;
    let q = L1HeaderView::from_raw(proof.reference_header).map_err(|_| L1HistoryError::Header)?;
    if q.number() != reference_number || q.hash() != reference_hash {
        return Err(L1HistoryError::Identity);
    }
    let mut history = if let Some(beacon) = proof.beacon {
        if header_root(&beacon.reference) != parent_beacon_root(q.raw())? {
            return Err(L1HistoryError::Identity);
        }
        let q_slot = network.slot(q.timestamp())?;
        if q_slot / 32 < network.epochs[2] || beacon.reference.slot >= q_slot {
            return Err(L1HistoryError::Schema);
        }
        Some(HistoryState::new(
            &beacon.reference,
            network.schema(beacon.reference.slot)?,
        )?)
    } else {
        None
    };

    let mut headers = Vec::with_capacity(proof.anchors.len());
    let mut previous = None;
    for anchor in proof.anchors {
        let header = if anchor.raw_header == q.raw() {
            q
        } else {
            L1HeaderView::from_raw(anchor.raw_header).map_err(|_| L1HistoryError::Header)?
        };
        if previous.is_some_and(|n| n >= header.number()) || header.number() > reference_number {
            return Err(L1HistoryError::Identity);
        }
        previous = Some(header.number());
        if header.number() == reference_number {
            if header.raw() != q.raw()
                || anchor.beacon_header.is_some()
                || !anchor.payload_siblings.is_empty()
            {
                return Err(L1HistoryError::Identity);
            }
        } else {
            let target = anchor.beacon_header.as_ref().ok_or(L1HistoryError::Shape)?;
            let beacon = proof.beacon.as_ref().ok_or(L1HistoryError::Shape)?;
            if network.slot(header.timestamp())? != target.slot {
                return Err(L1HistoryError::Schema);
            }
            let schema = network.schema(target.slot)?;
            // Body field 9 is the execution payload (gindex 25). Its field 12
            // is block_hash, at a depth selected from this target's own slot.
            merkle::verify_branch(
                25 * schema.payload_base + 12,
                header.hash(),
                anchor.payload_siblings,
                target.body_root,
            )?;
            history
                .as_mut()
                .ok_or(L1HistoryError::Shape)?
                .add_target(&beacon.reference, target)?;
        }
        headers.push(header);
    }
    if let (Some(mut state), Some(beacon)) = (history, proof.beacon) {
        if headers[0].number() == reference_number {
            return Err(L1HistoryError::Shape);
        }
        state.verify_periods(&network, &beacon)?;
        merkle::verify(
            state.leaves,
            beacon.state_helpers,
            beacon.reference.state_root,
        )?;
    }
    Ok(headers)
}

#[cfg(test)]
mod tests;
