//! Minimal synthetic proposal construction using production stateless validation.
use super::{
    observer::{FixtureExecutor, OpcodeObservation},
    storage::{ProgramStorageState, ProgramStorageStateReport},
};
use alethia_reth_block::config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes};
use alloy_consensus::{
    SignableTransaction, TrieAccount, TxEip1559, TxReceipt as _, constants::KECCAK_EMPTY,
    transaction::SignerRecoverable,
};
use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256, keccak256};
use alloy_signer::SignerSync;
use alloy_signer_local_fixture::PrivateKeySigner;
use alloy_sol_types::{SolCall, sol};
use alloy_trie::EMPTY_ROOT_HASH;
use anyhow::{Result, bail};
use raiko2_primitives::{
    ExecutionWitness, ProofType, StatelessInput, SupportedChainSpecs, WitnessHeader,
    WitnessStateNode,
    blob::util::{blob_to_commitment, blob_to_proof_of_equivalence, commitment_to_version_hash},
    builtin_taiko_chain_spec,
    chain_spec::{ForkCondition, ForkId, TaikoFork},
};
use raiko2_primitives_shasta::{GuestInput, build_proof_carry_data_from_witness_spec};
use raiko2_protocol::InputDataSource;
use raiko2_protocol_shasta::{
    TaikoManifest,
    libhash::hash_proposal,
    shasta::{
        BlobSlice, DerivationSource, calculate_shasta_difficulty, encode_extra_data,
        manifest::{BlockManifest, DerivationSourceManifest},
    },
};
use raiko2_stateless::reconstruct_block_from_transactions_with_executor_and_witness_resources;
use reth_ethereum_primitives::TransactionSigned;
use risc0_ethereum_trie::Trie;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use taiko_client_protocol::{
    FixedKSigner,
    shasta::constants::{calculate_next_block_eip4396_base_fee_for_parent, min_base_fee_for_chain},
};
const OVERHEAD_CHAIN_ID: u64 = 167_000;
const OVERHEAD_PARENT_ANCHOR_BLOCK_NUMBER: u64 = 7;
const OVERHEAD_BLOCK_NUMBER: u64 = 1_166_000;
const OVERHEAD_ANCHOR_GAS_LIMIT: u64 = 1_000_000;
const CONTROLLED_CHECKPOINT_STORE: Address = Address::repeat_byte(0x88);
const CONTROLLED_RESOLVER: Address = Address::repeat_byte(0x99);

sol! {
    #[derive(Debug)]
    struct AnchorV4Checkpoint {
        uint48 blockNumber;
        bytes32 blockHash;
        bytes32 stateRoot;
    }

    function anchorV4(AnchorV4Checkpoint _checkpoint) external;
}

pub(super) struct BuiltProgram {
    pub input: GuestInput,
    pub receipts: Vec<Vec<ControlledNativeReceipt>>,
    pub observations: Vec<OpcodeObservation>,
    pub storage: Option<ProgramStorageStateReport>,
}
/// Native execution receipt data; gas is the charged cumulative-receipt delta.
#[derive(Clone, Debug, Serialize)]
pub struct ControlledNativeReceipt {
    pub transaction_index: usize,
    pub success: bool,
    pub gas_used: u64,
    pub cumulative_gas_used: u64,
}

pub(super) trait ControlledMptFixture {
    fn apply_fixture(
        &self,
        state: &mut ControlledTrieState,
        candidates: &[TransactionSigned],
    ) -> Result<()>;

    fn minimized_fixture_witness(
        &self,
        state: &ControlledTrieState,
        candidates: &[TransactionSigned],
    ) -> Result<(B256, Vec<WitnessStateNode>)>;
}
const CONTROLLED_CANDIDATE_FINAL_BALANCE: U256 =
    U256::from_limbs([10_000_000_000_000_000, 0, 0, 0]);
fn sample_l1_header(number: u64, state_root: B256) -> alloy_consensus::Header {
    alloy_consensus::Header {
        number,
        parent_hash: B256::repeat_byte(0xaa),
        state_root,
        ..Default::default()
    }
}

