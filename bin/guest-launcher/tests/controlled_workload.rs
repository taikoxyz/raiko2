#[path = "../src/controlled_workload.rs"]
mod controlled_workload;

use std::collections::BTreeMap;

use alloy_consensus::Transaction as _;
use alloy_primitives::{Address, B256};
use controlled_workload::{
    ControlledBlockRowSpec, ControlledBlockSplit, ControlledExecutionIdentity, ControlledFootprint,
    ControlledLane, ControlledOperationUnits, ControlledOverheadLane, ControlledProgram,
    ControlledStateHoldoutLane, ControlledStateHoldoutPairSpec, ControlledTrace,
    ControlledWorkloadSpec, PairedPrecompileShape, build_controlled_block_fixture,
    build_controlled_block_fixture_with_extra_prestate_account_for_test,
    build_controlled_state_holdout_fixtures, build_required_overhead_fixtures,
    check_revm_opcode_semantics, controlled_block_row_id, controlled_execution_row_id,
    controlled_opcode_identity, controlled_opcode_identity_bundle, controlled_opcode_workload_spec,
    controlled_overhead_workload_id, controlled_precompile_workload_spec, controlled_workload_id,
    observe_controlled_block_fixture, operation_units_delta, trace_precompile_workload,
    trace_revm_opcode_workload, validate_controlled_block_fixture,
    validate_controlled_state_holdout_fixtures, validate_fixed_footprint, validate_precompile_pair,
    validate_required_overhead_fixtures,
};
use raiko2_primitives::{
    OpcodeLabInput, OpcodeLabStorageAccess, OpcodeLabStorageInput, OpcodeLabStorageLane,
    OpcodeLabStorageOperation, PrecompileLabInput, PrecompileLabLane, SupportedChainSpecs,
    chain_spec::{ForkCondition, ForkId, TaikoFork},
};
use raiko2_protocol_shasta::libhash::hash_proposal;
use raiko2_zkgas_trace::{PricingBasis, ProposalTraceStatus};
use serde_json::json;
use sha2::{Digest, Sha256};

fn workload_spec() -> ControlledWorkloadSpec {
    ControlledWorkloadSpec {
        schema_version: 1,
        key_id: "opcode:0x01".into(),
        case_id: "add".into(),
        target_count: 4,
        lane: ControlledLane::Target,
        state: BTreeMap::from([("balance".into(), json!(1))]),
        environment: BTreeMap::from([("timestamp".into(), json!(2))]),
        input: BTreeMap::from([("calldata".into(), json!("0x"))]),
        expected_operation_deltas: BTreeMap::from([("opcode:0x01".into(), 4)]),
        expected_feature_deltas: BTreeMap::new(),
    }
}

fn word(value: u64) -> [u8; 32] {
    alloy_primitives::U256::from(value).to_be_bytes()
}

fn encode_fixed_programs(programs: &[Vec<u8>]) -> Vec<u8> {
    let mut encoded = OpcodeLabInput::FIXED_MICROPROGRAM_MAGIC.to_vec();
    encoded.extend(u32::try_from(programs.len()).unwrap().to_be_bytes());
    for program in programs {
        encoded.extend(u32::try_from(program.len()).unwrap().to_be_bytes());
        encoded.extend(program);
    }
    encoded
}

fn push32(program: &mut Vec<u8>, value: [u8; 32]) {
    program.push(0x7f);
    program.extend(value);
}

fn stateful_input(
    scenario: &str,
    access: OpcodeLabStorageAccess,
    operation: OpcodeLabStorageOperation,
    original_value: [u8; 32],
    slot: [u8; 32],
    program: Vec<u8>,
    target_raw_gas: u64,
) -> OpcodeLabInput {
    let bytecode = encode_fixed_programs(&[program]);
    let measurement_opcode = match &operation {
        OpcodeLabStorageOperation::Load { .. } => 0x54,
        OpcodeLabStorageOperation::Store { .. } => 0x55,
    };
    OpcodeLabInput {
        case: format!("{scenario}-case"),
        scenario: scenario.into(),
        opcode: measurement_opcode,
        target_count: 1,
        target_raw_gas,
        tx_gas_limit: Some(2_500_000),
        fixed_bytecode_len: Some(bytecode.len() as u64),
        generator_max_count: Some(1),
        bytecode,
        storage: Some(OpcodeLabStorageInput {
            measurement_opcode,
            lane: OpcodeLabStorageLane::Target,
            slot,
            original_value,
            access,
            operation,
        }),
    }
}

fn block_row_spec() -> ControlledBlockRowSpec {
    ControlledBlockRowSpec {
        row_id: String::new(),
        workload_family: "pop_family".into(),
        split: ControlledBlockSplit::Fit,
        block_count: 1,
        transaction_count: 1,
        program: ControlledProgram::OpcodeLoop {
            family: "pop_family".into(),
            count: 1,
            scenario: "push0_pop".into(),
        },
        expected_final_state_root: B256::from_slice(
            &alloy_primitives::hex::decode(
                "e0244879d78f6b8c11f5f1ce32b582b64aeef0e025238503063c24005c8ad8c4",
            )
            .unwrap(),
        ),
        expected_raw_gas_by_key: BTreeMap::from([
            ("opcode:0x03".into(), 3),
            ("opcode:0x15".into(), 6),
            ("opcode:0x50".into(), 4),
            ("opcode:0x56".into(), 8),
            ("opcode:0x57".into(), 20),
            ("opcode:0x5b".into(), 3),
            ("opcode:0x60".into(), 15),
            ("opcode:0x62".into(), 3),
            ("opcode:0x80".into(), 6),
            ("opcode:0x90".into(), 3),
        ]),
        expected_features: BTreeMap::from([
            ("proposal_startup".into(), 1),
            ("block_base".into(), 1),
            ("tx_base".into(), 1),
            ("native_value_transfer".into(), 0),
        ]),
        expected_diagnostics: BTreeMap::from([
            ("guest_input_bincode_length".into(), 332_681),
            ("witness_node_count".into(), 6),
            ("witness_byte_count".into(), 633),
            ("blob_count".into(), 1),
            ("kzg_invocation_count".into(), 1),
            ("calldata_length".into(), 0),
            ("bytecode_length".into(), 256),
            ("touched_state_key_count".into(), 9),
        ]),
    }
}

fn witness_topology_pair(extra_account_count: usize) -> ControlledStateHoldoutPairSpec {
    serde_json::from_value(json!({
        "pair_id": format!("witness_topology_{extra_account_count}"),
        "kind": "witness_topology",
        "scale": extra_account_count,
        "control": {"extra_account_count": 0},
        "target": {"extra_account_count": extra_account_count},
    }))
    .expect("parse canonical witness topology pair")
}

fn dirty_accounts_pair(transaction_count: u64) -> ControlledStateHoldoutPairSpec {
    serde_json::from_value(json!({
        "pair_id": format!("dirty_accounts_{transaction_count}"),
        "kind": "dirty_accounts",
        "scale": transaction_count,
        "control": {
            "transaction_count": transaction_count,
            "value": 1,
            "recipient_mode": "single",
        },
        "target": {
            "transaction_count": transaction_count,
            "value": 1,
            "recipient_mode": "distinct",
        },
    }))
    .expect("parse canonical dirty-account pair")
}

