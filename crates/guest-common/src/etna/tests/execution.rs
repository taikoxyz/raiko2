//! Offline end-to-end composition fixtures. Blocks use genuine EVM execution and MPT roots;
//! Comet/DA/L1-history context is synthetic and does not authenticate a real committee or QC.
use super::*;
use crate::etna::{verify_blob_range_statement_execution, EtnaBlockExecutionWitness};
use alethia_reth_block::{
    config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes},
    derived_block::assemble_filtered_block,
    executor::TaikoBlockExecutor,
};
use alethia_reth_chainspec::{spec::TaikoDevnetConfigExt, TAIKO_DEVNET};
use alethia_reth_consensus::eip4396::{calculate_next_block_eip4396_base_fee, MIN_BASE_FEE};
use alloy_consensus::{SignableTransaction, TrieAccount, TxEip1559};
use alloy_eips_execution::{
    eip2718::Encodable2718,
    eip2935::{HISTORY_STORAGE_ADDRESS, HISTORY_STORAGE_CODE},
    eip4788::{BEACON_ROOTS_ADDRESS, BEACON_ROOTS_CODE},
};
use alloy_primitives::{keccak256, Bytes, TxKind};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use raiko2_primitives::{ExecutionWitness, WitnessHeader, WitnessStateNode};
use reth_ethereum_primitives::{Block, BlockBody};
use reth_evm::{block::BlockExecutor, ConfigureEvm};
use reth_primitives_traits::{RecoveredBlock, SealedHeader, SignerRecoverable};
use reth_revm::{
    db::{states::bundle_state::BundleRetention, InMemoryDB},
    state::{AccountInfo, Bytecode},
    State,
};
use risc0_ethereum_trie::Trie;
use std::{collections::BTreeMap, sync::Arc};

const TARGET: Address = Address::with_last_byte(0x22);
const STAKING: Address = alloy_primitives::address!("1670010000000000000000000000000000010003");

fn config() -> TaikoEvmConfig {
    TaikoEvmConfig::new(Arc::new(
        TAIKO_DEVNET.clone_with_devnet_etna_timestamp(100).unwrap(),
    ))
}