fn canonical_golden_touch_signature(tx: &TxEip1559) -> Result<Signature> {
    let signature_hash = tx.signature_hash();
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(signature_hash.as_slice());
    Ok(FixedKSigner::golden_touch()?
        .sign_with_predefined_k(&hash_bytes)?
        .signature)
}

fn anchor_tx(
    checkpoint: &AnchorV4Checkpoint,
    anchor_address: Address,
    nonce: u64,
    base_fee_per_gas: u64,
) -> Result<TransactionSigned> {
    let tx = TxEip1559 {
        chain_id: OVERHEAD_CHAIN_ID,
        nonce,
        gas_limit: OVERHEAD_ANCHOR_GAS_LIMIT,
        max_fee_per_gas: u128::from(base_fee_per_gas),
        max_priority_fee_per_gas: 0,
        to: TxKind::Call(anchor_address),
        value: U256::ZERO,
        access_list: Default::default(),
        input: anchorV4Call {
            _checkpoint: checkpoint.clone(),
        }
        .abi_encode()
        .into(),
    };
    let signature = canonical_golden_touch_signature(&tx)?;
    Ok(tx.into_signed(signature).into())
}

fn controlled_candidate_signer() -> Result<PrivateKeySigner> {
    // Public, fixed offline test identity. Never read a wallet or credential source.
    Ok(PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11))?)
}
fn candidate_transactions(code_input: &Bytes, count: usize) -> Result<Vec<TransactionSigned>> {
    let signer = controlled_candidate_signer()?;
    (0..count)
        .map(|nonce| {
            let tx = TxEip1559 {
                chain_id: OVERHEAD_CHAIN_ID,
                nonce: u64::try_from(nonce)?,
                gas_limit: 15_000_000,
                max_fee_per_gas: 25_000_000,
                max_priority_fee_per_gas: 0,
                to: TxKind::Call(Address::repeat_byte(0x66)),
                value: U256::ZERO,
                access_list: Default::default(),
                input: code_input.clone(),
            };
            let signature = signer.sign_hash_sync(&tx.signature_hash())?;
            Ok(tx.into_signed(signature).into())
        })
        .collect()
}
#[derive(Clone, Default)]
pub(super) struct ControlledTrieState {
    pub(super) accounts: BTreeMap<B256, TrieAccount>,
    pub(super) storages: BTreeMap<B256, BTreeMap<B256, U256>>,
}

impl ControlledTrieState {
    fn insert_account(&mut self, address: Address, balance: U256, nonce: u64, code_hash: B256) {
        self.accounts.insert(
            keccak256(address),
            TrieAccount {
                nonce,
                balance,
                storage_root: EMPTY_ROOT_HASH,
                code_hash,
            },
        );
    }

    fn account_balance(&self, address: Address) -> Option<U256> {
        self.accounts
            .get(&keccak256(address))
            .map(|account| account.balance)
    }

    fn apply(&mut self, outcome: &raiko2_stateless::FilteredBlockExecutionOutcome) {
        let post_state = &outcome.hashed_state;
        for (hashed_address, storage_changes) in &post_state.storages {
            let storage = self.storages.entry(*hashed_address).or_default();
            // reth v2.6 post-state storage is a delta without a wipe marker. Preserve every
            // unmentioned slot in the controlled pre-state and apply only the explicit changes.
            for (hashed_slot, value) in &storage_changes.storage {
                if value.is_zero() {
                    storage.remove(hashed_slot);
                } else {
                    storage.insert(*hashed_slot, *value);
                }
            }
        }
        for (hashed_address, account) in &post_state.accounts {
            if let Some(account) = account {
                self.accounts.insert(
                    *hashed_address,
                    TrieAccount {
                        nonce: account.nonce,
                        balance: account.balance,
                        storage_root: EMPTY_ROOT_HASH,
                        code_hash: account.get_bytecode_hash(),
                    },
                );
            } else {
                self.accounts.remove(hashed_address);
                self.storages.remove(hashed_address);
            }
        }
    }

