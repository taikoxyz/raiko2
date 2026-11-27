//! State-authenticated native committee, range-QC and LastCommit miss-bitmap composition.

use super::{verify_blob_range_statement_execution, EtnaBlockExecutionWitness, L1AnchorProof};
use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::spec::TaikoExecutorSpec;
use alloy_consensus::Header;
use alloy_primitives::{Address, B256, U256};
use anyhow::{ensure, Context, Result};
use etna_core::{
    comet::{
        codec::{decode_canonical_block, decode_canonical_commit},
        native::{Block as CometBlock, Commit, CommitSig},
        validators::Validators,
        verify::{block_parts, header_hash, verify_body_at_origin, CertifiedBlockRef},
    },
    extra_data::ExtraData,
    fallback::FallbackKeys,
    l1_header::L1HeaderView,
    miss,
    params::config_hash,
    staking::{key_to_seat_slot, snapshot_slots, StakingWords, SNAPSHOT_SLOT_COUNT},
};
use raiko2_primitives::{ExecutionWitness, WitnessStateNode};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_etna::{
    consensus::{authenticate_origin, derive_signers, origin_slots},
    coverage::{verify_required_native_commits, CoverageContext, ExpectedValidatorSets},
    range::{RangeBlock, RangeProjectionContext},
    statement::LandStatement,
};
use raiko2_stateless::read_storage_batch_with_witness_resources;
use reth_ethereum_primitives::TransactionSigned;
use std::collections::BTreeMap;
use tendermint::block::Header as CometHeader;

/// Deployment parameters and decode budgets supplied by the authenticated caller.
#[derive(Clone, Copy, Debug)]
pub struct ConsensusParameters {
    /// L2 staking proxy committed by the Inbox configuration hash.
    pub staking: Address,
    /// U committed by the Inbox configuration hash.
    pub max_uncheckpointed: u64,
    /// Maximum original canonical protobuf bytes for one block.
    pub max_block_bytes: usize,
    /// Maximum canonical protobuf bytes for one standalone certificate.
    pub max_commit_bytes: usize,
}

/// Untrusted Inbox proof resources for a distinct anchor, identified by its number, never its root.
#[derive(Clone, Copy, Debug)]
pub struct L1CommitteeWitness<'a> {
    /// Number matched against a header already authenticated by the native L1 history verifier.
    pub anchor_number: u64,
    /// Existing account/storage proof nodes; cached witness headers convey no authority.
    pub witness: &'a ExecutionWitness,
    /// Preimage that must open the proven slot 278.
    pub fallback_keys: &'a FallbackKeys,
}

/// Supplied bootstrap data for p above the generation origin.
#[derive(Clone, Copy, Debug)]
pub struct ParentConsensusWitness<'a> {
    /// Header whose hash must equal the authenticated checkpoint's parent Comet hash.
    pub header: &'a CometHeader,
    /// Set that signs p, checked against that header's validators_hash.
    pub active: &'a Validators,
    /// Set that signs p+1, checked against that header's next_validators_hash.
    pub next: &'a Validators,
    /// Raw p-anchor header, bound to p's authenticated EL extraData and beacon root.
    /// This alternative supplies only p's committee inputs, never ordinary covered-anchor authority.
    pub anchor_header: &'a [u8],
}

/// Borrowed untrusted native consensus inputs. No set schedule or approved-state flag is accepted.
#[derive(Clone, Copy, Debug)]
pub struct ConsensusWitness<'a> {
    /// Exact canonical raw blocks for every covered height, including their LastCommit bodies.
    pub original_blocks: &'a [Vec<u8>],
    /// Exactly the required canonical standalone commits, in ascending height order.
    pub required_commits: &'a [Vec<u8>],
    /// Preimage of the generation origin's fixed fallback hash.
    pub origin_fallback_keys: &'a FallbackKeys,
    /// Existing Inbox witness nodes under the already authenticated L1 reference header's root.
    pub reference_state: &'a ExecutionWitness,
    /// One resource per distinct anchor used by covered blocks and, when p>o, by p.
    /// Entries are in increasing anchor-number order; unused and duplicate entries fail.
    pub anchors: &'a [L1CommitteeWitness<'a>],
    /// Required exactly when p exceeds the generation origin.
    pub parent: Option<ParentConsensusWitness<'a>>,
}