fn transaction(nonce: u64) -> TransactionSigned {
    let tx = TxEip1559 {
        chain_id: 167_001,
        nonce,
        gas_limit: 40_000,
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

struct ExecutingRange {
    authenticated: AuthenticatedRangeFixture,
    witnesses: Vec<ExecutionWitness>,
}

impl ExecutingRange {
    fn new(count: usize, with_transactions: bool) -> Self {
        let config = config();
        let mut range = RangeFixture::new(count);
        let mut db = InMemoryDB::default();
        let mut state_trie = Trie::default();
        let mut storage_tries = BTreeMap::<Address, Trie>::new();
        let codes = vec![
            // Ordinary first transaction reverts and is nevertheless a valid listed transaction.
            Bytes::from_static(&[0x60, 0, 0x60, 0, 0xfd]),
            HISTORY_STORAGE_CODE.clone(),
            BEACON_ROOTS_CODE.clone(),
            Bytes::from_static(&[
                0x60, 4, 0x35, 0x60, 0, 0x55, 0x60, 36, 0x35, 0x60, 1, 0x55, 0,
            ]),
        ];
        for (address, code) in [
            TARGET,
            HISTORY_STORAGE_ADDRESS,
            BEACON_ROOTS_ADDRESS,
            STAKING,
        ]
        .into_iter()
        .zip(&codes)
        {
            let info = AccountInfo {
                nonce: 1,
                balance: U256::ZERO,
                code_hash: keccak256(code),
                code: Some(Bytecode::new_raw(code.clone())),
                ..Default::default()
            };
            state_trie.insert(
                keccak256(address),
                alloy_rlp::encode(TrieAccount {
                    nonce: info.nonce,
                    balance: info.balance,
                    storage_root: alloy_trie::EMPTY_ROOT_HASH,
                    code_hash: info.code_hash,
                }),
            );
            db.insert_account_info(address, info);
        }
        let signer = transaction(0).recover_signer().unwrap();
        let info = AccountInfo {
            balance: U256::from(1_000_000_000_000_000_000u64),
            ..Default::default()
        };
        db.insert_account_info(signer, info.clone());
        state_trie.insert(
            keccak256(signer),
            alloy_rlp::encode(TrieAccount {
                nonce: 0,
                balance: info.balance,
                storage_root: alloy_trie::EMPTY_ROOT_HASH,
                code_hash: info.code_hash,
            }),
        );
        range.parent = Header {
            number: 0,
            timestamp: 99,
            gas_limit: 30_000_000,
            base_fee_per_gas: Some(25_000_000),
            state_root: state_trie.hash_slow(),
            ..Default::default()
        };
        let mut headers = vec![WitnessHeader::from_header(range.parent.clone())];
        let mut witnesses = Vec::new();
        for (index, fixture) in range.blocks.iter_mut().enumerate() {
            let parent = headers.last().unwrap().full_header().unwrap().clone();
            let mut nodes = state_trie.rlp_nodes();
            for storage in storage_tries.values() {
                nodes.extend(storage.rlp_nodes());
            }
            witnesses.push(ExecutionWitness {
                state: nodes
                    .into_iter()
                    .map(WitnessStateNode::from_bytes)
                    .collect(),
                codes: codes.clone(),
                headers: headers.clone(),
                ..Default::default()
            });
            let parent_block_time = if parent.number == 0 {
                1
            } else {
                parent.timestamp - headers[headers.len() - 2].timestamp
            };
            let base_fee = if parent.number == 0 {
                25_000_000
            } else {
                calculate_next_block_eip4396_base_fee(
                    &parent,
                    parent_block_time,
                    4,
                    parent.base_fee_per_gas.unwrap(),
                    MIN_BASE_FEE,
                )
            };
            let attrs = TaikoNextBlockEnvAttributes {
                timestamp: 100 + u64::try_from(index).unwrap() * 4,
                suggested_fee_recipient: Address::with_last_byte(0x44),
                prev_randao: B256::ZERO,
                gas_limit: 30_000_000,
                extra_data: fixture.execution.extra_data.clone(),
                base_fee_per_gas: base_fee,
                parent_beacon_block_root: fixture.execution.parent_beacon_block_root,
            };
            let transactions = if with_transactions {
                (0..3)
                    .map(|offset| transaction(u64::try_from(index).unwrap() * 3 + offset))
                    .collect::<Vec<_>>()
            } else {
                vec![]
            };
            let recovered = transactions
                .iter()
                .cloned()
                .map(|tx| tx.try_into_recovered().unwrap())
                .collect::<Vec<_>>();
            let sealed = SealedHeader::seal_slow(parent);
            let mut state = State::builder()
                .with_database(&mut db)
                .with_bundle_update()
                .build();
            let env = config.next_evm_env(&sealed, &attrs).unwrap();
            let ctx = config
                .context_for_next_block(&sealed, attrs.clone())
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
            assert_eq!(output.receipts.len(), transactions.len());
            if let Some(receipt) = output.receipts.first() {
                assert!(!alloy_consensus::TxReceipt::status(receipt));
            }
            state.merge_transitions(BundleRetention::Reverts);
            let bundle = state.take_bundle();
            drop(state);
            // Independently apply real EVM state changes to complete test tries. Production
            // verification still uses the existing sparse witnessed trie and its own mutation.
            for (address, account) in bundle.state() {
                let Some(info) = &account.info else {
                    state_trie.remove(keccak256(address));
                    continue;
                };
                assert!(!account.was_destroyed(), "fixture has no destruction");
                let storage = storage_tries.entry(*address).or_default();
                for (slot, value) in &account.storage {
                    let key = keccak256(B256::from(*slot));
                    if value.present_value.is_zero() {
                        storage.remove(key);
                    } else {
                        storage.insert(key, alloy_rlp::encode(value.present_value));
                    }
                    db.insert_account_storage(*address, *slot, value.present_value)
                        .unwrap();
                }
                db.insert_account_info(*address, info.clone());
                state_trie.insert(
                    keccak256(address),
                    alloy_rlp::encode(TrieAccount {
                        nonce: info.nonce,
                        balance: info.balance,
                        code_hash: info.code_hash,
                        storage_root: storage.hash_slow(),
                    }),
                );
            }
            let template = RecoveredBlock::new_unhashed(
                Block {
                    header: Header {
                        number: sealed.number + 1,
                        timestamp: attrs.timestamp,
                        beneficiary: attrs.suggested_fee_recipient,
                        gas_limit: attrs.gas_limit,
                        extra_data: attrs.extra_data,
                        base_fee_per_gas: Some(attrs.base_fee_per_gas),
                        parent_beacon_block_root: attrs.parent_beacon_block_root,
                        ..Default::default()
                    },
                    body: BlockBody::default(),
                },
                vec![],
            );
            // This is assembly only: the strict execute_block above committed the complete list.
            let block = assemble_filtered_block(
                &config,
                &sealed,
                &template,
                recovered,
                &output,
                finalized.load(std::sync::atomic::Ordering::Relaxed),
                state_trie.hash_slow(),
            )
            .unwrap();
            fixture.execution = block.header().clone();
            fixture.transactions = transactions;
            let encoded: Vec<_> = fixture
                .transactions
                .iter()
                .map(|tx| {
                    alloy_rlp::encode(
                        alloy_eips_execution::eip2718::Encodable2718::encoded_2718(tx).as_slice(),
                    )
                })
                .collect();
            let encoded_refs: Vec<_> = encoded.iter().map(Vec::as_slice).collect();
            fixture.item = list(&[&alloy_rlp::encode(&fixture.execution), &list(&encoded_refs)]);
            fixture.comet.header.height = fixture.execution.number.try_into().unwrap();
            headers.push(WitnessHeader::from_header(fixture.execution.clone()));
        }
        range.context.origin_height = 0;
        range.context.parent_height = 0;
        range.context.last_height = u64::try_from(count).unwrap();
        range.context.parent_el_hash = range.parent.hash_slow();
        range.context.last_el_hash = range.blocks.last().unwrap().execution.hash_slow();
        range.reseal();
        let (source, hashes) = prove_blobs(pack(&v6_payload(
            &range.decoded_payload(),
            Compression::default(),
        )));
        Self {
            authenticated: AuthenticatedRangeFixture {
                range,
                source,
                hashes,
            },
            witnesses,
        }
    }

    fn rebind(&mut self) {
        let range = &mut self.authenticated.range;
        for block in &mut range.blocks {
            let encoded: Vec<_> = block
                .transactions
                .iter()
                .map(|tx| {
                    alloy_rlp::encode(
                        alloy_eips_execution::eip2718::Encodable2718::encoded_2718(tx).as_slice(),
                    )
                })
                .collect();
            let encoded_refs: Vec<_> = encoded.iter().map(Vec::as_slice).collect();
            block.item = list(&[&alloy_rlp::encode(&block.execution), &list(&encoded_refs)]);
        }
        rebuild_execution_items(range);
        let (source, hashes) = prove_blobs(pack(&v6_payload(
            &range.decoded_payload(),
            Compression::default(),
        )));
        self.authenticated.source = source;
        self.authenticated.hashes = hashes;
    }

    fn verify(&self) -> anyhow::Result<B256> {
        self.verify_with_config(&config())
    }

    fn verify_with_config(&self, execution_config: &TaikoEvmConfig) -> anyhow::Result<B256> {
        let fixture = &self.authenticated;
        let range = &fixture.range;
        let leaves = range.cut_leaves();
        let statement = range.statement(&fixture.hashes, &leaves);
        let cursors = vec![0; range.blocks.len()];
        let witnesses: Vec<_> = self
            .witnesses
            .iter()
            .map(|witness| EtnaBlockExecutionWitness {
                witness,
                ancestor_headers: &witness.headers,
            })
            .collect();
        verify_blob_range_statement_execution(
            &fixture.source,
            &range.inputs(),
            &range.context,
            &statement,
            &cursors,
            &range.parent,
            range.l1_proof(&range.openings(&range.l1_headers)),
            &witnesses,
            &[],
            execution_config,
        )
    }
}

#[test]
fn native_etna_executes_authenticated_empty_and_full_two_block_ranges() {
    for count in [1, 2] {
        for with_transactions in [false, true] {
            let fixture = ExecutingRange::new(count, with_transactions);
            let range = &fixture.authenticated.range;
            let leaves = range.cut_leaves();
            let statement = range.statement(&fixture.authenticated.hashes, &leaves);
            assert_eq!(fixture.verify().unwrap(), statement.try_digest().unwrap());
        }
    }
}

#[test]
fn native_etna_rejects_an_alternate_first_or_interior_witness_parent() {
    for index in [0, 1] {
        let mut fixture = ExecutingRange::new(2, true);
        let witness_parent = fixture.witnesses[index].headers.last_mut().unwrap();
        let original_hash = witness_parent.hash;
        let mut header = witness_parent.full_header().unwrap().clone();
        header.state_root = B256::repeat_byte(0xf1);
        *witness_parent = WitnessHeader::from_header_with_hash(header, original_hash);
        let error = fixture.verify().unwrap_err();
        assert!(error.to_string().contains("witness parent"), "{error:?}");
    }
}

#[test]
fn native_etna_rejects_missing_witnesses_and_bad_pre_state() {
    let mut fixture = ExecutingRange::new(2, false);
    fixture.witnesses.pop();
    assert!(fixture
        .verify()
        .unwrap_err()
        .to_string()
        .contains("witness count"));
    let mut fixture = ExecutingRange::new(1, true);
    fixture.witnesses[0].state.clear();
    assert!(fixture.verify().is_err());
}

#[test]
fn native_etna_derives_transaction_identity_from_committed_bytes() {
    use alloy_consensus::Signed;
    let mut fixture = ExecutingRange::new(1, true);
    let tx = fixture.authenticated.range.blocks[0].transactions[0].clone();
    let hash = B256::repeat_byte(0xab);
    let TransactionSigned::Eip1559(signed) = tx else {
        panic!("expected EIP1559 fixture");
    };
    fixture.authenticated.range.blocks[0].transactions[0] = TransactionSigned::Eip1559(
        Signed::new_unchecked(signed.tx().clone(), *signed.signature(), hash),
    );
    assert!(
        fixture.verify().is_ok(),
        "typed transaction cached hash must not determine execution identity"
    );
}

#[test]
fn native_etna_rejects_committed_invalid_interior_transaction_without_filtering() {
    let mut fixture = ExecutingRange::new(1, true);
    let block = &mut fixture.authenticated.range.blocks[0];
    block.transactions[1] = transaction(99);
    block.execution.transactions_root = calculate_transaction_root(&block.transactions);
    fixture.rebind();
    let error = fixture.verify().unwrap_err();
    assert!(
        format!("{error:#}").to_lowercase().contains("nonce"),
        "{error:?}"
    );
}

#[test]
fn native_etna_rejects_da_bound_wrong_post_state_and_body_commitments() {
    for mutation in ["state", "transactions", "receipts", "withdrawals"] {
        let mut fixture = ExecutingRange::new(1, true);
        let header = &mut fixture.authenticated.range.blocks[0].execution;
        match mutation {
            "state" => header.state_root = B256::repeat_byte(0xcc),
            "transactions" => header.transactions_root = B256::repeat_byte(0xcc),
            "receipts" => header.receipts_root = B256::repeat_byte(0xcc),
            "withdrawals" => header.withdrawals_root = Some(B256::repeat_byte(0xcc)),
            _ => unreachable!(),
        }
        fixture.rebind();
        // Coherently rebind header, envelope, DA, statement and Comet identity. Projection
        // succeeds; only complete execution/body checking may prevent the digest.
        let range = &fixture.authenticated.range;
        let leaves = range.cut_leaves();
        let statement = range.statement(&fixture.authenticated.hashes, &leaves);
        fixture
            .authenticated
            .verify_statement(&statement, &[0])
            .unwrap();
        assert!(fixture.verify().is_err(), "{mutation}");
    }
}

#[test]
fn native_etna_uses_only_the_trusted_fork_schedule() {
    let fixture = ExecutingRange::new(1, false);
    let legacy = TaikoEvmConfig::new(TAIKO_DEVNET.clone());
    assert!(fixture
        .verify_with_config(&legacy)
        .unwrap_err()
        .to_string()
        .contains("Etna is not active"));
}

#[test]
fn native_etna_recomputes_all_cached_ancestor_metadata() {
    let mut fixture = ExecutingRange::new(2, true);
    for witness in &mut fixture.witnesses {
        for header in &mut witness.headers {
            header.hash = B256::repeat_byte(0xee);
            header.number = u64::MAX;
            header.parent_hash = B256::repeat_byte(0xef);
            header.timestamp = 0;
        }
    }
    assert!(fixture.verify().is_ok());
}

#[test]
fn native_etna_recovers_signers_from_original_transaction_signatures() {
    use alloy_consensus::Signed;
    let mut fixture = ExecutingRange::new(1, true);
    let tx = fixture.authenticated.range.blocks[0].transactions[0].clone();
    let TransactionSigned::Eip1559(signed) = tx else {
        panic!("expected EIP1559 fixture");
    };
    fixture.authenticated.range.blocks[0].transactions[0] =
        TransactionSigned::Eip1559(Signed::new_unhashed(
            signed.tx().clone(),
            alloy_primitives::Signature::new(U256::ZERO, U256::ZERO, false),
        ));
    let block = &mut fixture.authenticated.range.blocks[0];
    block.execution.transactions_root = calculate_transaction_root(&block.transactions);
    fixture.rebind();
    let error = fixture.verify().unwrap_err();
    assert!(
        format!("{error:#}").to_lowercase().contains("signer"),
        "{error:?}"
    );
}

impl ExecutingRange {
    fn input(&self) -> raiko2_protocol_etna::input::RangeInput {
        use raiko2_protocol_etna::input::{
            L1HistoryInput, LandStatementInput, RangeBlockInput, RangeInput,
        };
        let range = &self.authenticated.range;
        let cuts = range.cut_leaves();
        let statement = range.statement(&self.authenticated.hashes, &cuts);
        RangeInput {
            blocks: range
                .blocks
                .iter()
                .map(|block| RangeBlockInput {
                    comet_header: block.comet.header.clone(),
                    comet_transactions: block.comet.data.clone(),
                    execution_header: block.execution.clone(),
                    execution_transactions: block
                        .transactions
                        .iter()
                        .map(Encodable2718::encoded_2718)
                        .collect(),
                })
                .collect(),
            statement: LandStatementInput::from(&statement),
            post_block_cursors: vec![0; range.blocks.len()],
            parent_execution_header: range.parent.clone(),
            l1_history: L1HistoryInput::from(range.l1_proof(&range.openings(&range.l1_headers))),
        }
    }

    fn verify_input(
        &self,
        input: &raiko2_protocol_etna::input::RangeInput,
    ) -> anyhow::Result<B256> {
        let resources: Vec<_> = self
            .witnesses
            .iter()
            .map(|witness| EtnaBlockExecutionWitness {
                witness,
                ancestor_headers: &witness.headers,
            })
            .collect();
        crate::etna::verify_range_input_execution(
            input,
            &self.authenticated.source,
            &self.authenticated.range.context,
            &resources,
            &[],
            L1HistoryLimits {
                max_headers: 131_072,
                max_proof_nodes: 128,
            },
            &config(),
        )
    }
}

#[test]
fn native_etna_owned_input_roundtrip_replays_original_range() {
    for (count, transactions) in [(1, false), (1, true), (2, true)] {
        let fixture = ExecutingRange::new(count, transactions);
        let input = fixture.input();
        let encoded = serde_json::to_vec(&input).unwrap();
        let decoded = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            fixture.verify_input(&decoded).unwrap(),
            fixture.verify().unwrap()
        );
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), encoded);
    }
}

