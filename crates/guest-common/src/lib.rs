//! Helpers for zkVM guest programs.

use alethia_reth_block::config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes};
use alethia_reth_chainspec::{
    hardfork::{TaikoHardfork, TaikoHardforks},
    spec::TaikoChainSpec,
};
use alethia_reth_consensus::validation::{validate_anchor_transaction, AnchorValidationContext};
use alethia_reth_primitives::addresses::TAIKO_GOLDEN_TOUCH_ADDRESS;
use alloy_consensus::{
    transaction::{SignerRecoverable, Transaction as _},
    BlockHeader as _, Header,
};
use alloy_primitives::{Address, B256, U256};
use alloy_sol_types::{sol, SolCall};
use anyhow::{bail, ensure, Context, Result};
use raiko2_primitives::{
    shasta_checkpoint_storage_slots, ChainSpec, ProofType, StatelessInput, SupportedChainSpecs,
    WitnessHeader,
};
use raiko2_primitives_shasta::{
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_output, SHASTA_PROPOSAL_ID_MAX,
    },
    roll_proposal_ancestor_headers_in_place, should_bypass_stalled_anchor_linkage,
    validate_anchor_progression, validate_source_aware_anchor_progression,
    verify_proposal_mode_blob_usage, AnchorSourceSpan, GuestInput, ShastaZkAggregationGuestInput,
};
use raiko2_protocol_shasta::libhash::{hash_proposal, hash_shasta_subproof_input};
use raiko2_protocol_shasta::shasta::{
    calculate_shasta_difficulty,
    constants::{DERIVATION_SOURCE_MAX_BLOCKS, UNZEN_DERIVATION_SOURCE_MAX_BLOCKS},
    encode_extra_data,
    manifest::BlockManifest,
    prepare_source_manifest_with_max_blocks, ParentBlockContext, ProposalMetadata,
};
use raiko2_stateless::{
    read_parent_storage_with_witness_resources,
    reconstruct_block_from_transactions_with_witness_resources,
};
use std::sync::Arc;

const ANCHOR_BLOCK_STATE_SLOT: u64 = 256;

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;
}

pub struct TaikoRuntime {
    pub chain_spec: Arc<TaikoChainSpec>,
    pub evm_config: TaikoEvmConfig,
}

const ANCHOR_GAS_LIMIT: u64 = 1_000_000;

#[cfg(feature = "bench")]
fn bench_report_start(label: &str) {
    println!("cycle-tracker-report-start: {label}");
}

#[cfg(not(feature = "bench"))]
fn bench_report_start(_label: &str) {}

#[cfg(feature = "bench")]
fn bench_report_end(label: &str) {
    println!("cycle-tracker-report-end: {label}");
}

#[cfg(not(feature = "bench"))]
fn bench_report_end(_label: &str) {}