#[test]
fn witness_topology_state_holdouts_are_isolated_and_deterministic() {
    for extra_account_count in [1, 8, 32] {
        let spec = witness_topology_pair(extra_account_count);
        let fixtures =
            build_controlled_state_holdout_fixtures(&spec).expect("build witness topology pair");
        let observations = validate_controlled_state_holdout_fixtures(&fixtures)
            .expect("trace witness topology pair");
        let [control, target] = observations.as_slice() else {
            panic!("state holdout must produce exactly two observations")
        };

        assert_eq!(control.lane, ControlledStateHoldoutLane::Control);
        assert_eq!(target.lane, ControlledStateHoldoutLane::Target);
        assert_eq!(target.actual_features, control.actual_features);
        assert_eq!(target.actual_raw_gas_by_key, control.actual_raw_gas_by_key);
        assert_eq!(control.actual_features["tx_base"], 0);
        assert_eq!(control.actual_features["native_value_transfer"], 0);
        assert!(
            target.actual_diagnostics["witness_node_count"]
                > control.actual_diagnostics["witness_node_count"]
        );
        assert!(
            target.actual_diagnostics["witness_byte_count"]
                > control.actual_diagnostics["witness_byte_count"]
        );

        let rebuilt = build_controlled_state_holdout_fixtures(&spec)
            .expect("rebuild deterministic witness topology pair");
        let rebuilt_observations = validate_controlled_state_holdout_fixtures(&rebuilt)
            .expect("retrace deterministic witness topology pair");
        for (original, rebuilt) in observations.iter().zip(rebuilt_observations.iter()) {
            assert_eq!(original.guest_input_sha256, rebuilt.guest_input_sha256);
            assert_eq!(
                original.actual_final_state_root,
                rebuilt.actual_final_state_root
            );
        }
    }
}

#[test]
fn dirty_accounts_state_holdouts_change_only_recipient_topology() {
    for transaction_count in [2, 8, 32] {
        let spec = dirty_accounts_pair(transaction_count);
        let fixtures =
            build_controlled_state_holdout_fixtures(&spec).expect("build dirty-account pair");
        let observations = validate_controlled_state_holdout_fixtures(&fixtures)
            .expect("trace dirty-account pair");
        let [control_fixture, target_fixture] = fixtures.as_slice() else {
            panic!("state holdout must produce exactly two fixtures")
        };
        let [control, target] = observations.as_slice() else {
            panic!("state holdout must produce exactly two observations")
        };

        assert_eq!(control.lane, ControlledStateHoldoutLane::Control);
        assert_eq!(target.lane, ControlledStateHoldoutLane::Target);
        assert_eq!(target.actual_features, control.actual_features);
        assert_eq!(target.actual_raw_gas_by_key, control.actual_raw_gas_by_key);
        assert!(target.actual_raw_gas_by_key.is_empty());
        assert_eq!(
            target.actual_features["native_value_transfer"],
            i64::try_from(transaction_count).unwrap()
        );
        assert!(
            target.actual_diagnostics["touched_state_key_count"]
                > control.actual_diagnostics["touched_state_key_count"]
        );
        for observation in [control, target] {
            assert_eq!(
                observation.started_candidate_transaction_count,
                transaction_count
            );
            assert_eq!(
                observation.committed_candidate_transaction_count,
                transaction_count
            );
            assert_eq!(observation.unattempted_candidate_transaction_count, 0);
        }

        let candidate_shape = |fixture: &controlled_workload::ControlledStateHoldoutFixture| {
            fixture.guest_input.witnesses[0]
                .block
                .body
                .transactions
                .iter()
                .skip(1)
                .map(|transaction| {
                    (
                        transaction.nonce(),
                        transaction.value(),
                        transaction.to().expect("native transfer recipient"),
                    )
                })
                .collect::<Vec<_>>()
        };
        let control_shape = candidate_shape(control_fixture);
        let target_shape = candidate_shape(target_fixture);
        assert_eq!(
            control_shape.len(),
            usize::try_from(transaction_count).unwrap()
        );
        assert_eq!(target_shape.len(), control_shape.len());
        assert_eq!(
            control_shape
                .iter()
                .map(|(nonce, value, _)| (*nonce, *value))
                .collect::<Vec<_>>(),
            target_shape
                .iter()
                .map(|(nonce, value, _)| (*nonce, *value))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            control_shape
                .iter()
                .map(|(_, _, recipient)| recipient)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            1
        );
        assert_eq!(
            target_shape
                .iter()
                .map(|(_, _, recipient)| recipient)
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            usize::try_from(transaction_count).unwrap()
        );
    }
}

#[test]
fn state_holdout_spec_uses_the_strict_canonical_pair_schema() {
    let witness = witness_topology_pair(8);
    let dirty = dirty_accounts_pair(8);
    assert!(build_controlled_state_holdout_fixtures(&witness).is_ok());
    assert!(build_controlled_state_holdout_fixtures(&dirty).is_ok());

    assert!(
        serde_json::from_value::<ControlledStateHoldoutPairSpec>(json!({
            "kind": "dirty_accounts",
            "transaction_count": 8,
            "value": 1,
        }))
        .is_err(),
        "the compact internal builder shape is not a CLI schema",
    );

    let mut unknown_lane_field = serde_json::to_value(&dirty).expect("serialize pair");
    unknown_lane_field["target"]["extra"] = json!(true);
    assert!(
        serde_json::from_value::<ControlledStateHoldoutPairSpec>(unknown_lane_field).is_err(),
        "lane objects reject unknown fields",
    );
    let mut unknown_pair_field = serde_json::to_value(&dirty).expect("serialize pair");
    unknown_pair_field["extra"] = json!(true);
    assert!(
        serde_json::from_value::<ControlledStateHoldoutPairSpec>(unknown_pair_field).is_err(),
        "pair objects reject unknown fields",
    );

    let mut invalid_pairs = Vec::new();
    let witness_value = serde_json::to_value(&witness).expect("serialize witness pair");
    let dirty_value = serde_json::to_value(&dirty).expect("serialize dirty pair");
    for (pointer, replacement) in [
        ("/pair_id", json!("witness_topology_32")),
        ("/scale", json!(32)),
        ("/control/extra_account_count", json!(1)),
        ("/target/extra_account_count", json!(1)),
    ] {
        let mut invalid = witness_value.clone();
        *invalid.pointer_mut(pointer).expect("witness pointer") = replacement;
        invalid_pairs.push(invalid);
    }
    for (pointer, replacement) in [
        ("/pair_id", json!("dirty_accounts_32")),
        ("/scale", json!(32)),
        ("/control/transaction_count", json!(2)),
        ("/target/transaction_count", json!(2)),
        ("/control/value", json!(2)),
        ("/target/value", json!(2)),
        ("/control/recipient_mode", json!("distinct")),
        ("/target/recipient_mode", json!("single")),
    ] {
        let mut invalid = dirty_value.clone();
        *invalid.pointer_mut(pointer).expect("dirty pointer") = replacement;
        invalid_pairs.push(invalid);
    }
    for invalid in invalid_pairs {
        let parsed = serde_json::from_value::<ControlledStateHoldoutPairSpec>(invalid)
            .expect("invalid canonical combination remains structurally valid");
        assert!(
            build_controlled_state_holdout_fixtures(&parsed).is_err(),
            "noncanonical pair combinations are rejected",
        );
    }
}

#[test]
fn controlled_block_row_id_binds_every_semantic_field() {
    let original = block_row_spec();
    let original_id = controlled_block_row_id(&original).unwrap();
    assert_eq!(original_id.len(), 64);

    let mut mutations = Vec::new();
    let mut changed = original.clone();
    changed.workload_family = "push_family".into();
    mutations.push(changed);
    let mut changed = original.clone();
    changed.split = ControlledBlockSplit::Holdout;
    mutations.push(changed);
    let mut changed = original.clone();
    changed.block_count = 2;
    mutations.push(changed);
    let mut changed = original.clone();
    changed.transaction_count = 2;
    mutations.push(changed);
    let mut changed = original.clone();
    changed.program = ControlledProgram::Empty;
    mutations.push(changed);
    let mut changed = original.clone();
    changed
        .expected_raw_gas_by_key
        .insert("opcode:0x50".into(), 5);
    mutations.push(changed);
    let mut changed = original.clone();
    changed.expected_features.insert("tx_base".into(), 2);
    mutations.push(changed);
    let mut changed = original.clone();
    changed
        .expected_diagnostics
        .insert("bytecode_length".into(), 257);
    mutations.push(changed);
    let mut changed = original.clone();
    changed.expected_final_state_root = B256::repeat_byte(0x55);
    mutations.push(changed);

    for changed in mutations {
        assert_ne!(controlled_block_row_id(&changed).unwrap(), original_id);
    }

    let mut stored_id_only = original;
    stored_id_only.row_id = "manifest-owned-copy".into();
    assert_eq!(
        controlled_block_row_id(&stored_id_only).unwrap(),
        original_id
    );
}

