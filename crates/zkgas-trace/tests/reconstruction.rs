#![allow(missing_docs)]

use alloy_primitives::{B256, U256};
use raiko2_zkgas_trace::{
    ProposalTrace, ProposalTraceStatus, ProposalTraceSummary, RecoveredTransactionOccurrence,
    TransactionDisposition, classify_started_occurrences,
    classify_started_occurrences_with_statuses, is_native_value_transfer, trace_shasta_proposal,
};
use serde_json::json;

const fn hash(byte: u8) -> B256 {
    B256::with_last_byte(byte)
}

#[test]
fn proposal_trace_schema_version_fails_closed_and_current_version_roundtrips() {
    let valid = json!({
        "schema_version": 4,
        "guest_input_sha256": "0x00",
        "guest_input_bincode_length": 0,
        "status": "complete",
        "blocks": [],
        "partial_blocks": [],
        "recovery_failures": [],
        "parity": {"passed": true, "mismatch_fields": []},
    });
    let roundtrip: ProposalTrace =
        serde_json::from_value(valid.clone()).expect("current trace schema");
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);

    for unsupported in [1, 2, 3, 5] {
        let mut malformed = valid.clone();
        malformed["schema_version"] = json!(unsupported);
        assert!(
            serde_json::from_value::<ProposalTrace>(malformed).is_err(),
            "schema version {unsupported} must fail closed",
        );
    }
}

#[test]
fn proposal_trace_summary_schema_version_fails_closed_and_current_version_roundtrips() {
    let valid = json!({
        "schema_version": 4,
        "full_trace_encoding": "json+gzip",
        "guest_input_sha256": "0x00",
        "guest_input_bincode_length": 0,
        "status": "complete",
        "parity_passed": true,
        "block_count": 0,
        "partial_block_count": 0,
        "recovery_failure_count": 0,
        "operation_count": 0,
    });
    let roundtrip: ProposalTraceSummary =
        serde_json::from_value(valid.clone()).expect("current summary schema");
    assert_eq!(serde_json::to_value(roundtrip).unwrap(), valid);

    for unsupported in [1, 2, 3, 5] {
        let mut malformed = valid.clone();
        malformed["schema_version"] = json!(unsupported);
        assert!(
            serde_json::from_value::<ProposalTraceSummary>(malformed).is_err(),
            "summary schema version {unsupported} must fail closed",
        );
    }
}

#[test]
fn duplicate_hashes_are_joined_as_an_ordered_one_use_subsequence() {
    let started = vec![
        RecoveredTransactionOccurrence::new(0, Some(0), hash(1), false),
        RecoveredTransactionOccurrence::new(1, Some(1), hash(2), false),
        RecoveredTransactionOccurrence::new(2, Some(2), hash(1), false),
        RecoveredTransactionOccurrence::new(3, Some(3), hash(3), false),
    ];

    let classified = classify_started_occurrences(&started, &[hash(2), hash(1)]).unwrap();

    assert_eq!(classified[0].disposition, TransactionDisposition::Attempted);
    assert_eq!(
        classified[1].disposition,
        TransactionDisposition::CommittedSuccess
    );
    assert_eq!(
        classified[2].disposition,
        TransactionDisposition::CommittedSuccess
    );
    assert_eq!(classified[3].disposition, TransactionDisposition::Attempted);
}

#[test]
fn unconsumed_committed_occurrence_is_rejected() {
    let started = vec![RecoveredTransactionOccurrence::new(
        0,
        Some(0),
        hash(1),
        true,
    )];
    let error = classify_started_occurrences(&started, &[hash(2)]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unconsumed committed transaction")
    );
}

#[test]
fn untouched_suffix_is_unattempted_and_has_no_started_index() {
    let occurrences = [
        RecoveredTransactionOccurrence::new(0, Some(0), hash(1), true),
        RecoveredTransactionOccurrence::new(1, Some(1), hash(2), false),
        RecoveredTransactionOccurrence::new(2, Some(2), hash(3), false),
    ];
    let classified = classify_started_occurrences(&occurrences[..2], &[hash(1)]).unwrap();
    let untouched = occurrences[2].clone().into_unattempted();

    assert_eq!(
        classified[0].disposition,
        TransactionDisposition::CommittedSuccess
    );
    assert_eq!(classified[1].disposition, TransactionDisposition::Attempted);
    assert_eq!(untouched.started_tx_index, None);
    assert_eq!(untouched.disposition, TransactionDisposition::Unattempted);
}