#[test]
fn native_etna_owned_input_rejects_changed_original_and_interior_transactions() {
    let fixture = ExecutingRange::new(2, true);
    for block in 0..2 {
        let mut encoded = serde_json::to_value(fixture.input()).unwrap();
        encoded["blocks"][block]["execution_transactions"][0] =
            serde_json::to_value(transaction(99).encoded_2718()).unwrap();
        let decoded = serde_json::from_value(encoded).unwrap();
        assert!(fixture.verify_input(&decoded).is_err(), "block {block}");
    }
    for malformed in [vec![], vec![0x02], {
        let mut raw = transaction(0).encoded_2718();
        raw.push(0);
        raw
    }] {
        let mut input = fixture.input();
        input.blocks[0].execution_transactions[0] = malformed;
        let decoded = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(fixture.verify_input(&decoded).is_err());
    }
}

#[test]
fn native_etna_owned_input_rejects_raw_header_and_statement_reference_mutations() {
    let fixture = ExecutingRange::new(2, true);
    for mutation in [
        "reference",
        "anchor",
        "q_number",
        "q_hash",
        "comet_bytes",
        "execution_header",
    ] {
        let mut input = fixture.input();
        match mutation {
            "reference" => input.l1_history.reference_header[0] ^= 1,
            "anchor" => input.l1_history.anchors[0].raw_header[0] ^= 1,
            "q_number" => input.statement.input.l1_reference_block_number += U256::ONE,
            "q_hash" => input.statement.l1_reference_block_hash = B256::ZERO,
            "comet_bytes" => input.blocks[0].comet_transactions[0][0] ^= 1,
            "execution_header" => input.blocks[0].execution_header.state_root = B256::ZERO,
            _ => unreachable!(),
        }
        let decoded = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(fixture.verify_input(&decoded).is_err(), "{mutation}");
    }
}