fn materialize_opcode_block_row(
    family: &str,
    scenario: &str,
    count: u64,
) -> (
    ControlledBlockRowSpec,
    controlled_workload::ControlledBlockObservation,
    (Vec<u8>, usize, u64),
) {
    let mut spec = ControlledBlockRowSpec {
        row_id: String::new(),
        workload_family: family.into(),
        split: if family == "static_count_control" {
            ControlledBlockSplit::Diagnostic
        } else if count == 32 {
            ControlledBlockSplit::Holdout
        } else {
            ControlledBlockSplit::Fit
        },
        block_count: 1,
        transaction_count: 1,
        program: ControlledProgram::OpcodeLoop {
            family: family.into(),
            count,
            scenario: scenario.into(),
        },
        expected_final_state_root: B256::ZERO,
        expected_raw_gas_by_key: BTreeMap::new(),
        expected_features: BTreeMap::new(),
        expected_diagnostics: BTreeMap::new(),
    };
    spec.row_id = controlled_block_row_id(&spec).unwrap();
    let fixture = build_controlled_block_fixture(&spec).expect("build production GuestInput");
    let code = fixture.guest_input.witnesses[0]
        .witness
        .codes
        .iter()
        .find(|code| code.len() == 256)
        .expect("controlled 256-byte contract code")
        .to_vec();
    let candidate = fixture.guest_input.witnesses[0]
        .block
        .body
        .transactions
        .last()
        .expect("controlled candidate transaction");
    let shape = (code, candidate.input().len(), candidate.gas_limit());
    let observed = observe_controlled_block_fixture(&fixture).expect("trace production GuestInput");
    spec.expected_final_state_root = observed.actual_final_state_root;
    spec.expected_raw_gas_by_key = observed.actual_raw_gas_by_key.clone();
    spec.expected_features = observed.actual_features.clone();
    spec.expected_diagnostics = observed.actual_diagnostics.clone();
    spec.row_id = controlled_block_row_id(&spec).unwrap();
    let fixture =
        build_controlled_block_fixture(&spec).expect("rebuild frozen production GuestInput");
    let validated = validate_controlled_block_fixture(&fixture).expect("validate frozen row");
    (spec, validated, shape)
}

#[test]
fn fixed_limit_opcode_families_encode_count_only_in_push3_immediate() {
    for (family, scenario, anchor) in [
        ("pop_family", "push0_pop", "opcode:0x50"),
        ("push_family", "push0_stack", "opcode:0x5f"),
        ("dup_family", "push0_dup1_then_pop", "opcode:0x80"),
        ("swap_family", "push0_pair_swap1_then_pop", "opcode:0x90"),
    ] {
        let counts = [1, 2, 4, 8, 16, 32];
        let materialized = counts
            .into_iter()
            .map(|count| materialize_opcode_block_row(family, scenario, count))
            .collect::<Vec<_>>();
        let observations = materialized
            .iter()
            .map(|(_, observation, _)| observation)
            .collect::<Vec<_>>();
        let anchor_units = observations
            .iter()
            .map(|observation| observation.actual_raw_gas_by_key[anchor])
            .collect::<Vec<_>>();
        assert!(
            anchor_units.windows(2).all(|pair| pair[0] < pair[1]),
            "{family} trace did not increase anchor work: {anchor_units:?}"
        );
        assert!(
            observations.iter().all(|observation| {
                !observation
                    .actual_raw_gas_by_key
                    .contains_key("opcode:0x35")
                    && !observation
                        .actual_raw_gas_by_key
                        .contains_key("opcode:0x5a")
            }),
            "{family} trace retained a runtime count-source opcode"
        );
        let shapes = materialized
            .iter()
            .map(|(_, _, shape)| shape)
            .collect::<Vec<_>>();
        assert!(
            shapes
                .iter()
                .all(|shape| shape.0.len() == 256 && shape.0[0] == 0x62),
            "{family} must use a fixed-width PUSH3 count in 256-byte code"
        );
        assert!(
            shapes.iter().all(|shape| shape.1 == 0),
            "{family} count variants must use empty transaction input"
        );
        assert!(
            shapes.iter().all(|shape| shape.2 == 100_000),
            "{family} transaction gas limits must remain fixed"
        );
    }
}

#[test]
fn static_count_control_discards_push3_before_a_fixed_sequence() {
    let counts = [1, 2, 4, 8, 16, 32];
    let materialized = counts
        .into_iter()
        .map(|count| {
            materialize_opcode_block_row("static_count_control", "push3_pop_fixed_pop", count)
        })
        .collect::<Vec<_>>();
    let (_, baseline_observation, baseline_shape) = &materialized[0];
    for (_, observation, shape) in &materialized[1..] {
        assert_eq!(
            observation.actual_raw_gas_by_key,
            baseline_observation.actual_raw_gas_by_key
        );
        assert_eq!(
            observation.actual_features,
            baseline_observation.actual_features
        );
        assert_eq!(
            observation.actual_diagnostics,
            baseline_observation.actual_diagnostics
        );
        assert_eq!(
            observation.guest_input_bincode_length,
            baseline_observation.guest_input_bincode_length
        );
        assert_eq!(shape.1, baseline_shape.1);
        assert_eq!(shape.2, baseline_shape.2);
        assert_eq!(&shape.0[4..], &baseline_shape.0[4..]);
    }
    assert!(materialized.iter().all(|(_, _, shape)| {
        shape.0.len() == 256 && shape.0[0] == 0x62 && shape.1 == 0 && shape.2 == 100_000
    }));
}

#[test]
fn touched_state_key_count_comes_from_built_topology_and_rejects_stale_manifest() {
    let (spec, normal, _) = materialize_opcode_block_row("pop_family", "push0_pop", 1);
    let extra_account = Address::repeat_byte(0x99);
    let mutated_fixture =
        build_controlled_block_fixture_with_extra_prestate_account_for_test(&spec, extra_account)
            .expect("build topology-mutated production GuestInput");
    let mutated = observe_controlled_block_fixture(&mutated_fixture)
        .expect("trace topology-mutated production GuestInput");
    assert_eq!(
        mutated.actual_diagnostics["touched_state_key_count"],
        normal.actual_diagnostics["touched_state_key_count"] + 1,
    );

    let mut stale = spec;
    stale.expected_final_state_root = mutated.actual_final_state_root;
    stale.expected_raw_gas_by_key = mutated.actual_raw_gas_by_key;
    stale.expected_features = mutated.actual_features;
    stale.expected_diagnostics = mutated.actual_diagnostics;
    stale.expected_diagnostics.insert(
        "touched_state_key_count".into(),
        normal.actual_diagnostics["touched_state_key_count"],
    );
    stale.row_id = controlled_block_row_id(&stale).unwrap();
    let stale_fixture =
        build_controlled_block_fixture_with_extra_prestate_account_for_test(&stale, extra_account)
            .expect("rebuild topology-mutated production GuestInput");
    let error = validate_controlled_block_fixture(&stale_fixture)
        .expect_err("stale topology diagnostic must fail closed");
    assert!(error.to_string().contains("diagnostic mismatch"));
}

#[test]
fn controlled_block_fixture_uses_post_unzen_trace_and_matches_frozen_row() {
    let mut spec = block_row_spec();
    spec.row_id = controlled_block_row_id(&spec).unwrap();
    let fixture = build_controlled_block_fixture(&spec).expect("build production GuestInput");
    let observation = validate_controlled_block_fixture(&fixture)
        .expect("host trace must match every frozen block-row field");

    assert_eq!(observation.row_id, spec.row_id);
    assert_eq!(
        observation.actual_raw_gas_by_key,
        spec.expected_raw_gas_by_key
    );
    assert_eq!(observation.actual_features, spec.expected_features);
    assert_eq!(observation.actual_diagnostics, spec.expected_diagnostics);
    assert!(observation.minimum_block_timestamp > observation.unzen_activation_timestamp);
    assert_eq!(
        observation.operation_phase_ownership,
        "transaction_non_anchor_only"
    );
    assert_eq!(observation.system_operation_ownership, "block_base");
    assert_eq!(observation.anchor_operation_ownership, "block_base");
}