impl TaikoRuntime {
    fn from_chain_spec(chain_spec: &raiko2_primitives::ChainSpec) -> Result<Self> {
        let chain_spec = chain_spec.to_taiko_chain_spec()?;
        let evm_config = TaikoEvmConfig::new(chain_spec.clone());
        Ok(Self {
            chain_spec,
            evm_config,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct DecodedAnchorCheckpoint {
    block_number: u64,
    block_hash: B256,
    state_root: B256,
}

fn validate_known_chain_spec(chain_spec: &ChainSpec) -> Result<()> {
    let verified_chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(chain_spec.chain_id)
        .with_context(|| {
            format!(
                "unknown chain_id {}: not in trusted chain-spec list",
                chain_spec.chain_id
            )
        })?;

    ensure!(
        chain_spec.is_taiko == verified_chain_spec.is_taiko,
        "unexpected is_taiko"
    );

    let runtime_chain_spec = chain_spec
        .align_taiko_runtime_forks()
        .context("failed to align GuestInput chain_spec runtime forks")?;
    let verified_runtime_chain_spec = verified_chain_spec
        .align_taiko_runtime_forks()
        .context("failed to align trusted chain_spec runtime forks")?;

    ensure!(
        runtime_chain_spec.max_spec_id == verified_runtime_chain_spec.max_spec_id,
        "unexpected max_spec_id"
    );
    ensure!(
        runtime_chain_spec.hard_forks == verified_runtime_chain_spec.hard_forks,
        "unexpected hard_forks"
    );
    ensure!(
        chain_spec.eip_1559_constants == verified_chain_spec.eip_1559_constants,
        "unexpected eip_1559_constants"
    );
    ensure!(
        chain_spec.l1_contract == verified_chain_spec.l1_contract,
        "unexpected l1_contract"
    );
    ensure!(
        chain_spec.l2_contract == verified_chain_spec.l2_contract,
        "unexpected l2_contract"
    );
    ensure!(
        chain_spec.checkpoint_store_contract == verified_chain_spec.checkpoint_store_contract,
        "unexpected checkpoint_store_contract"
    );
    ensure!(
        chain_spec.verifier_address_forks == verified_chain_spec.verifier_address_forks,
        "unexpected verifier_address_forks"
    );

    Ok(())
}

fn decode_anchor_checkpoint(
    block: &reth_ethereum_primitives::Block,
) -> Result<DecodedAnchorCheckpoint> {
    let anchor_tx = block
        .body
        .transactions()
        .next()
        .context("missing anchor transaction")?;
    let input = anchor_tx.input();
    ensure!(
        input.starts_with(&anchorV4Call::SELECTOR),
        "block {} first transaction is not anchorV4",
        block.header.number
    );

    let decoded = anchorV4Call::abi_decode(input).with_context(|| {
        format!(
            "failed to decode anchorV4 calldata for block {}",
            block.header.number
        )
    })?;

    Ok(DecodedAnchorCheckpoint {
        block_number: decoded._checkpoint.blockNumber.to::<u64>(),
        block_hash: decoded._checkpoint.blockHash,
        state_root: decoded._checkpoint.stateRoot,
    })
}

fn validate_l1_anchor_linkage(
    guest_input: &GuestInput,
    anchor_checkpoints: &[DecodedAnchorCheckpoint],
    parent_anchor_block_number: u64,
    source_spans: Option<&[AnchorSourceSpan]>,
) -> Result<()> {
    let proposal = &guest_input.taiko.proposal_event.proposal;
    let origin_block_number = proposal.originBlockNumber.to::<u64>();
    let origin_block_hash = proposal.originBlockHash;
    let anchor_block_numbers = anchor_checkpoints
        .iter()
        .map(|checkpoint| checkpoint.block_number)
        .collect::<Vec<_>>();
    let bypass_stalled_anchor_linkage = should_bypass_stalled_anchor_linkage(
        &anchor_block_numbers,
        parent_anchor_block_number,
        origin_block_number,
        guest_input.taiko.chain_spec.chain_id,
    );
    ensure!(
        guest_input.taiko.l1_header.number == origin_block_number,
        "taiko.l1_header.number mismatch: expected {}, got {}",
        origin_block_number,
        guest_input.taiko.l1_header.number
    );
    ensure!(
        guest_input.taiko.l1_header.hash_slow() == origin_block_hash,
        "taiko.l1_header hash mismatch"
    );

    if bypass_stalled_anchor_linkage {
        let parent_checkpoint =
            verified_parent_shasta_checkpoint(guest_input, parent_anchor_block_number)?;
        for checkpoint in anchor_checkpoints {
            ensure!(
                checkpoint == &parent_checkpoint,
                "anchor checkpoint ({}, {:?}, {:?}) does not match parent checkpoint ({}, {:?}, {:?})",
                checkpoint.block_number,
                checkpoint.block_hash,
                checkpoint.state_root,
                parent_checkpoint.block_number,
                parent_checkpoint.block_hash,
                parent_checkpoint.state_root
            );
        }
        return Ok(());
    }

    let l1_header_anchor_start_index = if let Some(source_spans) = source_spans {
        let plan = validate_source_aware_anchor_progression(
            &anchor_block_numbers,
            source_spans,
            parent_anchor_block_number,
            origin_block_number,
            guest_input.taiko.chain_spec.chain_id,
        )
        .map_err(anyhow::Error::msg)?;
        if plan.requires_parent_checkpoint {
            let parent_checkpoint =
                verified_parent_shasta_checkpoint(guest_input, parent_anchor_block_number)?;
            let forced_checkpoints = anchor_checkpoints
                .get(..plan.l1_header_anchor_start_index)
                .context("invalid forced anchor checkpoint range")?;
            for checkpoint in forced_checkpoints {
                ensure!(
                    checkpoint == &parent_checkpoint,
                    "forced inclusion anchor checkpoint ({}, {:?}, {:?}) does not match parent checkpoint ({}, {:?}, {:?})",
                    checkpoint.block_number,
                    checkpoint.block_hash,
                    checkpoint.state_root,
                    parent_checkpoint.block_number,
                    parent_checkpoint.block_hash,
                    parent_checkpoint.state_root
                );
            }
        }
        plan.l1_header_anchor_start_index
    } else {
        validate_anchor_progression(
            &anchor_block_numbers,
            parent_anchor_block_number,
            origin_block_number,
            guest_input.taiko.chain_spec.chain_id,
        )
        .map_err(anyhow::Error::msg)?;
        0
    };
    let header_anchor_checkpoints = anchor_checkpoints
        .get(l1_header_anchor_start_index..)
        .context("invalid L1 header anchor checkpoint range")?;

    if guest_input.taiko.l1_ancestor_headers.is_empty() {
        bail!("taiko.l1_ancestor_headers must not be empty");
    }

    let mut checkpoint_index = 0usize;
    let mut previous_header_number = None;
    let mut previous_header_hash = None;
    let mut last_header_number = 0u64;
    let mut last_header_hash = B256::ZERO;

    for (index, header) in guest_input.taiko.l1_ancestor_headers.iter().enumerate() {
        let header_hash = header.hash_slow();
        if let Some(previous_number) = previous_header_number {
            ensure!(
                header.number == previous_number + 1,
                "taiko.l1_ancestor_headers must be contiguous at index {index}"
            );
        }
        if let Some(previous_hash) = previous_header_hash {
            ensure!(
                header.parent_hash == previous_hash,
                "taiko.l1_ancestor_headers parent hash mismatch at index {index}"
            );
        }

        loop {
            let Some(checkpoint) = header_anchor_checkpoints.get(checkpoint_index) else {
                break;
            };
            if checkpoint.block_number != header.number {
                break;
            }

            ensure!(
                checkpoint.block_hash == header_hash && checkpoint.state_root == header.state_root,
                "anchor checkpoint ({}, {:?}, {:?}) not found in taiko.l1_ancestor_headers",
                checkpoint.block_number,
                checkpoint.block_hash,
                checkpoint.state_root
            );
            checkpoint_index += 1;
        }

        previous_header_number = Some(header.number);
        previous_header_hash = Some(header_hash);
        last_header_number = header.number;
        last_header_hash = header_hash;
    }

    ensure!(
        last_header_number == origin_block_number,
        "taiko.l1_ancestor_headers last block number mismatch: expected {}, got {}",
        origin_block_number,
        last_header_number
    );
    ensure!(
        last_header_hash == origin_block_hash,
        "taiko.l1_ancestor_headers last hash mismatch"
    );
    if let Some(checkpoint) = header_anchor_checkpoints.get(checkpoint_index) {
        ensure!(
            false,
            "anchor checkpoint ({}, {:?}, {:?}) not found in taiko.l1_ancestor_headers",
            checkpoint.block_number,
            checkpoint.block_hash,
            checkpoint.state_root
        );
    }

    Ok(())
}

fn initial_proposal_ancestor_headers(guest_input: &GuestInput) -> Result<Vec<WitnessHeader>> {
    let headers = guest_input.initial_proposal_ancestor_headers();
    ensure!(!headers.is_empty(), "missing proposal ancestor headers");
    Ok(headers)
}

fn last_full_header(headers: &[WitnessHeader]) -> Result<Header> {
    headers
        .iter()
        .rev()
        .find_map(|header| header.full_header().cloned())
        .context("missing parent header in proposal ancestor headers")
}

// Anchor._blockState is read as a raw 256-bit storage word. In the current
// contract layout, anchorBlockNumber is a uint48 packed into the least
// significant 48 bits of slot 256.
fn anchor_block_number_from_storage_word(word: U256) -> u64 {
    let bytes = word.to_be_bytes::<32>();
    let mut value = [0u8; 8];
    value[2..].copy_from_slice(&bytes[26..]);
    u64::from_be_bytes(value)
}

fn storage_word_as_b256(word: U256) -> B256 {
    B256::from(word.to_be_bytes::<32>())
}

fn verified_parent_shasta_checkpoint(
    guest_input: &GuestInput,
    block_number: u64,
) -> Result<DecodedAnchorCheckpoint> {
    let first_witness = guest_input
        .witnesses
        .first()
        .context("GuestInput must contain least one witness")?;
    let checkpoint_store = first_witness
        .chain_spec
        .checkpoint_store_contract
        .context("missing chain_spec.checkpoint_store_contract parent checkpoint validation")?;
    let ancestor_headers = initial_proposal_ancestor_headers(guest_input)?;
    let (block_hash_slot, state_root_slot) = shasta_checkpoint_storage_slots(block_number);
    let block_hash = storage_word_as_b256(
        read_parent_storage_with_witness_resources(
            checkpoint_store,
            block_hash_slot,
            &first_witness.witness,
            &ancestor_headers,
            guest_input.proposal_state_nodes(),
        )
        .context("failed to read parent CheckpointStore blockHash")?,
    );
    let state_root = storage_word_as_b256(
        read_parent_storage_with_witness_resources(
            checkpoint_store,
            state_root_slot,
            &first_witness.witness,
            &ancestor_headers,
            guest_input.proposal_state_nodes(),
        )
        .context("failed to read parent CheckpointStore stateRoot")?,
    );
    ensure!(
        block_hash != B256::ZERO,
        "parent CheckpointStore blockHash is zero"
    );
    ensure!(
        state_root != B256::ZERO,
        "parent CheckpointStore stateRoot is zero"
    );

    Ok(DecodedAnchorCheckpoint {
        block_number,
        block_hash,
        state_root,
    })
}

fn verified_parent_anchor_block_number(guest_input: &GuestInput) -> Result<u64> {
    let first_witness = guest_input
        .witnesses
        .first()
        .context("GuestInput must contain least one witness")?;
    let anchor_address = first_witness
        .chain_spec
        .l2_contract
        .context("missing chain_spec.l2_contract for parent anchor state validation")?;
    let ancestor_headers = initial_proposal_ancestor_headers(guest_input)?;
    let storage_word = read_parent_storage_with_witness_resources(
        anchor_address,
        U256::from(ANCHOR_BLOCK_STATE_SLOT),
        &first_witness.witness,
        &ancestor_headers,
        guest_input.proposal_state_nodes(),
    )
    .context("failed to read parent Anchor._blockState.anchorBlockNumber")?;
    let verified_anchor = anchor_block_number_from_storage_word(storage_word);

    if let Some(host_anchor) = guest_input.taiko.prover_data.last_anchor_block_number {
        ensure!(
            host_anchor == verified_anchor,
            "prover_data.last_anchor_block_number mismatch: expected {}, got {}",
            verified_anchor,
            host_anchor
        );
    }

    Ok(verified_anchor)
}

fn shasta_fork_timestamp(chain_spec: &TaikoChainSpec) -> Result<u64> {
    match chain_spec.taiko_fork_activation(TaikoHardfork::Shasta) {
        alloy_hardforks::ForkCondition::Timestamp(timestamp) => Ok(timestamp),
        other => Err(anyhow::anyhow!(
            "unsupported Shasta fork activation condition: {other:?}"
        )),
    }
}

fn derivation_source_max_blocks(chain_spec: &TaikoChainSpec, proposal_timestamp: u64) -> usize {
    if chain_spec
        .taiko_fork_activation(TaikoHardfork::Unzen)
        .active_at_timestamp(proposal_timestamp)
    {
        UNZEN_DERIVATION_SOURCE_MAX_BLOCKS
    } else {
        DERIVATION_SOURCE_MAX_BLOCKS
    }
}

struct DerivedShastaBlocks {
    blocks: Vec<BlockManifest>,
    source_spans: Vec<AnchorSourceSpan>,
}

fn derive_expected_shasta_blocks(
    guest_input: &GuestInput,
    runtime: &TaikoRuntime,
    parent_anchor_block_number: u64,
) -> Result<Option<DerivedShastaBlocks>> {
    let proposal = &guest_input.taiko.proposal_event.proposal;
    if proposal.sources.is_empty() {
        return Ok(None);
    }
    ensure!(
        proposal
            .sources
            .last()
            .is_some_and(|source| !source.isForcedInclusion),
        "last Shasta derivation source must be a normal source"
    );

    ensure!(
        guest_input.taiko.data_sources.len() == proposal.sources.len(),
        "data source count ({}) does not match proposal source count ({})",
        guest_input.taiko.data_sources.len(),
        proposal.sources.len()
    );

    let fork_timestamp = shasta_fork_timestamp(runtime.chain_spec.as_ref())?;
    let mut parent = {
        let headers = initial_proposal_ancestor_headers(guest_input)?;
        let header = last_full_header(&headers)?;
        ParentBlockContext {
            timestamp: header.timestamp,
            gas_limit: header.gas_limit,
            block_number: header.number,
            anchor_block_number: parent_anchor_block_number,
        }
    };
    let meta = ProposalMetadata {
        proposal_timestamp: proposal.timestamp.to::<u64>(),
        origin_block_number: proposal.originBlockNumber.to::<u64>(),
        proposer: proposal.proposer,
        chain_id: guest_input.taiko.chain_spec.chain_id,
    };

    let mut blocks = Vec::new();
    let mut source_spans = Vec::with_capacity(proposal.sources.len());
    let max_blocks =
        derivation_source_max_blocks(runtime.chain_spec.as_ref(), meta.proposal_timestamp);
    for (source_index, source) in proposal.sources.iter().enumerate() {
        let manifest = prepare_source_manifest_with_max_blocks(
            source,
            guest_input.taiko.data_sources.get(source_index),
            parent,
            meta,
            fork_timestamp,
            max_blocks,
        )
        .with_context(|| format!("failed to prepare derivation source {source_index}"))?;

        let block_count = manifest.blocks.len();
        for block in manifest.blocks {
            parent = ParentBlockContext {
                timestamp: block.timestamp,
                gas_limit: block.gas_limit.saturating_add(ANCHOR_GAS_LIMIT),
                block_number: parent.block_number + 1,
                anchor_block_number: block.anchor_block_number,
            };
            blocks.push(block);
        }
        source_spans.push(AnchorSourceSpan {
            is_forced_inclusion: source.isForcedInclusion,
            block_count,
        });
    }

    ensure!(
        blocks.len() == guest_input.witnesses.len(),
        "witness count ({}) does not match derived manifest block count ({})",
        guest_input.witnesses.len(),
        blocks.len()
    );

    Ok(Some(DerivedShastaBlocks {
        blocks,
        source_spans,
    }))
}

fn validate_anchor_transaction_binding(
    stateless_input: &StatelessInput,
    expected_block: &BlockManifest,
) -> Result<()> {
    let block = &stateless_input.block;
    let anchor_tx = block
        .body
        .transactions()
        .next()
        .context("missing anchor transaction")?;
    let anchor_signer = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);
    let pre_state_account = stateless_input
        .accounts
        .get(&anchor_signer)
        .context("missing anchor signer account in pre-state callers")?;
    ensure!(
        anchor_tx.nonce() == pre_state_account.nonce,
        "anchor transaction nonce mismatch: expected {}, got {}",
        pre_state_account.nonce,
        anchor_tx.nonce()
    );

    let checkpoint = decode_anchor_checkpoint(block)?;
    ensure!(
        checkpoint.block_number == expected_block.anchor_block_number,
        "anchor checkpoint block number mismatch: expected {}, got {}",
        expected_block.anchor_block_number,
        checkpoint.block_number
    );

    Ok(())
}

fn validate_anchor_transaction_common(
    stateless_input: &StatelessInput,
    chain_spec: &TaikoChainSpec,
) -> Result<()> {
    let block = &stateless_input.block;
    let anchor_tx = block
        .body
        .transactions()
        .next()
        .context("missing anchor transaction")?;
    let expected_anchor_recipient = stateless_input
        .chain_spec
        .l2_contract
        .context("missing chain_spec.l2_contract for Shasta anchor validation")?;
    ensure!(
        anchor_tx.to() == Some(expected_anchor_recipient),
        "anchor transaction recipient mismatch: expected {expected_anchor_recipient:?}, got {:?}",
        anchor_tx.to()
    );
    ensure!(
        anchor_tx.chain_id() == Some(stateless_input.chain_spec.chain_id),
        "anchor transaction chain_id mismatch: expected {}, got {:?}",
        stateless_input.chain_spec.chain_id,
        anchor_tx.chain_id()
    );
    let base_fee = block
        .header
        .base_fee_per_gas()
        .context("missing base fee per gas in Shasta block header")?;

    validate_anchor_transaction(
        anchor_tx,
        chain_spec,
        AnchorValidationContext {
            timestamp: block.header.timestamp,
            block_number: block.header.number,
            base_fee_per_gas: base_fee,
        },
    )
    .map_err(|err| anyhow::anyhow!(err))?;
    ensure!(
        anchor_tx.max_fee_per_gas() == u128::from(base_fee),
        "anchor transaction max_fee_per_gas mismatch: expected {base_fee}, got {}",
        anchor_tx.max_fee_per_gas()
    );
    ensure!(
        anchor_tx.max_priority_fee_per_gas() == Some(0),
        "anchor transaction max_priority_fee_per_gas mismatch"
    );
    if let Some(access_list) = anchor_tx.access_list() {
        ensure!(
            access_list.0.is_empty(),
            "anchor transaction access list must be empty"
        );
    }

    Ok(())
}

fn validate_shasta_manifest_block_metadata(
    guest_input: &GuestInput,
    stateless_input: &StatelessInput,
    expected_block: &BlockManifest,
    parent_header: &Header,
) -> Result<()> {
    let block = &stateless_input.block;
    let expected_extra_data = encode_extra_data(
        guest_input.taiko.proposal_event.proposal.basefeeSharingPctg,
        guest_input.taiko.proposal_id,
    );
    let expected_mix_hash = calculate_shasta_difficulty(
        B256::from(parent_header.difficulty.to_be_bytes::<32>()),
        block.header.number,
    );

    ensure!(
        block.header.timestamp == expected_block.timestamp,
        "block {} timestamp mismatch: expected {}, got {}",
        block.header.number,
        expected_block.timestamp,
        block.header.timestamp
    );
    ensure!(
        block.header.beneficiary == expected_block.coinbase,
        "block {} coinbase mismatch: expected {:?}, got {:?}",
        block.header.number,
        expected_block.coinbase,
        block.header.beneficiary
    );
    ensure!(
        block.header.gas_limit == expected_block.gas_limit.saturating_add(ANCHOR_GAS_LIMIT),
        "block {} gas limit mismatch: expected {}, got {}",
        block.header.number,
        expected_block.gas_limit.saturating_add(ANCHOR_GAS_LIMIT),
        block.header.gas_limit
    );
    ensure!(
        block.header.extra_data == expected_extra_data,
        "block {} extra_data mismatch",
        block.header.number
    );
    ensure!(
        block.header.mix_hash == expected_mix_hash,
        "block {} mix_hash mismatch",
        block.header.number
    );

    validate_anchor_transaction_binding(stateless_input, expected_block)?;

    Ok(())
}

fn validate_manifest_transactions_match_canonical(
    stateless_input: &StatelessInput,
    expected_block: &BlockManifest,
) -> Result<()> {
    let block = &stateless_input.block;

    let actual_transactions = block.body.transactions().collect::<Vec<_>>();
    ensure!(
        actual_transactions.len() == expected_block.transactions.len() + 1,
        "block {} transaction count mismatch: expected {}, got {}",
        block.header.number,
        expected_block.transactions.len() + 1,
        actual_transactions.len()
    );

    for (tx_index, (expected_tx, actual_tx)) in expected_block
        .transactions
        .iter()
        .zip(actual_transactions.iter().skip(1))
        .enumerate()
    {
        ensure!(
            alloy_rlp::encode(expected_tx) == alloy_rlp::encode(actual_tx),
            "block {} transaction {} mismatch",
            block.header.number,
            tx_index + 1
        );
    }

    Ok(())
}

fn validate_generated_block_matches_canonical(
    generated_block: &reth_ethereum_primitives::Block,
    canonical_block: &reth_ethereum_primitives::Block,
) -> Result<()> {
    ensure!(
        generated_block.header == canonical_block.header,
        "generated block header mismatch"
    );

    let generated_transactions = generated_block.body.transactions().collect::<Vec<_>>();
    let canonical_transactions = canonical_block.body.transactions().collect::<Vec<_>>();
    ensure!(
        generated_transactions.len() == canonical_transactions.len(),
        "generated block transaction count mismatch: expected {}, got {}",
        canonical_transactions.len(),
        generated_transactions.len()
    );

    for (tx_index, (generated_tx, canonical_tx)) in generated_transactions
        .iter()
        .zip(canonical_transactions.iter())
        .enumerate()
    {
        ensure!(
            alloy_rlp::encode(generated_tx) == alloy_rlp::encode(canonical_tx),
            "generated block transaction {} mismatch",
            tx_index
        );
    }

    Ok(())
}

fn manifest_transactions_for_reconstruction(
    expected_block: &BlockManifest,
) -> Result<Vec<reth_ethereum_primitives::TransactionSigned>> {
    expected_block
        .transactions
        .iter()
        .map(|transaction| {
            alloy_rlp::decode_exact(alloy_rlp::encode(transaction))
                .context("failed to decode Shasta manifest transaction")
        })
        .collect()
}

fn shasta_block_env_attributes(
    stateless_input: &StatelessInput,
) -> Result<TaikoNextBlockEnvAttributes> {
    let block = &stateless_input.block;
    Ok(TaikoNextBlockEnvAttributes {
        timestamp: block.header.timestamp,
        suggested_fee_recipient: block.header.beneficiary,
        prev_randao: block.header.mix_hash,
        gas_limit: block.header.gas_limit,
        extra_data: block.header.extra_data.clone(),
        base_fee_per_gas: block
            .header
            .base_fee_per_gas()
            .context("missing base fee per gas in Shasta block header")?,
    })
}

fn prove_shasta_proposal_with_block_verifier<V>(
    guest_input: &GuestInput,
    proof_type: ProofType,
    mut verify_block: V,
) -> Result<B256>
where
    V: FnMut(
        usize,
        &StatelessInput,
        Option<&BlockManifest>,
        Option<&Header>,
        &[WitnessHeader],
        &TaikoRuntime,
    ) -> Result<B256>,
{
    bench_report_start("proposal_blob_usage");
    verify_proposal_mode_blob_usage(guest_input)
        .context("proposal mode blob usage verification failed")?;
    bench_report_end("proposal_blob_usage");

    let proof_carry_data = &guest_input.proof_carry_data;
    ensure!(
        !guest_input.witnesses.is_empty(),
        "GuestInput must contain at least one witness"
    );

    bench_report_start("proposal_invariants");
    let first_chain_spec = &guest_input.witnesses.first().expect("checked").chain_spec;
    validate_known_chain_spec(first_chain_spec)?;
    ensure!(
        guest_input.taiko.chain_spec.chain_id == first_chain_spec.chain_id,
        "taiko.chain_spec.chain_id mismatch: expected {}, got {}",
        first_chain_spec.chain_id,
        guest_input.taiko.chain_spec.chain_id
    );
    ensure!(
        guest_input.taiko.chain_spec.is_taiko == first_chain_spec.is_taiko,
        "taiko.chain_spec.is_taiko mismatch: expected {}, got {}",
        first_chain_spec.is_taiko,
        guest_input.taiko.chain_spec.is_taiko
    );

    for (i, witness) in guest_input.witnesses.iter().enumerate() {
        ensure!(
            witness.chain_spec == *first_chain_spec,
            "witness {i} chain_spec mismatch"
        );
    }

    ensure!(
        proof_carry_data.chain_id == first_chain_spec.chain_id,
        "proof_carry_data.chain_id mismatch: expected {}, got {}",
        first_chain_spec.chain_id,
        proof_carry_data.chain_id
    );
    let verifier_proof_type = match proof_type {
        ProofType::Native => ProofType::Sgx,
        other => other,
    };
    let expected_verifier = first_chain_spec
        .get_fork_verifier_address(
            guest_input
                .witnesses
                .first()
                .expect("checked")
                .block
                .header
                .number,
            guest_input
                .witnesses
                .first()
                .expect("checked")
                .block
                .header
                .timestamp,
            verifier_proof_type,
        )
        .with_context(|| {
            format!("failed to resolve verifier address for proof type {verifier_proof_type}")
        })?;
    ensure!(
        proof_carry_data.verifier == expected_verifier,
        "proof_carry_data.verifier mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.proposal_id == guest_input.taiko.proposal_id,
        "proof_carry_data.proposal_id mismatch: expected {}, got {}",
        guest_input.taiko.proposal_id,
        proof_carry_data.transition_input.proposal_id
    );
    ensure!(
        proof_carry_data.transition_input.actual_prover
            == guest_input.taiko.prover_data.actual_prover,
        "proof_carry_data.actual_prover mismatch"
    );

    let proposal = &guest_input.taiko.proposal_event.proposal;
    let proposal_event_id = proposal.id.to::<u64>();
    ensure!(
        guest_input.taiko.proposal_id <= SHASTA_PROPOSAL_ID_MAX,
        "taiko.proposal_id does not fit in uint48: {}",
        guest_input.taiko.proposal_id
    );
    ensure!(
        guest_input.taiko.proposal_id == proposal_event_id,
        "taiko.proposal_id mismatch: expected {}, got {}",
        proposal_event_id,
        guest_input.taiko.proposal_id
    );
    let expected_proposal_hash = hash_proposal(proposal);
    ensure!(
        proof_carry_data.transition_input.proposal_hash == expected_proposal_hash,
        "proof_carry_data.proposal_hash mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.parent_proposal_hash == proposal.parentProposalHash,
        "proof_carry_data.parent_proposal_hash mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.transition.proposer == proposal.proposer,
        "proof_carry_data.transition.proposer mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.transition.timestamp == proposal.timestamp.to::<u64>(),
        "proof_carry_data.transition.timestamp mismatch"
    );
    bench_report_end("proposal_invariants");

    bench_report_start("proposal_runtime");
    let runtime = TaikoRuntime::from_chain_spec(first_chain_spec)
        .context("Failed to build Taiko runtime from GuestInput chain_spec")?;
    bench_report_end("proposal_runtime");

    bench_report_start("proposal_derivation");
    let parent_anchor_block_number = verified_parent_anchor_block_number(guest_input)?;
    let derived_blocks =
        derive_expected_shasta_blocks(guest_input, &runtime, parent_anchor_block_number)?;
    let expected_blocks = derived_blocks
        .as_ref()
        .map(|derived_blocks| derived_blocks.blocks.as_slice());
    let source_spans = derived_blocks
        .as_ref()
        .map(|derived_blocks| derived_blocks.source_spans.as_slice());
    let mut proposal_ancestor_headers = initial_proposal_ancestor_headers(guest_input)?;
    let mut canonical_parent_header = expected_blocks.and_then(|_| {
        proposal_ancestor_headers
            .last()
            .and_then(|header| header.full_header().cloned())
    });
    bench_report_end("proposal_derivation");

    let mut anchor_checkpoints = Vec::with_capacity(guest_input.witnesses.len());
    let mut first_parent_block_hash = None;
    let mut previous_block_hash: Option<B256> = None;
    let mut previous_block_number: Option<u64> = None;
    let mut last_block_number = None;
    let mut last_block_hash = None;
    let mut last_state_root = None;

    bench_report_start("proposal_stateless_validation");
    for (index, stateless_input) in guest_input.witnesses.iter().enumerate() {
        let block = &stateless_input.block;
        validate_anchor_transaction_common(stateless_input, runtime.chain_spec.as_ref())
            .with_context(|| format!("anchor transaction validation failed at index {index}"))?;
        let expected_block = expected_blocks.and_then(|blocks| blocks.get(index));
        let parent_header = canonical_parent_header.as_ref();

        if let (Some(expected_block), Some(parent_header)) = (expected_block, parent_header) {
            validate_shasta_manifest_block_metadata(
                guest_input,
                stateless_input,
                expected_block,
                parent_header,
            )
            .with_context(|| format!("canonical Shasta derivation mismatch at index {index}"))?;
        }
        if let Some(prev_hash) = previous_block_hash {
            ensure!(
                block.header.parent_hash == prev_hash,
                "block {index} must link to previous block hash"
            );
        }
        if let Some(previous_number) = previous_block_number {
            ensure!(
                previous_number + 1 == block.header.number,
                "block {index} must increment block number by 1"
            );
        }

        let validated_hash = verify_block(
            index,
            stateless_input,
            expected_block,
            parent_header,
            &proposal_ancestor_headers,
            &runtime,
        )
        .with_context(|| format!("stateless block validation failed at index {index}"))?;
        first_parent_block_hash.get_or_insert(block.header.parent_hash);
        previous_block_hash = Some(validated_hash);
        previous_block_number = Some(block.header.number);
        last_block_number = Some(block.header.number);
        last_block_hash = Some(validated_hash);
        last_state_root = Some(block.header.state_root);
        anchor_checkpoints.push(decode_anchor_checkpoint(block)?);
        if canonical_parent_header.is_some() {
            canonical_parent_header = Some(block.header.clone());
        }
        roll_proposal_ancestor_headers_in_place(
            &mut proposal_ancestor_headers,
            &block.header,
            validated_hash,
        );
    }
    bench_report_end("proposal_stateless_validation");

    bench_report_start("proposal_anchor_linkage");
    validate_l1_anchor_linkage(
        guest_input,
        &anchor_checkpoints,
        parent_anchor_block_number,
        source_spans,
    )?;
    let first_parent_block_hash = first_parent_block_hash.expect("checked");
    let last_block_number = last_block_number.expect("checked");
    let last_block_hash = last_block_hash.expect("checked");
    let last_state_root = last_state_root.expect("checked");
    ensure!(
        proof_carry_data.transition_input.parent_block_hash == first_parent_block_hash,
        "proof_carry_data.parent_block_hash mismatch"
    );
    ensure!(
        proof_carry_data
            .transition_input
            .checkpoint
            .blockNumber
            .to::<u64>()
            == last_block_number,
        "proof_carry_data.checkpoint.blockNumber mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.checkpoint.blockHash == last_block_hash,
        "proof_carry_data.checkpoint.blockHash mismatch"
    );
    ensure!(
        proof_carry_data.transition_input.checkpoint.stateRoot == last_state_root,
        "proof_carry_data.checkpoint.stateRoot mismatch"
    );
    bench_report_end("proposal_anchor_linkage");

    bench_report_start("proposal_output_hash");
    let output = hash_shasta_subproof_input(proof_carry_data);
    bench_report_end("proposal_output_hash");

    Ok(output)
}

pub fn prove_shasta_proposal(guest_input: &GuestInput) -> Result<B256> {
    prove_shasta_proposal_for_proof_type(guest_input, ProofType::Native)
}

pub fn prove_shasta_proposal_for_proof_type(
    guest_input: &GuestInput,
    proof_type: ProofType,
) -> Result<B256> {
    prove_shasta_proposal_with_block_verifier(
        guest_input,
        proof_type,
        |index, stateless_input, expected_block, _parent_header, ancestor_headers, runtime| {
            let expected_block = expected_block
                .with_context(|| format!("missing expected Shasta block at index {index}"))?;
            let anchor_tx = stateless_input
                .block
                .body
                .transactions()
                .next()
                .cloned()
                .context("missing canonical anchor transaction")?
                .try_into_recovered()
                .map_err(|_| anyhow::anyhow!("failed to recover canonical anchor transaction"))?;
            let outcome = reconstruct_block_from_transactions_with_witness_resources(
                anchor_tx,
                manifest_transactions_for_reconstruction(expected_block)?,
                shasta_block_env_attributes(stateless_input)?,
                &stateless_input.witness,
                ancestor_headers,
                guest_input.proposal_state_nodes(),
                stateless_input.accounts.clone(),
                &runtime.chain_spec,
                &runtime.evm_config,
            )
            .map_err(|e| anyhow::anyhow!(e))
            .with_context(|| format!("failed to reconstruct Shasta block at index {index}"))?;
            let generated_block = outcome.filtered_block.into_block();
            validate_generated_block_matches_canonical(&generated_block, &stateless_input.block)
                .with_context(|| format!("generated Shasta block mismatch at index {index}"))?;
            Ok(generated_block.header.hash_slow())
        },
    )
}

pub fn prove_shasta_proposal_with_validator<V>(
    guest_input: &GuestInput,
    validate_block: V,
) -> Result<B256>
where
    V: FnMut(&StatelessInput, &[WitnessHeader], &TaikoRuntime) -> Result<B256>,
{
    prove_shasta_proposal_with_validator_for_proof_type(
        guest_input,
        ProofType::Native,
        validate_block,
    )
}

pub fn prove_shasta_proposal_with_validator_for_proof_type<V>(
    guest_input: &GuestInput,
    proof_type: ProofType,
    mut validate_block: V,
) -> Result<B256>
where
    V: FnMut(&StatelessInput, &[WitnessHeader], &TaikoRuntime) -> Result<B256>,
{
    prove_shasta_proposal_with_block_verifier(
        guest_input,
        proof_type,
        |index, stateless_input, expected_block, _parent_header, ancestor_headers, runtime| {
            if let Some(expected_block) = expected_block {
                validate_manifest_transactions_match_canonical(stateless_input, expected_block)
                    .with_context(|| {
                        format!("canonical Shasta derivation mismatch at index {index}")
                    })?;
            }

            validate_block(stateless_input, ancestor_headers, runtime)
        },
    )
}

pub fn aggregate_shasta_zk_with_verifier<V>(
    input: &ShastaZkAggregationGuestInput,
    image_id_b256: B256,
    mut verify_proof: V,
) -> Result<B256>
where
    V: FnMut(usize, &B256) -> Result<()>,
{
    ensure!(
        input.block_inputs.len() == input.proof_carry_data_vec.len(),
        "block_inputs/proof_carry_data_vec length mismatch: {} vs {}",
        input.block_inputs.len(),
        input.proof_carry_data_vec.len()
    );
    ensure!(
        !input.proof_carry_data_vec.is_empty(),
        "proof_carry_data_vec must not be empty"
    );

    for (i, block_input) in input.block_inputs.iter().enumerate() {
        verify_proof(i, block_input)
            .with_context(|| format!("proof verification failed at index {i}"))?;

        let expected = hash_shasta_subproof_input(&input.proof_carry_data_vec[i]);
        ensure!(
            *block_input == expected,
            "block_input mismatch at index {i}"
        );
    }

    let commitment = build_shasta_commitment_from_proof_carry_data_vec(&input.proof_carry_data_vec)
        .context("invalid proof_carry_data_vec")?;
    let first = input.proof_carry_data_vec.first().expect("checked");
    let aggregation_hash = shasta_aggregation_output(
        &commitment,
        first.chain_id,
        first.verifier,
        input.prover_address,
    );

    Ok(shasta_zk_aggregation_output(
        image_id_b256,
        aggregation_hash,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::{constants::KECCAK_EMPTY, SignableTransaction, TrieAccount, TxEip1559};
    use alloy_primitives::{keccak256, Address, Signature, TxKind, U256};
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use raiko2_primitives::ProofType;
    use raiko2_primitives::{
        ChainSpec, ExecutionWitness, StatelessInput, SupportedChainSpecs, WitnessStateNode,
    };
    use raiko2_primitives_shasta::build_proof_carry_data;
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use raiko2_protocol_shasta::shasta::ProofCarryData;
    use raiko2_protocol_shasta::TaikoManifest;
    use risc0_ethereum_trie::Trie;

    const TEST_PARENT_ANCHOR_BLOCK_NUMBER: u64 = 7;
    const TEST_SHASTA_BLOCK_NUMBER: u64 = 1_166_000;

    fn taiko_mainnet_chain_spec() -> ChainSpec {
        SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_000)
            .expect("supported taiko mainnet chain spec")
    }

    #[test]
    fn validate_known_chain_spec_rejects_unlisted_chain_id() {
        let mut spec = taiko_mainnet_chain_spec();
        spec.chain_id = 424_242; // absent from config/chain_spec_list_default.json
        let err = validate_known_chain_spec(&spec)
            .expect_err("unlisted chain id must be rejected (F-1 regression)");
        assert!(
            err.to_string().contains("unknown chain_id"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_known_chain_spec_accepts_listed_chain() {
        validate_known_chain_spec(&taiko_mainnet_chain_spec())
            .expect("listed mainnet chain spec must validate");
    }

    #[test]
    fn validate_known_chain_spec_accepts_runtime_aligned_devnet() {
        let spec = SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_001)
            .expect("supported taiko devnet chain spec")
            .align_taiko_runtime_forks()
            .expect("runtime-aligned devnet chain spec");

        validate_known_chain_spec(&spec).expect("runtime-aligned devnet chain spec must validate");
    }

    #[test]
    fn validate_known_chain_spec_rejects_tampered_field() {
        let mut spec = taiko_mainnet_chain_spec();
        spec.l2_contract = Some(Address::ZERO); // diverge from the trusted spec
        let err =
            validate_known_chain_spec(&spec).expect_err("tampered l2_contract must be rejected");
        assert!(
            err.to_string().contains("l2_contract"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_known_chain_spec_rejects_tampered_checkpoint_store() {
        let mut spec = taiko_mainnet_chain_spec();
        spec.checkpoint_store_contract = None; // diverge from the trusted spec
        let err = validate_known_chain_spec(&spec)
            .expect_err("tampered checkpoint_store_contract must be rejected");
        assert!(
            err.to_string().contains("checkpoint_store_contract"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validate_known_chain_spec_rejects_tampered_is_taiko() {
        let mut spec = taiko_mainnet_chain_spec();
        spec.is_taiko = !spec.is_taiko; // diverge from the trusted spec
        let err = validate_known_chain_spec(&spec).expect_err("tampered is_taiko must be rejected");
        assert!(
            err.to_string().contains("is_taiko"),
            "unexpected error: {err}"
        );
    }

    fn sample_l1_header(number: u64, state_root: B256) -> alloy_consensus::Header {
        alloy_consensus::Header {
            number,
            parent_hash: B256::from([0xAA; 32]),
            state_root,
            ..Default::default()
        }
    }

    fn test_anchor_address() -> Address {
        taiko_mainnet_chain_spec()
            .l2_contract
            .expect("test chain spec must define Anchor address")
    }

    fn unsigned_anchor_tx(checkpoint: &AnchorV4Checkpoint, to: Address) -> TxEip1559 {
        TxEip1559 {
            chain_id: 167_000,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(to),
            value: U256::ZERO,
            access_list: Default::default(),
            input: anchorV4Call {
                _checkpoint: checkpoint.clone(),
            }
            .abi_encode()
            .into(),
        }
    }

    fn anchor_tx_with_signature(
        checkpoint: &AnchorV4Checkpoint,
        signature: Signature,
    ) -> reth_ethereum_primitives::TransactionSigned {
        unsigned_anchor_tx(checkpoint, test_anchor_address())
            .into_signed(signature)
            .into()
    }

    fn golden_touch_signer() -> PrivateKeySigner {
        [
            "92954368afd3caa1f3ce3ead0069c1af",
            "414054aefe1ef9aeacc1bf426222ce38",
        ]
        .concat()
        .parse()
        .expect("golden touch signer")
    }

    fn anchor_tx(checkpoint: &AnchorV4Checkpoint) -> reth_ethereum_primitives::TransactionSigned {
        let tx = unsigned_anchor_tx(checkpoint, test_anchor_address());
        let signature = golden_touch_signer()
            .sign_hash_sync(&tx.signature_hash())
            .expect("golden touch anchor signature");
        tx.into_signed(signature).into()
    }

    fn parent_anchor_state_witness(
        anchor_address: Address,
        anchor_block_number: u64,
        checkpoint_store: Address,
        checkpoint: DecodedAnchorCheckpoint,
    ) -> (B256, Vec<WitnessStateNode>) {
        let mut storage_trie = Trie::default();
        storage_trie.insert(
            keccak256(B256::from(U256::from(ANCHOR_BLOCK_STATE_SLOT))),
            alloy_rlp::encode(U256::from(anchor_block_number)),
        );

        let (block_hash_slot, state_root_slot) =
            shasta_checkpoint_storage_slots(checkpoint.block_number);
        let mut checkpoint_storage_trie = Trie::default();
        checkpoint_storage_trie.insert(
            keccak256(B256::from(block_hash_slot.to_be_bytes::<32>())),
            alloy_rlp::encode(U256::from_be_slice(checkpoint.block_hash.as_slice())),
        );
        checkpoint_storage_trie.insert(
            keccak256(B256::from(state_root_slot.to_be_bytes::<32>())),
            alloy_rlp::encode(U256::from_be_slice(checkpoint.state_root.as_slice())),
        );

        let mut state_trie = Trie::default();
        state_trie.insert(
            keccak256(anchor_address),
            alloy_rlp::encode(TrieAccount {
                nonce: 0,
                balance: U256::ZERO,
                storage_root: storage_trie.hash_slow(),
                code_hash: KECCAK_EMPTY,
            }),
        );
        state_trie.insert(
            keccak256(checkpoint_store),
            alloy_rlp::encode(TrieAccount {
                nonce: 0,
                balance: U256::ZERO,
                storage_root: checkpoint_storage_trie.hash_slow(),
                code_hash: KECCAK_EMPTY,
            }),
        );

        let state_root = state_trie.hash_slow();
        let state_nodes = state_trie
            .rlp_nodes()
            .into_iter()
            .chain(storage_trie.rlp_nodes())
            .chain(checkpoint_storage_trie.rlp_nodes())
            .map(WitnessStateNode::from_bytes)
            .collect();
        (
            state_root,
            ExecutionWitness::canonicalize_state_nodes(state_nodes),
        )
    }

    fn replace_parent_checkpoint(
        guest_input: &mut GuestInput,
        checkpoint: DecodedAnchorCheckpoint,
    ) {
        let chain_spec = &guest_input.witnesses[0].chain_spec;
        let anchor_address = chain_spec
            .l2_contract
            .expect("test chain spec must define Anchor address");
        let checkpoint_store = chain_spec
            .checkpoint_store_contract
            .expect("test chain spec must define checkpoint store address");
        let (parent_state_root, parent_state_nodes) = parent_anchor_state_witness(
            anchor_address,
            TEST_PARENT_ANCHOR_BLOCK_NUMBER,
            checkpoint_store,
            checkpoint,
        );
        let mut parent_header = guest_input.witnesses[0].witness.headers[0]
            .header
            .as_ref()
            .expect("parent header")
            .clone();
        parent_header.state_root = parent_state_root;
        let parent_witness_header = WitnessHeader::from_header(parent_header.clone());
        guest_input.proposal_ancestor_headers = vec![parent_witness_header.clone()];
        guest_input.witnesses[0].witness.headers = vec![parent_witness_header];
        guest_input.witnesses[0].witness.state = parent_state_nodes;
        guest_input.witnesses[0].block.header.parent_hash = parent_header.hash_slow();
    }

    fn guest_input_with_single_block() -> GuestInput {
        let chain_spec = taiko_mainnet_chain_spec();
        let anchor_address = chain_spec
            .l2_contract
            .expect("test chain spec must define Anchor address");
        let checkpoint_store = chain_spec
            .checkpoint_store_contract
            .expect("test chain spec must define checkpoint store address");
        let l1_header = sample_l1_header(TEST_PARENT_ANCHOR_BLOCK_NUMBER, B256::from([0x66; 32]));
        let parent_checkpoint = DecodedAnchorCheckpoint {
            block_number: l1_header.number,
            block_hash: l1_header.hash_slow(),
            state_root: l1_header.state_root,
        };
        let (parent_state_root, parent_state_nodes) = parent_anchor_state_witness(
            anchor_address,
            TEST_PARENT_ANCHOR_BLOCK_NUMBER,
            checkpoint_store,
            parent_checkpoint,
        );
        let parent_header = alloy_consensus::Header {
            number: TEST_SHASTA_BLOCK_NUMBER - 1,
            timestamp: u64::MAX / 2 - 1,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(1),
            state_root: parent_state_root,
            ..Default::default()
        };
        let parent_witness_header = WitnessHeader::from_header(parent_header.clone());
        let mut input = StatelessInput {
            chain_spec,
            ..Default::default()
        };
        input.block.header.number = TEST_SHASTA_BLOCK_NUMBER;
        input.block.header.timestamp = u64::MAX / 2;
        input.block.header.parent_hash = parent_header.hash_slow();
        input.block.header.base_fee_per_gas = Some(1);
        input.block.header.state_root = B256::from([1u8; 32]);
        input.witness.headers = vec![parent_witness_header.clone()];
        input.witness.state = parent_state_nodes;
        let checkpoint = AnchorV4Checkpoint {
            blockNumber: l1_header.number.try_into().expect("fits in uint48"),
            blockHash: l1_header.hash_slow(),
            stateRoot: l1_header.state_root,
        };
        input.block.body.transactions.push(anchor_tx(&checkpoint));

        let mut guest_input = GuestInput {
            witnesses: vec![input],
            taiko: TaikoManifest {
                proposal_id: 42,
                ..Default::default()
            },
            ..Default::default()
        };
        guest_input.proposal_ancestor_headers = vec![parent_witness_header];
        guest_input.taiko.chain_spec.name = "taiko_mainnet".to_string();
        guest_input.taiko.chain_spec.chain_id = 167_000;
        guest_input.taiko.chain_spec.is_taiko = true;
        guest_input.taiko.l1_header = l1_header.clone();
        guest_input.taiko.l1_ancestor_headers = vec![l1_header.clone()];
        guest_input.taiko.prover_data.actual_prover = Address::from([0x22; 20]);
        guest_input.taiko.proposal_event.proposal.id = guest_input
            .taiko
            .proposal_id
            .try_into()
            .expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.proposer = Address::from([0x33; 20]);
        guest_input.taiko.proposal_event.proposal.timestamp =
            123u64.try_into().expect("timestamp fits in uint48");
        guest_input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x44; 32]);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            l1_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = l1_header.hash_slow();
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");
        guest_input
    }

    fn error_chain_contains(err: &anyhow::Error, expected: &str) -> bool {
        err.chain()
            .map(ToString::to_string)
            .any(|message| message.contains(expected))
    }

    #[test]
    fn anchor_block_number_from_storage_word_reads_low_uint48() {
        let anchor_block_number = 0x0102_0304_0506u128;
        let word = U256::from((0xDEAD_BEEFu128 << 64) | (0xABCDu128 << 48) | anchor_block_number);

        assert_eq!(
            anchor_block_number_from_storage_word(word),
            anchor_block_number as u64
        );
    }

    #[test]
    fn prove_shasta_proposal_builds_expected_hash() {
        let guest_input = guest_input_with_single_block();
        let proof_carry_data = guest_input.proof_carry_data.clone();

        let subproof_input_hash = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect("proposal proving should succeed");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&proof_carry_data)
        );
    }

    #[test]
    fn accepts_missing_last_anchor_hint_from_parent_state() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.prover_data.last_anchor_block_number = None;
        let proof_carry_data = guest_input.proof_carry_data.clone();

        let subproof_input_hash = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect("proposal proving should read parent anchor state");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&proof_carry_data)
        );
    }

    #[test]
    fn rejects_mismatched_last_anchor_hint() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.prover_data.last_anchor_block_number =
            Some(TEST_PARENT_ANCHOR_BLOCK_NUMBER + 1);

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("mismatched last anchor hint should fail");

        assert!(error_chain_contains(
            &err,
            "prover_data.last_anchor_block_number mismatch"
        ));
    }

    #[test]
    fn rejects_empty_witnesses() {
        let guest_input = GuestInput::default();
        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |_stateless_input, _ancestor_headers, _runtime| Ok(B256::ZERO),
        )
        .expect_err("empty witnesses should fail");
        assert!(err
            .to_string()
            .contains("must contain at least one witness"));
    }

    #[test]
    fn rejects_parent_hash_mismatch() {
        let mut guest_input = guest_input_with_single_block();
        let chain_spec = guest_input.witnesses[0].chain_spec.clone();

        let mut first = guest_input.witnesses.remove(0);
        first.block.header.parent_hash = B256::from([1u8; 32]);

        let first_hash = first.block.header.hash_slow();

        let mut second = StatelessInput {
            chain_spec,
            ..Default::default()
        };
        second.block.header.number = first.block.header.number + 1;
        second.block.header.timestamp = first.block.header.timestamp;
        second.block.header.parent_hash = B256::from([2u8; 32]);
        second.block.header.base_fee_per_gas = first.block.header.base_fee_per_gas;
        second.block.body.transactions = first.block.body.transactions.clone();
        guest_input.witnesses = vec![first, second];
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("expected parent-hash mismatch to fail");

        assert!(error_chain_contains(
            &err,
            "must link to previous block hash"
        ));
        assert_ne!(
            guest_input.witnesses[1].block.header.parent_hash,
            first_hash
        );
    }

    #[test]
    fn rejects_block_number_gap() {
        let mut guest_input = guest_input_with_single_block();
        let mut second = guest_input.witnesses[0].clone();
        second.block.header.number = guest_input.witnesses[0].block.header.number + 2;
        second.block.header.parent_hash = guest_input.witnesses[0].block.header.hash_slow();
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("block number gap should fail");

        assert!(error_chain_contains(
            &err,
            "must increment block number by 1"
        ));
    }

    #[test]
    fn rejects_witness_chain_id_mismatch() {
        let mut guest_input = guest_input_with_single_block();
        let mut second = guest_input.witnesses[0].clone();
        second.block.header.number = guest_input.witnesses[0].block.header.number + 1;
        second.block.header.parent_hash = guest_input.witnesses[0].block.header.hash_slow();
        second.chain_spec.chain_id = 167_001;
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("chain_id mismatch should fail");

        assert!(err.to_string().contains("chain_spec mismatch"));
    }

    #[test]
    fn rejects_carry_checkpoint_mismatch() {
        let mut guest_input = guest_input_with_single_block();
        guest_input
            .proof_carry_data
            .transition_input
            .checkpoint
            .blockHash = B256::from([0x99; 32]);

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("checkpoint mismatch should fail");

        assert!(error_chain_contains(&err, "checkpoint.blockHash mismatch"));
    }

    #[test]
    fn accepts_anchor_sequences_that_do_not_grow_past_last_anchor_with_l1_headers() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.prover_data.last_anchor_block_number = Some(7);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let subproof_input_hash = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect("repeated last anchor should validate with headers");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&guest_input.proof_carry_data)
        );
    }

