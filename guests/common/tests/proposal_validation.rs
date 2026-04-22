#![allow(missing_docs)]

use alethia_reth_block::config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes};
use alethia_reth_primitives::addresses::TAIKO_GOLDEN_TOUCH_ADDRESS;
use alloy_consensus::{constants::KECCAK_EMPTY, SignableTransaction, TrieAccount, TxEip1559};
use alloy_primitives::{keccak256, Address, Bytes, B256};
use alloy_primitives::{Signature, TxKind, U256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{sol, SolCall, SolValue};
use alloy_trie::EMPTY_ROOT_HASH;
use raiko2_guest_common::{prove_shasta_proposal, prove_shasta_proposal_with_validator};
use raiko2_primitives::{
    ChainSpec, ProofType, StatelessInput, SupportedChainSpecs, WitnessStateNode,
};
use raiko2_primitives_shasta::{build_proof_carry_data, GuestInput};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_shasta::libhash::hash_proposal;
use raiko2_protocol_shasta::shasta::{
    manifest::{BlockManifest, DerivationSourceManifest},
    BlobSlice, DerivationSource,
};
use raiko2_protocol_shasta::TaikoManifest;
use raiko2_stateless::reconstruct_block_from_transactions_with_witness_resources;
use reth_primitives_traits::SignedTransaction;
use risc0_ethereum_trie::Trie;

const GOLDEN_TOUCH_PRIVATE_KEY: &str =
    "0x92954368afd3caa1f3ce3ead0069c1af414054aefe1ef9aeacc1bf426222ce38";
const INVALID_TX_PRIVATE_KEY: &str =
    "0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;

    struct ShastaDifficultyInput {
        bytes32 parentDifficulty;
        uint256 blockNumber;
    }
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

fn sign_tx(tx: TxEip1559, private_key: &str) -> reth_ethereum_primitives::TransactionSigned {
    let signer: PrivateKeySigner = private_key.parse().expect("valid signer key");
    let signature = signer
        .sign_hash_sync(&tx.signature_hash())
        .expect("sign transaction");
    tx.into_signed(signature).into()
}

fn guest_input_with_single_block() -> GuestInput {
    let chain_spec = taiko_mainnet_chain_spec();
    let parent_header = alloy_consensus::Header {
        number: 0,
        timestamp: (u64::MAX / 2) - 1,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(1),
        ..Default::default()
    };
    let mut input = StatelessInput {
        chain_spec,
        ..Default::default()
    };
    input.block.header.number = 1;
    input.block.header.timestamp = u64::MAX / 2;
    input.block.header.parent_hash = parent_header.hash_slow();
    input.block.header.state_root = B256::from([1u8; 32]);
    input.witness.headers = vec![raiko2_primitives::WitnessHeader::from_header(parent_header)];
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

fn canonical_inline_source_guest_input() -> GuestInput {
    let mut guest_input = guest_input_with_single_block();
    let chain_spec = guest_input.witnesses[0].chain_spec.clone();
    let parent_timestamp = 1_775_135_700u64;
    let block_timestamp = parent_timestamp + 1;
    let proposal_timestamp = parent_timestamp + 100;
    let parent_header = alloy_consensus::Header {
        number: 0,
        timestamp: parent_timestamp,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(1),
        ..Default::default()
    };
    let l1_header = sample_l1_header(7, B256::from([0x66; 32]));
    let checkpoint = AnchorV4Checkpoint {
        blockNumber: l1_header.number.try_into().expect("fits in uint48"),
        blockHash: l1_header.hash_slow(),
        stateRoot: l1_header.state_root,
    };
    let anchor_address = chain_spec
        .l2_contract
        .expect("shasta chain has l2 contract");
    let anchor_tx: reth_ethereum_primitives::TransactionSigned = TxEip1559 {
        chain_id: chain_spec.chain_id,
        nonce: 0,
        gas_limit: 1_000_000,
        max_fee_per_gas: 1,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(anchor_address),
        value: U256::ZERO,
        access_list: Default::default(),
        input: anchorV4Call {
            _checkpoint: checkpoint.clone(),
        }
        .abi_encode()
        .into(),
    }
    .into_signed(Signature::test_signature())
    .into();
    let anchor_signer = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);

    guest_input.witnesses[0].witness.headers = vec![raiko2_primitives::WitnessHeader::from_header(
        parent_header.clone(),
    )];
    guest_input.witnesses[0].accounts.insert(
        anchor_signer,
        TrieAccount {
            nonce: 0,
            balance: U256::ZERO,
            storage_root: B256::ZERO,
            code_hash: B256::ZERO,
        },
    );
    guest_input.witnesses[0].block.header.number = 1;
    guest_input.witnesses[0].block.header.parent_hash = parent_header.hash_slow();
    guest_input.witnesses[0].block.header.timestamp = block_timestamp;
    guest_input.witnesses[0].block.header.beneficiary =
        guest_input.taiko.proposal_event.proposal.proposer;
    guest_input.witnesses[0].block.header.gas_limit = 31_000_000;
    guest_input.witnesses[0].block.header.base_fee_per_gas = Some(1);
    guest_input.witnesses[0].block.header.difficulty = U256::ZERO;
    guest_input.witnesses[0].block.header.mix_hash = alloy_primitives::keccak256(
        ShastaDifficultyInput {
            parentDifficulty: B256::ZERO,
            blockNumber: U256::from(1),
        }
        .abi_encode(),
    );
    guest_input.witnesses[0].block.header.extra_data = [7u8, 0, 0, 0, 0, 0, 42].to_vec().into();
    guest_input.witnesses[0].block.body.transactions = vec![anchor_tx];

    guest_input.taiko.l1_header = l1_header.clone();
    guest_input.taiko.l1_ancestor_headers = vec![l1_header.clone()];
    guest_input.taiko.proposal_event.proposal.timestamp =
        proposal_timestamp.try_into().expect("fits in uint48");
    guest_input.taiko.proposal_event.proposal.originBlockNumber =
        l1_header.number.try_into().expect("fits in uint48");
    guest_input.taiko.proposal_event.proposal.originBlockHash = l1_header.hash_slow();
    guest_input.taiko.proposal_event.proposal.basefeeSharingPctg = 7;
    guest_input.taiko.proposal_event.proposal.sources = vec![DerivationSource {
        isForcedInclusion: false,
        blobSlice: BlobSlice::default(),
    }];
    guest_input.taiko.data_sources = vec![InputDataSource {
        tx_data_from_calldata: DerivationSourceManifest {
            blocks: vec![BlockManifest {
                timestamp: block_timestamp,
                coinbase: guest_input.taiko.proposal_event.proposal.proposer,
                anchor_block_number: 7,
                gas_limit: 30_000_000,
                transactions: Vec::new(),
            }],
        }
        .encode_and_compress()
        .expect("manifest payload"),
        ..Default::default()
    }];
    guest_input.proof_carry_data.chain_id = chain_spec.chain_id;
    guest_input.proof_carry_data.transition_input.proposal_id = guest_input.taiko.proposal_id;
    guest_input.proof_carry_data.transition_input.proposal_hash =
        hash_proposal(&guest_input.taiko.proposal_event.proposal);
    guest_input
        .proof_carry_data
        .transition_input
        .parent_proposal_hash = guest_input.taiko.proposal_event.proposal.parentProposalHash;
    guest_input
        .proof_carry_data
        .transition_input
        .parent_block_hash = parent_header.hash_slow();
    guest_input.proof_carry_data.transition_input.actual_prover =
        guest_input.taiko.prover_data.actual_prover;
    guest_input
        .proof_carry_data
        .transition_input
        .transition
        .proposer = guest_input.taiko.proposal_event.proposal.proposer;
    guest_input
        .proof_carry_data
        .transition_input
        .transition
        .timestamp = proposal_timestamp;
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .blockNumber = 1u64.try_into().expect("fits in uint48");
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .blockHash = guest_input.witnesses[0].block.header.hash_slow();
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .stateRoot = guest_input.witnesses[0].block.header.state_root;

    guest_input
}

fn one_pass_guest_input_with_skipped_invalid_tx() -> GuestInput {
    let mut guest_input = canonical_inline_source_guest_input();
    let chain_spec = guest_input.witnesses[0].chain_spec.clone();
    let taiko_chain_spec = chain_spec
        .to_taiko_chain_spec()
        .expect("taiko chain spec conversion");
    let evm_config = TaikoEvmConfig::new(taiko_chain_spec.clone());
    let valid_base_fee = 25_000_000u64;
    let funded_balance = U256::from(1_000_000_000_000_000_000u128);
    let anchor_signer = Address::from(TAIKO_GOLDEN_TOUCH_ADDRESS);
    let golden_touch_signer: PrivateKeySigner = GOLDEN_TOUCH_PRIVATE_KEY
        .parse()
        .expect("valid golden touch key");
    assert_eq!(golden_touch_signer.address(), anchor_signer);

    let checkpoint = AnchorV4Checkpoint {
        blockNumber: guest_input
            .taiko
            .l1_header
            .number
            .try_into()
            .expect("fits in uint48"),
        blockHash: guest_input.taiko.l1_header.hash_slow(),
        stateRoot: guest_input.taiko.l1_header.state_root,
    };
    let anchor_tx: reth_ethereum_primitives::TransactionSigned = sign_tx(
        TxEip1559 {
            chain_id: chain_spec.chain_id,
            nonce: 0,
            gas_limit: 1_000_000,
            max_fee_per_gas: u128::from(valid_base_fee),
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(
                chain_spec
                    .l2_contract
                    .expect("shasta chain should define l2 contract"),
            ),
            value: U256::ZERO,
            access_list: Default::default(),
            input: anchorV4Call {
                _checkpoint: checkpoint,
            }
            .abi_encode()
            .into(),
        },
        GOLDEN_TOUCH_PRIVATE_KEY,
    );
    guest_input.witnesses[0].block.body.transactions = vec![anchor_tx.clone()];
    guest_input.witnesses[0].block.header.base_fee_per_gas = Some(valid_base_fee);

    let invalid_tx: reth_ethereum_primitives::TransactionSigned = sign_tx(
        TxEip1559 {
            chain_id: chain_spec.chain_id,
            nonce: 1,
            gas_limit: 21_000,
            max_fee_per_gas: u128::from(valid_base_fee),
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::repeat_byte(0x55)),
            value: U256::ZERO,
            access_list: Default::default(),
            input: Bytes::new(),
        },
        INVALID_TX_PRIVATE_KEY,
    );
    let invalid_signer = invalid_tx
        .clone()
        .try_into_recovered()
        .expect("recover invalid tx signer")
        .signer();

    let parent_header = alloy_consensus::Header {
        number: 0,
        timestamp: 1_775_135_700u64,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(valid_base_fee),
        state_root: B256::ZERO,
        ..Default::default()
    };

    let mut trie = Trie::default();
    trie.insert(
        keccak256(anchor_signer),
        alloy_rlp::encode(TrieAccount {
            nonce: 0,
            balance: funded_balance,
            storage_root: EMPTY_ROOT_HASH,
            code_hash: KECCAK_EMPTY,
        }),
    );
    trie.insert(
        keccak256(invalid_signer),
        alloy_rlp::encode(TrieAccount {
            nonce: 0,
            balance: funded_balance,
            storage_root: EMPTY_ROOT_HASH,
            code_hash: KECCAK_EMPTY,
        }),
    );

    let witness_parent_header = alloy_consensus::Header {
        state_root: trie.hash_slow(),
        ..parent_header
    };
    guest_input.witnesses[0].witness.headers = vec![raiko2_primitives::WitnessHeader::from_header(
        witness_parent_header.clone(),
    )];
    guest_input.witnesses[0].witness.state = trie
        .rlp_nodes()
        .into_iter()
        .map(WitnessStateNode::from_bytes)
        .collect();
    guest_input.witnesses[0].accounts.insert(
        anchor_signer,
        TrieAccount {
            nonce: 0,
            balance: funded_balance,
            storage_root: EMPTY_ROOT_HASH,
            code_hash: KECCAK_EMPTY,
        },
    );
    guest_input.witnesses[0].accounts.insert(
        invalid_signer,
        TrieAccount {
            nonce: 0,
            balance: funded_balance,
            storage_root: EMPTY_ROOT_HASH,
            code_hash: KECCAK_EMPTY,
        },
    );

    let outcome = reconstruct_block_from_transactions_with_witness_resources(
        Some(
            anchor_tx
                .clone()
                .try_into_recovered()
                .expect("recover anchor signer"),
        ),
        vec![invalid_tx.clone()],
        TaikoNextBlockEnvAttributes {
            timestamp: guest_input.witnesses[0].block.header.timestamp,
            suggested_fee_recipient: guest_input.witnesses[0].block.header.beneficiary,
            prev_randao: guest_input.witnesses[0].block.header.mix_hash,
            gas_limit: guest_input.witnesses[0].block.header.gas_limit,
            extra_data: guest_input.witnesses[0].block.header.extra_data.clone(),
            base_fee_per_gas: valid_base_fee,
        },
        &guest_input.witnesses[0].witness,
        &guest_input.witnesses[0].witness.headers,
        &[],
        guest_input.witnesses[0].accounts.clone(),
        &taiko_chain_spec,
        &evm_config,
    )
    .expect("candidate block reconstruction should succeed");

    guest_input.witnesses[0].block = outcome.block.into_block();
    guest_input.witnesses[0].block.header.parent_hash = witness_parent_header.hash_slow();
    guest_input.taiko.data_sources = vec![InputDataSource {
        tx_data_from_calldata: DerivationSourceManifest {
            blocks: vec![BlockManifest {
                timestamp: guest_input.witnesses[0].block.header.timestamp,
                coinbase: guest_input.taiko.proposal_event.proposal.proposer,
                anchor_block_number: 7,
                gas_limit: 30_000_000,
                transactions: vec![invalid_tx.into()],
            }],
        }
        .encode_and_compress()
        .expect("manifest payload"),
        ..Default::default()
    }];
    guest_input
        .proof_carry_data
        .transition_input
        .parent_block_hash = witness_parent_header.hash_slow();
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .blockHash = guest_input.witnesses[0].block.header.hash_slow();
    guest_input
        .proof_carry_data
        .transition_input
        .checkpoint
        .stateRoot = guest_input.witnesses[0].block.header.state_root;

    guest_input
}