#[test]
fn controlled_block_fixture_rejects_frozen_trace_drift_before_sp1() {
    let mut spec = block_row_spec();
    spec.expected_diagnostics
        .insert("witness_byte_count".into(), 632);
    spec.row_id = controlled_block_row_id(&spec).unwrap();
    let fixture = build_controlled_block_fixture(&spec).expect("build production GuestInput");
    let error = validate_controlled_block_fixture(&fixture)
        .expect_err("host trace drift must reject the row before a caller can launch SP1");
    assert!(error.to_string().contains("diagnostic mismatch"));
}

#[test]
fn workload_id_ignores_execution_provenance_but_binds_semantics() {
    let original = workload_spec();
    let original_id = controlled_workload_id(&original).unwrap();
    assert_eq!(
        original_id, "fbfd36dc326e1bc58fed254ed42f3afdba381531aeb8e002decd00d66ae229cf",
        "Rust identity must equal Python canonical_json identity",
    );

    let execution_a = ControlledExecutionIdentity {
        backend: "sp1".into(),
        backend_input_sha256: "a".repeat(64),
        repeat_index: 0,
        run_id: "run-a".into(),
        workload_id: original_id.clone(),
    };
    let execution_b = ControlledExecutionIdentity {
        backend: "risc0".into(),
        backend_input_sha256: "b".repeat(64),
        repeat_index: 2,
        run_id: "run-b".into(),
        workload_id: original_id.clone(),
    };
    assert_eq!(controlled_workload_id(&original).unwrap(), original_id);
    assert_eq!(
        controlled_execution_row_id(&execution_a).unwrap(),
        "94f2ad599df6a69db84315884e9cdb5e9fbef3d1a273eb363045aacfae2f5f52",
        "Rust execution identity must equal Python canonical_json identity",
    );
    assert_ne!(
        controlled_execution_row_id(&execution_a).unwrap(),
        controlled_execution_row_id(&execution_b).unwrap()
    );

    for changed in [
        {
            let mut value = original.clone();
            value.target_count = 5;
            value
        },
        {
            let mut value = original.clone();
            value.lane = ControlledLane::Control;
            value
        },
        {
            let mut value = original.clone();
            value.input.insert("calldata".into(), json!("0x01"));
            value
        },
        {
            let mut value = original.clone();
            value.expected_feature_deltas.insert("tx_base".into(), 1);
            value
        },
    ] {
        assert_ne!(controlled_workload_id(&changed).unwrap(), original_id);
    }
}

#[test]
fn fixed_footprint_rejects_growing_bytecode_or_non_target_work() {
    let fixed = [
        ControlledFootprint {
            target_count: 0,
            tx_gas_limit: 1_000_024,
            bytecode_len: 64,
            input_len: 32,
            non_target_counts: BTreeMap::from([("push1".into(), 4)]),
            non_target_raw_gas: 12,
        },
        ControlledFootprint {
            target_count: 4,
            tx_gas_limit: 1_000_024,
            bytecode_len: 64,
            input_len: 32,
            non_target_counts: BTreeMap::from([("push1".into(), 4)]),
            non_target_raw_gas: 12,
        },
    ];
    validate_fixed_footprint(&fixed).unwrap();

    let mut growing_code = fixed.clone();
    growing_code[1].bytecode_len += 1;
    assert_eq!(
        validate_fixed_footprint(&growing_code)
            .unwrap_err()
            .to_string(),
        "confounded_template: controlled bytecode/input footprint changed"
    );

    let mut growing_helper = fixed.clone();
    growing_helper[1]
        .non_target_counts
        .insert("push1".into(), 5);
    assert_eq!(
        validate_fixed_footprint(&growing_helper)
            .unwrap_err()
            .to_string(),
        "confounded_template: non-target executed work changed"
    );

    let mut changing_gas_limit = fixed.clone();
    changing_gas_limit[1].tx_gas_limit += 1;
    assert_eq!(
        validate_fixed_footprint(&changing_gas_limit)
            .unwrap_err()
            .to_string(),
        "confounded_template: transaction gas limit changed"
    );
}

#[test]
fn precompile_target_and_control_require_identical_loop_and_fold_shape() {
    let target = PairedPrecompileShape {
        lane: ControlledLane::Target,
        target_count: 8,
        input_len: 32,
        loop_iterations: 8,
        output_len: 32,
        folded_bytes_per_iteration: 40,
    };
    let control = PairedPrecompileShape {
        lane: ControlledLane::Control,
        ..target.clone()
    };
    validate_precompile_pair(&target, &control).unwrap();

    let mismatched = PairedPrecompileShape {
        folded_bytes_per_iteration: 39,
        ..control
    };
    assert!(validate_precompile_pair(&target, &mismatched).is_err());
}

#[test]
fn revm_trace_executes_and_binds_the_exact_sp1_input() {
    let input = OpcodeLabInput {
        case: "add".into(),
        scenario: "arithmetic".into(),
        opcode: 0x01,
        target_count: 1,
        target_raw_gas: 3,
        tx_gas_limit: Some(1_000_024),
        bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
        generator_max_count: Some(8),
        fixed_bytecode_len: Some(6),
        storage: None,
    };
    let encoded = bincode::serialize(&input).unwrap();
    let expected_input_sha256 = alloy_primitives::hex::encode(Sha256::digest(encoded));

    let trace = trace_revm_opcode_workload(&input).unwrap();
    let alternate_backend_encoding = OpcodeLabInput {
        fixed_bytecode_len: None,
        ..input.clone()
    };
    let alternate_trace = trace_revm_opcode_workload(&alternate_backend_encoding).unwrap();

    assert_eq!(
        trace.schema_version, 1,
        "stateless trace schema stays compatible"
    );
    assert_eq!(trace.backend_input_sha256, expected_input_sha256);
    assert_eq!(
        trace.backend_input_len,
        bincode::serialize(&input).unwrap().len()
    );
    assert_eq!(trace.target_opcode, 0x01);
    assert_eq!(trace.executed_target_count, 1);
    assert_eq!(trace.executed_target_raw_gas, 3);
    assert_eq!(trace.non_target_counts.get("opcode:0x60"), Some(&2));
    assert_eq!(trace.non_target_counts.get("opcode:0x00"), Some(&1));
    assert_eq!(trace.non_target_raw_gas, 6);
    assert_eq!(trace.total_raw_gas, 9);
    assert_eq!(trace.tx_gas_limit, 1_000_024);
    assert_eq!(trace.bytecode_len, 6);
    assert_eq!(trace.workload_id.len(), 64);
    assert_eq!(trace.workload_id, alternate_trace.workload_id);
    assert_ne!(
        trace.backend_input_sha256,
        alternate_trace.backend_input_sha256
    );
    assert!(matches!(
        ControlledTrace::RevmOpcode(Box::new(trace)),
        ControlledTrace::RevmOpcode(_)
    ));
}

#[test]
fn opcode_identity_evidence_binds_the_deserialized_input_and_trace_identities() {
    let input = OpcodeLabInput {
        case: "add".into(),
        scenario: "arithmetic".into(),
        opcode: 0x01,
        target_count: 1,
        target_raw_gas: 3,
        tx_gas_limit: Some(1_000_024),
        bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
        generator_max_count: Some(8),
        fixed_bytecode_len: Some(6),
        storage: None,
    };

    let identity = controlled_opcode_identity(&input).unwrap();
    let trace = trace_revm_opcode_workload(&input).unwrap();

    assert_eq!(identity.schema_version, 1);
    assert_eq!(identity.input, input);
    assert_eq!(identity.backend_input_sha256, trace.backend_input_sha256);
    assert_eq!(identity.backend_input_len, trace.backend_input_len);
    assert_eq!(identity.workload_id, trace.workload_id);
    assert_eq!(
        identity.transaction_envelope_sha256,
        trace.transaction_envelope_sha256
    );
    assert_eq!(identity.access_list_sha256, trace.access_list_sha256);
    assert_eq!(identity.prestate_sha256, trace.prestate_sha256);
}

