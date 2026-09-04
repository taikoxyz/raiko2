#![allow(missing_docs)]

use alloy_primitives::{Address, B256, Uint, address};
use raiko2_guests::load_risc0_shasta_guest_elves;
use raiko2_primitives_shasta::ShastaRisc0AggregationGuestInput;
use raiko2_protocol_shasta::{
    libhash::hash_shasta_subproof_input,
    shasta::{Checkpoint, ProofCarryData, ShastaTransitionInput, TransitionInputData},
};
use risc0_zkvm::{
    Digest as Risc0Digest, ExecutorEnv, FakeReceipt, Receipt, ReceiptClaim, SessionInfo,
    VerifierContext, compute_image_id, local_executor,
};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

const EXECUTION_PO2: u32 = 20;
const MILLION_CYCLES: u64 = 1_000_000;
const PER_CHILD_MCYCLES: u64 = 180;
const BLOCKER: &str = "the current aggregation guest rejects claim-correct development receipts before an assumption syscall can be established";

#[derive(Serialize)]
struct ObservationOutput {
    aggregation_image_id: String,
    aggregation_elf_sha256: String,
    proposal_image_id: String,
    execution_po2: u32,
    binding_checks: BindingChecks,
    execution_capability: ExecutionCapability,
    blocker: Option<&'static str>,
    observations: Vec<ObservationRow>,
}

#[derive(Serialize)]
struct BindingChecks {
    claim_journal_is_exact_carry_hash: bool,
    mismatched_claim_rejected: bool,
    mismatched_image_rejected: bool,
}

#[derive(Serialize)]
struct ExecutionCapability {
    claim_correct_execution_succeeded: bool,
    assumption_syscall_established: bool,
}

#[derive(Serialize)]
struct ObservationRow {
    aggregation_image_id: String,
    child_count: u32,
    actual_user_mcycles: u64,
    predicted_mcycles: u64,
    signed_error_mcycles: i64,
    absolute_error_percent: String,
    underquote_percent: String,
}

struct DevelopmentReceipts {
    encoded: Vec<Vec<u8>>,
    claims: Vec<ReceiptClaim>,
}

fn carry_sequence(count: usize) -> Vec<ProofCarryData> {
    let mut carries = Vec::with_capacity(count);
    let mut parent_proposal_hash = B256::ZERO;
    let mut parent_block_hash = B256::repeat_byte(0x40);

    for index in 0..count {
        let ordinal = u8::try_from(index + 1).expect("calibration count fits u8");
        let proposal_hash = B256::repeat_byte(ordinal);
        let checkpoint_block_hash = B256::repeat_byte(0x80 + ordinal);
        carries.push(ProofCarryData {
            chain_id: 167_000,
            verifier: address!("00000000000000000000000000000000000000aa"),
            transition_input: TransitionInputData {
                proposal_id: u64::from(ordinal),
                proposal_hash,
                parent_proposal_hash,
                parent_block_hash,
                actual_prover: address!("00000000000000000000000000000000000000bb"),
                transition: ShastaTransitionInput {
                    proposer: Address::ZERO,
                    timestamp: 100 + u64::from(ordinal),
                },
                checkpoint: Checkpoint {
                    blockNumber: Uint::from(10 + u64::from(ordinal)),
                    blockHash: checkpoint_block_hash,
                    stateRoot: B256::repeat_byte(0xa0 + ordinal),
                },
            },
        });
        parent_proposal_hash = proposal_hash;
        parent_block_hash = checkpoint_block_hash;
    }

    carries
}

fn image_words(image_id: Risc0Digest) -> [u32; 8] {
    let mut words = [0; 8];
    words.copy_from_slice(image_id.as_words());
    words
}

fn development_receipts(
    carries: &[ProofCarryData],
    proposal_image_id: Risc0Digest,
) -> Result<DevelopmentReceipts, Box<dyn std::error::Error>> {
    let mut receipts = Vec::with_capacity(carries.len());
    let mut claims = Vec::with_capacity(carries.len());
    for carry in carries {
        let journal = hash_shasta_subproof_input(carry).to_vec();
        let claim = ReceiptClaim::ok(proposal_image_id, journal);
        let receipt = Receipt::try_from(FakeReceipt::new(claim.clone()))?;
        receipts.push(bincode::serialize(&receipt)?);
        claims.push(claim);
    }
    Ok(DevelopmentReceipts {
        encoded: receipts,
        claims,
    })
}

fn aggregation_input(
    carries: Vec<ProofCarryData>,
    receipts: Vec<Vec<u8>>,
    proposal_image_id: Risc0Digest,
) -> ShastaRisc0AggregationGuestInput {
    ShastaRisc0AggregationGuestInput {
        image_id: image_words(proposal_image_id),
        proof_carry_data_vec: carries,
        receipts,
        prover_address: Address::ZERO,
    }
}