#[test]
fn receipt_status_distinguishes_committed_failure_and_rejects_count_mismatch() {
    let started = vec![
        RecoveredTransactionOccurrence::new(0, Some(0), hash(1), false),
        RecoveredTransactionOccurrence::new(1, Some(1), hash(2), false),
    ];
    let classified =
        classify_started_occurrences_with_statuses(&started, &[hash(1), hash(2)], &[true, false])
            .unwrap();
    assert_eq!(
        classified[0].disposition,
        TransactionDisposition::CommittedSuccess
    );
    assert_eq!(
        classified[1].disposition,
        TransactionDisposition::CommittedFailure
    );

    let error = classify_started_occurrences_with_statuses(&started, &[hash(1)], &[]).unwrap_err();
    assert!(error.to_string().contains("receipt-count mismatch"));
}

#[test]
fn native_transfer_requires_positive_no_code_committed_call_success() {
    assert!(is_native_value_transfer(
        TransactionDisposition::CommittedSuccess,
        U256::from(1),
        true,
        false,
    ));
    for (case, disposition, value, is_call, has_operation_trace) in [
        (
            "zero-value",
            TransactionDisposition::CommittedSuccess,
            U256::ZERO,
            true,
            false,
        ),
        (
            "create",
            TransactionDisposition::CommittedSuccess,
            U256::from(1),
            false,
            false,
        ),
        (
            "contract-or-precompile",
            TransactionDisposition::CommittedSuccess,
            U256::from(1),
            true,
            true,
        ),
        (
            "reverted-or-halted",
            TransactionDisposition::CommittedFailure,
            U256::from(1),
            true,
            false,
        ),
        (
            "attempted",
            TransactionDisposition::Attempted,
            U256::from(1),
            true,
            false,
        ),
        (
            "unattempted",
            TransactionDisposition::Unattempted,
            U256::from(1),
            true,
            false,
        ),
    ] {
        assert!(
            !is_native_value_transfer(disposition, value, is_call, has_operation_trace,),
            "{case}"
        );
    }
}

#[test]
fn repository_fixture_passes_fresh_state_ab_gate_and_traces_adjacent_transactions() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json");
    let bytes = std::fs::read(&fixture).expect("read proposal fixture");
    let input = serde_json::from_slice(&bytes).expect("parse proposal fixture");

    let trace = trace_shasta_proposal(&input).expect("trace proposal");
    let serialized = serde_json::to_value(&trace).expect("serialize versioned trace");
    assert_eq!(serialized["schema_version"], 4);
    assert_eq!(trace.summary().schema_version, 4);
    assert_eq!(
        trace.status,
        ProposalTraceStatus::Complete,
        "failure={:?} parity={:?}",
        trace.failure,
        trace.parity
    );
    assert!(trace.parity.passed);
    assert!(trace.parity.mismatch_fields.is_empty());
    assert!(trace.public_output.is_some());
    assert!(trace.guest_input_sha256.starts_with("0x"));
    assert_eq!(trace.guest_input_sha256.len(), 66);
    assert!(trace.guest_input_bincode_length > 0);
    assert!(!trace.blocks.is_empty());

    let adjacent = trace
        .blocks
        .iter()
        .find(|block| block.started_transaction_count >= 2)
        .expect("fixture must execute adjacent transactions");
    assert!(
        adjacent
            .operations
            .iter()
            .any(|operation| operation.tx_index == Some(0)),
        "first transaction operation trace"
    );
    assert!(
        adjacent
            .operations
            .iter()
            .any(|operation| operation.tx_index == Some(1)),
        "second transaction operation trace"
    );
    let first_last = adjacent
        .operations
        .iter()
        .rposition(|operation| operation.tx_index == Some(0))
        .expect("first transaction operation");
    let second_first = adjacent
        .operations
        .iter()
        .position(|operation| operation.tx_index == Some(1))
        .expect("second transaction operation");
    assert!(
        first_last < second_first,
        "transaction execution must stay interleaved"
    );
}

#[test]
fn ordinary_failure_still_runs_traced_pass_and_retains_rejected_diagnostics() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json");
    let bytes = std::fs::read(&fixture).expect("read proposal fixture");
    let mut input: raiko2_primitives_shasta::GuestInput =
        serde_json::from_slice(&bytes).expect("parse proposal fixture");
    input.witnesses[0].block.header.gas_used += 1;

    let trace = trace_shasta_proposal(&input).expect("normalize failed proposal trace");

    assert_eq!(trace.status, ProposalTraceStatus::Failed);
    assert!(!trace.parity.passed);
    assert_eq!(trace.failure.expect("ordinary failure").stage, "ordinary");
    assert!(
        trace.blocks.is_empty(),
        "rejected block must not be complete"
    );
    let partial = trace
        .partial_blocks
        .first()
        .expect("traced pass must retain rejected block diagnostics");
    assert_eq!(partial.block_index, 0);
    assert!(!partial.operations.is_empty());
}
