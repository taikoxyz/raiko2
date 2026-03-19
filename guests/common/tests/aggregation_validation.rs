#![allow(missing_docs)]

use alloy_primitives::{Address, B256};
use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_primitives::chain_spec::Eip1559Constants;
use raiko2_primitives::{ChainSpec, StatelessInput};
use raiko2_primitives_shasta::{
    GuestInput, ShastaZkAggregationGuestInput, build_proof_carry_data,
};
use raiko2_protocol_shasta::TaikoManifest;
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
use reth_revm::primitives::hardfork::SpecId;

fn taiko_mainnet_chain_spec() -> ChainSpec {
    ChainSpec::new_single(
        "taiko_mainnet".to_string(),
        167_000,
        SpecId::CANCUN,
        Eip1559Constants::default(),
        true,
    )
}

fn sample_proof_carry_data() -> raiko2_protocol_shasta::shasta::ProofCarryData {
    let chain_spec = taiko_mainnet_chain_spec();
    let mut input = StatelessInput {
        chain_spec,
        ..Default::default()
    };
    input.block.header.number = 1;
    input.block.header.parent_hash = B256::from([9u8; 32]);
    input.block.header.state_root = B256::from([1u8; 32]);

    let mut guest_input = GuestInput {
        witnesses: vec![input],
        taiko: TaikoManifest {
            proposal_id: 42,
            ..Default::default()
        },
        ..Default::default()
    };
    guest_input.taiko.prover_data.actual_prover = Address::from([0x22; 20]);
    guest_input.taiko.proposal_event.proposal.proposer = Address::from([0x33; 20]);
    guest_input.taiko.proposal_event.proposal.timestamp =
        123u64.try_into().expect("timestamp fits in uint48");
    guest_input.taiko.proposal_event.proposal.parentProposalHash = B256::from([0x44; 32]);

    build_proof_carry_data(&guest_input)
}

#[test]
fn aggregate_rejects_empty_proof_carry_data_vec() {
    let input = ShastaZkAggregationGuestInput {
        image_id: [1u32; 8],
        block_inputs: vec![],
        proof_carry_data_vec: vec![],
        prover_address: Address::ZERO,
    };

    let err = aggregate_shasta_zk_with_verifier(&input, B256::ZERO, |_i, _block_input| Ok(()))
        .expect_err("empty proof carry data should fail");

    assert!(err
        .to_string()
        .contains("proof_carry_data_vec must not be empty"));
}

#[test]
fn aggregate_rejects_block_input_length_mismatch() {
    let proof_carry_data = sample_proof_carry_data();
    let input = ShastaZkAggregationGuestInput {
        image_id: [1u32; 8],
        block_inputs: vec![
            hash_shasta_subproof_input(&proof_carry_data),
            hash_shasta_subproof_input(&proof_carry_data),
        ],
        proof_carry_data_vec: vec![proof_carry_data],
        prover_address: Address::ZERO,
    };

    let err = aggregate_shasta_zk_with_verifier(&input, B256::ZERO, |_i, _block_input| Ok(()))
        .expect_err("length mismatch should fail");

    assert!(err
        .to_string()
        .contains("block_inputs/proof_carry_data_vec length mismatch"));
}