#[test]
fn opcode_identity_bundle_is_deterministic_and_contains_a_separate_real_report() {
    let input = OpcodeLabInput {
        case: "add".into(),
        scenario: "arithmetic".into(),
        opcode: 0x01,
        target_count: 1,
        target_raw_gas: 3,
        tx_gas_limit: Some(1_000_024),
        bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
        generator_max_count: Some(8),
        fixed_bytecode_len: Some(6),
        storage: None,
    };

    let first = controlled_opcode_identity_bundle(&input).unwrap();
    let second = controlled_opcode_identity_bundle(&input).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.schema_version, 1);
    assert!(first.expected_public_values.starts_with("0x"));
    assert_eq!(first.expected_public_values.len(), 66);
    assert_eq!(first.identity.input, input);
    assert_eq!(
        first.report.guest_input_sha256,
        format!("0x{}", first.identity.backend_input_sha256)
    );
    assert_eq!(
        first.report.guest_input_bincode_length,
        first.identity.backend_input_len
    );
    let ControlledTrace::RevmOpcode(trace) = &first.report.controlled_trace else {
        panic!("identity bundle must contain a REVM opcode report")
    };
    assert_eq!(
        trace.backend_input_sha256,
        first.identity.backend_input_sha256
    );
}

#[test]
fn native_identity_public_values_match_the_frozen_guest_baseline() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/revm-opcode-lab-add-32.json");
    let input: OpcodeLabInput = serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();

    let bundle = controlled_opcode_identity_bundle(&input).unwrap();

    assert_eq!(
        bundle.expected_public_values,
        "0x9318bc580c9b2aa315a8649bd205867ef84a5d28fecdb187ec57ba86f409ec16"
    );
}

#[test]
fn stateful_sload_trace_uses_osaka_and_records_exact_warm_and_cold_ledgers() {
    let slot = word(7);
    let value = word(9);
    let mut program = Vec::new();
    push32(&mut program, slot);
    program.extend([0x54, 0x50, 0x00]);

    let traces = [
        (OpcodeLabStorageAccess::Warm, 100),
        (OpcodeLabStorageAccess::Cold, 2_100),
    ]
    .map(|(access, raw_gas)| {
        let input = stateful_input(
            "sload",
            access,
            OpcodeLabStorageOperation::Load {
                expected_value: value,
            },
            value,
            slot,
            program.clone(),
            raw_gas,
        );
        (input.clone(), trace_revm_opcode_workload(&input).unwrap())
    });

    for (input, trace) in &traces {
        let workload = controlled_opcode_workload_spec(input);
        assert_eq!(trace.schema_version, 2);
        assert_eq!(trace.evm_spec, "osaka");
        assert_eq!(trace.revm_version, "41.0.0");
        assert_eq!(trace.shared_constructor, "raiko2-opcode-lab");
        assert_eq!(trace.executed_opcode_counts["opcode:0x54"], 1);
        assert_eq!(
            trace.executed_opcode_raw_gas["opcode:0x54"],
            input.target_raw_gas
        );
        assert_eq!(trace.executed_measurement_count, 1);
        assert_eq!(trace.executed_measurement_raw_gas, input.target_raw_gas);
        assert_eq!(trace.result_statuses.get("success"), Some(&1));
        assert_eq!(trace.backend_input_sha256.len(), 64);
        assert_eq!(trace.transaction_envelope_sha256.len(), 64);
        assert_eq!(trace.access_list_sha256.len(), 64);
        assert_eq!(trace.prestate_sha256.len(), 64);
        assert_eq!(trace.bytecode_sha256.len(), 64);
        assert_eq!(trace.program_sha256.len(), 1);
        assert_eq!(
            trace.semantic_check.backend_input_sha256,
            trace.backend_input_sha256
        );
        assert!(trace.semantic_check.passed);
        assert_eq!(workload.environment["evm_spec"], "osaka");
        assert_eq!(
            workload.state["storage"],
            serde_json::to_value(input.storage.as_ref().unwrap()).unwrap()
        );
        assert_eq!(
            trace.semantic_check.programs[0].observed_load_values,
            vec![format!("0x{}", alloy_primitives::hex::encode(value))]
        );
    }
    assert_ne!(
        traces[0].1.access_list_sha256,
        traces[1].1.access_list_sha256
    );
    assert_eq!(traces[0].1.executed_target_raw_gas, 100);
    assert_eq!(traces[1].1.executed_target_raw_gas, 2_100);
}

#[test]
fn stateful_sstore_semantics_cover_clean_branches_and_high_u256_words() {
    let high_slot: alloy_primitives::U256 = alloy_primitives::U256::from(1) << 255;
    let high_value = high_slot + alloy_primitives::U256::from(1);
    let cases = [
        (word(0), word(0), OpcodeLabStorageAccess::Warm, 100),
        (word(0), word(0), OpcodeLabStorageAccess::Cold, 2_200),
        (word(1), word(1), OpcodeLabStorageAccess::Warm, 100),
        (word(1), word(1), OpcodeLabStorageAccess::Cold, 2_200),
        (word(0), word(1), OpcodeLabStorageAccess::Warm, 20_000),
        (word(0), word(1), OpcodeLabStorageAccess::Cold, 22_100),
        (word(1), word(0), OpcodeLabStorageAccess::Warm, 2_900),
        (word(1), word(0), OpcodeLabStorageAccess::Cold, 5_000),
        (word(1), word(2), OpcodeLabStorageAccess::Warm, 2_900),
        (word(1), word(2), OpcodeLabStorageAccess::Cold, 5_000),
        (
            high_slot.to_be_bytes(),
            high_value.to_be_bytes(),
            OpcodeLabStorageAccess::Warm,
            2_900,
        ),
    ];

    for (original, new, access, raw_gas) in cases {
        let slot = high_slot.to_be_bytes();
        let mut program = Vec::new();
        push32(&mut program, new);
        push32(&mut program, slot);
        program.extend([0x55, 0x5b, 0x00]);
        let input = stateful_input(
            "sstore-clean",
            access,
            OpcodeLabStorageOperation::Store {
                current_value: original,
                new_value: new,
            },
            original,
            slot,
            program,
            raw_gas,
        );

        let trace = trace_revm_opcode_workload(&input).unwrap();
        let semantic = check_revm_opcode_semantics(&input).unwrap();
        let expected = format!("0x{}", alloy_primitives::hex::encode(new));
        assert_eq!(trace.executed_opcode_counts["opcode:0x55"], 1);
        assert_eq!(trace.executed_measurement_count, 1);
        assert_eq!(trace.executed_prefix_count, 0);
        assert_eq!(trace.executed_measurement_raw_gas, raw_gas);
        assert_eq!(
            semantic.programs[0].expected_final_storage.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            semantic.programs[0].observed_final_storage.as_deref(),
            Some(expected.as_str())
        );
        assert!(semantic.passed);
    }
}

#[test]
fn dirty_sstore_trace_separates_prefix_from_measured_transition() {
    let slot = word(3);
    for (original, current, new) in [
        (word(0), word(1), word(2)),
        (word(0), word(1), word(0)),
        (word(1), word(2), word(0)),
        (word(1), word(2), word(1)),
    ] {
        let mut program = Vec::new();
        push32(&mut program, current);
        push32(&mut program, slot);
        program.push(0x55);
        push32(&mut program, new);
        push32(&mut program, slot);
        program.extend([0x55, 0x5b, 0x00]);
        let input = stateful_input(
            "sstore-dirty",
            OpcodeLabStorageAccess::Warm,
            OpcodeLabStorageOperation::Store {
                current_value: current,
                new_value: new,
            },
            original,
            slot,
            program,
            100,
        );

        let trace = trace_revm_opcode_workload(&input).unwrap();
        assert_eq!(trace.executed_opcode_counts["opcode:0x55"], 2);
        assert_eq!(trace.executed_measurement_count, 1);
        assert_eq!(trace.executed_prefix_count, 1);
        assert_eq!(trace.executed_target_count, 1);
        assert_eq!(trace.executed_target_raw_gas, 100);
        assert_eq!(trace.non_target_counts["opcode:0x55"], 1);
        assert_eq!(
            trace.semantic_check.programs[0]
                .observed_final_storage
                .as_deref(),
            Some(format!("0x{}", alloy_primitives::hex::encode(new)).as_str())
        );
    }
}