    fn witness(&mut self) -> (B256, Vec<WitnessStateNode>) {
        let mut nodes = Vec::new();
        for (hashed_address, account) in &mut self.accounts {
            let mut storage_trie = Trie::default();
            if let Some(storage) = self.storages.get(hashed_address) {
                for (hashed_slot, value) in storage {
                    storage_trie.insert(*hashed_slot, alloy_rlp::encode(*value));
                }
            }
            account.storage_root = storage_trie.hash_slow();
            nodes.extend(
                storage_trie
                    .rlp_nodes()
                    .into_iter()
                    .map(WitnessStateNode::from_bytes),
            );
        }
        let mut state_trie = Trie::default();
        for (hashed_address, account) in &self.accounts {
            state_trie.insert(*hashed_address, alloy_rlp::encode(account));
        }
        let root = state_trie.hash_slow();
        nodes.extend(
            state_trie
                .rlp_nodes()
                .into_iter()
                .map(WitnessStateNode::from_bytes),
        );
        (root, ExecutionWitness::canonicalize_state_nodes(nodes))
    }
}

fn immutable_u64_word(value: u64) -> [u8; 32] {
    U256::from_limbs([value, 0, 0, 0]).to_be_bytes()
}

fn address_word(value: Address) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[12..].copy_from_slice(value.as_slice());
    word
}

fn patch_anchor_immutable(
    code: &mut [u8],
    artifact: &Value,
    ast_id: &str,
    word: [u8; 32],
) -> Result<()> {
    let references = artifact["deployedBytecode"]["immutableReferences"][ast_id]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Anchor artifact is missing immutable {ast_id}"))?;
    for reference in references {
        let start = reference["start"]
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| anyhow::anyhow!("Anchor immutable {ast_id} has invalid offset"))?;
        let length = reference["length"].as_u64().unwrap_or_default();
        if length != 32 || start.saturating_add(32) > code.len() {
            bail!("Anchor immutable {ast_id} is outside deployed bytecode");
        }
        code[start..start + 32].copy_from_slice(&word);
    }
    Ok(())
}

fn controlled_anchor_code(
    anchor_address: Address,
    checkpoint_store_address: Address,
) -> Result<Bytes> {
    // Use the checked-in Anchor runtime artifact with this post-Unzen fixture profile.
    // Patch constructor immutables as deployment would; the artifact does not select the fork.
    let artifact: Value = serde_json::from_str(include_str!(
        "../../../../scripts/regression/shasta/Anchor.json"
    ))?;
    let object = artifact["deployedBytecode"]["object"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Anchor artifact is missing deployed bytecode"))?;
    let mut code = alloy_primitives::hex::decode(object)?;
    // Solidity AST ids are fixed by the checked-in artifact: UUPS __self, resolver,
    // checkpointStore, and l1ChainId respectively.
    patch_anchor_immutable(&mut code, &artifact, "19589", address_word(anchor_address))?;
    patch_anchor_immutable(
        &mut code,
        &artifact,
        "3130",
        address_word(CONTROLLED_RESOLVER),
    )?;
    patch_anchor_immutable(
        &mut code,
        &artifact,
        "375",
        address_word(checkpoint_store_address),
    )?;
    patch_anchor_immutable(&mut code, &artifact, "378", immutable_u64_word(1))?;
    Ok(code.into())
}
fn overhead_prestate(
    chain_spec: &raiko2_primitives::ChainSpec,
    candidates: &[TransactionSigned],
    code: &Bytes,
) -> Result<(ControlledTrieState, Vec<Bytes>)> {
    let anchor_address = chain_spec
        .l2_contract
        .ok_or_else(|| anyhow::anyhow!("missing Anchor address"))?;
    let anchor_code = controlled_anchor_code(anchor_address, CONTROLLED_CHECKPOINT_STORE)?;
    let checkpoint_code = Bytes::from_static(&[0x00]);
    let mut state = ControlledTrieState::default();
    state.insert_account(anchor_address, U256::ZERO, 0, keccak256(&anchor_code));
    state.insert_account(
        CONTROLLED_CHECKPOINT_STORE,
        U256::ZERO,
        0,
        keccak256(&checkpoint_code),
    );
    let signer = candidates[0]
        .recover_signer()
        .map_err(|_| anyhow::anyhow!("unrecoverable user signature"))?;
    state.insert_account(signer, CONTROLLED_CANDIDATE_FINAL_BALANCE, 0, KECCAK_EMPTY);
    state.insert_account(Address::repeat_byte(0x66), U256::ZERO, 0, keccak256(code));
    Ok((state, vec![anchor_code, checkpoint_code, code.clone()]))
}
#[derive(Debug)]
struct ControlledKonaPayloadOverflow;

impl std::fmt::Display for ControlledKonaPayloadOverflow {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("controlled manifest does not fit one Kona blob")
    }
}

