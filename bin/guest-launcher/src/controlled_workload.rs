use std::collections::BTreeMap;

use alethia_reth_block::config::{TaikoEvmConfig, TaikoNextBlockEnvAttributes};
use alloy_consensus::{
    SignableTransaction, Transaction as _, TrieAccount, TxEip1559, constants::KECCAK_EMPTY,
    transaction::SignerRecoverable,
};
use alloy_primitives::{Address, B256, Bytes, Signature, TxKind, U256, keccak256};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use alloy_sol_types::{SolCall, sol};
use alloy_trie::EMPTY_ROOT_HASH;
use anyhow::{Result, bail};
use raiko2_opcode_lab::{
    OPCODE_LAB_SPEC_ID, build_benchmark_db, build_benchmark_tx, build_context_benchmark_block_env,
    build_context_benchmark_tx, context_opcode_public_values, fold_revm_opcode_execution_result,
    fold_revm_opcode_program, revm_opcode_public_values,
};
use raiko2_primitives::{
    ContextOpcodeLabInputV1, ExecutionWitness, OpcodeLabInput, OpcodeLabStorageInput,
    OpcodeLabStorageLane, OpcodeLabStorageOperation, PrecompileLabInput, PrecompileLabLane,
    ProofType, StatelessInput, SupportedChainSpecs, WitnessHeader, WitnessStateNode,
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
use raiko2_stateless::reconstruct_block_from_transactions_with_witness_resources;
use raiko2_zkgas_trace::{
    OperationComponent, OperationPhase, PricingBasis, ProposalTrace, ProposalTraceStatus,
    TransactionDisposition, trace_shasta_proposal,
};
use reth_ethereum_primitives::TransactionSigned;
use revm::{
    Context, ExecuteEvm, InspectEvm, Inspector, MainBuilder, MainContext,
    bytecode::Bytecode,
    context::{BlockEnv, TxEnv},
    context_interface::result::ExecutionResult,
    database::BENCH_TARGET,
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
    primitives::U256 as RevmU256,
};
use risc0_ethereum_trie::Trie;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use taiko_client_protocol::FixedKSigner;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledLane {
    Target,
    Control,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ControlledWorkloadSpec {
    pub schema_version: u32,
    pub key_id: String,
    pub case_id: String,
    pub target_count: u64,
    pub lane: ControlledLane,
    pub state: BTreeMap<String, Value>,
    pub environment: BTreeMap<String, Value>,
    pub input: BTreeMap<String, Value>,
    pub expected_operation_deltas: BTreeMap<String, i64>,
    pub expected_feature_deltas: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ControlledExecutionIdentity {
    pub backend: String,
    pub backend_input_sha256: String,
    pub repeat_index: u32,
    pub run_id: String,
    pub workload_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlledFootprint {
    pub target_count: u64,
    pub tx_gas_limit: u64,
    pub bytecode_len: usize,
    pub input_len: usize,
    pub non_target_counts: BTreeMap<String, u64>,
    pub non_target_raw_gas: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairedPrecompileShape {
    pub lane: ControlledLane,
    pub target_count: u64,
    pub input_len: usize,
    pub loop_iterations: u64,
    pub output_len: usize,
    pub folded_bytes_per_iteration: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeTrace {
    pub schema_version: u32,
    pub workload_id: String,
    pub backend_input_sha256: String,
    pub backend_input_len: usize,
    pub target_opcode: u8,
    pub declared_target_count: u64,
    pub declared_target_raw_gas: u64,
    pub tx_gas_limit: u64,
    pub executed_target_count: u64,
    pub executed_target_raw_gas: u64,
    pub non_target_counts: BTreeMap<String, u64>,
    pub non_target_raw_gas: u64,
    pub total_raw_gas: u64,
    pub bytecode_len: usize,
    pub evm_spec: &'static str,
    pub revm_version: &'static str,
    pub shared_constructor: &'static str,
    pub transaction_envelope_sha256: String,
    pub block_environment_sha256: String,
    pub access_list_sha256: String,
    pub prestate_sha256: String,
    pub bytecode_sha256: String,
    pub program_sha256: Vec<String>,
    pub executed_opcode_counts: BTreeMap<String, u64>,
    pub executed_opcode_raw_gas: BTreeMap<String, u64>,
    pub executed_measurement_count: u64,
    pub executed_measurement_raw_gas: u64,
    pub executed_prefix_count: u64,
    pub result_statuses: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<OpcodeLabStorageInput>,
    pub semantic_check: ControlledOpcodeSemanticCheck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeIdentityEvidence {
    pub schema_version: u32,
    pub input: OpcodeLabInput,
    pub backend_input_sha256: String,
    pub backend_input_len: usize,
    pub workload_id: String,
    pub transaction_envelope_sha256: String,
    pub block_environment_sha256: String,
    pub access_list_sha256: String,
    pub prestate_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeIdentityReport {
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub controlled_trace: ControlledTrace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeIdentityBundle {
    pub schema_version: u32,
    pub expected_public_values: String,
    pub identity: ControlledOpcodeIdentityEvidence,
    pub report: ControlledOpcodeIdentityReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledContextOpcodeIdentityEvidence {
    pub schema_version: u32,
    pub input: ContextOpcodeLabInputV1,
    pub backend_input_sha256: String,
    pub backend_input_len: usize,
    pub workload_id: String,
    pub transaction_envelope_sha256: String,
    pub block_environment_sha256: String,
    pub access_list_sha256: String,
    pub prestate_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledContextOpcodeIdentityBundle {
    pub schema_version: u32,
    pub expected_public_values: String,
    pub identity: ControlledContextOpcodeIdentityEvidence,
    pub report: ControlledOpcodeIdentityReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeProgramSemantic {
    pub program_index: usize,
    pub result_status: String,
    pub expected_load_values: Vec<String>,
    pub observed_load_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_final_storage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_final_storage: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOpcodeSemanticCheck {
    pub schema_version: u32,
    pub backend_input_sha256: String,
    pub checked_programs: usize,
    pub passed: bool,
    pub programs: Vec<ControlledOpcodeProgramSemantic>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledPrecompileTrace {
    pub schema_version: u32,
    pub workload_id: String,
    pub pair_id: String,
    pub backend_input_sha256: String,
    pub backend_input_len: usize,
    pub address: u8,
    pub target_count: u64,
    pub target_raw_gas: u64,
    pub lane: ControlledLane,
    pub input_len: usize,
    pub output_len: usize,
    pub loop_iterations: u64,
    pub folded_bytes_per_iteration: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlledTrace {
    RevmOpcode(Box<ControlledOpcodeTrace>),
    Precompile(ControlledPrecompileTrace),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledOverheadLane {
    Target,
    Control,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOperationUnits {
    pub pricing_basis: PricingBasis,
    pub units: i64,
    pub event_count: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControlledOverheadFixture {
    pub case_id: String,
    pub overhead_key_id: String,
    pub lane: ControlledOverheadLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_kind: Option<String>,
    pub target_count: u64,
    pub expected_operation_deltas: BTreeMap<String, ControlledOperationUnits>,
    pub expected_feature_deltas: BTreeMap<String, i64>,
    pub guest_input: GuestInput,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOverheadWorkloadSpec {
    pub schema_version: u32,
    pub overhead_key_id: String,
    pub case_id: String,
    pub lane: ControlledOverheadLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_kind: Option<String>,
    pub target_count: u64,
    pub guest_input_canonical_sha256: String,
    pub expected_operation_deltas: BTreeMap<String, ControlledOperationUnits>,
    pub expected_feature_deltas: BTreeMap<String, i64>,
    pub operation_phase_ownership: &'static str,
    pub system_operation_ownership: &'static str,
    pub anchor_operation_ownership: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledOverheadObservation {
    pub case_id: String,
    pub overhead_key_id: String,
    pub lane: ControlledOverheadLane,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_kind: Option<String>,
    pub target_count: u64,
    pub workload_spec: ControlledOverheadWorkloadSpec,
    pub workload_id: String,
    pub backend_input_sha256: String,
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub public_output: B256,
    /// Transaction-phase pricing units keyed by canonical measurement identity.
    pub absolute_operation_pricing_units: BTreeMap<String, ControlledOperationUnits>,
    pub absolute_feature_counts: BTreeMap<String, i64>,
    /// Host-trace-discovered operation delta, frozen in the sample before SP1 execution.
    pub observed_operation_deltas: BTreeMap<String, ControlledOperationUnits>,
    pub expected_operation_deltas: BTreeMap<String, ControlledOperationUnits>,
    pub operation_phase_ownership: &'static str,
    pub system_operation_ownership: &'static str,
    pub anchor_operation_ownership: &'static str,
    pub expected_feature_deltas: BTreeMap<String, i64>,
    pub started_candidate_transaction_count: usize,
    pub committed_candidate_transaction_count: usize,
    pub unattempted_candidate_transaction_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledBlockSplit {
    Fit,
    Holdout,
    Diagnostic,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ControlledProgram {
    Empty,
    NativeTransfer {
        value: u64,
    },
    OpcodeLoop {
        family: String,
        count: u64,
        scenario: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ControlledBlockRowSpec {
    pub row_id: String,
    pub workload_family: String,
    pub split: ControlledBlockSplit,
    pub block_count: usize,
    pub transaction_count: u64,
    pub program: ControlledProgram,
    pub expected_final_state_root: B256,
    pub expected_raw_gas_by_key: BTreeMap<String, i64>,
    pub expected_features: BTreeMap<String, i64>,
    pub expected_diagnostics: BTreeMap<String, i64>,
}

#[derive(Clone, Debug)]
pub struct ControlledBlockFixture {
    pub spec: ControlledBlockRowSpec,
    pub guest_input: GuestInput,
    controlled_bytecode_length: usize,
    touched_state_key_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledBlockObservation {
    pub row_id: String,
    pub workload_family: String,
    pub split: ControlledBlockSplit,
    pub backend_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub public_output: B256,
    pub actual_final_state_root: B256,
    pub actual_raw_gas_by_key: BTreeMap<String, i64>,
    pub actual_features: BTreeMap<String, i64>,
    pub actual_diagnostics: BTreeMap<String, i64>,
    pub unzen_activation_timestamp: u64,
    pub minimum_block_timestamp: u64,
    pub operation_phase_ownership: &'static str,
    pub system_operation_ownership: &'static str,
    pub anchor_operation_ownership: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessTopologyLaneSpec {
    pub extra_account_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DirtyAccountsRecipientMode {
    Single,
    Distinct,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirtyAccountsLaneSpec {
    pub transaction_count: u64,
    pub value: u64,
    pub recipient_mode: DirtyAccountsRecipientMode,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "kind")]
pub enum ControlledStateHoldoutPairSpec {
    WitnessTopology {
        pair_id: String,
        scale: usize,
        control: WitnessTopologyLaneSpec,
        target: WitnessTopologyLaneSpec,
    },
    DirtyAccounts {
        pair_id: String,
        scale: u64,
        control: DirtyAccountsLaneSpec,
        target: DirtyAccountsLaneSpec,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ControlledStateHoldout {
    WitnessTopology { extra_account_count: usize },
    DirtyAccounts { transaction_count: u64, value: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlledStateHoldoutLane {
    Control,
    Target,
}

#[derive(Clone, Debug)]
pub struct ControlledStateHoldoutFixture {
    pub spec: ControlledStateHoldoutPairSpec,
    pub pair_id: String,
    pub lane: ControlledStateHoldoutLane,
    pub guest_input: GuestInput,
    touched_state_key_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledStateHoldoutObservation {
    pub pair_id: String,
    pub lane: ControlledStateHoldoutLane,
    pub backend_input_sha256: String,
    pub guest_input_sha256: String,
    pub guest_input_bincode_length: usize,
    pub public_output: B256,
    pub actual_final_state_root: B256,
    pub actual_raw_gas_by_key: BTreeMap<String, i64>,
    pub actual_features: BTreeMap<String, i64>,
    pub actual_diagnostics: BTreeMap<String, i64>,
    pub started_candidate_transaction_count: u64,
    pub committed_candidate_transaction_count: u64,
    pub unattempted_candidate_transaction_count: u64,
    pub operation_phase_ownership: &'static str,
    pub system_operation_ownership: &'static str,
    pub anchor_operation_ownership: &'static str,
}

pub fn controlled_block_row_id(spec: &ControlledBlockRowSpec) -> Result<String> {
    let mut semantics = serde_json::to_value(spec)?;
    semantics
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("controlled block row must serialize as an object"))?
        .remove("row_id");
    Ok(alloy_primitives::hex::encode(Sha256::digest(
        serde_json::to_vec(&semantics)?,
    )))
}

fn controlled_overhead_workload_spec(
    fixture: &ControlledOverheadFixture,
) -> Result<ControlledOverheadWorkloadSpec> {
    let guest_input_canonical = bincode::serialize(&fixture.guest_input)?;
    Ok(ControlledOverheadWorkloadSpec {
        schema_version: 2,
        overhead_key_id: fixture.overhead_key_id.clone(),
        case_id: fixture.case_id.clone(),
        lane: fixture.lane,
        baseline_kind: fixture.baseline_kind.clone(),
        target_count: fixture.target_count,
        guest_input_canonical_sha256: alloy_primitives::hex::encode(Sha256::digest(
            guest_input_canonical,
        )),
        expected_operation_deltas: fixture.expected_operation_deltas.clone(),
        expected_feature_deltas: fixture.expected_feature_deltas.clone(),
        operation_phase_ownership: "transaction_non_anchor_only",
        system_operation_ownership: "block_base",
        anchor_operation_ownership: "block_base",
    })
}

pub fn controlled_overhead_workload_id(fixture: &ControlledOverheadFixture) -> Result<String> {
    let value = BTreeMap::from([
        ("kind", Value::String("controlled".into())),
        (
            "workload_spec",
            serde_json::to_value(controlled_overhead_workload_spec(fixture)?)?,
        ),
    ]);
    sha256_json(&value)
}

fn absolute_transaction_operation_units(
    trace: &ProposalTrace,
) -> Result<BTreeMap<String, ControlledOperationUnits>> {
    let mut absolute = BTreeMap::new();
    for block in &trace.blocks {
        let anchor_started_indices = block
            .transactions
            .iter()
            .filter(|transaction| transaction.is_anchor)
            .filter_map(|transaction| transaction.started_tx_index)
            .collect::<std::collections::BTreeSet<_>>();
        for operation in &block.operations {
            if operation.phase != OperationPhase::Transaction
                || operation
                    .tx_index
                    .is_some_and(|index| anchor_started_indices.contains(&index))
            {
                continue;
            }
            let (key, pricing_basis, units) = match operation.component {
                OperationComponent::Opcode {
                    opcode,
                    pricing_basis: Some(PricingBasis::RawGasSlope),
                    interpreter_raw_gas: Some(raw_gas),
                    spawned,
                    ..
                } => {
                    let key = if matches!(opcode, 0xf0 | 0xf1 | 0xf2 | 0xf4 | 0xf5 | 0xfa) {
                        match spawned {
                            Some(true) => format!("opcode:0x{opcode:02x}:spawned"),
                            Some(false) => format!("opcode:0x{opcode:02x}:nonspawned"),
                            None => bail!("CALL/CREATE operation is missing spawned identity"),
                        }
                    } else {
                        format!("opcode:0x{opcode:02x}")
                    };
                    (key, PricingBasis::RawGasSlope, i64::try_from(raw_gas)?)
                }
                OperationComponent::Opcode {
                    opcode,
                    pricing_basis: Some(PricingBasis::FixedPerEvent),
                    spawned: Some(true),
                    ..
                } => (
                    format!("opcode:0x{opcode:02x}:spawned"),
                    PricingBasis::FixedPerEvent,
                    1,
                ),
                OperationComponent::Opcode { opcode, .. } => {
                    bail!("transaction opcode 0x{opcode:02x} has no canonical pricing units")
                }
                OperationComponent::OpcodeFeatureError {
                    opcode, ref error, ..
                } => {
                    bail!(
                        "transaction opcode 0x{opcode:02x} is missing typed model input: {error:?}"
                    )
                }
                OperationComponent::Precompile {
                    address,
                    pricing_basis: PricingBasis::RawGasSlope,
                    native_gas,
                } => {
                    if address[..19].iter().any(|byte| *byte != 0) {
                        bail!("noncanonical precompile address in controlled trace: {address}");
                    }
                    (
                        format!("precompile:0x{:02x}", address[19]),
                        PricingBasis::RawGasSlope,
                        i64::try_from(native_gas)?,
                    )
                }
                OperationComponent::Precompile { address, .. } => {
                    bail!("precompile {address} has a non-raw-gas pricing basis")
                }
            };
            if units == 0 {
                continue;
            }
            let entry = absolute.entry(key).or_insert(ControlledOperationUnits {
                pricing_basis,
                units: 0,
                event_count: 0,
            });
            if entry.pricing_basis != pricing_basis {
                bail!("one measurement identity has multiple pricing bases");
            }
            entry.units = entry
                .units
                .checked_add(units)
                .ok_or_else(|| anyhow::anyhow!("controlled operation pricing units overflow"))?;
            entry.event_count = entry
                .event_count
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("controlled operation event count overflow"))?;
        }
    }
    Ok(absolute)
}

fn observe_overhead_fixture(
    fixture: &ControlledOverheadFixture,
) -> Result<ControlledOverheadObservation> {
    let trace = trace_shasta_proposal(&fixture.guest_input)?;
    if trace.status != ProposalTraceStatus::Complete
        || !trace.parity.passed
        || !trace.partial_blocks.is_empty()
        || !trace.recovery_failures.is_empty()
    {
        let (stage, error) = trace
            .failure
            .as_ref()
            .map(|failure| (failure.stage.as_str(), failure.error.as_str()))
            .unwrap_or(("unknown", "trace failed without typed failure diagnostics"));
        bail!(
            "controlled overhead fixture {} {:?} did not complete the production trace path: trace stage={stage}, error={error}",
            fixture.case_id,
            fixture.lane,
        );
    }
    let absolute_operation_pricing_units = absolute_transaction_operation_units(&trace)?;
    let transactions = trace.blocks.iter().flat_map(|block| &block.transactions);
    let transaction_rows = transactions.collect::<Vec<_>>();
    let started_candidate_transaction_count = transaction_rows
        .iter()
        .filter(|transaction| {
            !transaction.is_anchor && transaction.disposition != TransactionDisposition::Unattempted
        })
        .count();
    let committed_candidate_transaction_count = transaction_rows
        .iter()
        .filter(|transaction| {
            !transaction.is_anchor
                && matches!(
                    transaction.disposition,
                    TransactionDisposition::CommittedSuccess
                        | TransactionDisposition::CommittedFailure
                )
        })
        .count();
    let unattempted_candidate_transaction_count = transaction_rows
        .iter()
        .filter(|transaction| {
            !transaction.is_anchor && transaction.disposition == TransactionDisposition::Unattempted
        })
        .count();
    let block_count = i64::try_from(trace.blocks.len())?;
    let started_transaction_count = i64::try_from(started_candidate_transaction_count)?;
    let native_value_transfer_count = trace.blocks.iter().try_fold(0i64, |total, block| {
        Ok::<_, anyhow::Error>(total + i64::try_from(block.native_value_transfer_count)?)
    })?;
    let backend_input_sha256 = trace
        .guest_input_sha256
        .trim_start_matches("0x")
        .to_string();
    let workload_spec = controlled_overhead_workload_spec(fixture)?;
    let workload_id = controlled_overhead_workload_id(fixture)?;
    Ok(ControlledOverheadObservation {
        case_id: fixture.case_id.clone(),
        overhead_key_id: fixture.overhead_key_id.clone(),
        lane: fixture.lane,
        baseline_kind: fixture.baseline_kind.clone(),
        target_count: fixture.target_count,
        workload_spec,
        workload_id,
        backend_input_sha256: backend_input_sha256.clone(),
        guest_input_sha256: trace.guest_input_sha256,
        guest_input_bincode_length: trace.guest_input_bincode_length,
        public_output: trace
            .public_output
            .ok_or_else(|| anyhow::anyhow!("complete controlled trace is missing public output"))?,
        absolute_operation_pricing_units,
        absolute_feature_counts: BTreeMap::from([
            ("proposal_startup".into(), 1),
            ("block_base".into(), block_count),
            ("tx_base".into(), started_transaction_count),
            ("native_value_transfer".into(), native_value_transfer_count),
        ]),
        observed_operation_deltas: BTreeMap::new(),
        expected_operation_deltas: fixture.expected_operation_deltas.clone(),
        operation_phase_ownership: "transaction_non_anchor_only",
        system_operation_ownership: "block_base",
        anchor_operation_ownership: "block_base",
        expected_feature_deltas: fixture.expected_feature_deltas.clone(),
        started_candidate_transaction_count,
        committed_candidate_transaction_count,
        unattempted_candidate_transaction_count,
    })
}

pub(crate) fn operation_units_delta(
    target: &BTreeMap<String, ControlledOperationUnits>,
    control: &BTreeMap<String, ControlledOperationUnits>,
) -> Result<BTreeMap<String, ControlledOperationUnits>> {
    target
        .keys()
        .chain(control.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|key| {
            let target_value = target.get(&key);
            let control_value = control.get(&key);
            let pricing_basis = match (target_value, control_value) {
                (Some(target), Some(control)) if target.pricing_basis != control.pricing_basis => {
                    return Some(Err(anyhow::anyhow!(
                        "operation {key} changes pricing basis across the controlled pair"
                    )));
                }
                (Some(target), _) => target.pricing_basis,
                (_, Some(control)) => control.pricing_basis,
                (None, None) => unreachable!(),
            };
            let units = target_value
                .map(|value| value.units)
                .unwrap_or_default()
                .checked_sub(control_value.map(|value| value.units).unwrap_or_default())
                .ok_or_else(|| anyhow::anyhow!("controlled operation pricing units overflow"));
            let event_count = target_value
                .map(|value| value.event_count)
                .unwrap_or_default()
                .checked_sub(
                    control_value
                        .map(|value| value.event_count)
                        .unwrap_or_default(),
                )
                .ok_or_else(|| anyhow::anyhow!("controlled operation event count overflow"));
            let (units, event_count) = match (units, event_count) {
                (Ok(units), Ok(event_count)) => (units, event_count),
                (Err(error), _) | (_, Err(error)) => return Some(Err(error)),
            };
            (units != 0 || event_count != 0).then_some(Ok((
                key,
                ControlledOperationUnits {
                    pricing_basis,
                    units,
                    event_count,
                },
            )))
        })
        .collect()
}

fn signed_map_delta(
    target: &BTreeMap<String, i64>,
    control: &BTreeMap<String, i64>,
) -> BTreeMap<String, i64> {
    target
        .keys()
        .chain(control.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|key| {
            let delta = target.get(&key).copied().unwrap_or_default()
                - control.get(&key).copied().unwrap_or_default();
            (delta != 0).then_some((key, delta))
        })
        .collect()
}

fn expected_feature_delta(
    fixture: &ControlledOverheadFixture,
    observation: &ControlledOverheadObservation,
    control: Option<&ControlledOverheadObservation>,
) -> BTreeMap<String, i64> {
    if fixture.baseline_kind.as_deref() == Some("mathematical_zero_baseline") {
        return observation.absolute_feature_counts.clone();
    }
    let empty = BTreeMap::new();
    let mut delta = signed_map_delta(
        &observation.absolute_feature_counts,
        control
            .map(|value| &value.absolute_feature_counts)
            .unwrap_or(&empty),
    );
    for key in fixture.expected_feature_deltas.keys() {
        delta.entry(key.clone()).or_default();
    }
    delta
}

pub fn validate_required_overhead_fixtures(
    fixtures: &[ControlledOverheadFixture],
) -> Result<Vec<ControlledOverheadObservation>> {
    let mut observations = fixtures
        .iter()
        .map(observe_overhead_fixture)
        .collect::<Result<Vec<_>>>()?;
    for (index, fixture) in fixtures.iter().enumerate() {
        let observation = &observations[index];
        let control = observations.iter().find(|candidate| {
            candidate.case_id == fixture.case_id
                && candidate.lane == ControlledOverheadLane::Control
        });
        let feature_delta = expected_feature_delta(fixture, observation, control);
        if feature_delta != fixture.expected_feature_deltas {
            bail!(
                "controlled overhead feature delta mismatch for {} {:?}: declared={:?}, observed={feature_delta:?}",
                fixture.case_id,
                fixture.lane,
                fixture.expected_feature_deltas,
            );
        }
        let operation_delta =
            if fixture.baseline_kind.as_deref() == Some("mathematical_zero_baseline") {
                observation.absolute_operation_pricing_units.clone()
            } else if fixture.lane == ControlledOverheadLane::Control {
                BTreeMap::new()
            } else {
                let empty = BTreeMap::new();
                operation_units_delta(
                    &observation.absolute_operation_pricing_units,
                    control
                        .map(|value| &value.absolute_operation_pricing_units)
                        .unwrap_or(&empty),
                )?
            };
        observations[index].observed_operation_deltas = operation_delta.clone();
        if operation_delta != fixture.expected_operation_deltas {
            bail!(
                "controlled overhead operation delta mismatch for {} {:?}: declared={:?}, observed={operation_delta:?}",
                fixture.case_id,
                fixture.lane,
                fixture.expected_operation_deltas,
            );
        }
    }
    Ok(observations)
}

#[derive(Clone)]
enum CandidateKind {
    None,
    NoCodeNoValue,
    ControlledContract {
        code: Bytes,
        input: Bytes,
        fee_neutral: bool,
        gas_limit: u64,
    },
    NativeZero,
    NativeValue(u64),
    NativeValueTopology {
        value: u64,
        distinct_recipients: bool,
    },
}

struct BuiltOverheadGuestInput {
    guest_input: GuestInput,
    touched_state_key_count: usize,
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
) -> Result<TransactionSigned> {
    let tx = TxEip1559 {
        chain_id: OVERHEAD_CHAIN_ID,
        nonce,
        gas_limit: OVERHEAD_ANCHOR_GAS_LIMIT,
        max_fee_per_gas: 10_000_000,
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
    Ok(PrivateKeySigner::from_bytes(&B256::repeat_byte(0x11))?)
}

fn controlled_state_holdout_address(domain: &[u8], index: u64) -> Address {
    let mut hasher = Sha256::new();
    hasher.update(b"raiko2-controlled-state-holdout-v1");
    hasher.update(
        u64::try_from(domain.len())
            .expect("fixed holdout domain label length fits u64")
            .to_be_bytes(),
    );
    hasher.update(domain);
    hasher.update(index.to_be_bytes());
    let digest = hasher.finalize();
    Address::from_slice(&digest[12..])
}

fn candidate_transactions(kind: &CandidateKind, count: u64) -> Result<Vec<TransactionSigned>> {
    if matches!(kind, CandidateKind::None) {
        return Ok(Vec::new());
    }
    let value = match kind {
        CandidateKind::NativeValue(value) | CandidateKind::NativeValueTopology { value, .. } => {
            U256::from(*value)
        }
        _ => U256::ZERO,
    };
    let signer = controlled_candidate_signer()?;
    let input = match kind {
        CandidateKind::ControlledContract { input, .. } => input.clone(),
        _ => Bytes::new(),
    };
    let gas_limit = match kind {
        CandidateKind::ControlledContract { gas_limit, .. } => *gas_limit,
        _ => 100_000,
    };
    (0..count)
        .map(|nonce| {
            let recipient = match kind {
                CandidateKind::ControlledContract { .. } => Address::repeat_byte(0x66),
                CandidateKind::NativeValueTopology {
                    distinct_recipients,
                    ..
                } => controlled_state_holdout_address(
                    b"dirty-accounts-recipient",
                    if *distinct_recipients { nonce } else { 0 },
                ),
                _ => Address::repeat_byte(0x55),
            };
            let tx = TxEip1559 {
                chain_id: OVERHEAD_CHAIN_ID,
                nonce,
                gas_limit,
                max_fee_per_gas: 25_000_000,
                max_priority_fee_per_gas: 0,
                to: TxKind::Call(recipient),
                value,
                access_list: Default::default(),
                input: input.clone(),
            };
            let signature = signer.sign_hash_sync(&tx.signature_hash())?;
            Ok(tx.into_signed(signature).into())
        })
        .collect()
}

#[derive(Clone, Default)]
struct ControlledTrieState {
    accounts: BTreeMap<B256, TrieAccount>,
    storages: BTreeMap<B256, BTreeMap<B256, U256>>,
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

    fn topology_key_count(&self) -> usize {
        self.accounts.len() + self.storages.values().map(BTreeMap::len).sum::<usize>()
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
            if storage_changes.wiped {
                storage.clear();
            }
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

fn controlled_anchor_code(anchor_address: Address) -> Result<Bytes> {
    // This source-pinned artifact is the exact pre-Unzen Anchor ABI used by the production
    // Shasta path. The builder patches the same constructor immutables that deployment would.
    let artifact: Value = serde_json::from_str(include_str!(
        "../../../scripts/regression/shasta/Anchor.json"
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
        address_word(CONTROLLED_CHECKPOINT_STORE),
    )?;
    patch_anchor_immutable(&mut code, &artifact, "378", immutable_u64_word(1))?;
    Ok(code.into())
}

fn overhead_prestate(
    chain_spec: &raiko2_primitives::ChainSpec,
    candidates: &[TransactionSigned],
    controlled_contract_code: Option<&Bytes>,
    extra_prestate_accounts: &[Address],
) -> Result<(ControlledTrieState, Vec<Bytes>)> {
    let anchor_address = chain_spec
        .l2_contract
        .ok_or_else(|| anyhow::anyhow!("controlled chain is missing Anchor address"))?;
    let anchor_code = controlled_anchor_code(anchor_address)?;
    let checkpoint_store_code = Bytes::from_static(&[0x00]);
    let mut state = ControlledTrieState::default();
    state.insert_account(anchor_address, U256::ZERO, 0, keccak256(&anchor_code));
    state.insert_account(
        CONTROLLED_CHECKPOINT_STORE,
        U256::ZERO,
        0,
        keccak256(&checkpoint_store_code),
    );
    let mut codes = vec![anchor_code, checkpoint_store_code];
    if let Some(candidate) = candidates.first() {
        let signer = candidate
            .recover_signer()
            .map_err(|_| anyhow::anyhow!("controlled candidate signature is unrecoverable"))?;
        state.insert_account(signer, CONTROLLED_CANDIDATE_FINAL_BALANCE, 0, KECCAK_EMPTY);
        let code_hash = if let Some(code) = controlled_contract_code {
            codes.push(code.clone());
            keccak256(code)
        } else {
            KECCAK_EMPTY
        };
        for candidate in candidates {
            let recipient = candidate
                .to()
                .ok_or_else(|| anyhow::anyhow!("controlled candidate must be a call"))?;
            state.insert_account(recipient, U256::ZERO, 0, code_hash);
        }
    }
    for address in extra_prestate_accounts {
        state.insert_account(*address, U256::from(1), 0, KECCAK_EMPTY);
    }
    Ok((state, codes))
}

fn kona_blob(payload: &[u8]) -> Result<Vec<u8>> {
    // Fixture-only mirror of taiko-mono's decoder layout. This stays host-side and its output is
    // accepted only after the production blob/KZG verifier and manifest decoder consume it.
    const ROUNDS: usize = 1024;
    const MAX_PAYLOAD: usize = (4 * 31 + 3) * ROUNDS - 4;
    if payload.is_empty() || payload.len() > MAX_PAYLOAD {
        bail!("controlled manifest does not fit one Kona blob");
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

fn build_overhead_guest_input_with_topology(
    kind: CandidateKind,
    block_count: usize,
    candidate_count: u64,
    extra_prestate_accounts: &[Address],
) -> Result<BuiltOverheadGuestInput> {
    if !(1..=768).contains(&block_count) {
        bail!("controlled overhead fixture exceeds the frozen Unzen block bound");
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
    let candidates = candidate_transactions(&kind, candidate_count)?;
    let controlled_contract_code = match &kind {
        CandidateKind::ControlledContract { code, .. } => Some(code),
        _ => None,
    };
    let fee_neutral = matches!(
        &kind,
        CandidateKind::ControlledContract {
            fee_neutral: true,
            ..
        }
    );
    let base_fee_per_gas = 10_000_000;
    let (mut controlled_state, codes) = overhead_prestate(
        &chain_spec,
        &candidates,
        controlled_contract_code,
        extra_prestate_accounts,
    )?;
    let (mut prestate_root, mut state_nodes) = controlled_state.witness();
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
    let fee_recipient = if fee_neutral {
        controlled_candidate_signer()?.address()
    } else {
        proposer
    };
    let base_fee_share_pctg = if fee_neutral { 100 } else { 7 };
    let extra_data = encode_extra_data(base_fee_share_pctg, 42);
    let mut ancestor_headers = ancestor_headers;
    let proposal_ancestor_headers = ancestor_headers.clone();
    let mut parent = parent_header;
    let mut witnesses = Vec::with_capacity(block_count);
    let mut manifest_blocks = Vec::with_capacity(block_count);

    for index in 0..block_count {
        let block_number = OVERHEAD_BLOCK_NUMBER + u64::try_from(index)?;
        let block_timestamp = overhead_parent_timestamp
            .checked_add(1 + u64::try_from(index)?)
            .ok_or_else(|| anyhow::anyhow!("controlled overhead block timestamp overflow"))?;
        let transactions = if index == 0 {
            candidates.clone()
        } else {
            Vec::new()
        };
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
        let anchor = anchor_tx(&checkpoint, anchor_address, anchor_nonce)?;
        let outcome = reconstruct_block_from_transactions_with_witness_resources(
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
        )?;
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
        let hashed_state = format!("{:?}", outcome.hashed_state);
        let block = outcome.filtered_block.into_block();
        if block.header.state_root != post_state_root {
            bail!(
                "controlled fixture hashed post-state differs from rebuilt state: pre={prestate_root}, post={}, rebuilt={post_state_root}, hashed={hashed_state}",
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
    let mut guest_input = GuestInput {
        witnesses,
        proposal_ancestor_headers,
        taiko: TaikoManifest {
            proposal_id: 42,
            l1_header: l1_header.clone(),
            l1_ancestor_headers: vec![l1_header.clone()],
            proposal_event: raiko2_protocol_shasta::shasta::ShastaEventData {
                proposal: raiko2_protocol_shasta::shasta::Proposal {
                    id: 42u64.try_into()?,
                    proposer,
                    timestamp: overhead_parent_timestamp
                        .checked_add(u64::try_from(block_count)?)
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
    if fee_neutral
        && controlled_state.account_balance(controlled_candidate_signer()?.address())
            != Some(CONTROLLED_CANDIDATE_FINAL_BALANCE)
    {
        bail!("fee-neutral controlled sender did not finish at the frozen balance");
    }
    Ok(BuiltOverheadGuestInput {
        guest_input,
        touched_state_key_count: controlled_state.topology_key_count(),
    })
}

fn build_overhead_guest_input(
    kind: CandidateKind,
    block_count: usize,
    candidate_count: u64,
) -> Result<GuestInput> {
    Ok(
        build_overhead_guest_input_with_topology(kind, block_count, candidate_count, &[])?
            .guest_input,
    )
}

const CONTROLLED_BLOCK_BYTECODE_LENGTH: usize = 256;
const CONTROLLED_OPCODE_GAS_LIMIT: u64 = 100_000;
const CONTROLLED_OPCODE_MAX_COUNT: u64 = 32;
fn controlled_opcode_bytecode(family: &str, scenario: &str, count: u64) -> Result<Bytes> {
    let count = u32::try_from(count)?;
    if count > 0x00ff_ffff {
        bail!("controlled opcode count does not fit frozen PUSH3 encoding");
    }
    let count_bytes = count.to_be_bytes();
    let mut code = vec![
        0x62,
        count_bytes[1],
        count_bytes[2],
        count_bytes[3], // PUSH3 fixed-width count immediate
    ];
    if (family, scenario) == ("static_count_control", "push3_pop_fixed_pop") {
        code.extend([0x50, 0x60, 0x00, 0x50, 0x00]); // POP count; PUSH1 0; POP; STOP
        code.resize(CONTROLLED_BLOCK_BYTECODE_LENGTH, 0x00);
        return Ok(code.into());
    }
    let loop_offset = code.len();
    code.extend([0x5b, 0x80, 0x15, 0x60, 0x00, 0x57]); // JUMPDEST; DUP1; ISZERO; PUSH1 done; JUMPI
    let done_immediate_index = loop_offset + 4;
    match (family, scenario) {
        ("pop_family", "push0_pop") => {
            code.extend([0x60, 0x00, 0x50]); // PUSH1 0; POP
        }
        ("push_family", "push0_stack") => {
            code.extend([0x5f, 0x90]); // PUSH0; SWAP1, retaining the counter on top
        }
        ("dup_family", "push0_dup1_then_pop") => {
            code.extend([0x80, 0x50]); // DUP1; POP
        }
        ("swap_family", "push0_pair_swap1_then_pop") => {
            code.extend([0x80, 0x5f, 0x90, 0x50, 0x50]); // DUP1; PUSH0; SWAP1; POP; POP
        }
        _ => bail!("unknown controlled opcode family/scenario {family}/{scenario}"),
    }
    code.extend([
        0x60,
        0x01,
        0x90,
        0x03, // PUSH1 1; SWAP1; SUB
        0x60,
        u8::try_from(loop_offset)?,
        0x56, // PUSH1 loop; JUMP
    ]);
    let done_offset = code.len();
    code.extend([0x5b, 0x50, 0x00]); // JUMPDEST; POP; STOP
    code[done_immediate_index] = u8::try_from(done_offset)?;
    if code.len() > CONTROLLED_BLOCK_BYTECODE_LENGTH {
        bail!("controlled opcode bytecode exceeds frozen code-length class");
    }
    code.resize(CONTROLLED_BLOCK_BYTECODE_LENGTH, 0x00);
    Ok(code.into())
}

pub fn build_controlled_block_fixture(
    spec: &ControlledBlockRowSpec,
) -> Result<ControlledBlockFixture> {
    build_controlled_block_fixture_with_topology(spec, &[])
}

fn build_controlled_block_fixture_with_topology(
    spec: &ControlledBlockRowSpec,
    extra_prestate_accounts: &[Address],
) -> Result<ControlledBlockFixture> {
    let derived_row_id = controlled_block_row_id(spec)?;
    if spec.row_id != derived_row_id {
        bail!(
            "controlled block row ID differs from semantic content: declared={}, derived={derived_row_id}",
            spec.row_id
        );
    }
    let (kind, bytecode_length) = match &spec.program {
        ControlledProgram::Empty => {
            if spec.transaction_count != 0 {
                bail!("empty controlled program requires zero transactions");
            }
            (CandidateKind::None, 0)
        }
        ControlledProgram::NativeTransfer { value } => (
            if *value == 0 {
                CandidateKind::NativeZero
            } else {
                CandidateKind::NativeValue(*value)
            },
            0,
        ),
        ControlledProgram::OpcodeLoop {
            family,
            count,
            scenario,
        } => {
            if !(1..=CONTROLLED_OPCODE_MAX_COUNT).contains(count) {
                bail!("controlled opcode count is outside the frozen 1..=32 range");
            }
            if family != &spec.workload_family && spec.workload_family != "proposal_startup" {
                bail!("controlled opcode program family differs from workload family");
            }
            if spec.transaction_count == 0 {
                bail!("controlled opcode program requires at least one transaction");
            }
            let bytecode = controlled_opcode_bytecode(family, scenario, *count)?;
            let bytecode_length = bytecode.len();
            (
                CandidateKind::ControlledContract {
                    code: bytecode,
                    input: Bytes::new(),
                    fee_neutral: true,
                    gas_limit: CONTROLLED_OPCODE_GAS_LIMIT,
                },
                bytecode_length,
            )
        }
    };
    let built = build_overhead_guest_input_with_topology(
        kind,
        spec.block_count,
        spec.transaction_count,
        extra_prestate_accounts,
    )?;
    Ok(ControlledBlockFixture {
        spec: spec.clone(),
        guest_input: built.guest_input,
        controlled_bytecode_length: bytecode_length,
        touched_state_key_count: built.touched_state_key_count,
    })
}

#[cfg(test)]
pub fn build_controlled_block_fixture_with_extra_prestate_account_for_test(
    spec: &ControlledBlockRowSpec,
    address: Address,
) -> Result<ControlledBlockFixture> {
    build_controlled_block_fixture_with_topology(spec, &[address])
}

pub fn observe_controlled_block_fixture(
    fixture: &ControlledBlockFixture,
) -> Result<ControlledBlockObservation> {
    let trace = trace_shasta_proposal(&fixture.guest_input)?;
    if trace.status != ProposalTraceStatus::Complete
        || !trace.parity.passed
        || !trace.partial_blocks.is_empty()
        || !trace.recovery_failures.is_empty()
    {
        let (stage, error) = trace
            .failure
            .as_ref()
            .map(|failure| (failure.stage.as_str(), failure.error.as_str()))
            .unwrap_or(("unknown", "trace failed without typed failure diagnostics"));
        bail!(
            "controlled block row {} did not complete the production trace path: trace stage={stage}, error={error}",
            fixture.spec.row_id
        );
    }
    let operation_units = absolute_transaction_operation_units(&trace)?;
    if operation_units
        .values()
        .any(|units| units.pricing_basis != PricingBasis::RawGasSlope)
    {
        bail!("controlled block row contains spawned fixed-per-event work");
    }
    let actual_raw_gas_by_key = operation_units
        .into_iter()
        .map(|(key, value)| (key, value.units))
        .collect::<BTreeMap<_, _>>();
    if actual_raw_gas_by_key
        .keys()
        .any(|key| key.starts_with("precompile:") || key.ends_with(":spawned"))
    {
        bail!("controlled block row contains precompile or spawned work");
    }

    let started_candidate_transactions = trace
        .blocks
        .iter()
        .flat_map(|block| &block.transactions)
        .filter(|transaction| {
            !transaction.is_anchor && transaction.disposition != TransactionDisposition::Unattempted
        })
        .count();
    let native_value_transfers = trace
        .blocks
        .iter()
        .map(|block| block.native_value_transfer_count)
        .sum::<usize>();
    let actual_features = BTreeMap::from([
        ("proposal_startup".into(), 1),
        ("block_base".into(), i64::try_from(trace.blocks.len())?),
        (
            "tx_base".into(),
            i64::try_from(started_candidate_transactions)?,
        ),
        (
            "native_value_transfer".into(),
            i64::try_from(native_value_transfers)?,
        ),
    ]);
    let witness_node_count = fixture
        .guest_input
        .witnesses
        .iter()
        .map(|witness| witness.witness.state.len())
        .sum::<usize>();
    let witness_byte_count = fixture
        .guest_input
        .witnesses
        .iter()
        .flat_map(|witness| &witness.witness.state)
        .map(|node| node.bytes.len())
        .sum::<usize>();
    let blob_count = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.tx_data_from_blob.len())
        .sum::<usize>();
    let kzg_invocation_count = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.blob_commitments.len())
        .sum::<usize>();
    let calldata_length = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.tx_data_from_calldata.len())
        .sum::<usize>();
    let actual_diagnostics = BTreeMap::from([
        (
            "guest_input_bincode_length".into(),
            i64::try_from(trace.guest_input_bincode_length)?,
        ),
        (
            "witness_node_count".into(),
            i64::try_from(witness_node_count)?,
        ),
        (
            "witness_byte_count".into(),
            i64::try_from(witness_byte_count)?,
        ),
        ("blob_count".into(), i64::try_from(blob_count)?),
        (
            "kzg_invocation_count".into(),
            i64::try_from(kzg_invocation_count)?,
        ),
        ("calldata_length".into(), i64::try_from(calldata_length)?),
        (
            "bytecode_length".into(),
            i64::try_from(fixture.controlled_bytecode_length)?,
        ),
        (
            "touched_state_key_count".into(),
            i64::try_from(fixture.touched_state_key_count)?,
        ),
    ]);
    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(OVERHEAD_CHAIN_ID)
        .ok_or_else(|| anyhow::anyhow!("missing controlled Taiko chain spec"))?;
    let unzen_activation_timestamp =
        match chain_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)) {
            Some(ForkCondition::Timestamp(timestamp)) => *timestamp,
            _ => bail!("controlled Taiko chain is missing timestamp-based Unzen activation"),
        };
    let minimum_block_timestamp = fixture
        .guest_input
        .witnesses
        .iter()
        .map(|witness| witness.block.header.timestamp)
        .min()
        .ok_or_else(|| anyhow::anyhow!("controlled block row has no witnesses"))?;
    if minimum_block_timestamp <= unzen_activation_timestamp {
        bail!("controlled block row is not strictly post-Unzen");
    }
    Ok(ControlledBlockObservation {
        row_id: fixture.spec.row_id.clone(),
        workload_family: fixture.spec.workload_family.clone(),
        split: fixture.spec.split,
        backend_input_sha256: trace
            .guest_input_sha256
            .trim_start_matches("0x")
            .to_string(),
        guest_input_bincode_length: trace.guest_input_bincode_length,
        public_output: trace
            .public_output
            .ok_or_else(|| anyhow::anyhow!("complete controlled trace is missing public output"))?,
        actual_final_state_root: fixture
            .guest_input
            .witnesses
            .last()
            .ok_or_else(|| anyhow::anyhow!("controlled block row has no final witness"))?
            .block
            .header
            .state_root,
        actual_raw_gas_by_key,
        actual_features,
        actual_diagnostics,
        unzen_activation_timestamp,
        minimum_block_timestamp,
        operation_phase_ownership: "transaction_non_anchor_only",
        system_operation_ownership: "block_base",
        anchor_operation_ownership: "block_base",
    })
}

pub fn validate_controlled_block_fixture(
    fixture: &ControlledBlockFixture,
) -> Result<ControlledBlockObservation> {
    let observation = observe_controlled_block_fixture(fixture)?;
    if observation.actual_raw_gas_by_key != fixture.spec.expected_raw_gas_by_key {
        bail!(
            "controlled block row {} raw-gas mismatch: declared={:?}, observed={:?}",
            fixture.spec.row_id,
            fixture.spec.expected_raw_gas_by_key,
            observation.actual_raw_gas_by_key
        );
    }
    if observation.actual_features != fixture.spec.expected_features {
        bail!(
            "controlled block row {} feature mismatch: declared={:?}, observed={:?}",
            fixture.spec.row_id,
            fixture.spec.expected_features,
            observation.actual_features
        );
    }
    if observation.actual_diagnostics != fixture.spec.expected_diagnostics {
        bail!(
            "controlled block row {} diagnostic mismatch: declared={:?}, observed={:?}",
            fixture.spec.row_id,
            fixture.spec.expected_diagnostics,
            observation.actual_diagnostics
        );
    }
    if observation.actual_final_state_root != fixture.spec.expected_final_state_root {
        bail!(
            "controlled block row {} final-state mismatch: declared={}, observed={}",
            fixture.spec.row_id,
            fixture.spec.expected_final_state_root,
            observation.actual_final_state_root,
        );
    }
    Ok(observation)
}

impl ControlledStateHoldoutPairSpec {
    fn validate_and_normalize(&self) -> Result<(&str, ControlledStateHoldout)> {
        match self {
            Self::WitnessTopology {
                pair_id,
                scale,
                control,
                target,
            } => {
                if !matches!(scale, 1 | 8 | 32) {
                    bail!("witness topology scale must be one of 1, 8, or 32");
                }
                let expected_pair_id = format!("witness_topology_{scale}");
                if pair_id != &expected_pair_id {
                    bail!("witness topology pair_id must be {expected_pair_id}, got {pair_id}");
                }
                if control.extra_account_count != 0 || target.extra_account_count != *scale {
                    bail!(
                        "witness topology {pair_id} must use control extra_account_count 0 and target extra_account_count {scale}"
                    );
                }
                Ok((
                    pair_id,
                    ControlledStateHoldout::WitnessTopology {
                        extra_account_count: *scale,
                    },
                ))
            }
            Self::DirtyAccounts {
                pair_id,
                scale,
                control,
                target,
            } => {
                if !matches!(scale, 2 | 8 | 32) {
                    bail!("dirty-account scale must be one of 2, 8, or 32");
                }
                let expected_pair_id = format!("dirty_accounts_{scale}");
                if pair_id != &expected_pair_id {
                    bail!("dirty-account pair_id must be {expected_pair_id}, got {pair_id}");
                }
                if control.transaction_count != *scale || target.transaction_count != *scale {
                    bail!(
                        "dirty-account {pair_id} control and target transaction_count must equal scale {scale}"
                    );
                }
                if control.value != 1 || target.value != 1 {
                    bail!("dirty-account {pair_id} control and target value must equal 1");
                }
                if control.recipient_mode != DirtyAccountsRecipientMode::Single
                    || target.recipient_mode != DirtyAccountsRecipientMode::Distinct
                {
                    bail!(
                        "dirty-account {pair_id} requires single-recipient control and distinct-recipient target"
                    );
                }
                Ok((
                    pair_id,
                    ControlledStateHoldout::DirtyAccounts {
                        transaction_count: *scale,
                        value: 1,
                    },
                ))
            }
        }
    }
}

pub fn build_controlled_state_holdout_fixtures(
    spec: &ControlledStateHoldoutPairSpec,
) -> Result<Vec<ControlledStateHoldoutFixture>> {
    let (pair_id, normalized) = spec.validate_and_normalize()?;
    let (control, target) = match normalized {
        ControlledStateHoldout::WitnessTopology {
            extra_account_count,
        } => {
            let extra_accounts = (0..extra_account_count)
                .map(|index| {
                    controlled_state_holdout_address(
                        b"witness-topology-extra-account",
                        u64::try_from(index).expect("frozen witness scale fits u64"),
                    )
                })
                .collect::<Vec<_>>();
            (
                build_overhead_guest_input_with_topology(CandidateKind::None, 1, 0, &[])?,
                build_overhead_guest_input_with_topology(
                    CandidateKind::None,
                    1,
                    0,
                    &extra_accounts,
                )?,
            )
        }
        ControlledStateHoldout::DirtyAccounts {
            transaction_count,
            value,
        } => (
            build_overhead_guest_input_with_topology(
                CandidateKind::NativeValueTopology {
                    value,
                    distinct_recipients: false,
                },
                1,
                transaction_count,
                &[],
            )?,
            build_overhead_guest_input_with_topology(
                CandidateKind::NativeValueTopology {
                    value,
                    distinct_recipients: true,
                },
                1,
                transaction_count,
                &[],
            )?,
        ),
    };
    Ok(vec![
        ControlledStateHoldoutFixture {
            spec: spec.clone(),
            pair_id: pair_id.to_string(),
            lane: ControlledStateHoldoutLane::Control,
            guest_input: control.guest_input,
            touched_state_key_count: control.touched_state_key_count,
        },
        ControlledStateHoldoutFixture {
            spec: spec.clone(),
            pair_id: pair_id.to_string(),
            lane: ControlledStateHoldoutLane::Target,
            guest_input: target.guest_input,
            touched_state_key_count: target.touched_state_key_count,
        },
    ])
}

fn observe_controlled_state_holdout_fixture(
    fixture: &ControlledStateHoldoutFixture,
) -> Result<ControlledStateHoldoutObservation> {
    let trace = trace_shasta_proposal(&fixture.guest_input)?;
    if trace.status != ProposalTraceStatus::Complete
        || !trace.parity.passed
        || !trace.partial_blocks.is_empty()
        || !trace.recovery_failures.is_empty()
    {
        let (stage, error) = trace
            .failure
            .as_ref()
            .map(|failure| (failure.stage.as_str(), failure.error.as_str()))
            .unwrap_or(("unknown", "trace failed without typed failure diagnostics"));
        bail!(
            "controlled state holdout {} {:?} did not complete the production trace path: trace stage={stage}, error={error}",
            fixture.pair_id,
            fixture.lane,
        );
    }
    let operation_units = absolute_transaction_operation_units(&trace)?;
    if !operation_units.is_empty() {
        bail!(
            "controlled state holdout {} {:?} contains transaction operation work: {:?}",
            fixture.pair_id,
            fixture.lane,
            operation_units.keys().collect::<Vec<_>>(),
        );
    }
    let actual_raw_gas_by_key = BTreeMap::new();
    let candidate_transactions = trace
        .blocks
        .iter()
        .flat_map(|block| &block.transactions)
        .filter(|transaction| !transaction.is_anchor)
        .collect::<Vec<_>>();
    let started_candidate_transaction_count = u64::try_from(
        candidate_transactions
            .iter()
            .filter(|transaction| transaction.disposition != TransactionDisposition::Unattempted)
            .count(),
    )?;
    let committed_candidate_transaction_count = u64::try_from(
        candidate_transactions
            .iter()
            .filter(|transaction| {
                transaction.disposition == TransactionDisposition::CommittedSuccess
            })
            .count(),
    )?;
    let unattempted_candidate_transaction_count = u64::try_from(
        candidate_transactions
            .iter()
            .filter(|transaction| transaction.disposition == TransactionDisposition::Unattempted)
            .count(),
    )?;
    let native_value_transfer_count = trace.blocks.iter().try_fold(0u64, |total, block| {
        total
            .checked_add(u64::try_from(block.native_value_transfer_count)?)
            .ok_or_else(|| anyhow::anyhow!("native value transfer count overflow"))
    })?;
    let actual_features = BTreeMap::from([
        ("proposal_startup".into(), 1),
        ("block_base".into(), i64::try_from(trace.blocks.len())?),
        (
            "tx_base".into(),
            i64::try_from(started_candidate_transaction_count)?,
        ),
        (
            "native_value_transfer".into(),
            i64::try_from(native_value_transfer_count)?,
        ),
    ]);
    let witness_node_count = fixture
        .guest_input
        .witnesses
        .iter()
        .map(|witness| witness.witness.state.len())
        .sum::<usize>();
    let witness_byte_count = fixture
        .guest_input
        .witnesses
        .iter()
        .flat_map(|witness| &witness.witness.state)
        .map(|node| node.bytes.len())
        .sum::<usize>();
    let blob_count = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.tx_data_from_blob.len())
        .sum::<usize>();
    let kzg_invocation_count = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.blob_commitments.len())
        .sum::<usize>();
    let calldata_length = fixture
        .guest_input
        .taiko
        .data_sources
        .iter()
        .map(|source| source.tx_data_from_calldata.len())
        .sum::<usize>();
    let actual_diagnostics = BTreeMap::from([
        (
            "guest_input_bincode_length".into(),
            i64::try_from(trace.guest_input_bincode_length)?,
        ),
        (
            "witness_node_count".into(),
            i64::try_from(witness_node_count)?,
        ),
        (
            "witness_byte_count".into(),
            i64::try_from(witness_byte_count)?,
        ),
        ("blob_count".into(), i64::try_from(blob_count)?),
        (
            "kzg_invocation_count".into(),
            i64::try_from(kzg_invocation_count)?,
        ),
        ("calldata_length".into(), i64::try_from(calldata_length)?),
        ("bytecode_length".into(), 0),
        (
            "touched_state_key_count".into(),
            i64::try_from(fixture.touched_state_key_count)?,
        ),
    ]);
    Ok(ControlledStateHoldoutObservation {
        pair_id: fixture.pair_id.clone(),
        lane: fixture.lane,
        backend_input_sha256: trace
            .guest_input_sha256
            .trim_start_matches("0x")
            .to_string(),
        guest_input_sha256: trace.guest_input_sha256,
        guest_input_bincode_length: trace.guest_input_bincode_length,
        public_output: trace
            .public_output
            .ok_or_else(|| anyhow::anyhow!("complete controlled trace is missing public output"))?,
        actual_final_state_root: fixture
            .guest_input
            .witnesses
            .last()
            .ok_or_else(|| anyhow::anyhow!("controlled state holdout has no final witness"))?
            .block
            .header
            .state_root,
        actual_raw_gas_by_key,
        actual_features,
        actual_diagnostics,
        started_candidate_transaction_count,
        committed_candidate_transaction_count,
        unattempted_candidate_transaction_count,
        operation_phase_ownership: "transaction_non_anchor_only",
        system_operation_ownership: "block_base",
        anchor_operation_ownership: "block_base",
    })
}

pub fn validate_controlled_state_holdout_fixtures(
    fixtures: &[ControlledStateHoldoutFixture],
) -> Result<Vec<ControlledStateHoldoutObservation>> {
    let [control_fixture, target_fixture] = fixtures else {
        bail!("controlled state holdout requires exactly one control and one target fixture");
    };
    if control_fixture.lane != ControlledStateHoldoutLane::Control
        || target_fixture.lane != ControlledStateHoldoutLane::Target
        || control_fixture.spec != target_fixture.spec
        || control_fixture.pair_id != target_fixture.pair_id
    {
        bail!("controlled state holdout pair identity or lane ordering differs");
    }
    let control = observe_controlled_state_holdout_fixture(control_fixture)?;
    let target = observe_controlled_state_holdout_fixture(target_fixture)?;
    if target.actual_features != control.actual_features {
        bail!("controlled state holdout changed the frozen four-term feature vector");
    }
    if target.actual_raw_gas_by_key != control.actual_raw_gas_by_key {
        bail!("controlled state holdout changed the frozen operation-cost vector");
    }
    let (_, normalized) = control_fixture.spec.validate_and_normalize()?;
    match normalized {
        ControlledStateHoldout::WitnessTopology { .. } => {
            let expected_features = BTreeMap::from([
                ("proposal_startup".into(), 1),
                ("block_base".into(), 1),
                ("tx_base".into(), 0),
                ("native_value_transfer".into(), 0),
            ]);
            if control.actual_features != expected_features {
                bail!("witness topology holdout changed its frozen feature counts");
            }
            if control.started_candidate_transaction_count != 0
                || target.started_candidate_transaction_count != 0
            {
                bail!("witness topology holdout must not contain candidate transactions");
            }
            if target.actual_diagnostics["witness_node_count"]
                <= control.actual_diagnostics["witness_node_count"]
                || target.actual_diagnostics["witness_byte_count"]
                    <= control.actual_diagnostics["witness_byte_count"]
            {
                bail!("witness topology target did not increase witness topology");
            }
        }
        ControlledStateHoldout::DirtyAccounts {
            transaction_count, ..
        } => {
            let expected_count = i64::try_from(transaction_count)?;
            let expected_features = BTreeMap::from([
                ("proposal_startup".into(), 1),
                ("block_base".into(), 1),
                ("tx_base".into(), expected_count),
                ("native_value_transfer".into(), expected_count),
            ]);
            if control.actual_features != expected_features {
                bail!("dirty-account holdout changed its frozen feature counts");
            }
            if control.started_candidate_transaction_count != transaction_count
                || target.started_candidate_transaction_count != transaction_count
                || control.committed_candidate_transaction_count != transaction_count
                || target.committed_candidate_transaction_count != transaction_count
                || control.unattempted_candidate_transaction_count != 0
                || target.unattempted_candidate_transaction_count != 0
            {
                bail!("dirty-account holdout transactions did not all commit successfully");
            }
            if target.actual_diagnostics["touched_state_key_count"]
                <= control.actual_diagnostics["touched_state_key_count"]
            {
                bail!("dirty-account target did not increase touched state topology");
            }
        }
    }
    Ok(vec![control, target])
}

pub fn build_required_overhead_fixtures(
    target_count: u64,
) -> Result<Vec<ControlledOverheadFixture>> {
    let one_block = build_overhead_guest_input(CandidateKind::None, 1, 0)?;
    let target_blocks = build_overhead_guest_input(
        CandidateKind::None,
        usize::try_from(target_count.saturating_add(1))?,
        0,
    )?;
    let no_code = build_overhead_guest_input(CandidateKind::NoCodeNoValue, 1, target_count)?;
    let no_code_one = build_overhead_guest_input(CandidateKind::NoCodeNoValue, 1, 1)?;
    let contract = build_overhead_guest_input(
        CandidateKind::ControlledContract {
            code: Bytes::from_static(&[0x5f]),
            input: Bytes::new(),
            fee_neutral: false,
            gas_limit: 100_000,
        },
        1,
        target_count,
    )?;
    let native_zero = build_overhead_guest_input(CandidateKind::NativeZero, 1, target_count)?;
    let native_value = build_overhead_guest_input(CandidateKind::NativeValue(1), 1, target_count)?;
    let empty_operations = BTreeMap::new();
    let count_delta = i64::try_from(target_count)?;
    let push0_units = count_delta
        .checked_mul(2)
        .ok_or_else(|| anyhow::anyhow!("controlled PUSH0 raw-gas units overflow"))?;
    let contract_operation_deltas = if push0_units == 0 {
        BTreeMap::new()
    } else {
        BTreeMap::from([(
            "opcode:0x5f".into(),
            ControlledOperationUnits {
                pricing_basis: PricingBasis::RawGasSlope,
                units: push0_units,
                event_count: count_delta,
            },
        )])
    };
    let fixtures = vec![
        ControlledOverheadFixture {
            case_id: "tx_base_no_code_no_value".into(),
            overhead_key_id: "tx_base".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::from([("tx_base".into(), count_delta)]),
            guest_input: no_code.clone(),
        },
        ControlledOverheadFixture {
            case_id: "tx_base_no_code_no_value".into(),
            overhead_key_id: "tx_base".into(),
            lane: ControlledOverheadLane::Control,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::new(),
            guest_input: one_block.clone(),
        },
        ControlledOverheadFixture {
            case_id: "tx_base_minimal_contract_call".into(),
            overhead_key_id: "tx_base".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: None,
            target_count,
            // PUSH0 costs two raw-gas units per execution. REVM's implicit STOP at EOF is
            // observable but has zero pricing units, so it is intentionally absent.
            expected_operation_deltas: contract_operation_deltas,
            expected_feature_deltas: BTreeMap::from([("tx_base".into(), count_delta)]),
            guest_input: contract,
        },
        ControlledOverheadFixture {
            case_id: "tx_base_minimal_contract_call".into(),
            overhead_key_id: "tx_base".into(),
            lane: ControlledOverheadLane::Control,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::new(),
            guest_input: one_block.clone(),
        },
        ControlledOverheadFixture {
            case_id: "native_transfer_positive_vs_zero".into(),
            overhead_key_id: "native_value_transfer".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::from([
                ("native_value_transfer".into(), count_delta),
                ("tx_base".into(), 0),
            ]),
            guest_input: native_value,
        },
        ControlledOverheadFixture {
            case_id: "native_transfer_positive_vs_zero".into(),
            overhead_key_id: "native_value_transfer".into(),
            lane: ControlledOverheadLane::Control,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::new(),
            guest_input: native_zero,
        },
        ControlledOverheadFixture {
            case_id: "block_base_one_vs_two_minimal_blocks".into(),
            overhead_key_id: "block_base".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::from([
                ("block_base".into(), count_delta),
                ("native_value_transfer".into(), 0),
                ("tx_base".into(), 0),
            ]),
            guest_input: target_blocks,
        },
        ControlledOverheadFixture {
            case_id: "block_base_one_vs_two_minimal_blocks".into(),
            overhead_key_id: "block_base".into(),
            lane: ControlledOverheadLane::Control,
            baseline_kind: None,
            target_count,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::new(),
            guest_input: one_block.clone(),
        },
        ControlledOverheadFixture {
            case_id: "startup_minimal_no_candidate_tx".into(),
            overhead_key_id: "proposal_startup".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: Some("mathematical_zero_baseline".into()),
            target_count: 1,
            expected_operation_deltas: empty_operations.clone(),
            expected_feature_deltas: BTreeMap::from([
                ("proposal_startup".into(), 1),
                ("block_base".into(), 1),
                ("native_value_transfer".into(), 0),
                ("tx_base".into(), 0),
            ]),
            guest_input: one_block,
        },
        ControlledOverheadFixture {
            case_id: "startup_minimal_one_no_code_tx".into(),
            overhead_key_id: "proposal_startup".into(),
            lane: ControlledOverheadLane::Target,
            baseline_kind: Some("mathematical_zero_baseline".into()),
            target_count: 1,
            expected_operation_deltas: empty_operations,
            expected_feature_deltas: BTreeMap::from([
                ("proposal_startup".into(), 1),
                ("block_base".into(), 1),
                ("native_value_transfer".into(), 0),
                ("tx_base".into(), 1),
            ]),
            guest_input: no_code_one,
        },
    ];
    Ok(fixtures)
}

fn controlled_lane(lane: PrecompileLabLane) -> ControlledLane {
    match lane {
        PrecompileLabLane::Target => ControlledLane::Target,
        PrecompileLabLane::Control => ControlledLane::Control,
    }
}

fn precompile_pair_spec(input: &PrecompileLabInput) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("case_id".into(), Value::String(input.case.clone())),
        (
            "input".into(),
            Value::Object(
                BTreeMap::from([
                    ("address".into(), Value::from(input.address)),
                    (
                        "calldata".into(),
                        Value::String(alloy_primitives::hex::encode_prefixed(&input.input)),
                    ),
                    (
                        "expected_output_size".into(),
                        input
                            .expected_output_size
                            .map(Value::from)
                            .unwrap_or(Value::Null),
                    ),
                    ("target_raw_gas".into(), Value::from(input.target_raw_gas)),
                ])
                .into_iter()
                .collect(),
            ),
        ),
        (
            "key_id".into(),
            Value::String(format!("precompile:0x{:02x}", input.address)),
        ),
        ("schema_version".into(), Value::from(1)),
        ("target_count".into(), Value::from(input.target_count)),
    ])
}

pub fn controlled_precompile_workload_spec(input: &PrecompileLabInput) -> ControlledWorkloadSpec {
    let key_id = format!("precompile:0x{:02x}", input.address);
    ControlledWorkloadSpec {
        schema_version: 1,
        key_id: key_id.clone(),
        case_id: input.case.clone(),
        target_count: input.target_count,
        lane: controlled_lane(input.lane),
        state: BTreeMap::new(),
        environment: BTreeMap::new(),
        input: BTreeMap::from([
            ("address".into(), Value::from(input.address)),
            (
                "calldata".into(),
                Value::String(alloy_primitives::hex::encode_prefixed(&input.input)),
            ),
            (
                "expected_output_size".into(),
                input
                    .expected_output_size
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            ),
            ("target_raw_gas".into(), Value::from(input.target_raw_gas)),
        ]),
        expected_operation_deltas: BTreeMap::from([(
            key_id,
            if input.lane == PrecompileLabLane::Target {
                i64::try_from(input.target_count).unwrap_or(i64::MAX)
            } else {
                0
            },
        )]),
        expected_feature_deltas: BTreeMap::new(),
    }
}

pub fn trace_precompile_workload(input: &PrecompileLabInput) -> Result<ControlledPrecompileTrace> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let output_len = usize::try_from(
        input
            .expected_output_size
            .ok_or_else(|| anyhow::anyhow!("paired precompile requires expected_output_size"))?,
    )?;
    let backend_input = bincode::serialize(input)?;
    let pair_value = BTreeMap::from([
        ("kind", Value::String("controlled_precompile_pair".into())),
        (
            "pair_spec",
            Value::Object(precompile_pair_spec(input).into_iter().collect()),
        ),
    ]);
    Ok(ControlledPrecompileTrace {
        schema_version: 1,
        workload_id: controlled_workload_id(&controlled_precompile_workload_spec(input))?,
        pair_id: alloy_primitives::hex::encode(Sha256::digest(serde_json::to_vec(&pair_value)?)),
        backend_input_sha256: alloy_primitives::hex::encode(Sha256::digest(&backend_input)),
        backend_input_len: backend_input.len(),
        address: input.address,
        target_count: input.target_count,
        target_raw_gas: input.target_raw_gas,
        lane: controlled_lane(input.lane),
        input_len: input.input.len(),
        output_len,
        loop_iterations: input.target_count,
        folded_bytes_per_iteration: 8usize.saturating_add(output_len),
    })
}

#[derive(Clone, Copy, Debug)]
struct OpcodeFootprintEvent {
    opcode: u8,
    raw_gas: u64,
}

#[derive(Default)]
struct OpcodeFootprintInspector {
    pending: Option<(u8, u64)>,
    counts: BTreeMap<u8, u64>,
    raw_gas: BTreeMap<u8, u64>,
    events: Vec<OpcodeFootprintEvent>,
}

impl<CTX> Inspector<CTX, EthInterpreter> for OpcodeFootprintInspector {
    fn step(&mut self, interpreter: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        self.pending = Some((interpreter.bytecode.opcode(), interpreter.gas.remaining()));
    }

    fn step_end(&mut self, interpreter: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let Some((opcode, gas_before)) = self.pending.take() else {
            return;
        };
        let raw_gas = gas_before.saturating_sub(interpreter.gas.remaining());
        *self.counts.entry(opcode).or_default() += 1;
        *self.raw_gas.entry(opcode).or_default() += raw_gas;
        self.events.push(OpcodeFootprintEvent { opcode, raw_gas });
    }
}

#[derive(Default)]
struct StorageSemanticInspector {
    pending_sload: bool,
    observed_loads: Vec<RevmU256>,
    storage_opcode_count: u64,
}

impl<CTX> Inspector<CTX, EthInterpreter> for StorageSemanticInspector {
    fn step(&mut self, interpreter: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let opcode = interpreter.bytecode.opcode();
        self.pending_sload = opcode == 0x54;
        if matches!(opcode, 0x54 | 0x55) {
            self.storage_opcode_count = self.storage_opcode_count.saturating_add(1);
        }
    }

    fn step_end(&mut self, interpreter: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        if self.pending_sload
            && let Ok(value) = interpreter.stack.peek(0)
        {
            self.observed_loads.push(value);
        }
        self.pending_sload = false;
    }
}

fn sha256_json(value: &BTreeMap<&str, Value>) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(alloy_primitives::hex::encode(Sha256::digest(bytes)))
}

pub fn controlled_workload_id(spec: &ControlledWorkloadSpec) -> Result<String> {
    if !matches!(spec.schema_version, 1 | 2) || spec.key_id.is_empty() || spec.case_id.is_empty() {
        bail!("invalid controlled workload specification");
    }
    let value = BTreeMap::from([
        ("kind", Value::String("controlled".into())),
        ("workload_spec", serde_json::to_value(spec)?),
    ]);
    sha256_json(&value)
}

pub fn controlled_execution_row_id(identity: &ControlledExecutionIdentity) -> Result<String> {
    if identity.backend.is_empty()
        || identity.run_id.is_empty()
        || identity.workload_id.len() != 64
        || identity.backend_input_sha256.len() != 64
    {
        bail!("invalid controlled execution identity");
    }
    let value = BTreeMap::from([
        ("backend", Value::String(identity.backend.clone())),
        (
            "backend_input_sha256",
            Value::String(identity.backend_input_sha256.clone()),
        ),
        ("kind", Value::String("controlled_execution".to_string())),
        ("repeat_index", Value::Number(identity.repeat_index.into())),
        ("run_id", Value::String(identity.run_id.clone())),
        ("workload_id", Value::String(identity.workload_id.clone())),
    ]);
    sha256_json(&value)
}

pub fn controlled_opcode_workload_spec(input: &OpcodeLabInput) -> ControlledWorkloadSpec {
    let key_id = format!("opcode:0x{:02x}", input.opcode);
    let lane = match input.storage.as_ref().map(|storage| storage.lane) {
        Some(OpcodeLabStorageLane::Control) => ControlledLane::Control,
        _ => ControlledLane::Target,
    };
    let state = input
        .storage
        .as_ref()
        .map(|storage| {
            BTreeMap::from([(
                "storage".into(),
                serde_json::to_value(storage).unwrap_or(Value::Null),
            )])
        })
        .unwrap_or_default();
    ControlledWorkloadSpec {
        schema_version: 2,
        key_id: key_id.clone(),
        case_id: input.case.clone(),
        target_count: input.target_count,
        lane,
        state,
        environment: BTreeMap::from([
            ("block_timestamp".into(), Value::from(1)),
            ("evm_spec".into(), Value::String("osaka".into())),
            ("revm_version".into(), Value::String("41.0.0".into())),
            (
                "shared_constructor".into(),
                Value::String("raiko2-opcode-lab".into()),
            ),
        ]),
        input: BTreeMap::from([
            (
                "bytecode".into(),
                Value::String(alloy_primitives::hex::encode_prefixed(&input.bytecode)),
            ),
            ("calldata".into(), Value::String("0x".into())),
            ("opcode".into(), Value::from(input.opcode)),
            ("target_raw_gas".into(), Value::from(input.target_raw_gas)),
            (
                "tx_value".into(),
                Value::String(alloy_primitives::hex::encode_prefixed([0u8; 32])),
            ),
            (
                "tx_gas_limit".into(),
                Value::from(input.execution_gas_limit()),
            ),
            (
                "generator_max_count".into(),
                input
                    .generator_max_count
                    .map(Value::from)
                    .unwrap_or(Value::Null),
            ),
        ]),
        expected_operation_deltas: BTreeMap::from([(
            key_id,
            i64::try_from(input.target_count).unwrap_or(i64::MAX),
        )]),
        expected_feature_deltas: BTreeMap::new(),
    }
}

pub fn controlled_context_opcode_workload_spec(
    input: &ContextOpcodeLabInputV1,
) -> ControlledWorkloadSpec {
    let mut spec = controlled_opcode_workload_spec(&input.legacy_input());
    spec.environment.insert(
        "block_timestamp".into(),
        Value::from(input.effective_block_timestamp()),
    );
    spec.environment.insert(
        "shared_constructor".into(),
        Value::String("raiko2-context-opcode-lab".into()),
    );
    spec.input.insert(
        "calldata".into(),
        Value::String(alloy_primitives::hex::encode_prefixed(&input.calldata)),
    );
    spec.input.insert(
        "tx_value".into(),
        Value::String(alloy_primitives::hex::encode_prefixed(input.tx_value)),
    );
    spec
}

fn result_status(result: &ExecutionResult) -> &'static str {
    match result {
        ExecutionResult::Success { .. } => "success",
        ExecutionResult::Revert { .. } => "revert",
        ExecutionResult::Halt { .. } => "halt",
    }
}

fn word_hex(value: RevmU256) -> String {
    alloy_primitives::hex::encode_prefixed(value.to_be_bytes::<32>())
}

fn storage_prestate_sha256(storage: Option<&OpcodeLabStorageInput>) -> Result<String> {
    let value = match storage {
        Some(storage) => serde_json::json!({
            "original_value": alloy_primitives::hex::encode_prefixed(storage.original_value),
            "slot": alloy_primitives::hex::encode_prefixed(storage.slot),
        }),
        None => Value::Null,
    };
    Ok(alloy_primitives::hex::encode(Sha256::digest(
        serde_json::to_vec(&value)?,
    )))
}

fn transaction_identities(tx: &TxEnv) -> Result<(String, String)> {
    let access_list = tx
        .access_list
        .0
        .iter()
        .map(|item| {
            serde_json::json!({
                "address": alloy_primitives::hex::encode_prefixed(item.address),
                "storage_keys": item
                    .storage_keys
                    .iter()
                    .map(alloy_primitives::hex::encode_prefixed)
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let access_list_sha256 =
        alloy_primitives::hex::encode(Sha256::digest(serde_json::to_vec(&access_list)?));
    let kind = match tx.kind {
        TxKind::Call(address) => serde_json::json!({
            "kind": "call",
            "address": alloy_primitives::hex::encode_prefixed(address),
        }),
        TxKind::Create => serde_json::json!({"kind": "create"}),
    };
    let envelope = BTreeMap::from([
        ("access_list", serde_json::to_value(&access_list)?),
        (
            "authorization_count",
            Value::from(tx.authorization_list.len()),
        ),
        (
            "blob_hashes",
            serde_json::to_value(
                tx.blob_hashes
                    .iter()
                    .map(alloy_primitives::hex::encode_prefixed)
                    .collect::<Vec<_>>(),
            )?,
        ),
        (
            "caller",
            Value::String(alloy_primitives::hex::encode_prefixed(tx.caller)),
        ),
        (
            "chain_id",
            tx.chain_id.map(Value::from).unwrap_or(Value::Null),
        ),
        (
            "data",
            Value::String(alloy_primitives::hex::encode_prefixed(&tx.data)),
        ),
        ("gas_limit", Value::from(tx.gas_limit)),
        ("gas_price", Value::String(tx.gas_price.to_string())),
        (
            "gas_priority_fee",
            tx.gas_priority_fee
                .map(|value| Value::String(value.to_string()))
                .unwrap_or(Value::Null),
        ),
        ("kind", kind),
        (
            "max_fee_per_blob_gas",
            Value::String(tx.max_fee_per_blob_gas.to_string()),
        ),
        ("nonce", Value::from(tx.nonce)),
        ("tx_type", Value::from(tx.tx_type)),
        ("value", Value::String(word_hex(tx.value))),
    ]);
    let transaction_envelope_sha256 =
        alloy_primitives::hex::encode(Sha256::digest(serde_json::to_vec(&envelope)?));
    Ok((transaction_envelope_sha256, access_list_sha256))
}

pub fn block_environment_sha256(block: &BlockEnv) -> Result<String> {
    let blob_excess_gas_and_price = block
        .blob_excess_gas_and_price
        .map(|blob| {
            serde_json::json!({
                "blob_gasprice": blob.blob_gasprice.to_string(),
                "excess_blob_gas": blob.excess_blob_gas,
            })
        })
        .unwrap_or(Value::Null);
    let environment = BTreeMap::from([
        ("basefee", Value::from(block.basefee)),
        ("blob_excess_gas_and_price", blob_excess_gas_and_price),
        (
            "beneficiary",
            Value::String(alloy_primitives::hex::encode_prefixed(block.beneficiary)),
        ),
        ("difficulty", Value::String(word_hex(block.difficulty))),
        ("gas_limit", Value::from(block.gas_limit)),
        ("number", Value::String(word_hex(block.number))),
        (
            "prevrandao",
            block
                .prevrandao
                .map(|value| Value::String(alloy_primitives::hex::encode_prefixed(value)))
                .unwrap_or(Value::Null),
        ),
        ("slot_num", Value::from(block.slot_num)),
        ("timestamp", Value::String(word_hex(block.timestamp))),
    ]);
    Ok(alloy_primitives::hex::encode(Sha256::digest(
        serde_json::to_vec(&environment)?,
    )))
}

pub fn controlled_opcode_identity(
    input: &OpcodeLabInput,
) -> Result<ControlledOpcodeIdentityEvidence> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let backend_input = bincode::serialize(input)?;
    let canonical_tx = build_benchmark_tx(input.execution_gas_limit(), input.storage.as_ref())?;
    let canonical_block = BlockEnv::default();
    let (transaction_envelope_sha256, access_list_sha256) = transaction_identities(&canonical_tx)?;
    Ok(ControlledOpcodeIdentityEvidence {
        schema_version: 2,
        input: input.clone(),
        backend_input_sha256: alloy_primitives::hex::encode(Sha256::digest(&backend_input)),
        backend_input_len: backend_input.len(),
        workload_id: controlled_workload_id(&controlled_opcode_workload_spec(input))?,
        transaction_envelope_sha256,
        block_environment_sha256: block_environment_sha256(&canonical_block)?,
        access_list_sha256,
        prestate_sha256: storage_prestate_sha256(input.storage.as_ref())?,
    })
}

fn exact_opcode_ledger(
    counts: &BTreeMap<u8, u64>,
    raw_gas: &BTreeMap<u8, u64>,
) -> (BTreeMap<String, u64>, BTreeMap<String, u64>) {
    let names = |values: &BTreeMap<u8, u64>| {
        values
            .iter()
            .map(|(opcode, value)| (format!("opcode:0x{opcode:02x}"), *value))
            .collect()
    };
    (names(counts), names(raw_gas))
}

fn measured_events<'a>(
    input: &OpcodeLabInput,
    programs: &'a [Vec<OpcodeFootprintEvent>],
) -> (Vec<&'a OpcodeFootprintEvent>, u64) {
    let Some(storage) = &input.storage else {
        return (
            programs
                .iter()
                .flatten()
                .filter(|event| event.opcode == input.opcode)
                .collect(),
            0,
        );
    };
    let dirty = matches!(
        &storage.operation,
        OpcodeLabStorageOperation::Store { current_value, .. }
            if current_value != &storage.original_value
    );
    let mut measured = Vec::new();
    let mut prefix_count = 0u64;
    for program in programs {
        let state_events = program
            .iter()
            .filter(|event| event.opcode == storage.measurement_opcode)
            .collect::<Vec<_>>();
        if dirty && !state_events.is_empty() {
            prefix_count = prefix_count.saturating_add(1);
        }
        if storage.lane == OpcodeLabStorageLane::Target {
            measured.extend(state_events.into_iter().skip(usize::from(dirty)));
        }
    }
    (measured, prefix_count)
}

/// Executes the canonical input in a separate host-native pass and checks the declared storage
/// result. This pass is not linked into or called by the measured guest.
pub fn check_revm_opcode_semantics(
    input: &OpcodeLabInput,
) -> Result<ControlledOpcodeSemanticCheck> {
    let backend_input = bincode::serialize(input)?;
    check_opcode_semantics_with_environment(
        input,
        &backend_input,
        build_benchmark_tx(input.execution_gas_limit(), input.storage.as_ref())?,
        BlockEnv::default(),
    )
}

fn check_opcode_semantics_with_environment(
    input: &OpcodeLabInput,
    backend_input: &[u8],
    canonical_tx: TxEnv,
    canonical_block: BlockEnv,
) -> Result<ControlledOpcodeSemanticCheck> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let backend_input_sha256 = alloy_primitives::hex::encode(Sha256::digest(backend_input));
    let mut observations = Vec::new();
    let mut observed_storage_opcode_count = 0u64;
    for (program_index, program) in input
        .execution_programs()
        .map_err(anyhow::Error::msg)?
        .into_iter()
        .enumerate()
    {
        let bytecode = Bytecode::new_legacy(program.to_vec().into());
        let context = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
            .modify_block_chained(|block| *block = canonical_block.clone())
            .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
        let mut evm = context.build_mainnet_with_inspector(StorageSemanticInspector::default());
        let execution = evm.inspect_tx(canonical_tx.clone())?;
        let status = result_status(&execution.result).to_string();
        if status != "success" {
            bail!("opcode-lab semantic program {program_index} did not succeed: {status}");
        }
        observed_storage_opcode_count =
            observed_storage_opcode_count.saturating_add(evm.inspector.storage_opcode_count);
        let observed_load_values = evm
            .inspector
            .observed_loads
            .iter()
            .copied()
            .map(word_hex)
            .collect::<Vec<_>>();
        let mut expected_load_values = Vec::new();
        let mut expected_final_storage = None;
        let mut observed_final_storage = None;
        if let Some(storage) = &input.storage {
            match &storage.operation {
                OpcodeLabStorageOperation::Load { expected_value } => {
                    if storage.lane == OpcodeLabStorageLane::Target {
                        expected_load_values =
                            vec![
                                alloy_primitives::hex::encode_prefixed(expected_value);
                                observed_load_values.len()
                            ];
                    }
                    if observed_load_values != expected_load_values {
                        bail!(
                            "SLOAD semantic output differs from declared expected value in program {program_index}"
                        );
                    }
                }
                OpcodeLabStorageOperation::Store {
                    current_value,
                    new_value,
                } => {
                    let dirty = current_value != &storage.original_value;
                    let measured = storage.lane == OpcodeLabStorageLane::Target
                        && evm.inspector.storage_opcode_count > u64::from(dirty);
                    let expected = if measured {
                        *new_value
                    } else if dirty {
                        *current_value
                    } else {
                        storage.original_value
                    };
                    let slot = RevmU256::from_be_bytes(storage.slot);
                    let observed = execution
                        .state
                        .get(&BENCH_TARGET)
                        .and_then(|account| account.storage.get(&slot))
                        .map(|value| value.present_value)
                        .unwrap_or_else(|| RevmU256::from_be_bytes(storage.original_value));
                    expected_final_storage = Some(alloy_primitives::hex::encode_prefixed(expected));
                    observed_final_storage = Some(word_hex(observed));
                    if expected_final_storage != observed_final_storage {
                        bail!(
                            "SSTORE semantic result differs from declared transition in program {program_index}"
                        );
                    }
                }
            }
        }
        observations.push(ControlledOpcodeProgramSemantic {
            program_index,
            result_status: status,
            expected_load_values,
            observed_load_values,
            expected_final_storage,
            observed_final_storage,
        });
    }
    if let Some(storage) = &input.storage {
        let dirty = matches!(
            &storage.operation,
            OpcodeLabStorageOperation::Store { current_value, .. }
                if current_value != &storage.original_value
        );
        let expected_storage_opcode_count = match (&storage.operation, storage.lane) {
            (OpcodeLabStorageOperation::Load { .. }, OpcodeLabStorageLane::Target) => {
                input.target_count
            }
            (OpcodeLabStorageOperation::Load { .. }, OpcodeLabStorageLane::Control) => 0,
            (OpcodeLabStorageOperation::Store { .. }, OpcodeLabStorageLane::Target) => {
                input.target_count.saturating_add(
                    u64::from(dirty)
                        .saturating_mul(u64::try_from(observations.len()).unwrap_or(u64::MAX)),
                )
            }
            (OpcodeLabStorageOperation::Store { .. }, OpcodeLabStorageLane::Control) => {
                u64::from(dirty)
                    .saturating_mul(u64::try_from(observations.len()).unwrap_or(u64::MAX))
            }
        };
        if observed_storage_opcode_count != expected_storage_opcode_count {
            bail!(
                "semantic storage opcode count {observed_storage_opcode_count} differs from declared {expected_storage_opcode_count}"
            );
        }
    }
    Ok(ControlledOpcodeSemanticCheck {
        schema_version: 1,
        backend_input_sha256,
        checked_programs: observations.len(),
        passed: true,
        programs: observations,
    })
}

pub fn trace_revm_opcode_workload(input: &OpcodeLabInput) -> Result<ControlledOpcodeTrace> {
    let identity = controlled_opcode_identity(input)?;
    let canonical_tx = build_benchmark_tx(input.execution_gas_limit(), input.storage.as_ref())?;
    let canonical_block = BlockEnv::default();
    let semantic_check = check_revm_opcode_semantics(input)?;
    trace_opcode_workload_with_environment(
        input,
        identity.workload_id,
        identity.backend_input_sha256,
        identity.backend_input_len,
        identity.transaction_envelope_sha256,
        identity.block_environment_sha256,
        identity.access_list_sha256,
        identity.prestate_sha256,
        canonical_tx,
        canonical_block,
        semantic_check,
        "raiko2-opcode-lab",
    )
}

#[allow(clippy::too_many_arguments)]
fn trace_opcode_workload_with_environment(
    input: &OpcodeLabInput,
    workload_id: String,
    backend_input_sha256: String,
    backend_input_len: usize,
    transaction_envelope_sha256: String,
    block_environment_sha256: String,
    access_list_sha256: String,
    prestate_sha256: String,
    canonical_tx: TxEnv,
    canonical_block: BlockEnv,
    semantic_check: ControlledOpcodeSemanticCheck,
    shared_constructor: &'static str,
) -> Result<ControlledOpcodeTrace> {
    let programs = input.execution_programs().map_err(anyhow::Error::msg)?;
    let program_sha256 = programs
        .iter()
        .map(|program| alloy_primitives::hex::encode(Sha256::digest(program)))
        .collect::<Vec<_>>();
    let mut inspector = OpcodeFootprintInspector::default();
    let mut program_events = Vec::with_capacity(programs.len());
    let mut result_statuses = BTreeMap::new();
    for program in programs {
        let bytecode = Bytecode::new_legacy(program.to_vec().into());
        let context = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
            .modify_block_chained(|block| *block = canonical_block.clone())
            .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
        let mut evm = context.build_mainnet_with_inspector(OpcodeFootprintInspector::default());
        let execution = evm.inspect_one_tx(canonical_tx.clone())?;
        *result_statuses
            .entry(result_status(&execution).to_string())
            .or_default() += 1;
        for (opcode, count) in evm.inspector.counts {
            *inspector.counts.entry(opcode).or_default() += count;
        }
        for (opcode, gas) in evm.inspector.raw_gas {
            *inspector.raw_gas.entry(opcode).or_default() += gas;
        }
        program_events.push(evm.inspector.events);
    }

    if result_statuses.len() != 1 || !result_statuses.contains_key("success") {
        bail!("opcode-lab trace contains a non-successful REVM result");
    }
    let (measurement_events, executed_prefix_count) = measured_events(input, &program_events);
    let executed_measurement_count = measurement_events.len() as u64;
    let executed_measurement_raw_gas = measurement_events.iter().map(|event| event.raw_gas).sum();
    let target_events = if input
        .storage
        .as_ref()
        .is_some_and(|storage| storage.lane == OpcodeLabStorageLane::Target)
    {
        measurement_events
    } else {
        program_events
            .iter()
            .flatten()
            .filter(|event| event.opcode == input.opcode)
            .collect()
    };
    let target_count = target_events.len() as u64;
    let target_raw_gas = target_events.iter().map(|event| event.raw_gas).sum();
    if target_count != input.target_count {
        bail!(
            "executed target count {target_count} differs from declared {}",
            input.target_count
        );
    }
    let expected_target_raw_gas = input.target_count.saturating_mul(input.target_raw_gas);
    if target_raw_gas != expected_target_raw_gas {
        bail!(
            "executed target raw gas {target_raw_gas} differs from declared {expected_target_raw_gas}"
        );
    }
    let (executed_opcode_counts, executed_opcode_raw_gas) =
        exact_opcode_ledger(&inspector.counts, &inspector.raw_gas);
    let total_raw_gas = inspector.raw_gas.values().copied().sum();
    let mut non_target_raw = inspector.raw_gas.clone();
    if let Some(value) = non_target_raw.get_mut(&input.opcode) {
        *value = value.saturating_sub(target_raw_gas);
    }
    non_target_raw.retain(|_, value| *value != 0);
    let non_target_raw_gas = non_target_raw.values().copied().sum();
    let mut non_target_count_values = inspector.counts.clone();
    if let Some(value) = non_target_count_values.get_mut(&input.opcode) {
        *value = value.saturating_sub(target_count);
    }
    non_target_count_values.retain(|_, value| *value != 0);
    let non_target_counts = non_target_count_values
        .into_iter()
        .map(|(opcode, count)| (format!("opcode:0x{opcode:02x}"), count))
        .collect();
    Ok(ControlledOpcodeTrace {
        schema_version: 3,
        workload_id,
        backend_input_sha256,
        backend_input_len,
        target_opcode: input.opcode,
        declared_target_count: input.target_count,
        declared_target_raw_gas: input.target_raw_gas,
        tx_gas_limit: input.execution_gas_limit(),
        executed_target_count: target_count,
        executed_target_raw_gas: target_raw_gas,
        non_target_counts,
        non_target_raw_gas,
        total_raw_gas,
        bytecode_len: input.bytecode.len(),
        evm_spec: "osaka",
        revm_version: "41.0.0",
        shared_constructor,
        transaction_envelope_sha256,
        block_environment_sha256,
        access_list_sha256,
        prestate_sha256,
        bytecode_sha256: alloy_primitives::hex::encode(Sha256::digest(&input.bytecode)),
        program_sha256,
        executed_opcode_counts,
        executed_opcode_raw_gas,
        executed_measurement_count,
        executed_measurement_raw_gas,
        executed_prefix_count,
        result_statuses,
        storage: input.storage.clone(),
        semantic_check,
    })
}

pub fn controlled_opcode_identity_bundle(
    input: &OpcodeLabInput,
) -> Result<ControlledOpcodeIdentityBundle> {
    let identity = controlled_opcode_identity(input)?;
    let trace = trace_revm_opcode_workload(input)?;
    let expected_public_values = expected_revm_opcode_public_values(input)?;
    Ok(ControlledOpcodeIdentityBundle {
        schema_version: 2,
        expected_public_values,
        report: ControlledOpcodeIdentityReport {
            guest_input_sha256: format!("0x{}", identity.backend_input_sha256),
            guest_input_bincode_length: identity.backend_input_len,
            controlled_trace: ControlledTrace::RevmOpcode(Box::new(trace)),
        },
        identity,
    })
}

pub fn controlled_context_opcode_identity_bundle(
    input: &ContextOpcodeLabInputV1,
) -> Result<ControlledContextOpcodeIdentityBundle> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let legacy = input.legacy_input();
    let backend_input = bincode::serialize(input)?;
    let backend_input_sha256 = alloy_primitives::hex::encode(Sha256::digest(&backend_input));
    let canonical_tx = build_context_benchmark_tx(input)?;
    let canonical_block = build_context_benchmark_block_env(input);
    let (transaction_envelope_sha256, access_list_sha256) = transaction_identities(&canonical_tx)?;
    let identity = ControlledContextOpcodeIdentityEvidence {
        schema_version: 2,
        input: input.clone(),
        backend_input_sha256: backend_input_sha256.clone(),
        backend_input_len: backend_input.len(),
        workload_id: controlled_workload_id(&controlled_context_opcode_workload_spec(input))?,
        transaction_envelope_sha256: transaction_envelope_sha256.clone(),
        block_environment_sha256: block_environment_sha256(&canonical_block)?,
        access_list_sha256: access_list_sha256.clone(),
        prestate_sha256: storage_prestate_sha256(input.storage.as_ref())?,
    };
    let semantic_check = check_opcode_semantics_with_environment(
        &legacy,
        &backend_input,
        canonical_tx.clone(),
        canonical_block.clone(),
    )?;
    let trace = trace_opcode_workload_with_environment(
        &legacy,
        identity.workload_id.clone(),
        backend_input_sha256,
        backend_input.len(),
        transaction_envelope_sha256,
        identity.block_environment_sha256.clone(),
        access_list_sha256,
        identity.prestate_sha256.clone(),
        canonical_tx,
        canonical_block,
        semantic_check,
        "raiko2-context-opcode-lab",
    )?;
    let expected_public_values = expected_context_opcode_public_values(input)?;
    Ok(ControlledContextOpcodeIdentityBundle {
        schema_version: 2,
        expected_public_values,
        report: ControlledOpcodeIdentityReport {
            guest_input_sha256: format!("0x{}", identity.backend_input_sha256),
            guest_input_bincode_length: identity.backend_input_len,
            controlled_trace: ControlledTrace::RevmOpcode(Box::new(trace)),
        },
        identity,
    })
}

fn expected_revm_opcode_public_values(input: &OpcodeLabInput) -> Result<String> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let mut accumulator = 0u64;
    for program in input.execution_programs().map_err(anyhow::Error::msg)? {
        let bytecode = Bytecode::new_legacy(program.to_vec().into());
        let context = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
            .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
        let execution = context.build_mainnet().transact(build_benchmark_tx(
            input.execution_gas_limit(),
            input.storage.as_ref(),
        )?)?;
        accumulator = fold_revm_opcode_program(
            accumulator,
            fold_revm_opcode_execution_result(&execution.result),
        );
    }
    Ok(format!(
        "{:#x}",
        revm_opcode_public_values(input, accumulator)
    ))
}

fn expected_context_opcode_public_values(input: &ContextOpcodeLabInputV1) -> Result<String> {
    input
        .validate_controlled_contract()
        .map_err(anyhow::Error::msg)?;
    let mut accumulator = 0u64;
    for program in input.execution_programs().map_err(anyhow::Error::msg)? {
        let bytecode = Bytecode::new_legacy(program.to_vec().into());
        let context = Context::mainnet()
            .modify_cfg_chained(|cfg| cfg.set_spec_and_mainnet_gas_params(OPCODE_LAB_SPEC_ID))
            .modify_block_chained(|block| *block = build_context_benchmark_block_env(input))
            .with_db(build_benchmark_db(bytecode, input.storage.as_ref()));
        let execution = context
            .build_mainnet()
            .transact(build_context_benchmark_tx(input)?)?;
        accumulator = fold_revm_opcode_program(
            accumulator,
            fold_revm_opcode_execution_result(&execution.result),
        );
    }
    Ok(format!(
        "{:#x}",
        context_opcode_public_values(&bincode::serialize(input)?, accumulator)
    ))
}

pub fn validate_fixed_footprint(rows: &[ControlledFootprint]) -> Result<()> {
    let Some(reference) = rows.first() else {
        bail!("controlled footprint is empty");
    };
    if rows.iter().any(|row| {
        row.bytecode_len != reference.bytecode_len || row.input_len != reference.input_len
    }) {
        bail!("confounded_template: controlled bytecode/input footprint changed");
    }
    if rows.iter().any(|row| {
        row.non_target_counts != reference.non_target_counts
            || row.non_target_raw_gas != reference.non_target_raw_gas
    }) {
        bail!("confounded_template: non-target executed work changed");
    }
    if rows
        .iter()
        .any(|row| row.tx_gas_limit != reference.tx_gas_limit)
    {
        bail!("confounded_template: transaction gas limit changed");
    }
    Ok(())
}

pub fn validate_precompile_pair(
    target: &PairedPrecompileShape,
    control: &PairedPrecompileShape,
) -> Result<()> {
    if target.lane != ControlledLane::Target || control.lane != ControlledLane::Control {
        bail!("precompile pair must contain target and control lanes");
    }
    if target.target_count != control.target_count
        || target.input_len != control.input_len
        || target.loop_iterations != control.loop_iterations
        || target.output_len != control.output_len
        || target.folded_bytes_per_iteration != control.folded_bytes_per_iteration
    {
        bail!("confounded_template: precompile target/control shape differs");
    }
    Ok(())
}