fn assert_rejected_with_message(guest_input: &GuestInput, expected: &str) {
    let err = prove_shasta_proposal_with_validator(
        guest_input,
        |stateless_input, _ancestor_headers, _runtime| Ok(stateless_input.block.header.hash_slow()),
    )
    .expect_err("guest input should fail validation");
    let messages = err.chain().map(ToString::to_string).collect::<Vec<_>>();
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "expected error containing '{expected}', got chain: {messages:?}",
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
fn accepts_canonical_inline_source_derivation() {
    let guest_input = canonical_inline_source_guest_input();

    prove_shasta_proposal_with_validator(
        &guest_input,
        |stateless_input, _ancestor_headers, _runtime| Ok(stateless_input.block.header.hash_slow()),
    )
    .expect("inline source derivation should validate");
}

#[test]
fn top_level_proposal_proof_reconstructs_block_and_skips_invalid_tx() {
    let guest_input = one_pass_guest_input_with_skipped_invalid_tx();

    prove_shasta_proposal(&guest_input)
        .expect("one-pass proposal proving should reconstruct the canonical block");
}

#[test]
fn rejects_inline_source_transaction_count_mismatch() {
    let mut guest_input = canonical_inline_source_guest_input();
    guest_input.witnesses[0].block.body.transactions.push(
        TxEip1559 {
            chain_id: 167_000,
            nonce: 1,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::repeat_byte(0x55)),
            value: U256::ZERO,
            access_list: Default::default(),
            input: Default::default(),
        }
        .into_signed(Signature::test_signature())
        .into(),
    );

    assert_rejected_with_message(
        &guest_input,
        "canonical Shasta derivation mismatch at index 0",
    );
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

    let err = prove_shasta_proposal_with_validator(
        &guest_input,
        |_stateless_input, _ancestor_headers, _runtime| Err(anyhow::anyhow!("boom")),
    )
    .expect_err("validator failure should bubble up");

    let messages = err.chain().map(ToString::to_string).collect::<Vec<_>>();
    assert!(messages
        .iter()
        .any(|message| message.contains("stateless block validation failed at index 0")));
    assert!(messages.iter().any(|message| message.contains("boom")));
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
