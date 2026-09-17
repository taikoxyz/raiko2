#[path = "../src/controlled_workload.rs"]
mod controlled_workload;

use std::collections::BTreeMap;

use controlled_workload::{
    ControlledExecutionIdentity, ControlledFootprint, ControlledLane, ControlledOverheadLane,
    ControlledTrace, ControlledWorkloadSpec, PairedPrecompileShape,
    build_required_overhead_fixtures, controlled_execution_row_id, controlled_overhead_workload_id,
    controlled_precompile_workload_spec, controlled_workload_id, trace_precompile_workload,
    trace_revm_opcode_workload, validate_fixed_footprint, validate_precompile_pair,
    validate_required_overhead_fixtures,
};
use raiko2_primitives::{OpcodeLabInput, PrecompileLabInput, PrecompileLabLane};
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
            bytecode_len: 64,
            input_len: 32,
            non_target_counts: BTreeMap::from([("push1".into(), 4)]),
            non_target_raw_gas: 12,
        },
        ControlledFootprint {
            target_count: 4,
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