#[test]
fn native_etna_owned_input_rejects_missing_extra_blocks_and_witnesses() {
    let mut fixture = ExecutingRange::new(2, true);
    for extra in [false, true] {
        let mut input = fixture.input();
        if extra {
            input.blocks.push(input.blocks[0].clone());
        } else {
            input.blocks.pop();
        }
        let decoded = serde_json::from_slice(&serde_json::to_vec(&input).unwrap()).unwrap();
        assert!(fixture.verify_input(&decoded).is_err());
    }
    let decoded = serde_json::from_slice(&serde_json::to_vec(&fixture.input()).unwrap()).unwrap();
    fixture.witnesses.push(fixture.witnesses[0].clone());
    assert!(fixture.verify_input(&decoded).is_err());
    fixture.witnesses.truncate(1);
    assert!(fixture.verify_input(&decoded).is_err());
}

#[test]
fn native_etna_owned_input_cannot_supply_authority_or_limits() {
    let fixture = ExecutingRange::new(1, false);
    for field in ["context", "limits", "config", "host_verified"] {
        let mut encoded = serde_json::to_value(fixture.input()).unwrap();
        encoded[field] = serde_json::json!({"max_headers": usize::MAX});
        assert!(
            serde_json::from_value::<raiko2_protocol_etna::input::RangeInput>(encoded).is_err(),
            "{field}"
        );
    }
    let mut encoded = serde_json::to_value(fixture.input()).unwrap();
    encoded["l1_history"]["limits"] = serde_json::json!({"max_headers": usize::MAX});
    assert!(serde_json::from_value::<raiko2_protocol_etna::input::RangeInput>(encoded).is_err());
}