impl std::error::Error for ControlledKonaPayloadOverflow {}

fn kona_blob(payload: &[u8]) -> Result<Vec<u8>> {
    // Fixture-only mirror of taiko-mono's decoder layout. This stays host-side and its output is
    // accepted only after the production blob/KZG verifier and manifest decoder consume it.
    const ROUNDS: usize = 1024;
    const MAX_PAYLOAD: usize = (4 * 31 + 3) * ROUNDS - 4;
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return Err(ControlledKonaPayloadOverflow.into());
    }
    fn read1(payload: &[u8], offset: &mut usize) -> u8 {
        let value = payload.get(*offset).copied().unwrap_or(0);
        *offset = offset.saturating_add(usize::from(*offset < payload.len()));
        value
    }
    fn read31(output: &mut [u8; 31], payload: &[u8], offset: &mut usize) {
        output.fill(0);
        let count = payload.len().saturating_sub(*offset).min(31);
        output[..count].copy_from_slice(&payload[*offset..*offset + count]);
        *offset += count;
    }
    fn write_fe(blob: &mut [u8], index: usize, header: u8, body: &[u8; 31]) {
        let start = index * 32;
        blob[start] = header;
        blob[start + 1..start + 32].copy_from_slice(body);
    }
    let mut blob = vec![0u8; 131_072];
    let mut offset = 0;
    let mut buf31 = [0u8; 31];
    for round in 0..ROUNDS {
        if offset >= payload.len() {
            break;
        }
        if round == 0 {
            let length = u32::try_from(payload.len())?;
            buf31.fill(0);
            buf31[1..4].copy_from_slice(&length.to_be_bytes()[1..]);
            let count = payload.len().min(27);
            buf31[4..4 + count].copy_from_slice(&payload[..count]);
            offset += count;
        } else {
            read31(&mut buf31, payload, &mut offset);
        }
        let x = read1(payload, &mut offset);
        write_fe(&mut blob, 4 * round, x & 0x3f, &buf31);
        read31(&mut buf31, payload, &mut offset);
        let y = read1(payload, &mut offset);
        write_fe(
            &mut blob,
            4 * round + 1,
            (y & 0x0f) | ((x & 0xc0) >> 2),
            &buf31,
        );
        read31(&mut buf31, payload, &mut offset);
        let z = read1(payload, &mut offset);
        write_fe(&mut blob, 4 * round + 2, z & 0x3f, &buf31);
        read31(&mut buf31, payload, &mut offset);
        write_fe(
            &mut blob,
            4 * round + 3,
            ((z & 0xc0) >> 2) | ((y & 0xf0) >> 4),
            &buf31,
        );
    }
    Ok(blob)
}
pub(super) fn build_program(
    code: &[u8],
    input: &[u8],
    block_count: usize,
    storage_state: Option<&ProgramStorageState>,
) -> Result<BuiltProgram> {
    if code.is_empty()
        || code.len() > 24_576
        || input.len() > 1_048_576
        || !(1..=32).contains(&block_count)
    {
        bail!("program exceeds code, calldata or block bounds");
    }
    if let Some(state) = storage_state {
        state.validate(block_count)?;
    }
    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(OVERHEAD_CHAIN_ID)
        .ok_or_else(|| anyhow::anyhow!("missing controlled Taiko chain spec"))?;
    let overhead_parent_timestamp =
        match chain_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)) {
            Some(ForkCondition::Timestamp(timestamp)) => *timestamp,
            Some(condition) => {
                bail!("controlled Taiko chain has non-timestamp Unzen activation: {condition:?}")
            }
            None => bail!("controlled Taiko chain is missing canonical Unzen activation"),
        };
    let runtime_chain_spec = builtin_taiko_chain_spec(OVERHEAD_CHAIN_ID)?;
    let evm_config = TaikoEvmConfig::new(runtime_chain_spec.clone());
    let candidates = candidate_transactions(&Bytes::copy_from_slice(input), block_count)?;
    let controlled_signer = controlled_candidate_signer()?;
    let base_fee_per_gas = 10_000_000;
    let timestamp_delta = 1;
    let (mut controlled_state, codes) =
        overhead_prestate(&chain_spec, &candidates, &Bytes::copy_from_slice(code))?;
    if let Some(state) = storage_state {
        state.apply_fixture(&mut controlled_state, &candidates)?;
    }
    let (mut prestate_root, full_state_nodes) = controlled_state.witness();
    let full_witness_node_count = full_state_nodes.len();
    let mut state_nodes = if let Some(topology) = storage_state {
        let (proof_root, proof_nodes) =
            topology.minimized_fixture_witness(&controlled_state, &candidates)?;
        if proof_root != prestate_root {
            bail!("controlled MPT proof root differs from the full prestate root");
        }
        proof_nodes
    } else {
        full_state_nodes
    };
    let minimized_witness_node_count = state_nodes.len();
    let l1_header = sample_l1_header(OVERHEAD_PARENT_ANCHOR_BLOCK_NUMBER, B256::repeat_byte(0x66));
    let checkpoint = AnchorV4Checkpoint {
        blockNumber: l1_header.number.try_into()?,
        blockHash: l1_header.hash_slow(),
        stateRoot: l1_header.state_root,
    };
    let anchor_address = chain_spec
        .l2_contract
        .ok_or_else(|| anyhow::anyhow!("controlled chain is missing Anchor address"))?;
    let mut previous_hash = B256::repeat_byte(0x77);
    let ancestor_headers = ((OVERHEAD_BLOCK_NUMBER - 256)..OVERHEAD_BLOCK_NUMBER)
        .map(|number| {
            let header = alloy_consensus::Header {
                parent_hash: previous_hash,
                number,
                timestamp: overhead_parent_timestamp
                    - (OVERHEAD_BLOCK_NUMBER - 1).saturating_sub(number),
                gas_limit: 31_000_000,
                base_fee_per_gas: Some(base_fee_per_gas),
                state_root: prestate_root,
                ..Default::default()
            };
            previous_hash = header.hash_slow();
            WitnessHeader::from_header(header)
        })
        .collect::<Vec<_>>();
    let parent_header = ancestor_headers
        .last()
        .expect("controlled ancestors are non-empty")
        .header
        .clone()
        .expect("controlled ancestors retain full headers");
    let proposer = Address::repeat_byte(0x33);
    let fee_recipient = controlled_signer.address();
    let base_fee_share_pctg = 100;
    let extra_data = encode_extra_data(base_fee_share_pctg, 42);
    let mut ancestor_headers = ancestor_headers;
    let proposal_ancestor_headers = ancestor_headers.clone();
    let mut parent = parent_header;
    let mut witnesses = Vec::with_capacity(block_count);
    let mut manifest_blocks = Vec::with_capacity(block_count);
    let mut native_receipts = Vec::with_capacity(block_count);
    let mut observations = Vec::with_capacity(block_count);

    for index in 0..block_count {
        let block_number = OVERHEAD_BLOCK_NUMBER + u64::try_from(index)?;
        let block_timestamp = overhead_parent_timestamp
            .checked_add(timestamp_delta)
            .and_then(|timestamp| timestamp.checked_add(u64::try_from(index).ok()?))
            .ok_or_else(|| anyhow::anyhow!("controlled overhead block timestamp overflow"))?;
        // Every block follows its actual parent, including workload-dependent gas use.
        // Reuse the protocol calculation so high-gas proposal fixtures remain valid.
        let grandparent_timestamp = ancestor_headers
            .get(ancestor_headers.len().saturating_sub(2))
            .and_then(|header| header.header.as_ref())
            .ok_or_else(|| anyhow::anyhow!("controlled grandparent header is missing"))?
            .timestamp;
        let base_fee_per_gas = calculate_next_block_eip4396_base_fee_for_parent(
            parent.number,
            parent.gas_limit,
            parent.gas_used,
            parent.timestamp,
            parent.base_fee_per_gas,
            grandparent_timestamp,
            min_base_fee_for_chain(OVERHEAD_CHAIN_ID),
        )
        .ok_or_else(|| anyhow::anyhow!("controlled parent base fee is missing"))?;
        let transactions = candidates[index..index + 1].to_vec();
        let witness = ExecutionWitness {
            state: state_nodes.clone(),
            codes: codes.clone(),
            headers: ancestor_headers.clone(),
            ..Default::default()
        };
        let block_env = TaikoNextBlockEnvAttributes {
            timestamp: block_timestamp,
            suggested_fee_recipient: fee_recipient,
            prev_randao: calculate_shasta_difficulty(
                B256::from(parent.difficulty.to_be_bytes::<32>()),
                block_number,
            ),
            gas_limit: 31_000_000,
            extra_data: extra_data.clone(),
            base_fee_per_gas,
            parent_beacon_block_root: None,
        };
        let anchor_nonce = u64::try_from(index)?;
        let anchor = anchor_tx(&checkpoint, anchor_address, anchor_nonce, base_fee_per_gas)?;
        let mut executor = FixtureExecutor::default();
        let outcome = reconstruct_block_from_transactions_with_executor_and_witness_resources(
            anchor
                .try_into_recovered()
                .map_err(|_| anyhow::anyhow!("controlled anchor signature is unrecoverable"))?,
            transactions.clone(),
            block_env,
            &witness,
            &ancestor_headers,
            &[],
            &runtime_chain_spec,
            &evm_config,
            &mut executor,
        )?;
        observations.push(executor.observation);
        let mut previous_cumulative_gas = 0;
        native_receipts.push(
            outcome
                .execution_result
                .receipts
                .iter()
                .enumerate()
                .map(|(transaction_index, receipt)| {
                    let cumulative_gas_used = receipt.cumulative_gas_used();
                    let gas_used = cumulative_gas_used
                        .checked_sub(previous_cumulative_gas)
                        .ok_or_else(|| {
                            anyhow::anyhow!("native receipt cumulative gas decreased")
                        })?;
                    previous_cumulative_gas = cumulative_gas_used;
                    Ok(ControlledNativeReceipt {
                        transaction_index,
                        success: receipt.status(),
                        gas_used,
                        cumulative_gas_used,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        );
        controlled_state.apply(&outcome);
        let (post_state_root, post_state_nodes) = controlled_state.witness();
        let golden_touch_hash = keccak256(Address::from([
            0x00, 0x00, 0x77, 0x77, 0x35, 0x36, 0x7b, 0x36, 0xbc, 0x9b, 0x61, 0xc5, 0x00, 0x22,
            0xd9, 0xd0, 0x70, 0x0d, 0xb4, 0xec,
        ]));
        let next_nonce = outcome
            .hashed_state
            .accounts
            .get(&golden_touch_hash)
            .and_then(Option::as_ref)
            .map(|account| account.nonce);
        if next_nonce != Some(anchor_nonce.saturating_add(1)) {
            bail!("repeated anchor produced an unexpected golden-touch nonce");
        }
        let block = outcome.filtered_block.into_block();
        if block.header.state_root != post_state_root {
            bail!(
                "controlled fixture hashed post-state differs from rebuilt state: pre={prestate_root}, post={}, rebuilt={post_state_root}",
                block.header.state_root,
            );
        }
        if index + 1 < block_count {
            prestate_root = post_state_root;
            state_nodes = post_state_nodes;
        }
        manifest_blocks.push(BlockManifest {
            timestamp: block_timestamp,
            coinbase: fee_recipient,
            anchor_block_number: OVERHEAD_PARENT_ANCHOR_BLOCK_NUMBER,
            gas_limit: 30_000_000,
            transactions: transactions
                .iter()
                .map(|tx| alloy_rlp::decode_exact(alloy_rlp::encode(tx)))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        });
        witnesses.push(StatelessInput {
            block: block.clone(),
            chain_spec: chain_spec.clone(),
            witness,
            ..Default::default()
        });
        ancestor_headers.push(WitnessHeader::from_header(block.header.clone()));
        parent = block.header;
    }

    let manifest = DerivationSourceManifest {
        blocks: manifest_blocks,
    };
    let blob = kona_blob(&manifest.encode_and_compress()?)?;
    let commitment = blob_to_commitment(&blob)?;
    let proof = blob_to_proof_of_equivalence(&blob, &commitment)?;
    let blob_hash = commitment_to_version_hash(&commitment);
    let source = DerivationSource {
        isForcedInclusion: false,
        blobSlice: BlobSlice {
            blobHashes: vec![blob_hash],
            offset: 0usize.try_into()?,
            timestamp: 0u64.try_into()?,
        },
    };
    let l1_ancestor_headers = vec![l1_header.clone()];
    let mut guest_input = GuestInput {
        witnesses,
        proposal_ancestor_headers,
        taiko: TaikoManifest {
            proposal_id: 42,
            l1_header: l1_header.clone(),
            l1_ancestor_headers,
            proposal_event: raiko2_protocol_shasta::shasta::ShastaEventData {
                proposal: raiko2_protocol_shasta::shasta::Proposal {
                    id: 42u64.try_into()?,
                    proposer,
                    timestamp: overhead_parent_timestamp
                        .checked_add(timestamp_delta)
                        .and_then(|timestamp| {
                            timestamp
                                .checked_add(u64::try_from(block_count.saturating_sub(1)).ok()?)
                        })
                        .ok_or_else(|| {
                            anyhow::anyhow!("controlled overhead proposal timestamp overflow")
                        })?
                        .try_into()?,
                    parentProposalHash: B256::repeat_byte(0x44),
                    originBlockNumber: l1_header.number.try_into()?,
                    originBlockHash: l1_header.hash_slow(),
                    basefeeSharingPctg: base_fee_share_pctg,
                    sources: vec![source],
                    ..Default::default()
                },
            },
            chain_spec: raiko2_protocol::ManifestChainSpec {
                name: chain_spec.name.clone(),
                chain_id: OVERHEAD_CHAIN_ID,
                is_taiko: true,
            },
            prover_data: raiko2_protocol::TaikoProverData {
                actual_prover: Address::repeat_byte(0x22),
                last_anchor_block_number: None,
                ..Default::default()
            },
            blob_proof_type: raiko2_protocol::BlobProofType::ProofOfEquivalence,
            data_sources: vec![InputDataSource {
                tx_data_from_blob: vec![blob],
                blob_commitments: vec![commitment.to_vec()],
                blob_proofs: vec![proof.to_vec()],
                ..Default::default()
            }],
        },
        ..Default::default()
    };
    guest_input.proof_carry_data =
        build_proof_carry_data_from_witness_spec(&guest_input, ProofType::Sp1)?;
    guest_input.proof_carry_data.transition_input.proposal_hash =
        hash_proposal(&guest_input.taiko.proposal_event.proposal);
    if controlled_state.account_balance(controlled_signer.address())
        != Some(CONTROLLED_CANDIDATE_FINAL_BALANCE)
    {
        bail!("fee-neutral fixture sender did not finish at expected balance");
    }
    Ok(BuiltProgram {
        input: guest_input,
        receipts: native_receipts,
        observations,
        storage: storage_state
            .map(|state| state.report(full_witness_node_count, minimized_witness_node_count)),
    })
}
