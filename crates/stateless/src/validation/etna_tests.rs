//! Synthetic, offline fixtures with genuine EVM execution and Merkle Patricia state.
//!
//! Assembly below is fixture preparation only. Every assertion calls the supplied-block
//! stateless verifier, which must execute all original transactions without filtering.
use super::*;
use alethia_reth_block::{config::TaikoNextBlockEnvAttributes, executor::TaikoBlockExecutor};
use alethia_reth_chainspec::{TAIKO_DEVNET, spec::TaikoDevnetConfigExt};
use alloy_consensus::{SignableTransaction, TrieAccount, TxEip1559, constants::KECCAK_EMPTY};
use alloy_eips::{
    eip2935::{HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE},
    eip4788::{BEACON_ROOTS_ADDRESS, BEACON_ROOTS_CODE},
};
use alloy_primitives::{Bytes, TxKind, keccak256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use reth_evm::block::BlockExecutor;
use reth_revm::{State, db::states::bundle_state::BundleRetention};
use risc0_ethereum_trie::Trie;

const ACTIVATION: u64 = 100;
const TARGET: Address = Address::with_last_byte(0x22);
const STAKING: Address = alloy_primitives::address!("1670010000000000000000000000000000010003");

fn spec() -> Arc<TaikoChainSpec> {
    Arc::new(
        TAIKO_DEVNET
            .clone_with_devnet_etna_timestamp(ACTIVATION)
            .unwrap(),
    )
}

fn transaction(nonce: u64, gas_limit: u64) -> TransactionSigned {
    let tx = TxEip1559 {
        chain_id: 167_001,
        nonce,
        gas_limit,
        max_fee_per_gas: 25_000_000,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(TARGET),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::new(),
    };
    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
    let signature = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
    tx.into_signed(signature).into()
}

fn extra_data() -> Bytes {
    // generation=2, anchor=17, miss bitmap bits 0 and 127 set. The EVM reads this
    // exact committed field; this test makes no claim about QC bitmap validity.
    let mut extra = vec![0; 29];
    extra[0] = 100;
    extra[6] = 2;
    extra[12] = 17;
    extra[13] = 0x80;
    extra[28] = 1;
    extra.into()
}

fn fixture(transactions: Vec<TransactionSigned>, target_code: Bytes) -> (Block, ExecutionWitness) {
    let target_succeeds = target_code.as_ref() == [0];
    let chain_spec = spec();
    let config = TaikoEvmConfig::new(chain_spec);
    let mut trie = Trie::default();
    let mut codes = vec![
        target_code,
        HISTORY_STORAGE_CODE.clone(),
        BEACON_ROOTS_CODE.clone(),
    ];
    // Record generation and bitmap from onBlockStart's actual calldata, using
    // real SSTORE operations. This also forces the witness to cover system calls.
    codes.push(Bytes::from_static(&[
        0x60, 4, 0x35, 0x60, 0, 0x55, 0x60, 36, 0x35, 0x60, 1, 0x55, 0,
    ]));
    for (address, code) in [
        TARGET,
        HISTORY_STORAGE_ADDRESS,
        BEACON_ROOTS_ADDRESS,
        STAKING,
    ]
    .into_iter()
    .zip(&codes)
    {
        trie.insert(
            keccak256(address),
            alloy_rlp::encode(TrieAccount {
                nonce: 1,
                balance: U256::ZERO,
                storage_root: alloy_trie::EMPTY_ROOT_HASH,
                code_hash: keccak256(code),
            }),
        );
    }
    for tx in &transactions {
        let signer = tx.recover_signer().unwrap();
        trie.insert(
            keccak256(signer),
            alloy_rlp::encode(TrieAccount {
                nonce: 0,
                balance: U256::from(1_000_000_000_000_000_000u64),
                storage_root: alloy_trie::EMPTY_ROOT_HASH,
                code_hash: KECCAK_EMPTY,
            }),
        );
    }
    let parent = Header {
        number: 0,
        timestamp: ACTIVATION - 1,
        gas_limit: 30_000_000,
        base_fee_per_gas: Some(25_000_000),
        state_root: trie.hash_slow(),
        ..Default::default()
    };
    let witness = ExecutionWitness {
        state: trie
            .rlp_nodes()
            .into_iter()
            .map(WitnessStateNode::from_bytes)
            .collect(),
        codes,
        headers: vec![WitnessHeader::from_header(parent.clone())],
        ..Default::default()
    };
    let attributes = TaikoNextBlockEnvAttributes {
        timestamp: ACTIVATION,
        suggested_fee_recipient: Address::with_last_byte(0x44),
        prev_randao: B256::ZERO,
        gas_limit: 30_000_000,
        extra_data: extra_data(),
        base_fee_per_gas: 25_000_000,
        parent_beacon_block_root: Some(B256::repeat_byte(0x55)),
    };
    let sealed_parent = SealedHeader::seal_slow(parent);
    let recovered = transactions
        .into_iter()
        .map(|tx| tx.try_into_recovered().unwrap())
        .collect::<Vec<_>>();
    let (mut sparse, bytecode) =
        SparseState::new_with_state_pool(&witness, &[], sealed_parent.state_root).unwrap();
    let db = WitnessDatabase::new(
        &sparse,
        bytecode,
        AncestorHashes::new(0, vec![sealed_parent.hash()]),
    );
    let mut state = State::builder()
        .with_database(db)
        .with_bundle_update()
        .build();
    let env = config.next_evm_env(&sealed_parent, &attributes).unwrap();
    let ctx = config
        .context_for_next_block(&sealed_parent, attributes.clone())
        .unwrap();
    let finalized = ctx.finalized_block_zk_gas.clone();
    let evm = config.evm_with_env(&mut state, env);
    let output = TaikoBlockExecutor::new(
        evm,
        ctx,
        config.executor_factory.spec().clone(),
        config.executor_factory.receipt_builder(),
    )
    .execute_block(recovered.iter().map(|tx| tx.as_recovered_ref()))
    .unwrap();
    assert_eq!(
        output.receipts.len(),
        recovered.len(),
        "fixture preparation must never filter"
    );
    if let Some(receipt) = output.receipts.first() {
        assert_eq!(
            receipt.status(),
            target_succeeds,
            "fixture must exercise its stated EVM status"
        );
    }
    state.merge_transitions(BundleRetention::Reverts);
    let bundle = state.take_bundle();
    drop(state);
    let post = HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle.state());
    let state_root = sparse.calculate_state_root(post).unwrap();
    let template = RecoveredBlock::new_unhashed(
        Block {
            header: Header {
                number: 1,
                timestamp: attributes.timestamp,
                beneficiary: attributes.suggested_fee_recipient,
                gas_limit: attributes.gas_limit,
                extra_data: attributes.extra_data,
                base_fee_per_gas: Some(attributes.base_fee_per_gas),
                parent_beacon_block_root: attributes.parent_beacon_block_root,
                ..Default::default()
            },
            body: BlockBody::default(),
        },
        vec![],
    );
    let assembled = assemble_filtered_block(
        &config,
        &sealed_parent,
        &template,
        recovered,
        &output,
        finalized.load(std::sync::atomic::Ordering::Relaxed),
        state_root,
    )
    .unwrap();
    (
        Block {
            header: assembled.header().clone(),
            body: assembled.body().clone(),
        },
        witness,
    )
}