#[test]
fn native_etna_owned_input_preflights_counts_and_transaction_byte_budget() {
    let fixture = ExecutingRange::new(1, true);
    let input = fixture.input();
    let resources: Vec<_> = fixture
        .witnesses
        .iter()
        .map(|witness| EtnaBlockExecutionWitness {
            witness,
            ancestor_headers: &witness.headers,
        })
        .collect();
    let mut context = fixture.authenticated.range.context;
    context.max_envelope_bytes = 0;
    let error = crate::etna::verify_range_input_execution(
        &input,
        &fixture.authenticated.source,
        &context,
        &resources,
        &[],
        L1HistoryLimits {
            max_headers: 131_072,
            max_proof_nodes: 128,
        },
        &config(),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("trusted envelope limit"),
        "{error:?}"
    );
    let mut input = input;
    input.post_block_cursors.clear();
    input.blocks[0].execution_transactions[0] = vec![0x02];
    let error = fixture.verify_input(&input).unwrap_err();
    assert!(error.to_string().contains("cursor count"), "{error:?}");
}

// Independent regressions reuse the existing genuine EVM/MPT fixture.

fn independent_roundtrip(
    input: &raiko2_protocol_etna::input::RangeInput,
) -> raiko2_protocol_etna::input::RangeInput {
    serde_json::from_slice(&serde_json::to_vec(input).unwrap()).unwrap()
}

