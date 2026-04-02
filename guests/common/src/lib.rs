//! Helpers for zkVM guest programs.

use alethia_reth_block::config::TaikoEvmConfig;
use alethia_reth_chainspec::spec::TaikoChainSpec;
use alloy_consensus::transaction::Transaction as _;
use alloy_primitives::B256;
use alloy_sol_types::{SolCall, sol};
use anyhow::{Context, Result, ensure};
use raiko2_primitives::{ChainSpec, StatelessInput, SupportedChainSpecs};
use raiko2_primitives_shasta::{
    GuestInput, ShastaZkAggregationGuestInput,
    instance::{
        build_shasta_commitment_from_proof_carry_data_vec, shasta_aggregation_output,
        shasta_zk_aggregation_output,
    },
    validate_anchor_progression, verify_proposal_mode_blob_usage,
};
use raiko2_protocol_shasta::libhash::{hash_proposal, hash_shasta_subproof_input};
use raiko2_stateless::validate_block;
use std::sync::Arc;

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
    let Some(verified_chain_spec) =
        SupportedChainSpecs::default().get_chain_spec_with_chain_id(chain_spec.chain_id)
    else {
        return Ok(());
    };

    ensure!(
        chain_spec.max_spec_id == verified_chain_spec.max_spec_id,
        "unexpected max_spec_id"
    );
    ensure!(
        chain_spec.hard_forks == verified_chain_spec.hard_forks,
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
        chain_spec.verifier_address_forks == verified_chain_spec.verifier_address_forks,
        "unexpected verifier_address_forks"
    );
    ensure!(
        chain_spec.is_taiko == verified_chain_spec.is_taiko,
        "unexpected is_taiko"
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
) -> Result<()> {
    let proposal = &guest_input.taiko.proposal_event.proposal;
    let origin_block_number = proposal.originBlockNumber.to::<u64>();
    let origin_block_hash = proposal.originBlockHash;
    let anchor_block_numbers = anchor_checkpoints
        .iter()
        .map(|checkpoint| checkpoint.block_number)
        .collect::<Vec<_>>();
    validate_anchor_progression(
        &anchor_block_numbers,
        guest_input
            .taiko
            .prover_data
            .last_anchor_block_number
            .unwrap_or_default(),
        origin_block_number,
        guest_input.taiko.chain_spec.chain_id,
    )
    .map_err(anyhow::Error::msg)?;

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
    ensure!(
        !guest_input.taiko.l1_ancestor_headers.is_empty(),
        "taiko.l1_ancestor_headers must not be empty"
    );

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
            let Some(checkpoint) = anchor_checkpoints.get(checkpoint_index) else {
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
    if let Some(checkpoint) = anchor_checkpoints.get(checkpoint_index) {
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

pub fn prove_shasta_proposal(guest_input: &GuestInput) -> Result<B256> {
    verify_proposal_mode_blob_usage(guest_input)
        .context("proposal mode blob usage verification failed")?;

    prove_shasta_proposal_with_validator(guest_input, |stateless_input, runtime| {
        validate_block(
            stateless_input.block.clone(),
            &stateless_input.witness,
            stateless_input.accounts.clone(),
            &runtime.chain_spec,
            &runtime.evm_config,
        )
        .map_err(|e| anyhow::anyhow!(e))
    })
}

pub fn prove_shasta_proposal_with_validator<V>(
    guest_input: &GuestInput,
    mut validate_block: V,
) -> Result<B256>
where
    V: FnMut(&StatelessInput, &TaikoRuntime) -> Result<B256>,
{
    let proof_carry_data = &guest_input.proof_carry_data;
    ensure!(
        !guest_input.witnesses.is_empty(),
        "GuestInput must contain at least one witness"
    );

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

    let runtime = TaikoRuntime::from_chain_spec(first_chain_spec)
        .context("Failed to build Taiko runtime from GuestInput chain_spec")?;

    let mut anchor_checkpoints = Vec::with_capacity(guest_input.witnesses.len());
    let mut first_parent_block_hash = None;
    let mut previous_block_hash: Option<B256> = None;
    let mut previous_block_number: Option<u64> = None;
    let mut last_block_number = None;
    let mut last_block_hash = None;
    let mut last_state_root = None;

    for (index, stateless_input) in guest_input.witnesses.iter().enumerate() {
        let block = &stateless_input.block;
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

        let validated_hash = validate_block(stateless_input, &runtime)
            .with_context(|| format!("stateless block validation failed at index {index}"))?;
        first_parent_block_hash.get_or_insert(block.header.parent_hash);
        previous_block_hash = Some(validated_hash);
        previous_block_number = Some(block.header.number);
        last_block_number = Some(block.header.number);
        last_block_hash = Some(validated_hash);
        last_state_root = Some(block.header.state_root);
        anchor_checkpoints.push(decode_anchor_checkpoint(block)?);
    }

    validate_l1_anchor_linkage(guest_input, &anchor_checkpoints)?;
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

    Ok(hash_shasta_subproof_input(proof_carry_data))
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
    use alloy_consensus::{SignableTransaction, TxEip1559};
    use alloy_primitives::{Address, Signature, TxKind, U256};
    use raiko2_primitives::ProofType;
    use raiko2_primitives::{ChainSpec, StatelessInput, SupportedChainSpecs};
    use raiko2_primitives_shasta::build_proof_carry_data;
    use raiko2_protocol_shasta::TaikoManifest;
    use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
    use raiko2_protocol_shasta::shasta::ProofCarryData;

    fn taiko_mainnet_chain_spec() -> ChainSpec {
        SupportedChainSpecs::default()
            .get_chain_spec_with_chain_id(167_000)
            .expect("supported taiko mainnet chain spec")
    }

    fn sample_l1_header(number: u64, state_root: B256) -> alloy_consensus::Header {
        let mut header = alloy_consensus::Header::default();
        header.number = number;
        header.parent_hash = B256::from([0xAA; 32]);
        header.state_root = state_root;
        header
    }

    fn anchor_tx(checkpoint: &AnchorV4Checkpoint) -> reth_ethereum_primitives::TransactionSigned {
        TxEip1559 {
            chain_id: 167_000,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            access_list: Default::default(),
            input: anchorV4Call {
                _checkpoint: checkpoint.clone(),
            }
            .abi_encode()
            .into(),
        }
        .into_signed(Signature::test_signature())
        .into()
    }

    fn guest_input_with_single_block() -> GuestInput {
        let chain_spec = taiko_mainnet_chain_spec();
        let mut input = StatelessInput {
            chain_spec,
            ..Default::default()
        };
        input.block.header.number = 1;
        input.block.header.timestamp = u64::MAX / 2;
        input.block.header.parent_hash = B256::from([9u8; 32]);
        input.block.header.state_root = B256::from([1u8; 32]);
        let l1_header = sample_l1_header(7, B256::from([0x66; 32]));
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

    #[test]
    fn prove_shasta_proposal_builds_expected_hash() {
        let guest_input = guest_input_with_single_block();
        let proof_carry_data = guest_input.proof_carry_data.clone();

        let subproof_input_hash =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect("proposal proving should succeed");

        assert_eq!(
            subproof_input_hash,
            hash_shasta_subproof_input(&proof_carry_data)
        );
    }

    #[test]
    fn rejects_empty_witnesses() {
        let guest_input = GuestInput::default();
        let err =
            prove_shasta_proposal_with_validator(&guest_input, |_stateless_input, _runtime| {
                Ok(B256::ZERO)
            })
            .expect_err("empty witnesses should fail");
        assert!(
            err.to_string()
                .contains("must contain at least one witness")
        );
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
        second.block.body.transactions = first.block.body.transactions.clone();
        guest_input.witnesses = vec![first, second];
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect_err("expected parent-hash mismatch to fail");

        assert!(err.to_string().contains("must link to previous block hash"));
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

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect_err("block number gap should fail");

        assert!(err.to_string().contains("must increment block number by 1"));
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

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
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

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect_err("checkpoint mismatch should fail");

        assert!(err.to_string().contains("checkpoint.blockHash mismatch"));
    }

    #[test]
    fn rejects_anchor_sequences_that_do_not_grow_past_last_anchor() {
        let mut guest_input = guest_input_with_single_block();
        guest_input.taiko.prover_data.last_anchor_block_number = Some(7);

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect_err("stalled anchor should fail");

        assert!(err.to_string().contains("did not grow"));
    }

    #[test]
    fn rejects_anchor_regression_within_batch() {
        let mut guest_input = guest_input_with_single_block();
        let first_block_hash = guest_input.witnesses[0].block.header.hash_slow();
        let header_six = sample_l1_header(6, B256::from([0x55; 32]));
        let mut header_seven = sample_l1_header(7, B256::from([0x66; 32]));
        header_seven.parent_hash = header_six.hash_slow();
        guest_input.taiko.l1_header = header_seven.clone();
        guest_input.taiko.l1_ancestor_headers = vec![header_six.clone(), header_seven.clone()];

        let mut second = guest_input.witnesses[0].clone();
        second.block.header.number = 2;
        second.block.header.parent_hash = first_block_hash;
        second.block.header.state_root = B256::from([0x22; 32]);
        second.block.body.transactions = vec![anchor_tx(&AnchorV4Checkpoint {
            blockNumber: header_six.number.try_into().expect("fits in uint48"),
            blockHash: header_six.hash_slow(),
            stateRoot: header_six.state_root,
        })];
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
            .expect_err("anchor regression should fail");

        assert!(err.to_string().contains("regressed below previous anchor"));
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
        second.block.header.number = 2;
        second.block.header.parent_hash = first_block_hash;
        second.block.header.state_root = B256::from([0x22; 32]);
        second.block.body.transactions = vec![anchor_tx(&checkpoint)];
        guest_input.witnesses.push(second);
        guest_input.proof_carry_data =
            build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

        let subproof_input_hash =
            prove_shasta_proposal_with_validator(&guest_input, |stateless_input, _runtime| {
                Ok(stateless_input.block.header.hash_slow())
            })
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

        let err =
            prove_shasta_proposal_with_validator(&guest_input, |_stateless_input, _runtime| {
                Ok(B256::ZERO)
            })
            .expect_err("expected chain_id mismatch to fail");

        assert!(
            err.to_string()
                .contains("proof_carry_data.chain_id mismatch")
        );
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

        assert!(
            prove_shasta_proposal_with_validator(&guest_input, |_stateless_input, _runtime| {
                Ok(B256::ZERO)
            })
            .is_err()
        );
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
        assert!(
            err.to_string()
                .contains("proof verification failed at index 0")
        );
    }
}