fn validate(block: Block, witness: &ExecutionWitness) -> Result<B256, StatelessValidationError> {
    let chain_spec = spec();
    validate_block(
        block,
        witness,
        &chain_spec,
        &TaikoEvmConfig::new(chain_spec.clone()),
    )
}

#[test]
fn etna_strict_replay_accepts_empty_block_with_system_state_changes() {
    let (block, witness) = fixture(vec![], Bytes::from_static(&[0]));
    assert_ne!(
        block.header.state_root,
        witness
            .headers
            .last()
            .unwrap()
            .full_header()
            .unwrap()
            .state_root
    );
    let expected = block.header.hash_slow();
    assert_eq!(validate(block, &witness).unwrap(), expected);
}

#[test]
fn etna_strict_replay_accepts_full_block_and_first_revert_or_out_of_gas() {
    for code in [
        Bytes::from_static(&[0]),
        Bytes::from_static(&[0x60, 0, 0x60, 0, 0xfd]),
        Bytes::from_static(&[0x5b, 0x60, 0, 0x56]),
    ] {
        let (block, witness) = fixture(vec![transaction(0, 40_000), transaction(1, 40_000)], code);
        assert_eq!(block.body.transactions.len(), 2);
        assert_eq!(
            validate(block.clone(), &witness).unwrap(),
            block.header.hash_slow()
        );
    }
}