#[test]
fn stateful_control_executes_only_the_declared_reference_and_dirty_prefix() {
    let slot = word(9);
    let mut load_program = Vec::new();
    push32(&mut load_program, slot);
    load_program.extend([0x19, 0x50, 0x00]);
    let mut load = stateful_input(
        "sload-control",
        OpcodeLabStorageAccess::Cold,
        OpcodeLabStorageOperation::Load {
            expected_value: word(7),
        },
        word(7),
        slot,
        load_program,
        3,
    );
    load.opcode = 0x19;
    load.storage.as_mut().unwrap().lane = OpcodeLabStorageLane::Control;

    let load_trace = trace_revm_opcode_workload(&load).unwrap();
    assert_eq!(load_trace.executed_target_count, 1);
    assert_eq!(load_trace.executed_measurement_count, 0);
    assert_eq!(load_trace.executed_prefix_count, 0);
    assert!(
        !load_trace
            .executed_opcode_counts
            .contains_key("opcode:0x54")
    );

    let mut store_program = Vec::new();
    push32(&mut store_program, word(2));
    push32(&mut store_program, slot);
    store_program.push(0x55);
    push32(&mut store_program, word(1));
    push32(&mut store_program, slot);
    store_program.extend([0x50, 0x50, 0x00]);
    let mut store = stateful_input(
        "sstore-dirty-control",
        OpcodeLabStorageAccess::Warm,
        OpcodeLabStorageOperation::Store {
            current_value: word(2),
            new_value: word(1),
        },
        word(1),
        slot,
        store_program,
        2,
    );
    store.opcode = 0x50;
    store.target_count = 2;
    store.storage.as_mut().unwrap().lane = OpcodeLabStorageLane::Control;

    let store_trace = trace_revm_opcode_workload(&store).unwrap();
    assert_eq!(store_trace.executed_target_count, 2);
    assert_eq!(store_trace.executed_measurement_count, 0);
    assert_eq!(store_trace.executed_prefix_count, 1);
    assert_eq!(store_trace.executed_opcode_counts["opcode:0x55"], 1);
    assert_eq!(store_trace.executed_opcode_counts["opcode:0x50"], 2);
    assert_eq!(
        store_trace.semantic_check.programs[0]
            .observed_final_storage
            .as_deref(),
        Some(format!("0x{}", alloy_primitives::hex::encode(word(2))).as_str())
    );
}

#[test]
fn stateful_trace_rejects_a_malformed_declared_transition() {
    let slot = word(0);
    let mut program = Vec::new();
    push32(&mut program, word(2));
    push32(&mut program, slot);
    program.extend([0x55, 0x5b, 0x00]);
    let input = stateful_input(
        "sstore-malformed",
        OpcodeLabStorageAccess::Warm,
        OpcodeLabStorageOperation::Store {
            current_value: word(0),
            new_value: word(1),
        },
        word(0),
        slot,
        program,
        20_000,
    );

    let error = trace_revm_opcode_workload(&input)
        .expect_err("declared transition must match the executed program");
    assert!(error.to_string().contains("declared SSTORE operation"));
}

#[test]
fn revm_trace_executes_the_osaka_clz_canary() {
    let input = OpcodeLabInput {
        case: "clz".into(),
        scenario: "osaka-canary".into(),
        opcode: 0x1e,
        target_count: 1,
        target_raw_gas: 5,
        tx_gas_limit: Some(100_000),
        bytecode: vec![0x60, 0x01, 0x1e, 0x00],
        generator_max_count: None,
        fixed_bytecode_len: Some(4),
        storage: None,
    };

    let trace = trace_revm_opcode_workload(&input).unwrap();
    assert_eq!(trace.evm_spec, "osaka");
    assert_eq!(trace.executed_target_count, 1);
    assert_eq!(trace.result_statuses.get("success"), Some(&1));
}

#[test]
fn precompile_trace_binds_typed_lane_and_exact_sp1_input() {
    let target = PrecompileLabInput {
        case: "identity".into(),
        scenario: "free text".into(),
        lane: PrecompileLabLane::Target,
        address: 4,
        target_count: 2,
        input_size: 4,
        target_raw_gas: 18,
        expected_output_size: Some(4),
        input: vec![1, 2, 3, 4],
    };
    let control = PrecompileLabInput {
        lane: PrecompileLabLane::Control,
        ..target.clone()
    };

    let target_trace = trace_precompile_workload(&target).unwrap();
    let control_trace = trace_precompile_workload(&control).unwrap();

    assert_eq!(
        target_trace.backend_input_sha256,
        alloy_primitives::hex::encode(Sha256::digest(bincode::serialize(&target).unwrap()))
    );
    assert_eq!(target_trace.pair_id, control_trace.pair_id);
    assert_ne!(target_trace.workload_id, control_trace.workload_id);
    assert_eq!(target_trace.lane, ControlledLane::Target);
    assert_eq!(control_trace.lane, ControlledLane::Control);
    assert_eq!(target_trace.output_len, 4);
    assert_eq!(target_trace.folded_bytes_per_iteration, 12);
    assert_eq!(
        controlled_precompile_workload_spec(&target).expected_operation_deltas["precompile:0x04"],
        2
    );
    assert_eq!(
        controlled_precompile_workload_spec(&control).expected_operation_deltas["precompile:0x04"],
        0
    );
    assert!(matches!(
        ControlledTrace::Precompile(target_trace),
        ControlledTrace::Precompile(_)
    ));
}

#[test]
fn revm_trace_rejects_declared_count_or_raw_gas_that_execution_does_not_match() {
    let input = OpcodeLabInput {
        case: "add".into(),
        scenario: "arithmetic".into(),
        opcode: 0x01,
        target_count: 2,
        target_raw_gas: 3,
        tx_gas_limit: Some(1_000_024),
        bytecode: vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00],
        generator_max_count: Some(8),
        fixed_bytecode_len: Some(6),
        storage: None,
    };
    assert!(
        trace_revm_opcode_workload(&input)
            .unwrap_err()
            .to_string()
            .contains("executed target count")
    );

    let wrong_gas = OpcodeLabInput {
        target_count: 1,
        target_raw_gas: 5,
        ..input
    };
    assert!(
        trace_revm_opcode_workload(&wrong_gas)
            .unwrap_err()
            .to_string()
            .contains("executed target raw gas")
    );
}

#[test]
fn fixed_footprint_add_sweep_has_constant_real_non_target_execution() {
    fn add_fixture(target_count: u64, max_count: u64) -> OpcodeLabInput {
        let programs = (0..max_count)
            .map(|index| {
                let mut program = Vec::new();
                for _ in 0..2 {
                    program.push(0x7f);
                    program.extend([0u8; 32]);
                }
                program.extend(if index < target_count {
                    [0x01, 0x00]
                } else {
                    [0x00, 0x01]
                });
                program
            })
            .collect::<Vec<_>>();
        let mut bytecode = vec![0xef, 0x4d, 0x50, 0x01];
        bytecode.extend(u32::try_from(programs.len()).unwrap().to_be_bytes());
        for program in programs {
            bytecode.extend(u32::try_from(program.len()).unwrap().to_be_bytes());
            bytecode.extend(program);
        }
        OpcodeLabInput {
            case: "add".into(),
            scenario: "arithmetic".into(),
            opcode: 0x01,
            target_count,
            target_raw_gas: 3,
            tx_gas_limit: Some(1_000_000 + max_count * 3),
            fixed_bytecode_len: Some(bytecode.len() as u64),
            generator_max_count: Some(max_count),
            bytecode,
            storage: None,
        }
    }

    let traces =
        [0, 1, 2, 4, 8].map(|count| trace_revm_opcode_workload(&add_fixture(count, 8)).unwrap());
    let reference = &traces[0];
    for trace in &traces[1..] {
        assert_eq!(trace.bytecode_len, reference.bytecode_len);
        assert_eq!(trace.non_target_counts, reference.non_target_counts);
        assert_eq!(trace.non_target_raw_gas, reference.non_target_raw_gas);
        assert_eq!(trace.tx_gas_limit, reference.tx_gas_limit);
    }
}

