#![allow(missing_docs)]

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Address, B256};
use alloy_primitives::{Signature, TxKind, U256};
use alloy_sol_types::{sol, SolCall};
use raiko2_guest_common::{prove_shasta_proposal, prove_shasta_proposal_with_validator};
use raiko2_primitives::{ChainSpec, ProofType, StatelessInput, SupportedChainSpecs};
use raiko2_primitives_shasta::{build_proof_carry_data, GuestInput};
use raiko2_protocol_shasta::TaikoManifest;

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;
}

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

fn assert_rejected_with_message(guest_input: &GuestInput, expected: &str) {
    let err = prove_shasta_proposal_with_validator(guest_input, |stateless_input, _runtime| {
        Ok(stateless_input.block.header.hash_slow())
    })
    .expect_err("guest input should fail validation");
    assert!(
        err.to_string().contains(expected),
        "expected error containing '{expected}', got '{err}'",
    );
}

#[test]
fn rejects_witness_is_taiko_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    let mut second = guest_input.witnesses[0].clone();
    second.block.header.number += 1;
    second.block.header.parent_hash = guest_input.witnesses[0].block.header.hash_slow();
    second.chain_spec.is_taiko = false;
    guest_input.witnesses.push(second);
    guest_input.proof_carry_data =
        build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data");

    assert_rejected_with_message(&guest_input, "chain_spec mismatch");
}

#[test]
fn rejects_proof_carry_data_proposal_id_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input.proof_carry_data.transition_input.proposal_id += 1;

    assert_rejected_with_message(&guest_input, "proof_carry_data.proposal_id mismatch");
}

#[test]
fn rejects_proof_carry_data_actual_prover_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input.proof_carry_data.transition_input.actual_prover = Address::from([0x55; 20]);

    assert_rejected_with_message(&guest_input, "proof_carry_data.actual_prover mismatch");
}

#[test]
fn rejects_proof_carry_data_proposal_hash_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input.proof_carry_data.transition_input.proposal_hash = B256::from([0x66; 32]);

    assert_rejected_with_message(&guest_input, "proof_carry_data.proposal_hash mismatch");
}

#[test]
fn rejects_proof_carry_data_parent_proposal_hash_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .parent_proposal_hash = B256::from([0x77; 32]);

    assert_rejected_with_message(
        &guest_input,
        "proof_carry_data.parent_proposal_hash mismatch",
    );
}

#[test]
fn rejects_transition_proposer_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .transition
        .proposer = Address::from([0x88; 20]);

    assert_rejected_with_message(
        &guest_input,
        "proof_carry_data.transition.proposer mismatch",
    );
}

#[test]
fn rejects_transition_timestamp_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .transition
        .timestamp += 1;

    assert_rejected_with_message(
        &guest_input,
        "proof_carry_data.transition.timestamp mismatch",
    );
}

#[test]
fn rejects_proof_carry_data_parent_block_hash_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .parent_block_hash = B256::from([0x99; 32]);

    assert_rejected_with_message(&guest_input, "proof_carry_data.parent_block_hash mismatch");
}

#[test]
fn rejects_checkpoint_block_number_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .blockNumber = 2u64.try_into().expect("block number fits in uint48");

    assert_rejected_with_message(
        &guest_input,
        "proof_carry_data.checkpoint.blockNumber mismatch",
    );
}

#[test]
fn rejects_checkpoint_state_root_mismatch() {
    let mut guest_input = guest_input_with_single_block();
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .stateRoot = B256::from([0xAB; 32]);

    assert_rejected_with_message(
        &guest_input,
        "proof_carry_data.checkpoint.stateRoot mismatch",
    );
}

#[test]
fn wraps_validator_error_with_block_index() {
    let guest_input = guest_input_with_single_block();

    let err = prove_shasta_proposal_with_validator(&guest_input, |_stateless_input, _runtime| {
        Err(anyhow::anyhow!("boom"))
    })
    .expect_err("validator failure should bubble up");

    assert!(err
        .to_string()
        .contains("stateless block validation failed at index 0"));
    assert!(err.chain().any(|cause| cause.to_string().contains("boom")));
}

#[test]
fn top_level_proposal_proof_rejects_invalid_blob_usage_before_execution() {
    let mut guest_input = guest_input_with_single_block();
    guest_input.taiko.data_sources.push(Default::default());
    let data_source = guest_input
        .taiko
        .data_sources
        .last_mut()
        .expect("data source should exist");
    data_source.tx_data_from_blob.push(vec![0u8; 32]);

    let err = prove_shasta_proposal(&guest_input)
        .expect_err("blob usage mismatch should fail before execution");

    assert!(err
        .to_string()
        .contains("proposal mode blob usage verification failed"));
    assert!(err.chain().any(|cause| cause
        .to_string()
        .contains("missing proposal source for data source index 0")));
}