#[test]
fn etna_strict_replay_rejects_invalid_interior_transaction_without_filtering() {
    let (mut block, witness) = fixture(
        vec![
            transaction(0, 40_000),
            transaction(1, 40_000),
            transaction(2, 40_000),
        ],
        Bytes::from_static(&[0]),
    );
    block.body.transactions[1] = transaction(99, 40_000);
    block.header.transactions_root = proofs::calculate_transaction_root(&block.body.transactions);
    let error = validate(block, &witness).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("nonce"),
        "{error:?}"
    );
}

#[test]
fn etna_strict_replay_rejects_wrong_post_state_receipts_body_and_parent() {
    let (block, witness) = fixture(vec![transaction(0, 40_000)], Bytes::from_static(&[0]));
    let mut wrong = block.clone();
    wrong.header.state_root = B256::repeat_byte(0x91);
    assert!(matches!(
        validate(wrong, &witness),
        Err(StatelessValidationError::PostStateRootMismatch { .. })
    ));
    let mut wrong = block.clone();
    wrong.header.receipts_root = B256::repeat_byte(0x92);
    assert!(validate(wrong, &witness).is_err());
    let mut wrong = block.clone();
    wrong.body.transactions.clear();
    assert!(validate(wrong, &witness).is_err());
    let mut wrong = block;
    wrong.header.parent_hash = B256::repeat_byte(0x93);
    assert!(validate(wrong, &witness).is_err());
}

#[test]
fn etna_strict_replay_requires_nonzero_root_at_activation_and_retains_unzen_rule() {
    let (block, witness) = fixture(vec![], Bytes::from_static(&[0]));
    for root in [None, Some(B256::ZERO)] {
        let mut wrong = block.clone();
        wrong.header.parent_beacon_block_root = root;
        assert!(validate(wrong, &witness).is_err());
    }
    let chain_spec = spec();
    for timestamp in [ACTIVATION - 1, ACTIVATION] {
        let mut candidate = block.clone();
        candidate.header.timestamp = timestamp;
        let error = validate_block(
            candidate,
            &ExecutionWitness::default(),
            &chain_spec,
            &TaikoEvmConfig::new(chain_spec.clone()),
        )
        .unwrap_err();
        if timestamp < ACTIVATION {
            assert!(
                error
                    .to_string()
                    .contains("non-zero parent_beacon_block_root is unsupported"),
                "{error:?}"
            );
        } else {
            assert!(
                matches!(error, StatelessValidationError::MissingAncestorHeader),
                "{error:?}"
            );
        }
    }
}

// Additional committed-input mutation regressions.
// Independent behavioral cases. Append inside validation/etna_tests.rs after its existing tests.
// These use its synthetic, offline genuine-EVM/MPT fixture; they do not certify real chain inputs.