#[test]
fn required_overhead_fixtures_reconstruct_through_the_production_proposal_path() {
    let fixtures = build_required_overhead_fixtures(2).expect("build controlled overhead fixtures");
    let observations = validate_required_overhead_fixtures(&fixtures)
        .expect("real executor observations must match declared overhead deltas");
    assert_eq!(fixtures.len(), 10);
    for key in [
        "proposal_startup",
        "block_base",
        "tx_base",
        "native_value_transfer",
    ] {
        assert!(
            fixtures
                .iter()
                .any(|fixture| fixture.overhead_key_id == key),
            "missing required overhead key {key}",
        );
    }
    assert_eq!(
        fixtures
            .iter()
            .filter(|fixture| fixture.overhead_key_id == "proposal_startup")
            .map(|fixture| fixture.lane)
            .collect::<Vec<_>>(),
        vec![
            ControlledOverheadLane::Target,
            ControlledOverheadLane::Target,
        ],
    );
    assert!(
        fixtures
            .iter()
            .filter(|fixture| fixture.overhead_key_id == "proposal_startup")
            .all(|fixture| fixture.baseline_kind.as_deref() == Some("mathematical_zero_baseline"))
    );
    for observation in &observations {
        if matches!(
            observation.case_id.as_str(),
            "tx_base_no_code_no_value"
                | "tx_base_minimal_contract_call"
                | "native_transfer_positive_vs_zero"
        ) && observation.lane == ControlledOverheadLane::Target
        {
            assert_eq!(observation.started_candidate_transaction_count, 2);
            assert_eq!(observation.committed_candidate_transaction_count, 2);
            assert_eq!(observation.unattempted_candidate_transaction_count, 0);
        }
        if observation.case_id == "tx_base_minimal_contract_call"
            && observation.lane == ControlledOverheadLane::Target
        {
            let serialized = serde_json::to_value(observation).expect("serialize observation");
            assert_eq!(
                serialized["observed_operation_deltas"],
                json!({
                    "opcode:0x5f": {
                        "pricing_basis": "raw_gas_slope",
                        "units": 4,
                        "event_count": 2,
                    }
                }),
                "two PUSH0 events consume two raw-gas units each; zero-gas implicit STOP is omitted",
            );
            assert_eq!(
                serialized["observed_operation_deltas"],
                serialized["expected_operation_deltas"]
            );
            assert_eq!(
                serialized["operation_phase_ownership"],
                "transaction_non_anchor_only"
            );
            assert_eq!(serialized["system_operation_ownership"], "block_base");
            assert!(serialized.get("absolute_operation_counts").is_none());
        }
        match observation.case_id.as_str() {
            "block_base_one_vs_two_minimal_blocks"
                if observation.lane == ControlledOverheadLane::Target =>
            {
                assert_eq!(observation.expected_feature_deltas["tx_base"], 0);
            }
            "startup_minimal_no_candidate_tx" => {
                assert_eq!(observation.absolute_feature_counts["tx_base"], 0);
            }
            "startup_minimal_one_no_code_tx" => {
                assert_eq!(observation.absolute_feature_counts["tx_base"], 1);
            }
            _ => {}
        }
    }
    for fixture in fixtures {
        raiko2_guest_common::prove_shasta_proposal(&fixture.guest_input).unwrap_or_else(|error| {
            let traced = raiko2_zkgas_trace::trace_shasta_proposal(&fixture.guest_input);
            panic!(
                "{} {:?} failed production proposal reconstruction: {error:?}; traced={traced:?}",
                fixture.case_id, fixture.lane
            )
        });
    }
}

#[test]
fn controlled_overhead_identity_uses_the_backend_guest_input_serialization() {
    let fixtures = build_required_overhead_fixtures(2).expect("build controlled overhead fixtures");
    let observations = validate_required_overhead_fixtures(&fixtures)
        .expect("real executor observations must match declared overhead deltas");
    let fixture = fixtures.first().expect("at least one overhead fixture");
    let observation = observations
        .first()
        .expect("at least one overhead observation");
    let guest_input_bincode =
        bincode::serialize(&fixture.guest_input).expect("serialize GuestInput");
    let expected_input_sha256 = alloy_primitives::hex::encode(Sha256::digest(&guest_input_bincode));

    assert_eq!(
        observation.workload_spec.guest_input_canonical_sha256,
        expected_input_sha256
    );
    assert_eq!(observation.backend_input_sha256, expected_input_sha256);
    assert_eq!(
        observation.guest_input_sha256.trim_start_matches("0x"),
        expected_input_sha256
    );
    assert_eq!(
        observation.guest_input_bincode_length,
        guest_input_bincode.len()
    );

    let workload_identity = BTreeMap::from([
        ("kind", json!("controlled")),
        (
            "workload_spec",
            serde_json::to_value(&observation.workload_spec).expect("serialize workload spec"),
        ),
    ]);
    let expected_workload_id = alloy_primitives::hex::encode(Sha256::digest(
        serde_json::to_vec(&workload_identity).expect("serialize workload identity"),
    ));
    assert_eq!(observation.workload_id, expected_workload_id);
    assert_eq!(
        controlled_overhead_workload_id(fixture).expect("derive workload ID"),
        expected_workload_id
    );

    let rebuilt = build_required_overhead_fixtures(2).expect("rebuild overhead fixtures");
    let rebuilt_observations =
        validate_required_overhead_fixtures(&rebuilt).expect("rebuild real executor observations");
    assert_eq!(
        bincode::serialize(&rebuilt.first().unwrap().guest_input).unwrap(),
        guest_input_bincode
    );
    assert_eq!(rebuilt_observations.first(), Some(observation));
}

#[test]
fn controlled_operation_units_preserve_event_count_and_exclude_synthetic_eof_stop() {
    let fixtures = build_required_overhead_fixtures(2).expect("build controlled overhead fixtures");
    let observations = validate_required_overhead_fixtures(&fixtures)
        .expect("real executor observations must match declared overhead deltas");
    let contract = observations
        .iter()
        .find(|observation| {
            observation.case_id == "tx_base_minimal_contract_call"
                && observation.lane == ControlledOverheadLane::Target
        })
        .expect("minimal contract target observation");
    let push0 = contract
        .observed_operation_deltas
        .get("opcode:0x5f")
        .expect("retained PUSH0 pricing row");

    assert_eq!(push0.pricing_basis, PricingBasis::RawGasSlope);
    assert_eq!(push0.units, 2 * push0.event_count);
    assert_eq!(push0.event_count, 2);
    assert_eq!(contract.workload_spec.schema_version, 2);
    assert!(
        !contract
            .absolute_operation_pricing_units
            .contains_key("opcode:0x00"),
        "synthetic zero-pricing-unit EOF STOP must stay outside the canonical pricing ledger",
    );
    assert!(
        !contract
            .observed_operation_deltas
            .contains_key("opcode:0x00")
    );
}

fn operation_units(
    pricing_basis: PricingBasis,
    units: i64,
    event_count: i64,
) -> ControlledOperationUnits {
    ControlledOperationUnits {
        pricing_basis,
        units,
        event_count,
    }
}

