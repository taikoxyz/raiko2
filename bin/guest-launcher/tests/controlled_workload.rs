#[path = "../src/controlled_workload.rs"]
mod controlled_workload;

use std::collections::BTreeMap;

use alloy_consensus::Transaction as _;
use alloy_primitives::{Address, B256};
use controlled_workload::{
    ControlledBlockRowSpec, ControlledBlockSplit, ControlledExecutionIdentity, ControlledFootprint,
    ControlledLane, ControlledOverheadLane, ControlledProgram, ControlledTrace,
    ControlledWorkloadSpec, PairedPrecompileShape, build_controlled_block_fixture,
    build_controlled_block_fixture_with_extra_prestate_account_for_test,
    build_required_overhead_fixtures, controlled_block_row_id, controlled_execution_row_id,
    controlled_overhead_workload_id, controlled_precompile_workload_spec, controlled_workload_id,
    observe_controlled_block_fixture, trace_precompile_workload, trace_revm_opcode_workload,
    validate_controlled_block_fixture, validate_fixed_footprint, validate_precompile_pair,
    validate_required_overhead_fixtures,
};
use raiko2_primitives::{
    OpcodeLabInput, PrecompileLabInput, PrecompileLabLane, SupportedChainSpecs,
    chain_spec::{ForkCondition, ForkId, TaikoFork},
};
use raiko2_protocol_shasta::libhash::hash_proposal;
use raiko2_zkgas_trace::ProposalTraceStatus;
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
                "37c594d56e8c7220efd843b3f7844205b23c64689400a5a32ce0308ba6a597e5",
            )
            .unwrap(),
        ),
        expected_raw_gas_by_key: BTreeMap::from([
            ("opcode:0x03".into(), 6),
            ("opcode:0x15".into(), 6),
            ("opcode:0x50".into(), 4),
            ("opcode:0x56".into(), 8),
            ("opcode:0x57".into(), 20),
            ("opcode:0x5a".into(), 2),
            ("opcode:0x5b".into(), 3),
            ("opcode:0x60".into(), 15),
            ("opcode:0x62".into(), 3),
            ("opcode:0x80".into(), 6),
            ("opcode:0x90".into(), 6),
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
        split: if count == 32 {
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
fn opcode_family_counts_change_trace_but_preserve_final_state_root() {
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
        let final_state_root = observations[0].actual_final_state_root;
        assert!(
            observations
                .iter()
                .all(|observation| observation.actual_final_state_root == final_state_root),
            "{family} count variants changed final state root"
        );
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
                    && observation
                        .actual_raw_gas_by_key
                        .contains_key("opcode:0x5a")
            }),
            "{family} trace did not replace CALLDATALOAD with modeled GAS"
        );
        let shapes = materialized
            .iter()
            .map(|(_, _, shape)| shape)
            .collect::<Vec<_>>();
        assert!(
            shapes.iter().all(|shape| shape.0 == shapes[0].0),
            "{family} count variants changed bytecode"
        );
        assert!(
            shapes.iter().all(|shape| shape.1 == 0),
            "{family} count variants must use empty transaction input"
        );
        let gas_limit_base = shapes[0].2 - counts[0];
        assert!(
            shapes
                .iter()
                .zip(counts)
                .all(|(shape, count)| shape.2 == gas_limit_base + count),
            "{family} transaction gas limits must be BASE + count"
        );
    }
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
    };
    let encoded = bincode::serialize(&input).unwrap();
    let expected_input_sha256 = alloy_primitives::hex::encode(Sha256::digest(encoded));

    let trace = trace_revm_opcode_workload(&input).unwrap();
    let alternate_backend_encoding = OpcodeLabInput {
        fixed_bytecode_len: None,
        ..input.clone()
    };
    let alternate_trace = trace_revm_opcode_workload(&alternate_backend_encoding).unwrap();

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
        ControlledTrace::RevmOpcode(trace),
        ControlledTrace::RevmOpcode(_)
    ));
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
