#![allow(missing_docs)]

use alloy_consensus::{SignableTransaction, TxEip1559};
use alloy_primitives::{Address, B256};
use alloy_primitives::{Signature, TxKind, U256};
use alloy_sol_types::{sol, SolCall};
use raiko2_guest_common::aggregate_shasta_zk_with_verifier;
use raiko2_primitives::{ChainSpec, ProofType, StatelessInput, SupportedChainSpecs};
use raiko2_primitives_shasta::{build_proof_carry_data, GuestInput, ShastaZkAggregationGuestInput};
use raiko2_protocol_shasta::libhash::hash_shasta_subproof_input;
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
    alloy_consensus::Header {
        number,
        parent_hash: B256::from([0xAA; 32]),
        state_root,
        ..Default::default()
    }
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

fn sample_proof_carry_data() -> raiko2_protocol_shasta::shasta::ProofCarryData {
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

    build_proof_carry_data(&guest_input, ProofType::Native).expect("build carry data")
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