fn execute_aggregation(
    elf: &[u8],
    input: &ShastaRisc0AggregationGuestInput,
    claims: &[ReceiptClaim],
) -> Result<SessionInfo, Box<dyn std::error::Error>> {
    let encoded = bincode::serialize(input)?;
    let mut env_builder = ExecutorEnv::builder();
    env_builder
        .write_frame(encoded.as_slice())
        .segment_limit_po2(EXECUTION_PO2);
    for claim in claims {
        env_builder.add_assumption(claim.clone());
    }
    let env = env_builder.build()?;
    Ok(local_executor().execute(env, elf)?)
}

fn has_unresolved_assumption_syscall(session: &SessionInfo) -> bool {
    session
        .receipt_claim
        .as_ref()
        .and_then(|claim| claim.output.as_value().ok())
        .and_then(Option::as_ref)
        .and_then(|output| output.assumptions.as_value().ok())
        .is_some_and(|assumptions| !assumptions.0.is_empty())
}

fn percent(numerator: u128, denominator: u128) -> String {
    let scaled = (numerator * 100_000_000 + denominator / 2) / denominator;
    format!("{}.{:06}", scaled / 1_000_000, scaled % 1_000_000)
}

fn observation_row(
    aggregation_image_id: &str,
    child_count: u32,
    session: &SessionInfo,
) -> ObservationRow {
    let actual = session.cycles().div_ceil(MILLION_CYCLES);
    let predicted = PER_CHILD_MCYCLES * u64::from(child_count);
    let absolute_error = actual.abs_diff(predicted);
    let underquote = actual.saturating_sub(predicted);

    ObservationRow {
        aggregation_image_id: aggregation_image_id.to_string(),
        child_count,
        actual_user_mcycles: actual,
        predicted_mcycles: predicted,
        signed_error_mcycles: i64::try_from(i128::from(predicted) - i128::from(actual))
            .expect("calibration error fits i64"),
        absolute_error_percent: percent(u128::from(absolute_error), u128::from(actual)),
        underquote_percent: percent(u128::from(underquote), u128::from(actual)),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::var("RISC0_DEV_MODE").is_ok_and(|value| value == "1") {
        return Err("set RISC0_DEV_MODE=1 for this local-only calibration probe".into());
    }

    let elves = load_risc0_shasta_guest_elves()?;
    let proposal_image_id = compute_image_id(&elves.proposal)?;
    let aggregation_image_id = compute_image_id(&elves.aggregation)?;
    let proposal_image_id_hex =
        alloy_primitives::hex::encode_prefixed(proposal_image_id.as_bytes());
    let aggregation_image_id_hex =
        alloy_primitives::hex::encode_prefixed(aggregation_image_id.as_bytes());
    let aggregation_elf_sha256 = hex::encode(Sha256::digest(&elves.aggregation));

    let carries = carry_sequence(1);
    let expected_journal = hash_shasta_subproof_input(&carries[0]);
    let development = development_receipts(&carries, proposal_image_id)?;
    let receipt: Receipt = bincode::deserialize(&development.encoded[0])?;
    let claim_journal_is_exact_carry_hash = receipt.journal.bytes == expected_journal.as_slice();
    let dev_context = VerifierContext::default().with_dev_mode(true);

    let mut mismatched_claim_receipt = receipt.clone();
    mismatched_claim_receipt.journal.bytes[0] ^= 1;
    let mismatched_claim_rejected = mismatched_claim_receipt
        .verify_with_context(&dev_context, proposal_image_id)
        .is_err();
    let mismatched_image_rejected = receipt
        .verify_with_context(&dev_context, Risc0Digest::ZERO)
        .is_err();

    let input = aggregation_input(carries, development.encoded, proposal_image_id);
    let claim_correct_session =
        execute_aggregation(&elves.aggregation, &input, &development.claims);
    let claim_correct_execution_succeeded = claim_correct_session.is_ok();
    let assumption_syscall_established = claim_correct_session
        .as_ref()
        .is_ok_and(has_unresolved_assumption_syscall);

    let mut observations = Vec::new();
    let blocker = if claim_correct_execution_succeeded && assumption_syscall_established {
        for child_count in 1..=5 {
            let carries = carry_sequence(child_count as usize);
            let development = development_receipts(&carries, proposal_image_id)?;
            let input = aggregation_input(carries, development.encoded, proposal_image_id);
            let session = execute_aggregation(&elves.aggregation, &input, &development.claims)?;
            if !has_unresolved_assumption_syscall(&session) {
                return Err(format!(
                    "aggregation count {child_count} executed without the required assumption syscall"
                )
                .into());
            }
            observations.push(observation_row(
                &aggregation_image_id_hex,
                child_count,
                &session,
            ));
        }
        None
    } else {
        Some(BLOCKER)
    };

    let output = ObservationOutput {
        aggregation_image_id: aggregation_image_id_hex,
        aggregation_elf_sha256,
        proposal_image_id: proposal_image_id_hex,
        execution_po2: EXECUTION_PO2,
        binding_checks: BindingChecks {
            claim_journal_is_exact_carry_hash,
            mismatched_claim_rejected,
            mismatched_image_rejected,
        },
        execution_capability: ExecutionCapability {
            claim_correct_execution_succeeded,
            assumption_syscall_established,
        },
        blocker,
        observations,
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