#[derive(Clone, Copy)]
struct Governance<'a> {
    fallback: &'a FallbackKeys,
    fallback_hash: B256,
    minimum: U256,
}

/// Strictly replays the DA-bound range and then authenticates its committee/QC/miss relation.
///
/// All 128 snapshot records and full-width height/power words come from existing execution
/// witnesses: block H+1's witness proves H's post-state. Each L2 root is hydrated once for its
/// snapshot plus all missing-validator seat reads. Inbox roots come only from the verified L1
/// history (or p's specifically limited authenticated-header alternative), never witness metadata.
///
/// The caller still authenticates the deployment/reference/checkpoint context and FI-disabled
/// profile/queue state, timing/rules and switching-network implementation requirements documented
/// by the existing strict verifier. This native component is not an SGX signature or an on-chain
/// landing authorization. Runtime signing remains unavailable until the whole relation is ready.
///
/// # Errors
/// Rejects any DA, SSZ, range, strict-EVM, origin/config/storage proof, signer commitment,
/// LastCommit/miss bitmap, exact required-certificate coverage or native certificate failure.
#[allow(clippy::too_many_arguments)]
pub fn verify_blob_range_statement_consensus(
    source: &InputDataSource,
    blocks: &[RangeBlock<'_, TransactionSigned>],
    context: &RangeProjectionContext,
    statement: &LandStatement<'_>,
    post_block_cursors: &[u64],
    parent_execution_header: &Header,
    l1_anchor_proof: L1AnchorProof<'_>,
    execution_witnesses: &[EtnaBlockExecutionWitness<'_>],
    shared_state_nodes: &[WitnessStateNode],
    config: &TaikoEvmConfig,
    parameters: &ConsensusParameters,
    consensus: ConsensusWitness<'_>,
) -> Result<B256> {
    ensure!(
        config.chain_spec().etna_staking_address() == Some(parameters.staking),
        "Etna configured staking target differs from the actual EVM block-start target"
    );
    ensure!(
        consensus.original_blocks.len() == blocks.len(),
        "Etna original block count mismatch"
    );
    ensure!(
        consensus.required_commits.len() <= blocks.len(),
        "too many Etna standalone commits"
    );
    ensure!(
        consensus.anchors.len() <= blocks.len().saturating_add(1),
        "too many Etna committee anchors"
    );
    ensure!(
        consensus
            .original_blocks
            .iter()
            .all(|raw| !raw.is_empty() && raw.len() <= parameters.max_block_bytes),
        "Etna original block exceeds trusted byte limit"
    );
    ensure!(
        consensus
            .required_commits
            .iter()
            .all(|raw| !raw.is_empty() && raw.len() <= parameters.max_commit_bytes),
        "Etna commit exceeds trusted byte limit"
    );
    let digest = verify_blob_range_statement_execution(
        source,
        blocks,
        context,
        statement,
        post_block_cursors,
        parent_execution_header,
        l1_anchor_proof,
        execution_witnesses,
        shared_state_nodes,
        config,
    )?;
    // The existing strict relation has now established every full EL header/root in this range.
    let l2_chain_id = u64::try_from(context.domain.l2_chain_id)
        .context("Etna L2 chain id exceeds config hash width")?;
    let expected_config = config_hash(
        l2_chain_id,
        parameters.staking,
        parameters.max_uncheckpointed,
    );
    ensure!(
        statement.config_hash == expected_config,
        "Etna configuration preimage mismatch"
    );
    let reference = L1HeaderView::from_raw(l1_anchor_proof.reference_header)?;
    let mut reference_slots = vec![
        (context.domain.inbox, U256::from(268)),
        (context.domain.inbox, U256::from(269)),
    ];
    reference_slots.extend(
        origin_slots(context.domain.generation)
            .map(|slot| (context.domain.inbox, U256::from_be_bytes(slot.0))),
    );
    let values = read_storage_batch_with_witness_resources(
        reference.state_root(),
        &reference_slots,
        consensus.reference_state,
        shared_state_nodes,
    )?;
    ensure!(
        values[0] == U256::from(context.domain.generation),
        "Etna proven current generation mismatch"
    );
    ensure!(
        B256::from(values[1]) == expected_config,
        "Etna proven Inbox config hash mismatch"
    );
    let origin_words: [U256; 4] = values[2..].try_into().context("Etna origin word count")?;
    let _origin = authenticate_origin(
        &origin_words,
        context.origin_height,
        consensus.origin_fallback_keys,
    )?;
    let origin_set = Validators::from_members(&consensus.origin_fallback_keys.validator_set())
        .context("invalid Etna origin fallback validators")?;

    let decoded = authenticate_bodies(blocks, context, consensus.original_blocks)?;
    let parent = authenticate_parent(context, consensus.parent)?;
    let governance = authenticate_governance(
        blocks,
        parent_execution_header,
        context,
        consensus,
        shared_state_nodes,
    )?;
    let mut schedule = vec![if context.parent_height < context.origin_height + 2 {
        origin_set.clone()
    } else {
        parent
            .context("missing Etna parent consensus bootstrap")?
            .next
            .clone()
    }];

    for (index, (block, body)) in blocks.iter().zip(&decoded).enumerate() {
        let state_header = if index == 0 {
            parent_execution_header
        } else {
            blocks[index - 1].execution_header
        };
        let state_height = state_header.number;
        let parent_signers = if index == 0 {
            parent.map(|parent| parent.active)
        } else {
            Some(&schedule[index - 1])
        };
        let missing = missing_validators(body, parent_signers, context.origin_height)?;
        // Read the snapshot even for an unusable header: all 128 records are authenticated.
        let read_snapshot = state_height > context.origin_height;
        let mut requests = Vec::with_capacity(
            if read_snapshot {
                SNAPSHOT_SLOT_COUNT
            } else {
                0
            } + missing.len(),
        );
        if read_snapshot {
            requests.extend(
                snapshot_slots().map(|slot| (parameters.staking, U256::from_be_bytes(slot.0))),
            );
        }
        requests.extend(missing.iter().map(|&address| {
            (
                parameters.staking,
                U256::from_be_bytes(key_to_seat_slot(address).0),
            )
        }));
        let values = read_storage_batch_with_witness_resources(
            state_header.state_root,
            &requests,
            execution_witnesses[index].witness,
            shared_state_nodes,
        )?;
        let seats = &values[if read_snapshot {
            SNAPSHOT_SLOT_COUNT
        } else {
            0
        }..];
        let mut seat_index = 0;
        let bitmap = miss::miss_bitmap(&missing, |_| {
            let value = seats[seat_index];
            seat_index += 1;
            value
        });
        ensure!(
            ExtraData::decode(block.execution_header.extra_data.as_ref())?.miss_bitmap == bitmap,
            "Etna miss bitmap mismatch at height {}",
            block.execution_header.number
        );
        let next = if read_snapshot {
            let snapshot = StakingWords::from_snapshot_values(&values[..SNAPSHOT_SLOT_COUNT])
                .context("Etna incomplete staking snapshot")?;
            let anchor = ExtraData::decode(state_header.extra_data.as_ref())?.anchor;
            let proven = governance
                .get(&anchor)
                .context("missing Etna deciding-anchor governance")?;
            derive_signers(
                state_height,
                &snapshot,
                proven.minimum,
                proven.fallback_hash,
                proven.fallback,
            )?
        } else {
            origin_set.clone()
        };
        schedule.push(next);
    }
    let headers: Vec<_> = blocks
        .iter()
        .map(|block| block.comet_header.clone())
        .collect();
    let expected: Vec<_> = schedule
        .windows(2)
        .map(|pair| ExpectedValidatorSets {
            active: &pair[0],
            next: &pair[1],
        })
        .collect();
    let commits = consensus
        .required_commits
        .iter()
        .map(|raw| decode_canonical_commit(raw))
        .collect::<Result<Vec<_>, _>>()?;
    // Native commit verification binds header/height/domain/keys; additionally bind its part set
    // to the original canonical body, even when a different valid standalone round is supplied.
    for commit in &commits {
        let offset = commit
            .height
            .value()
            .checked_sub(context.parent_height + 1)
            .and_then(|offset| usize::try_from(offset).ok())
            .context("Etna QC outside range")?;
        let raw = consensus
            .original_blocks
            .get(offset)
            .context("Etna QC outside range")?;
        let (raw, _) = decode_canonical_block(raw)?;
        ensure!(
            commit.block_id.part_set_header == block_parts(&raw)?,
            "Etna QC original block part-set mismatch"
        );
    }
    verify_required_native_commits(
        &headers,
        &expected,
        &CoverageContext {
            parent_height: context.parent_height,
            last_height: context.last_height,
            max_blocks: context.max_blocks,
        },
        &commits,
        &context.domain,
    )?;
    Ok(digest)
}

fn authenticate_parent<'a>(
    context: &RangeProjectionContext,
    parent: Option<ParentConsensusWitness<'a>>,
) -> Result<Option<ParentConsensusWitness<'a>>> {
    if context.parent_height == context.origin_height {
        ensure!(
            parent.is_none(),
            "unexpected Etna origin parent consensus bootstrap"
        );
        return Ok(None);
    }
    let parent = parent.context("missing Etna parent consensus bootstrap")?;
    ensure!(
        parent.header.height.value() == context.parent_height,
        "Etna parent Comet height mismatch"
    );
    ensure!(
        header_hash(parent.header).as_bytes() == context.parent_comet_hash.as_slice(),
        "Etna parent Comet hash mismatch"
    );
    ensure!(
        parent.active.hash() == parent.header.validators_hash,
        "Etna parent active validators mismatch"
    );
    ensure!(
        parent.next.hash() == parent.header.next_validators_hash,
        "Etna parent next validators mismatch"
    );
    Ok(Some(parent))
}