#[test]
fn owned_input_independent_rejects_all_list_count_mismatches() {
    let fixture = ExecutingRange::new(2, true);
    let original = fixture.input();
    assert!(fixture
        .verify_input(&independent_roundtrip(&original))
        .is_ok());
    for mutation in [
        "block_missing",
        "block_extra",
        "cursor_missing",
        "cursor_extra",
        "tx_missing",
        "tx_extra",
        "comet_missing",
        "comet_extra",
        "cut_missing",
        "cut_extra",
    ] {
        let mut input = independent_roundtrip(&original);
        match mutation {
            "block_missing" => {
                input.blocks.pop();
            }
            "block_extra" => {
                input
                    .blocks
                    .push(independent_roundtrip(&original).blocks.remove(1));
            }
            "cursor_missing" => {
                input.post_block_cursors.pop();
            }
            "cursor_extra" => input.post_block_cursors.push(0),
            "tx_missing" => {
                input.blocks[1].execution_transactions.pop();
            }
            "tx_extra" => {
                let tx = input.blocks[1].execution_transactions[0].clone();
                input.blocks[1].execution_transactions.push(tx);
            }
            "comet_missing" => {
                input.blocks[1].comet_transactions.pop();
            }
            "comet_extra" => {
                let tx = input.blocks[1].comet_transactions[0].clone();
                input.blocks[1].comet_transactions.push(tx);
            }
            "cut_missing" => {
                input.statement.cut_leaves.pop();
            }
            "cut_extra" => input.statement.cut_leaves.push(B256::ZERO),
            _ => unreachable!(),
        }
        assert!(fixture.verify_input(&input).is_err(), "{mutation}");
    }
}