    #[test]
    fn accepts_stalled_anchor_linkage_with_empty_l1_ancestor_headers() {
        let mut guest_input = guest_input_with_single_block();
        let origin_header = sample_l1_header(600, B256::from([0x77; 32]));
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers.clear();
        guest_input.taiko.prover_data.last_anchor_block_number = Some(7);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let subproof_input_hash = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect("stalled anchor should validate against parent checkpoint");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&guest_input.proof_carry_data)
        );
    }

    #[test]
    fn rejects_stalled_anchor_with_mismatched_origin_header() {
        let mut guest_input = guest_input_with_single_block();
        let origin_header = sample_l1_header(600, B256::from([0x77; 32]));
        guest_input.taiko.l1_header = sample_l1_header(600, B256::from([0x88; 32]));
        guest_input.taiko.l1_ancestor_headers.clear();
        guest_input.taiko.prover_data.last_anchor_block_number = Some(7);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("stalled anchor with mismatched origin header should fail");

        assert!(error_chain_contains(&err, "taiko.l1_header hash mismatch"));
    }

    #[test]
    fn rejects_stalled_anchor_missing_parent_checkpoint() {
        let mut guest_input = guest_input_with_single_block();
        let origin_header = sample_l1_header(600, B256::from([0x77; 32]));
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers.clear();
        guest_input.taiko.prover_data.last_anchor_block_number = Some(7);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        replace_parent_checkpoint(
            &mut guest_input,
            DecodedAnchorCheckpoint {
                block_number: TEST_PARENT_ANCHOR_BLOCK_NUMBER,
                block_hash: B256::ZERO,
                state_root: B256::ZERO,
            },
        );
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("stalled anchor without parent checkpoint should fail");

        assert!(error_chain_contains(
            &err,
            "parent CheckpointStore blockHash is zero"
        ));
    }

    #[test]
    fn rejects_stalled_anchor_checkpoint_state_root_mismatch() {
        let mut guest_input = guest_input_with_single_block();
        let parent_checkpoint =
            decode_anchor_checkpoint(&guest_input.witnesses[0].block).expect("fixture anchor");
        let origin_header = sample_l1_header(600, B256::from([0x77; 32]));
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers.clear();
        guest_input.taiko.prover_data.last_anchor_block_number =
            Some(parent_checkpoint.block_number);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: parent_checkpoint
                .block_number
                .try_into()
                .expect("fits in uint48"),
            blockHash: parent_checkpoint.block_hash,
            stateRoot: B256::from([0xFE; 32]),
        })];
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("forged stalled anchor state root should fail");

        assert!(error_chain_contains(
            &err,
            "does not match parent checkpoint"
        ));
    }

    #[test]
    fn rejects_empty_l1_ancestor_headers_without_stalled_anchor() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.l1_ancestor_headers.clear();

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("empty headers without stalled anchor should fail");

        assert!(error_chain_contains(
            &err,
            "taiko.l1_ancestor_headers must not be empty"
        ));
    }

    #[test]
    fn rejects_advancing_anchor_with_empty_l1_ancestor_headers() {
        let mut guest_input = guest_input_with_single_block();
        let origin_header = sample_l1_header(519, B256::from([0x77; 32]));
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers.clear();
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: 8u64.try_into().expect("fits in uint48"),
            blockHash: B256::from([0x88; 32]),
            stateRoot: B256::from([0x99; 32]),
        })];
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("advancing anchor without headers should fail");

        assert!(error_chain_contains(
            &err,
            "taiko.l1_ancestor_headers must not be empty"
        ));
    }

    #[test]
    fn rejects_anchor_not_signed_by_golden_touch() {
        let mut guest_input = guest_input_with_single_block();
        let checkpoint = decode_anchor_checkpoint(&guest_input.witnesses[0].block)
            .expect("fixture anchor checkpoint");
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx_with_signature(
            &AnchorV4Checkpoint {
                blockNumber: checkpoint.block_number.try_into().expect("fits in uint48"),
                blockHash: checkpoint.block_hash,
                stateRoot: checkpoint.state_root,
            },
            Signature::test_signature(),
        )];

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("non-golden-touch anchor signer should fail");

        let messages = err.chain().map(ToString::to_string).collect::<Vec<_>>();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("Anchor transaction sender must")),
            "unexpected error chain: {messages:?}"
        );
    }

    #[test]
    fn rejects_anchor_regression_within_batch() {
        let mut guest_input = guest_input_with_single_block();
        let header_seven = sample_l1_header(7, B256::from([0x66; 32]));
        let mut header_eight = sample_l1_header(8, B256::from([0x77; 32]));
        header_eight.parent_hash = header_seven.hash_slow();
        guest_input.taiko.l1_header = header_eight.clone();
        guest_input.taiko.l1_ancestor_headers = vec![header_seven.clone(), header_eight.clone()];
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            header_eight.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = header_eight.hash_slow();
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: header_eight.number.try_into().expect("fits in uint48"),
            blockHash: header_eight.hash_slow(),
            stateRoot: header_eight.state_root,
        })];
        let first_block_hash = guest_input.witnesses[0].block.header.hash_slow();

        let mut second = guest_input.witnesses[0].clone();
        second.block.header.number = guest_input.witnesses[0].block.header.number + 1;
        second.block.header.parent_hash = first_block_hash;
        second.block.header.state_root = B256::from([0x22; 32]);
        second.block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: header_seven.number.try_into().expect("fits in uint48"),
            blockHash: header_seven.hash_slow(),
            stateRoot: header_seven.state_root,
        })];
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect_err("anchor regression should fail");

        assert!(
            error_chain_contains(&err, "regressed below previous anchor"),
            "{err:?}"
        );
    }

    #[test]
    fn accepts_repeated_anchor_checkpoint_with_single_matching_l1_header() {
        let mut guest_input = guest_input_with_single_block();
        let first_block_hash = guest_input.witnesses[0].block.header.hash_slow();
        let checkpoint = {
            let header = &guest_input.taiko.l1_header;
            AnchorV4Checkpoint {
                blockNumber: header.number.try_into().expect("fits in uint48"),
                blockHash: header.hash_slow(),
                stateRoot: header.state_root,
            }
        };

        let mut second = guest_input.witnesses[0].clone();
        second.block.header.number = guest_input.witnesses[0].block.header.number + 1;
        second.block.header.parent_hash = first_block_hash;
        second.block.header.state_root = B256::from([0x22; 32]);
        second.block.body.transactions = vec![anchor_tx(&checkpoint)];
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let subproof_input_hash = prove_shasta_proposal_with_validator(
            &guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
        .expect("repeated anchor checkpoint should validate");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&guest_input.proof_carry_data)
        );
    }

    #[test]
    fn rejects_chain_id_mismatch_between_input_and_proof_carry_data() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.proof_carry_data.chain_id = 167_001;

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |_stateless_input, _ancestor_headers, _runtime| Ok(B256::ZERO),
        )
        .expect_err("expected chain_id mismatch to fail");

        assert!(err
            .to_string()
            .contains("proof_carry_data.chain_id mismatch"));
    }

    #[test]
    fn rejects_verifier_mismatch_between_chain_spec_and_proof_carry_data() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.proof_carry_data.verifier = Address::from([0x77; 20]);

        let err = prove_shasta_proposal_with_validator(
            &guest_input,
            |_stateless_input, _ancestor_headers, _runtime| Ok(B256::ZERO),
        )
        .expect_err("expected verifier mismatch to fail");

        assert!(err
            .to_string()
            .contains("proof_carry_data.verifier mismatch"));
    }

    #[test]
    fn rejects_non_taiko_chain_spec() {
        let mut chain_spec = taiko_mainnet_chain_spec();
        chain_spec.is_taiko = false;

        let input = StatelessInput {
            chain_spec,
            ..Default::default()
        };

        let proof_carry_data = ProofCarryData {
            chain_id: 1,
            ..Default::default()
        };
        let guest_input = GuestInput {
            witnesses: vec![input],
            proof_carry_data,
            ..Default::default()
        };

        assert!(prove_shasta_proposal_with_validator(
            &guest_input,
            |_stateless_input, _ancestor_headers, _runtime| { Ok(B256::ZERO) }
        )
        .is_err());
    }

    #[test]
    fn aggregate_shasta_zk_computes_expected_public_input() {
        let proof_carry_data = guest_input_with_single_block().proof_carry_data;

        let expected_block_input = hash_shasta_subproof_input(&proof_carry_data);

        let input = ShastaZkAggregationGuestInput {
            image_id: [1u32; 8],
            block_inputs: vec![expected_block_input],
            proof_carry_data_vec: vec![proof_carry_data.clone()],
            prover_address: Address::from([0x44; 20]),
        };

        let image_id_b256 = B256::from([0xAA; 32]);
        let mut calls = 0usize;

        let out = aggregate_shasta_zk_with_verifier(&input, image_id_b256, |_i, _block_input| {
            calls += 1;
            Ok(())
        })
        .expect("aggregation should succeed");

        assert_eq!(calls, 1);

        let commitment =
            build_shasta_commitment_from_proof_carry_data_vec(&input.proof_carry_data_vec)
                .expect("commitment should build");
        let first = input.proof_carry_data_vec.first().expect("checked");
        let aggregation_hash = shasta_aggregation_output(
            &commitment,
            first.chain_id,
            first.verifier,
            input.prover_address,
        );
        let expected = shasta_zk_aggregation_output(image_id_b256, aggregation_hash);

        assert_eq!(out, expected);
    }

    #[test]
    fn aggregate_rejects_invalid_proof_carry_sequence() {
        let first = guest_input_with_single_block().proof_carry_data;
        let mut second = first.clone();
        second.transition_input.proposal_id = first.transition_input.proposal_id;

        let input = ShastaZkAggregationGuestInput {
            image_id: [1u32; 8],
            block_inputs: vec![
                hash_shasta_subproof_input(&first),
                hash_shasta_subproof_input(&second),
            ],
            proof_carry_data_vec: vec![first, second],
            prover_address: Address::ZERO,
        };

        let err = aggregate_shasta_zk_with_verifier(&input, B256::ZERO, |_i, _block_input| Ok(()))
            .expect_err("invalid proof carry sequence should fail");
        assert!(err.to_string().contains("invalid proof_carry_data_vec"));
    }

    #[test]
    fn aggregate_rejects_block_input_mismatch() {
        let proof_carry_data = guest_input_with_single_block().proof_carry_data;
        let input = ShastaZkAggregationGuestInput {
            image_id: [1u32; 8],
            block_inputs: vec![B256::from([0xAA; 32])],
            proof_carry_data_vec: vec![proof_carry_data],
            prover_address: Address::ZERO,
        };

        let err = aggregate_shasta_zk_with_verifier(&input, B256::ZERO, |_i, _block_input| Ok(()))
            .expect_err("block input mismatch should fail");
        assert!(err.to_string().contains("block_input mismatch"));
    }

    #[test]
    fn aggregate_propagates_verifier_error() {
        let proof_carry_data = guest_input_with_single_block().proof_carry_data;
        let input = ShastaZkAggregationGuestInput {
            image_id: [1u32; 8],
            block_inputs: vec![hash_shasta_subproof_input(&proof_carry_data)],
            proof_carry_data_vec: vec![proof_carry_data],
            prover_address: Address::ZERO,
        };

        let err = aggregate_shasta_zk_with_verifier(&input, B256::ZERO, |_i, _block_input| {
            Err(anyhow::anyhow!("boom"))
        })
        .expect_err("verifier error should bubble up");
        assert!(err
            .to_string()
            .contains("proof verification failed at index 0"));
    }

    // ── Task 4: validate_l1_anchor_linkage edge cases ─────────────────────────

    fn prove_identity(guest_input: &GuestInput) -> anyhow::Result<B256> {
        prove_shasta_proposal_with_validator(
            guest_input,
            |stateless_input, _ancestor_headers, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            },
        )
    }

    /// Rebuild carry data after a mutation, then assert the prove path rejects with `expected`.
    fn assert_guest_rejects(mut guest_input: GuestInput, expected: &str) {
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");
        let err = prove_identity(&guest_input).expect_err("expected guest rejection");
        assert!(
            error_chain_contains(&err, expected),
            "error chain did not contain {expected:?}: {err:?}"
        );
    }

    /// Build a contiguous, parent-linked L1 header chain for `[start, end]` (test helper; the
    /// upstream base removed its own copy during the checkpoint-store rework).
    fn sample_l1_header_chain(start: u64, end: u64) -> Vec<alloy_consensus::Header> {
        let mut parent_hash = None;
        (start..=end)
            .map(|number| {
                let mut header = sample_l1_header(number, B256::from([number as u8; 32]));
                if let Some(hash) = parent_hash {
                    header.parent_hash = hash;
                }
                parent_hash = Some(header.hash_slow());
                header
            })
            .collect()
    }

    /// Known-good guest input whose anchor checkpoint sits at the parent anchor but the origin is
    /// only a few blocks ahead — within the anchor offset window, so the stalled-bypass does NOT
    /// apply and `validate_l1_anchor_linkage` runs the full `l1_ancestor_headers` linkage path
    /// (contiguity, parent-hash, origin pinning). A far-ahead origin would take the checkpoint-store
    /// bypass path instead and skip these checks.
    fn linked_chain_input() -> GuestInput {
        let mut guest_input = guest_input_with_single_block();
        let headers = sample_l1_header_chain(
            TEST_PARENT_ANCHOR_BLOCK_NUMBER,
            TEST_PARENT_ANCHOR_BLOCK_NUMBER + 5,
        );
        let anchor_header = headers.first().expect("anchor header").clone();
        let origin_header = headers.last().expect("origin header").clone();
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers = headers;
        guest_input.taiko.prover_data.last_anchor_block_number = Some(anchor_header.number);
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: anchor_header.number.try_into().expect("fits in uint48"),
            blockHash: anchor_header.hash_slow(),
            stateRoot: anchor_header.state_root,
        })];
        guest_input
    }

    #[test]
    fn linkage_guest_control_stalled_chain_proves() {
        let mut guest_input = linked_chain_input();
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");
        prove_identity(&guest_input).expect("known-good stalled chain proves");
    }

    #[test]
    fn linkage_accepts_forced_prefix_then_normal_anchor_catchup() {
        let mut guest_input = guest_input_with_single_block();
        let parent_checkpoint =
            decode_anchor_checkpoint(&guest_input.witnesses[0].block).expect("fixture anchor");
        let headers = sample_l1_header_chain(200, 205);
        let normal_header = headers.first().expect("normal anchor header").clone();
        let origin_header = headers.last().expect("origin header").clone();
        let normal_checkpoint = DecodedAnchorCheckpoint {
            block_number: normal_header.number,
            block_hash: normal_header.hash_slow(),
            state_root: normal_header.state_root,
        };
        let anchor_checkpoints = vec![
            parent_checkpoint,
            parent_checkpoint,
            parent_checkpoint,
            parent_checkpoint,
            normal_checkpoint,
        ];
        let source_spans = [
            AnchorSourceSpan {
                is_forced_inclusion: true,
                block_count: 1,
            },
            AnchorSourceSpan {
                is_forced_inclusion: true,
                block_count: 1,
            },
            AnchorSourceSpan {
                is_forced_inclusion: true,
                block_count: 1,
            },
            AnchorSourceSpan {
                is_forced_inclusion: true,
                block_count: 1,
            },
            AnchorSourceSpan {
                is_forced_inclusion: false,
                block_count: 1,
            },
        ];
        guest_input.taiko.l1_header = origin_header.clone();
        guest_input.taiko.l1_ancestor_headers = headers;
        guest_input.taiko.proposal_event.proposal.originBlockNumber =
            origin_header.number.try_into().expect("fits in uint48");
        guest_input.taiko.proposal_event.proposal.originBlockHash = origin_header.hash_slow();

        validate_l1_anchor_linkage(
            &guest_input,
            &anchor_checkpoints,
            parent_checkpoint.block_number,
            Some(&source_spans),
        )
        .expect("forced prefix should validate against parent checkpoint");
    }

    #[test]
    fn linkage_guest_rejects_non_contiguous_ancestor_headers() {
        let mut guest_input = linked_chain_input();
        // drop one middle header to create a number gap
        let mid = guest_input.taiko.l1_ancestor_headers.len() / 2;
        guest_input.taiko.l1_ancestor_headers.remove(mid);
        assert_guest_rejects(guest_input, "must be contiguous");
    }

    #[test]
    fn linkage_guest_rejects_broken_parent_hash_chain() {
        let mut guest_input = linked_chain_input();
        guest_input.taiko.l1_ancestor_headers[1].parent_hash = B256::from([0x01; 32]);
        assert_guest_rejects(guest_input, "parent hash mismatch");
    }

    #[test]
    fn linkage_guest_rejects_last_ancestor_not_origin_number() {
        let mut guest_input = linked_chain_input();
        // truncate the chain so the last header no longer reaches origin
        guest_input.taiko.l1_ancestor_headers.pop();
        assert_guest_rejects(guest_input, "last block number mismatch");
    }

    #[test]
    fn linkage_guest_rejects_l1_header_number_not_origin() {
        let mut guest_input = linked_chain_input();
        // keep originBlockNumber, but make taiko.l1_header.number disagree with it
        guest_input.taiko.l1_header.number += 1;
        assert_guest_rejects(guest_input, "l1_header.number mismatch");
    }

    #[test]
    fn linkage_guest_rejects_forged_checkpoint_state_root() {
        let mut guest_input = linked_chain_input();
        let anchor_header = guest_input.taiko.l1_ancestor_headers[0].clone();
        guest_input.witnesses[0].block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: anchor_header.number.try_into().expect("fits in uint48"),
            blockHash: anchor_header.hash_slow(),
            stateRoot: B256::from([0xFE; 32]), // forged
        })];
        assert_guest_rejects(guest_input, "not found in taiko.l1_ancestor_headers");
    }

    // ── Task 5: anchor-transaction common checks ──────────────────────────────

    fn golden_touch_sign(tx: TxEip1559) -> reth_ethereum_primitives::TransactionSigned {
        let signature = golden_touch_signer()
            .sign_hash_sync(&tx.signature_hash())
            .expect("golden touch signature");
        tx.into_signed(signature).into()
    }

    fn base_checkpoint() -> AnchorV4Checkpoint {
        // Matches the anchor checkpoint built inside guest_input_with_single_block (L1 block 7).
        let header = sample_l1_header(TEST_PARENT_ANCHOR_BLOCK_NUMBER, B256::from([0x66; 32]));
        AnchorV4Checkpoint {
            blockNumber: header.number.try_into().expect("fits in uint48"),
            blockHash: header.hash_slow(),
            stateRoot: header.state_root,
        }
    }

    #[test]
    fn anchortx_rejects_wrong_recipient() {
        let mut guest_input = guest_input_with_single_block();
        let tx = unsigned_anchor_tx(&base_checkpoint(), Address::from([0x99; 20])); // not l2_contract
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "recipient mismatch");
    }

    #[test]
    fn anchortx_rejects_nonzero_priority_fee() {
        let mut guest_input = guest_input_with_single_block();
        let mut tx = unsigned_anchor_tx(&base_checkpoint(), test_anchor_address());
        tx.max_priority_fee_per_gas = 1; // must be 0
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "max_priority_fee_per_gas mismatch");
    }

    #[test]
    fn anchortx_rejects_max_fee_not_equal_base_fee() {
        let mut guest_input = guest_input_with_single_block();
        let mut tx = unsigned_anchor_tx(&base_checkpoint(), test_anchor_address());
        tx.max_fee_per_gas = 2; // base fee in the fixture header is 1
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "max_fee_per_gas mismatch");
    }

    #[test]
    fn anchortx_rejects_nonempty_access_list() {
        let mut guest_input = guest_input_with_single_block();
        let mut tx = unsigned_anchor_tx(&base_checkpoint(), test_anchor_address());
        // Build a non-empty access list via hand-crafted RLP since alloy_eips/alloy_eip2930
        // are not direct dependencies. AccessList wraps Vec<AccessListItem>:
        //   AccessListItem = RLP list[address(20 bytes), storageKeys(list of B256)]
        //   AccessList = RLP list-of-items
        //
        // Encoding of AccessList([{addr:[0x01;20], keys:[B256::ZERO]}]):
        //   B256::ZERO (32 zeros): 0xa0 + [0x00;32]              = 33 bytes
        //   keys list (1 key):     0xe1 + [0xa0,0x00*32]         = 34 bytes (0xc0+33)
        //   address [0x01;20]:     0x94 + [0x01;20]              = 21 bytes
        //   item list (payload=55) 0xf7 + [0x94,0x01*20,0xe1,0xa0,0x00*32] = 56 bytes
        //   outer list (payload=56>55) needs long-list:
        //     [0xf8, 0x38] + item_list                           = 58 bytes total
        {
            use alloy_rlp::Decodable;
            let item_payload_bytes = {
                let mut v = Vec::new();
                v.push(0x94u8); // 20-byte string prefix
                v.extend_from_slice(&[0x01u8; 20]); // address
                v.push(0xe1u8); // keys list header: 0xc0+33
                v.push(0xa0u8); // 32-byte string prefix
                v.extend_from_slice(&[0x00u8; 32]); // B256::ZERO
                v
            }; // len = 21 + 34 = 55

            let item_list: Vec<u8> = {
                let mut v = vec![0xc0u8 + item_payload_bytes.len() as u8]; // 0xf7
                v.extend_from_slice(&item_payload_bytes);
                v
            }; // len = 56

            // Outer list: payload=56 > 55 → long-list encoding [0xf8, 56, payload...]
            let outer: Vec<u8> = {
                let mut v = vec![0xf8u8, 56u8]; // long-list, length=56
                v.extend_from_slice(&item_list);
                v
            }; // len = 58

            let mut buf: &[u8] = &outer;
            tx.access_list = Decodable::decode(&mut buf).expect("valid access list RLP");
        }
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "access list must be empty");
    }

    #[test]
    fn anchortx_rejects_wrong_chain_id() {
        let mut guest_input = guest_input_with_single_block();
        let mut tx = unsigned_anchor_tx(&base_checkpoint(), test_anchor_address());
        tx.chain_id = 167_001; // fixture chain is 167_000
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "chain_id mismatch");
    }

    #[test]
    fn anchortx_rejects_wrong_selector_first_tx() {
        // Replace the anchor tx with a non-anchorV4 call (wrong 4-byte selector) signed by golden touch.
        // The selector check fires inside validate_anchor_transaction (alethia-reth-consensus) before
        // reaching our decode_anchor_checkpoint path; the error emitted is "does not match the expected
        // selector" rather than "not anchorV4".
        let mut guest_input = guest_input_with_single_block();
        let mut tx = unsigned_anchor_tx(&base_checkpoint(), test_anchor_address());
        tx.input = alloy_primitives::Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]); // selector no longer matches anchorV4
        guest_input.witnesses[0].block.body.transactions = vec![golden_touch_sign(tx)];
        assert_guest_rejects(guest_input, "does not match the expected selector");
    }

    #[test]
    fn anchortx_rejects_missing_base_fee() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.witnesses[0].block.header.base_fee_per_gas = None;
        assert_guest_rejects(guest_input, "missing base fee per gas");
    }

    #[test]
    fn anchortx_rejects_missing_anchor_transaction() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.witnesses[0].block.body.transactions = Vec::new();
        assert_guest_rejects(guest_input, "missing anchor transaction");
    }

    #[test]
    fn linkage_guest_rejects_l1_header_hash_not_origin() {
        // l1_header.number still equals origin, but its hash no longer matches originBlockHash.
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.proposal_event.proposal.originBlockHash = B256::from([0xAB; 32]);
        assert_guest_rejects(guest_input, "l1_header hash mismatch");
    }

    #[test]
    fn linkage_guest_rejects_last_ancestor_hash_mismatch() {
        // l1_header check passes, but the LAST ancestor header's hash no longer matches the origin hash.
        let mut guest_input = linked_chain_input();
        let last = guest_input.taiko.l1_ancestor_headers.len() - 1;
        guest_input.taiko.l1_ancestor_headers[last].state_root = B256::from([0xCD; 32]);
        assert_guest_rejects(guest_input, "last hash mismatch");
    }

    // ── Task 7: derivation guards + empty-sources parity ─────────────────────

    #[test]
    fn derivation_guest_empty_sources_skips_derivation() {
        // P-2 (defensive no-op, NOT a soundness gap): empty proposal.sources makes
        // derive_expected_shasta_blocks return Ok(None), so the per-block manifest/derivation checks are
        // skipped. This is unreachable for any finalizable proof: the proof binds to hash_proposal(proposal)
        // (== the on-chain LibHashOptimized.hashProposal = keccak256(abi.encode(Proposal))), which commits to
        // the full `sources` array; and the inbox always builds `new DerivationSource[](toProcess + 1)`, i.e.
        // >= 1 source (the proposer cannot force it empty). So an empty-sources proposal can never match a
        // finalized on-chain proposal hash. This test pins the harmless current behavior; the client's
        // EmptyDerivationSources rejection is a client-side convenience, not a soundness requirement here.
        let guest_input = guest_input_with_single_block(); // sources are empty by construction
        assert!(guest_input.taiko.proposal_event.proposal.sources.is_empty());
        let mut gi = guest_input;
        gi.proof_carry_data =
            build_proof_carry_data(&gi, ProofType::Native).expect("build carry data");
        prove_identity(&gi).expect("empty-sources proposal still proves");
    }

    #[test]
    fn derivation_guest_rejects_last_source_forced_inclusion() {
        use raiko2_protocol::InputDataSource;
        use raiko2_protocol_shasta::shasta::DerivationSource;
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.proposal_event.proposal.sources = vec![DerivationSource {
            isForcedInclusion: true,
            ..Default::default()
        }];
        // data_sources must match sources len to pass verify_proposal_mode_blob_usage (the
        // earlier gate); then derive_expected_shasta_blocks fires the isForcedInclusion guard.
        guest_input.taiko.data_sources = vec![InputDataSource::default()];
        assert_guest_rejects(
            guest_input,
            "last Shasta derivation source must be a normal source",
        );
    }

    #[test]
    fn derivation_guest_rejects_data_source_count_mismatch() {
        use raiko2_protocol_shasta::shasta::DerivationSource;
        let mut guest_input = guest_input_with_single_block();
        // With proposal.sources non-empty but data_sources empty, verify_proposal_mode_blob_usage
        // rejects immediately (before derive_expected_shasta_blocks is ever reached) because
        // sources.len()==1 != data_sources.len()==0.
        guest_input.taiko.proposal_event.proposal.sources = vec![DerivationSource::default()];
        guest_input.taiko.data_sources = Vec::new();
        assert_guest_rejects(guest_input, "data source count");
    }
}