fn authenticate_bodies(
    blocks: &[RangeBlock<'_, TransactionSigned>],
    context: &RangeProjectionContext,
    originals: &[Vec<u8>],
) -> Result<Vec<CometBlock>> {
    blocks
        .iter()
        .zip(originals)
        .map(|(block, raw)| {
            let (raw_block, decoded) = decode_canonical_block(raw)?;
            ensure!(
                decoded.header == *block.comet_header && decoded.data == block.comet_transactions,
                "Etna original Comet body differs from the DA-bound range"
            );
            ensure!(
                raw_block
                    .evidence
                    .as_ref()
                    .is_none_or(|list| list.evidence.is_empty()),
                "Etna block contains forbidden evidence"
            );
            // This value serves only the shared body check's part-set argument. It is never a QC,
            // never enters signer derivation, and is never counted toward certificate coverage.
            let mut shape = Commit::default();
            shape.block_id.part_set_header = block_parts(&raw_block)?;
            verify_body_at_origin(
                CertifiedBlockRef {
                    block: &decoded,
                    raw_block: raw,
                    commit: &shape,
                },
                block.execution_header.number,
                context.origin_height,
            )?;
            Ok(decoded)
        })
        .collect()
}

fn missing_validators(
    body: &CometBlock,
    signers: Option<&Validators>,
    origin_height: u64,
) -> Result<Vec<Address>> {
    if body.header.height.value() == origin_height + 1 {
        ensure!(
            body.last_commit.is_none(),
            "Etna generation first block has LastCommit"
        );
        return Ok(vec![]);
    }
    let signers = signers.context("missing Etna parent signer set")?;
    let last = body
        .last_commit
        .as_ref()
        .context("missing Etna LastCommit")?;
    let committed: Vec<_> = last
        .signatures
        .iter()
        .map(|signature| matches!(signature, CommitSig::BlockIdFlagCommit { .. }))
        .collect();
    let addresses: Vec<_> = signers
        .validators()
        .iter()
        .map(|validator| Address::from_slice(validator.address.as_bytes()))
        .collect();
    miss::missing_addresses(&addresses, &committed)
        .context("Etna LastCommit signer-slot count mismatch")
}

fn authenticate_governance<'a>(
    blocks: &[RangeBlock<'_, TransactionSigned>],
    parent_header: &Header,
    context: &RangeProjectionContext,
    consensus: ConsensusWitness<'a>,
    shared: &[WitnessStateNode],
) -> Result<BTreeMap<u64, Governance<'a>>> {
    let mut roots = BTreeMap::new();
    for block in blocks {
        let anchor = ExtraData::decode(block.execution_header.extra_data.as_ref())?.anchor;
        let root = block
            .execution_header
            .parent_beacon_block_root
            .context("missing verified Etna anchor root")?;
        if let Some(previous) = roots.insert(anchor, root) {
            ensure!(previous == root, "Etna anchor root changed at one number");
        }
    }
    if context.parent_height > context.origin_height {
        let parent = consensus
            .parent
            .context("missing Etna parent consensus bootstrap")?;
        let anchor = ExtraData::decode(parent_header.extra_data.as_ref())?;
        let raw = L1HeaderView::from_raw(parent.anchor_header)?;
        ensure!(
            raw.number() == anchor.anchor
                && Some(raw.state_root()) == parent_header.parent_beacon_block_root,
            "Etna parent anchor header/root mismatch"
        );
        if let Some(previous) = roots.insert(anchor.anchor, raw.state_root()) {
            ensure!(
                previous == raw.state_root(),
                "Etna parent anchor disagrees with covered authenticated anchor"
            );
        }
    }
    ensure!(
        roots.len() == consensus.anchors.len(),
        "Etna distinct committee anchor count mismatch"
    );
    let mut result = BTreeMap::new();
    for ((&number, &root), resource) in roots.iter().zip(consensus.anchors) {
        ensure!(
            resource.anchor_number == number,
            "Etna committee anchor order or identity mismatch"
        );
        let values = read_storage_batch_with_witness_resources(
            root,
            &[
                (context.domain.inbox, U256::from(278)),
                (context.domain.inbox, U256::from(279)),
            ],
            resource.witness,
            shared,
        )?;
        let fallback_hash = B256::from(values[0]);
        ensure!(
            resource.fallback_keys.hash() == fallback_hash,
            "Etna proven anchor fallback hash mismatch"
        );
        result.insert(
            number,
            Governance {
                fallback: resource.fallback_keys,
                fallback_hash,
                minimum: values[1],
            },
        );
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::TrieAccount;
    use alloy_primitives::{hex, keccak256, KECCAK256_EMPTY};
    use etna_core::{domain::DeploymentDomain, key::ConsensusKey};
    use risc0_ethereum_trie::Trie;

    fn inbox_witness(
        inbox: Address,
        fallback: &FallbackKeys,
        minimum: u64,
    ) -> (B256, ExecutionWitness) {
        let mut storage = Trie::default();
        for (slot, value) in [
            (278, U256::from_be_bytes(fallback.hash().0)),
            (279, U256::from(minimum)),
        ] {
            storage.insert(
                keccak256(B256::from(U256::from(slot))),
                alloy_rlp::encode(value),
            );
        }
        let mut state = Trie::default();
        state.insert(
            keccak256(inbox),
            alloy_rlp::encode(TrieAccount {
                nonce: 1,
                balance: U256::ZERO,
                storage_root: storage.hash_slow(),
                code_hash: KECCAK256_EMPTY,
            }),
        );
        let root = state.hash_slow();
        let witness = ExecutionWitness {
            state: state
                .rlp_nodes()
                .into_iter()
                .chain(storage.rlp_nodes())
                .map(WitnessStateNode::from_bytes)
                .collect(),
            ..Default::default()
        };
        (root, witness)
    }

    #[test]
    fn distinct_anchor_governance_reads_cannot_reuse_other_roots_witnesses() {
        // This focused helper test supplies already-authenticated-header equivalents. It tests
        // distinct-root MPT composition; the integrated fixtures exercise the full native entry.
        let inbox = Address::with_last_byte(0x11);
        let fallback = FallbackKeys::new(vec![ConsensusKey::from_slice(&hex!(
            "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
        ))
        .unwrap()])
        .unwrap();
        let (first_root, first_witness) = inbox_witness(inbox, &fallback, 100);
        let (second_root, second_witness) = inbox_witness(inbox, &fallback, 200);
        assert_ne!(first_root, second_root);
        let execution_headers =
            [(18, first_root), (19, second_root)].map(|(anchor, root)| Header {
                extra_data: ExtraData {
                    basefee_sharing_pctg: 100,
                    generation: 2,
                    anchor,
                    miss_bitmap: 0,
                }
                .encode()
                .unwrap()
                .to_vec()
                .into(),
                parent_beacon_block_root: Some(root),
                ..Default::default()
            });
        let raw = hex::decode(
            include_str!("../../../protocol-etna/tests/fixtures/first-block.hex").trim(),
        )
        .unwrap();
        let (_, comet) = decode_canonical_block(&raw).unwrap();
        let blocks = execution_headers.each_ref().map(|header| RangeBlock {
            comet_header: &comet.header,
            comet_transactions: &[],
            execution_header: header,
            execution_transactions: &[],
        });
        let context = RangeProjectionContext {
            domain: DeploymentDomain {
                l1_chain_id: U256::from(1),
                l2_chain_id: U256::from(167_001),
                inbox,
                generation: 2,
            },
            origin_height: 0,
            parent_height: 0,
            last_height: 2,
            parent_el_hash: B256::ZERO,
            parent_comet_hash: B256::ZERO,
            last_el_hash: B256::ZERO,
            last_comet_hash: B256::ZERO,
            max_blocks: 2,
            max_envelope_bytes: 130_000,
        };
        let anchors = [
            L1CommitteeWitness {
                anchor_number: 18,
                witness: &first_witness,
                fallback_keys: &fallback,
            },
            L1CommitteeWitness {
                anchor_number: 19,
                witness: &second_witness,
                fallback_keys: &fallback,
            },
        ];
        let witness = ConsensusWitness {
            original_blocks: &[],
            required_commits: &[],
            origin_fallback_keys: &fallback,
            reference_state: &first_witness,
            anchors: &anchors,
            parent: None,
        };
        let proven =
            authenticate_governance(&blocks, &Header::default(), &context, witness, &[]).unwrap();
        assert_eq!(proven[&18].minimum, U256::from(100));
        assert_eq!(proven[&19].minimum, U256::from(200));
        let wrong_roots = [
            anchors[0],
            L1CommitteeWitness {
                witness: &first_witness,
                ..anchors[1]
            },
        ];
        assert!(authenticate_governance(
            &blocks,
            &Header::default(),
            &context,
            ConsensusWitness {
                anchors: &wrong_roots,
                ..witness
            },
            &[]
        )
        .is_err());
        assert!(authenticate_governance(
            &blocks,
            &Header::default(),
            &context,
            ConsensusWitness {
                anchors: &anchors[..1],
                ..witness
            },
            &[]
        )
        .is_err());
        let duplicate = [anchors[0], anchors[0]];
        assert!(authenticate_governance(
            &blocks,
            &Header::default(),
            &context,
            ConsensusWitness {
                anchors: &duplicate,
                ..witness
            },
            &[]
        )
        .is_err());
    }
}