#[test]
fn operation_units_delta_subtracts_units_and_events_independently() {
    let target = BTreeMap::from([(
        "opcode:0x5f".into(),
        operation_units(PricingBasis::RawGasSlope, 10, 5),
    )]);
    let control = BTreeMap::from([(
        "opcode:0x5f".into(),
        operation_units(PricingBasis::RawGasSlope, 4, 2),
    )]);

    assert_eq!(
        operation_units_delta(&target, &control).expect("positive delta"),
        BTreeMap::from([(
            "opcode:0x5f".into(),
            operation_units(PricingBasis::RawGasSlope, 6, 3),
        )]),
    );
    assert_eq!(
        operation_units_delta(&control, &target).expect("negative delta"),
        BTreeMap::from([(
            "opcode:0x5f".into(),
            operation_units(PricingBasis::RawGasSlope, -6, -3),
        )]),
    );
    assert!(
        operation_units_delta(&target, &target)
            .expect("zero delta")
            .is_empty()
    );
}

#[test]
fn operation_units_delta_retains_a_key_when_only_one_component_is_zero() {
    let target = BTreeMap::from([(
        "opcode:0x5f".into(),
        operation_units(PricingBasis::RawGasSlope, 4, 3),
    )]);
    let control = BTreeMap::from([(
        "opcode:0x5f".into(),
        operation_units(PricingBasis::RawGasSlope, 4, 1),
    )]);

    assert_eq!(
        operation_units_delta(&target, &control).expect("event-only delta"),
        BTreeMap::from([(
            "opcode:0x5f".into(),
            operation_units(PricingBasis::RawGasSlope, 0, 2),
        )]),
    );
}

#[test]
fn operation_units_delta_rejects_basis_changes_and_event_count_overflow() {
    let raw = BTreeMap::from([(
        "operation".into(),
        operation_units(PricingBasis::RawGasSlope, 1, i64::MAX),
    )]);
    let fixed = BTreeMap::from([(
        "operation".into(),
        operation_units(PricingBasis::FixedPerEvent, 1, 1),
    )]);
    assert!(
        operation_units_delta(&raw, &fixed)
            .expect_err("one key cannot change pricing basis")
            .to_string()
            .contains("changes pricing basis")
    );

    let negative_one = BTreeMap::from([(
        "operation".into(),
        operation_units(PricingBasis::RawGasSlope, 0, -1),
    )]);
    assert!(
        operation_units_delta(&raw, &negative_one)
            .expect_err("event-count subtraction must be checked")
            .to_string()
            .contains("event count overflow")
    );
}

#[test]
fn max_128_block_overhead_fixture_is_post_unzen_and_traces_all_129_blocks() {
    let fixtures = build_required_overhead_fixtures(128).expect("build max-128 fixtures");
    let fixture = fixtures
        .iter()
        .find(|fixture| {
            fixture.case_id == "block_base_one_vs_two_minimal_blocks"
                && fixture.lane == ControlledOverheadLane::Target
        })
        .expect("block-base target fixture");
    let trace = raiko2_zkgas_trace::trace_shasta_proposal(&fixture.guest_input)
        .expect("trace max-128 block fixture");
    assert_eq!(trace.status, ProposalTraceStatus::Complete, "{trace:?}");
    assert_eq!(trace.blocks.len(), 129);

    let chain_spec = SupportedChainSpecs::default()
        .get_chain_spec_with_chain_id(167_000)
        .expect("mainnet chain spec");
    let unzen_timestamp = match chain_spec.hard_forks.get(&ForkId::Taiko(TaikoFork::Unzen)) {
        Some(ForkCondition::Timestamp(timestamp)) => *timestamp,
        other => panic!("expected canonical mainnet Unzen timestamp, got {other:?}"),
    };
    let last_block_timestamp = fixture
        .guest_input
        .witnesses
        .last()
        .expect("last witness")
        .block
        .header
        .timestamp;
    assert!(
        fixture
            .guest_input
            .witnesses
            .iter()
            .all(|witness| witness.block.header.timestamp > unzen_timestamp)
    );
    assert!(
        fixture
            .guest_input
            .taiko
            .proposal_event
            .proposal
            .timestamp
            .to::<u64>()
            >= last_block_timestamp
    );
}

#[test]
fn overhead_trace_failure_names_fixture_stage_and_error() {
    let fixtures = build_required_overhead_fixtures(2).expect("build fixtures");
    let mut fixture = fixtures
        .into_iter()
        .find(|fixture| {
            fixture.case_id == "block_base_one_vs_two_minimal_blocks"
                && fixture.lane == ControlledOverheadLane::Target
        })
        .expect("block-base target fixture");
    let first_block_timestamp = fixture.guest_input.witnesses[0].block.header.timestamp;
    fixture.guest_input.taiko.proposal_event.proposal.timestamp = first_block_timestamp
        .try_into()
        .expect("timestamp fits u48");
    fixture
        .guest_input
        .proof_carry_data
        .transition_input
        .transition
        .timestamp = first_block_timestamp;
    fixture
        .guest_input
        .proof_carry_data
        .transition_input
        .proposal_hash = hash_proposal(&fixture.guest_input.taiko.proposal_event.proposal);

    let error = validate_required_overhead_fixtures(&[fixture])
        .expect_err("proposal timestamp before the final block must fail");
    let message = format!("{error:#}");
    assert!(
        message.contains("block_base_one_vs_two_minimal_blocks Target"),
        "{message}"
    );
    assert!(message.contains("stage=ordinary"), "{message}");
    assert!(
        message.contains("witness count (3) does not match derived manifest block count (1)"),
        "{message}"
    );
}

#[test]
fn controlled_overhead_identity_binds_semantics_beyond_shared_guest_input() {
    let fixtures = build_required_overhead_fixtures(2).expect("build fixtures");
    let base = fixtures
        .iter()
        .find(|fixture| {
            fixture.case_id == "tx_base_no_code_no_value"
                && fixture.lane == ControlledOverheadLane::Control
        })
        .expect("base control")
        .clone();
    let mut changed = base.clone();
    changed.case_id = "different_case".into();
    assert_ne!(
        controlled_overhead_workload_id(&base).unwrap(),
        controlled_overhead_workload_id(&changed).unwrap()
    );
    changed = base.clone();
    changed.lane = ControlledOverheadLane::Target;
    assert_ne!(
        controlled_overhead_workload_id(&base).unwrap(),
        controlled_overhead_workload_id(&changed).unwrap()
    );
    changed = base.clone();
    changed.target_count += 1;
    assert_ne!(
        controlled_overhead_workload_id(&base).unwrap(),
        controlled_overhead_workload_id(&changed).unwrap()
    );
}

#[test]
fn controlled_overhead_rejects_an_unexpected_transaction_operation() {
    let fixtures = build_required_overhead_fixtures(2).expect("build fixtures");
    let mut pair = fixtures
        .into_iter()
        .filter(|fixture| fixture.case_id == "tx_base_minimal_contract_call")
        .collect::<Vec<_>>();
    pair.iter_mut()
        .find(|fixture| fixture.lane == ControlledOverheadLane::Target)
        .expect("target fixture")
        .expected_operation_deltas
        .clear();
    let error = validate_required_overhead_fixtures(&pair)
        .expect_err("an empty declaration must not accept observed PUSH0 pricing units");
    assert!(
        error
            .to_string()
            .contains("controlled overhead operation delta mismatch"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn zero_count_overhead_fixtures_omit_zero_operation_units() {
    let fixtures = build_required_overhead_fixtures(0).expect("build zero-count fixtures");
    let observations = validate_required_overhead_fixtures(&fixtures)
        .expect("zero-count fixtures must pass the real proposal trace path");
    let contract = observations
        .iter()
        .find(|observation| {
            observation.case_id == "tx_base_minimal_contract_call"
                && observation.lane == ControlledOverheadLane::Target
        })
        .expect("minimal-contract target observation");
    assert!(contract.expected_operation_deltas.is_empty());
    assert!(contract.observed_operation_deltas.is_empty());
    assert!(contract.workload_spec.expected_operation_deltas.is_empty());
}