#[test]
fn independent_committed_generation_and_bitmap_drive_staking_state() {
    let (block, witness) = fixture(vec![], Bytes::from_static(&[0]));
    // Fixture staking bytecode stores generation and bitmap. Changing either committed
    // input while retaining the original post-state must be detected by execution.
    for offset in [6, 13, 28] {
        let mut changed = block.clone();
        let mut extra = changed.header.extra_data.to_vec();
        extra[offset] ^= 1;
        changed.header.extra_data = extra.into();
        assert!(
            matches!(
                validate(changed, &witness),
                Err(StatelessValidationError::PostStateRootMismatch { .. })
            ),
            "committed extraData byte {offset} must alter staking execution state"
        );
    }
}

#[test]
fn independent_committed_beacon_root_drives_system_state() {
    let (mut block, witness) = fixture(vec![], Bytes::from_static(&[0]));
    block.header.parent_beacon_block_root = Some(B256::repeat_byte(0x56));
    assert!(
        matches!(
            validate(block, &witness),
            Err(StatelessValidationError::PostStateRootMismatch { .. })
        ),
        "a different nonzero root must be consumed by EIP-4788"
    );
}

#[test]
fn independent_missing_system_bytecode_is_not_silently_treated_as_empty() {
    let (block, mut witness) = fixture(vec![], Bytes::from_static(&[0]));
    // Every system contract has a committed nonempty code hash in the parent trie.
    witness.codes.clear();
    assert!(
        validate(block, &witness).is_err(),
        "missing committed system code must fail witness-backed execution"
    );
}

#[test]
fn independent_unrecoverable_original_transaction_fails_whole_block() {
    let (mut block, witness) = fixture(vec![transaction(0, 40_000)], Bytes::from_static(&[0]));
    let invalid: TransactionSigned = TxEip1559 {
        chain_id: 167_001,
        nonce: 0,
        gas_limit: 40_000,
        max_fee_per_gas: 25_000_000,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(TARGET),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::new(),
    }
    .into_signed(alloy_primitives::Signature::new(
        U256::ZERO,
        U256::ZERO,
        false,
    ))
    .into();
    assert!(invalid.recover_signer().is_err());
    block.body.transactions[0] = invalid;
    block.header.transactions_root = proofs::calculate_transaction_root(&block.body.transactions);
    assert!(
        matches!(
            validate(block, &witness),
            Err(StatelessValidationError::SignerRecovery)
        ),
        "invalid original signature must fail rather than disappear"
    );
}

#[test]
fn independent_authenticated_parent_state_cannot_be_replaced_by_empty_state() {
    let (block, mut witness) = fixture(vec![transaction(0, 40_000)], Bytes::from_static(&[0]));
    witness.state.clear();
    assert!(
        validate(block, &witness).is_err(),
        "missing committed parent-state nodes must fail rather than synthesize accounts"
    );
}

#[test]
fn independent_validly_signed_underpriced_transaction_is_not_filtered() {
    let (mut block, witness) = fixture(vec![transaction(0, 40_000)], Bytes::from_static(&[0]));
    // The signature is recoverable, but the transaction cannot pay the block's base fee.
    let unsigned = TxEip1559 {
        chain_id: 167_001,
        nonce: 0,
        gas_limit: 40_000,
        max_fee_per_gas: 1,
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(TARGET),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::new(),
    };
    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11)).unwrap();
    let signature = signer.sign_hash_sync(&unsigned.signature_hash()).unwrap();
    let invalid: TransactionSigned = unsigned.into_signed(signature).into();
    assert_eq!(
        invalid.recover_signer().unwrap(),
        block.body.transactions[0].recover_signer().unwrap()
    );
    block.body.transactions[0] = invalid;
    block.header.transactions_root = proofs::calculate_transaction_root(&block.body.transactions);
    assert!(
        validate(block, &witness).is_err(),
        "a listed invalid transaction must fail rather than produce an empty block"
    );
}