#[test]
fn owned_input_independent_rejects_malformed_first_and_interior_transaction_bytes() {
    let fixture = ExecutingRange::new(2, true);
    let original = fixture.input();
    for block_index in [0, 1] {
        for mutation in ["empty", "truncated", "trailing", "type"] {
            let mut input = independent_roundtrip(&original);
            let bytes = &mut input.blocks[block_index].execution_transactions[1];
            match mutation {
                "empty" => bytes.clear(),
                "truncated" => {
                    bytes.pop();
                }
                "trailing" => bytes.push(0),
                "type" => bytes[0] = 0x7f,
                _ => unreachable!(),
            }
            assert!(
                fixture.verify_input(&input).is_err(),
                "block {block_index}, {mutation}"
            );
        }
    }
}

#[test]
fn owned_input_independent_preserves_selected_q_and_rejects_replacement() {
    let fixture = ExecutingRange::new(2, true);
    let original = fixture.input();
    for mutation in ["statement_number", "statement_hash", "raw_q", "raw_anchor"] {
        let mut input = independent_roundtrip(&original);
        match mutation {
            "statement_number" => input.statement.input.l1_reference_block_number += U256::ONE,
            "statement_hash" => input.statement.l1_reference_block_hash = B256::repeat_byte(0x91),
            "raw_q" => input.l1_history.reference_header.push(0),
            "raw_anchor" => input.l1_history.anchors[0].raw_header.push(0),
            _ => unreachable!(),
        }
        assert!(fixture.verify_input(&input).is_err(), "{mutation}");
    }
}

#[test]
fn owned_input_independent_rejects_foreign_authority_json() {
    use raiko2_protocol_etna::input::RangeInput;
    let fixture = ExecutingRange::new(1, false);
    let original = serde_json::to_value(fixture.input()).unwrap();
    for field in [
        "context",
        "domain",
        "limits",
        "config",
        "execution_config",
        "authenticated",
        "host_verified",
        "skip",
        "max_blocks",
        "max_envelope_bytes",
    ] {
        let mut value = original.clone();
        value[field] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<RangeInput>(value).is_err(),
            "{field}"
        );
    }
    for target in ["block", "statement"] {
        let mut value = original.clone();
        let object = if target == "block" {
            &mut value["blocks"][0]
        } else {
            &mut value["statement"]
        };
        object["host_verified"] = serde_json::json!(true);
        assert!(
            serde_json::from_value::<RangeInput>(value).is_err(),
            "{target}"
        );
    }
    for value in [
        serde_json::json!(-1),
        serde_json::json!(256),
        serde_json::json!("0"),
    ] {
        let mut encoded = original.clone();
        encoded["l1_history"]["reference_header"][0] = value;
        assert!(serde_json::from_value::<RangeInput>(encoded).is_err());
    }
}
