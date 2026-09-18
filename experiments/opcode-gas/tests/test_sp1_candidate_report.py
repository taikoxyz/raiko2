import copy
import pathlib
import sys
import tempfile
import unittest
from unittest import mock
from decimal import Decimal, getcontext

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from test_manifest import CONTROLLED_SCHEDULE_KEYS, controlled_manifest_data
from test_manifest import fixture_schedule


def controlled_manifest():
    return opcode_gas.parse_controlled_manifest(
        controlled_manifest_data(), schedule_keys=CONTROLLED_SCHEDULE_KEYS
    )


def repeated_point(count, prover_gas, instruction_count=None, **extra):
    if instruction_count is None:
        instruction_count = prover_gas * 2
    return {
        "count": count,
        "prover_gas_repeats": [str(prover_gas)] * 3,
        "instruction_count_repeats": [str(instruction_count)] * 3,
        "case_input_sha256_repeats": ["a" * 64] * 3,
        "exit_code_repeats": [0, 0, 0],
        "public_values_repeats": ["0x01", "0x01", "0x01"],
        "isolation": {
            "status": "passed",
            "bytecode_size": 64,
            "input_size": 32,
            "non_target_counts": {"push1": 4},
            "non_target_raw_gas": "12",
        },
        **extra,
    }


def linear_observations(slope=1200, intercept=10000, max_count=8):
    return [
        repeated_point(count, intercept + slope * count)
        for count in [0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048]
        if count <= max_count
    ]


def accepted_case(case_id, key_id, basis, slope, raw_gas=None):
    result = {
        "case_id": case_id,
        "measurement_key_id": key_id,
        "pricing_basis": basis,
        "status": "accepted",
        "g_p": str(slope),
        "checkpoint": {
            "count": 8,
            "observed_delta_p": str(8 * slope),
            "predicted_delta_p": str(8 * slope),
            "ape_p": "0",
            "status": "passed",
        },
        "secondary": {"status": "available", "g_s": str(slope * 2)},
    }
    if raw_gas is not None:
        result["target_raw_gas"] = str(raw_gas)
    return result


def persist_execution_identity(root, manifest, revision="a" * 40):
    guest_artifact = root / "sp1-test.elf"
    guest_artifact.write_bytes(b"test SP1 guest artifact")
    guest_artifacts = {
        guest_artifact.name: opcode_gas.sha256_file(guest_artifact)
    }
    identity = {
        "implementation_revision": revision,
        "alethia_reth_revision": "d" * 40,
        "rust_version": "rustc test",
        "sp1_sdk_version": "test-sdk",
        "controlled_manifest_sha256": "b" * 64,
        "controlled_manifest_rows_sha256": "c" * 64,
        "complete_schedule_sha256": "e" * 64,
        "guest_artifacts": guest_artifacts,
        "guest_artifacts_sha256": opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(guest_artifacts)
        ),
        "normalization_reference_key": "opcode:0x01",
        "sp1_execution_parameters": {"mode": "execute"},
        "primary_metric": "proverGas",
        "sp1_instruction_count": "secondary_non_gating",
        "workload_identity_schema_version": 1,
        "workload_canonicalization": "sha256(canonical_json(workload_spec))",
        "primary_formulas": {"candidate_cost": "g_p(k) / r(k)"},
        "q_formula": list(opcode_gas.Q_FORMULA),
        "out_of_fit_checkpoint": {"mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS},
        "quality_gates": {"checkpoint_ape_max": 0.10},
        "bridge": {"model": manifest.bridge_model},
    }
    calibration_id = opcode_gas.sha256_bytes(opcode_gas.canonical_json(identity))[:24]
    run = root / calibration_id
    run.mkdir(parents=True, exist_ok=True)
    experiment = {
        "schema_version": 1,
        "calibration_id": calibration_id,
        "dirty_state": False,
        "calibration_identity": identity,
        **{
            key: identity[key]
            for key in opcode_gas.EXPERIMENT_IDENTITY_DUPLICATE_FIELDS
        },
    }
    (run / "experiment.json").write_text(opcode_gas.json.dumps(experiment) + "\n")
    provenance = opcode_gas.experiment_provenance_declaration(experiment)
    (run / "provenance.json").write_text(opcode_gas.json.dumps(provenance) + "\n")
    return run, identity


def persist_completed_controlled_run(root, manifest, rows, overhead, revision="a" * 40):
    """Create the exact persisted artifacts consumed by candidate sealing."""
    run, identity = persist_execution_identity(root, manifest, revision)

    fit = {
        "schema_version": 1,
        "generator_max_count": 8,
        "case_results": rows,
    }
    overhead = {
        **overhead,
        "generator_max_count": 8,
        "overhead_generator_max_count": 8,
    }
    fit_path = run / "controlled-fit.json"
    overhead_path = run / "controlled-overheads.json"
    raw_path = run / "controlled-runs.generator-max-8.jsonl"
    overhead_raw_path = run / "controlled-overhead-runs.generator-max-8.jsonl"
    fit_path.write_text(opcode_gas.json.dumps(fit) + "\n")
    overhead_path.write_text(opcode_gas.json.dumps(overhead) + "\n")
    raw_path.write_text("{}\n")
    overhead_raw_path.write_text("{}\n")
    decision = {
        "generator_max_count": 8,
        "raw_runs": raw_path.name,
        "raw_runs_sha256": opcode_gas.sha256_file(raw_path),
        "fit": fit_path.name,
        "fit_sha256": opcode_gas.sha256_file(fit_path),
        "overhead_runs": overhead_raw_path.name,
        "overhead_runs_sha256": opcode_gas.sha256_file(overhead_raw_path),
        "overhead_generator_max_count": 8,
        "overhead_fit": overhead_path.name,
        "overhead_fit_sha256": opcode_gas.sha256_file(overhead_path),
        "decision": "complete",
    }
    (run / "controlled-decisions.json").write_text(
        opcode_gas.json.dumps({"schema_version": 1, "rounds": [decision]}) + "\n"
    )
    (run / "controlled-decisions.sha256").write_text(
        opcode_gas.sha256_file(run / "controlled-decisions.json") + "\n"
    )
    bridge = {
        "schema_version": 1,
        "implementation_revision": revision,
        "bridge_key_ids": list(manifest.bridge_key_ids),
        "model": manifest.bridge_model,
        "controlled_ape_max": "0.10",
        "proposal_ape_max": "0.10",
        "missing_data": "insufficient_data_is_sealable_and_non_gating",
    }
    (run / "bridge").mkdir()
    (run / "bridge" / "bridge-manifest.json").write_text(
        opcode_gas.json.dumps(bridge) + "\n"
    )
    return run, identity


class MeasurementGateTests(unittest.TestCase):
    def test_accepts_first_passing_prefix_and_excludes_checkpoint_from_fit(self):
        observations = linear_observations(max_count=32)
        result = opcode_gas.evaluate_controlled_sweep(
            observations,
            pricing_basis="raw_gas_slope",
            target_raw_gas=3,
            generator_max_count=32,
        )

        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["selected_counts"], [0, 1, 2, 4])
        self.assertEqual(result["checkpoint"]["count"], 8)
        self.assertNotIn(8, result["ols_counts"])
        self.assertEqual(result["g_p"], "1200")
        self.assertEqual(result["c_p"], "400")

    def test_rejects_repeat_identity_noise_and_primary_quality_failures(self):
        mutations = []
        for field, value, reason in [
            ("case_input_sha256_repeats", ["a" * 64, "b" * 64, "a" * 64], "repeat_identity"),
            ("exit_code_repeats", [0, 1, 0], "repeat_identity"),
            ("public_values_repeats", ["0x01", "0x02", "0x01"], "repeat_identity"),
            ("prover_gas_repeats", ["10000", "10001", "10000"], "repeat_noise_p"),
        ]:
            rows = linear_observations()
            rows[0][field] = value
            mutations.append((rows, reason))
        flat = [repeated_point(count, 10_000) for count in [0, 1, 2, 4, 8]]
        mutations.append((flat, "primary_slope"))
        low_signal = [repeated_point(count, 10_000 + count) for count in [0, 1, 2, 4, 8]]
        mutations.append((low_signal, "primary_signal"))
        nonlinear = [
            repeated_point(count, value)
            for count, value in zip([0, 1, 2, 4, 8], [10_000, 11_200, 12_400, 15_000, 30_000])
        ]
        mutations.append((nonlinear, "extrapolation_check_failed"))

        for rows, reason in mutations:
            with self.subTest(reason=reason):
                result = opcode_gas.evaluate_controlled_sweep(
                    rows,
                    pricing_basis="raw_gas_slope",
                    target_raw_gas=3,
                    generator_max_count=8,
                )
                self.assertEqual(result["status"], "rejected")
                self.assertIn(reason, result["reasons"])

    def test_checkpoint_boundary_missing_nonpositive_and_generator_bound(self):
        passing = linear_observations(slope=1100, max_count=8)
        passing[-1]["prover_gas_repeats"] = ["18000"] * 3  # predicted 8800, observed 8000: exact 10%
        accepted = opcode_gas.evaluate_controlled_sweep(
            passing,
            pricing_basis="raw_gas_slope",
            target_raw_gas=3,
            generator_max_count=8,
        )
        self.assertEqual(accepted["status"], "accepted")

        failing = linear_observations(slope=1100, max_count=8)
        failing[-1]["prover_gas_repeats"] = ["17999"] * 3  # predicted 8800, observed 7999: >10%
        rejected = opcode_gas.evaluate_controlled_sweep(
            failing,
            pricing_basis="raw_gas_slope",
            target_raw_gas=3,
            generator_max_count=16,
        )
        self.assertEqual(rejected["status"], "rejected")
        self.assertEqual(rejected["selected_counts"], [0, 1, 2, 4])
        self.assertIn("extrapolation_check_failed", rejected["reasons"])

        for rows, max_count in [
            (linear_observations(max_count=4), 4),
            (linear_observations(max_count=8), 7),
        ]:
            with self.subTest(max_count=max_count):
                result = opcode_gas.evaluate_controlled_sweep(
                    rows,
                    pricing_basis="raw_gas_slope",
                    target_raw_gas=3,
                    generator_max_count=max_count,
                )
                self.assertEqual(result["status"], "rejected")
                self.assertIn("checkpoint", " ".join(result["reasons"]))

    def test_all_frozen_prefix_to_checkpoint_mappings(self):
        self.assertEqual(
            opcode_gas.CONTROLLED_PREFIXES,
            (
                (0, 1, 2, 4),
                (0, 1, 2, 4, 8, 16),
                (0, 1, 2, 4, 8, 16, 32, 64),
                (0, 1, 2, 4, 8, 16, 32, 64, 128, 256),
                (0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024),
            ),
        )
        self.assertEqual(
            opcode_gas.OUT_OF_FIT_CHECKPOINTS,
            {"4": 8, "16": 32, "64": 128, "256": 512, "1024": 2048},
        )

    def test_isolation_and_paired_precompile_contract(self):
        rows = linear_observations()
        rows[1]["isolation"]["bytecode_size"] = 65
        result = opcode_gas.evaluate_controlled_sweep(
            rows,
            pricing_basis="raw_gas_slope",
            target_raw_gas=3,
            generator_max_count=8,
        )
        self.assertEqual(result["status"], "rejected")
        self.assertIn("confounded_template", result["reasons"])

        paired = [
            {
                "count": count,
                "target": repeated_point(count, 20_000 + 1_500 * count),
                "control": repeated_point(count, 10_000 + 300 * count),
                "shape": {"loop": "same", "input_size": 32, "output_fold": "same"},
            }
            for count in [0, 1, 2, 4, 8]
        ]
        result = opcode_gas.evaluate_paired_precompile_sweep(
            paired, target_raw_gas=3, generator_max_count=8
        )
        self.assertEqual(result["g_p"], "1200")
        self.assertEqual(result["c_p"], "400")
        self.assertIn("secondary", result)


class CandidateConstructionTests(unittest.TestCase):
    def test_overhead_residual_uses_raw_gas_units_not_operation_event_count(self):
        rows = []
        for count in [0, 1, 2, 4, 8]:
            for lane in ["target", "control"]:
                workload_id = opcode_gas.sha256_bytes(f"{lane}:{count}".encode())
                for repeat_index in range(3):
                    rows.append(
                        {
                            "case": "tx_base_no_code_no_value",
                            "lane": lane,
                            "target_count": count,
                            "workload_id": workload_id,
                            "backend_input_sha256": "b" * 64,
                            "execution_row_id": opcode_gas.controlled_execution_row_id(
                                workload_id,
                                backend="sp1",
                                run_id="calibration",
                                repeat_index=repeat_index,
                                backend_input_sha256="b" * 64,
                            ),
                            "repeat_index": repeat_index,
                            "status": "accepted",
                            "gas": (
                                10_000 + 1_200 * count
                                if lane == "target"
                                else 10_000
                            ),
                            "total_instruction_count": (
                                20_000 + 2_400 * count
                                if lane == "target"
                                else 20_000
                            ),
                            "exit_code": 0,
                            "public_values": f"{lane}:{count}",
                            "expected_feature_deltas": (
                                {"tx_base": count} if lane == "target" else {}
                            ),
                            "observed_operation_deltas": (
                                {
                                    "opcode:0x01": {
                                        "pricing_basis": "raw_gas_slope",
                                        "units": 2 * count,
                                    }
                                }
                                if lane == "target" and count
                                else {}
                            ),
                        }
                    )

        points, reasons = opcode_gas._paired_overhead_points(
            controlled_manifest(),
            "tx_base",
            "tx_base_no_code_no_value",
            rows,
            {
                "opcode:0x01": {
                    "pricing_basis": "raw_gas_slope",
                    "cost": Decimal("100"),
                    "secondary_cost": Decimal("200"),
                }
            },
            {},
            {},
        )
        self.assertEqual(reasons, [])
        self.assertEqual(points[1]["instruction_count_repeats"], ["2000"] * 3)
        result = opcode_gas.evaluate_controlled_sweep(
            points,
            pricing_basis="fixed_per_event",
            target_raw_gas=None,
            generator_max_count=8,
        )
        self.assertEqual(result["f_p"], "1000")
        unavailable, reasons = opcode_gas._paired_overhead_points(
            controlled_manifest(),
            "tx_base",
            "tx_base_no_code_no_value",
            rows,
            {
                "opcode:0x01": {
                    "pricing_basis": "raw_gas_slope",
                    "cost": Decimal("100"),
                }
            },
            {},
            {},
        )
        self.assertEqual(reasons, [])
        self.assertEqual(unavailable[1]["instruction_count_repeats"], [])
        self.assertEqual(
            unavailable[1]["secondary_unavailable_reason"],
            "unresidualized_dependencies",
        )

    def test_controlled_overhead_fit_residualizes_all_four_required_terms(self):
        rows = []

        def add(case, lane, count, gas, features, *, startup=False):
            workload_id = opcode_gas.sha256_bytes(
                f"{case}:{lane}:{count}".encode()
            )
            for repeat_index in range(3):
                rows.append(
                    {
                        "case": case,
                        "lane": lane,
                        "target_count": 1 if startup else count,
                        "generator_max_count": 8,
                        "workload_id": workload_id,
                        "backend_input_sha256": "b" * 64,
                        "execution_row_id": opcode_gas.controlled_execution_row_id(
                            workload_id,
                            backend="sp1",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256="b" * 64,
                        ),
                        "repeat_index": repeat_index,
                        "status": "accepted",
                        "gas": gas,
                        "total_instruction_count": gas * 2,
                        "exit_code": 0,
                        "public_values": f"{case}:{lane}:{count}",
                        "expected_feature_deltas": features,
                        "expected_operation_deltas": {},
                        "baseline_kind": (
                            "mathematical_zero_baseline" if startup else None
                        ),
                    }
                )

        for count in [0, 1, 2, 4, 8]:
            for case in [
                "tx_base_no_code_no_value",
                "tx_base_minimal_contract_call",
            ]:
                add(case, "target", count, 1_000 + 1_000 * count, {"tx_base": count})
                add(case, "control", count, 1_000, {})
            add(
                "native_transfer_positive_vs_zero",
                "target",
                count,
                2_000 + 2_000 * count,
                {"native_value_transfer": count, "tx_base": 0},
            )
            add("native_transfer_positive_vs_zero", "control", count, 2_000, {})
            add(
                "block_base_one_vs_two_minimal_blocks",
                "target",
                count,
                3_000 + 3_000 * count,
                {"block_base": count, "tx_base": 0, "native_value_transfer": 0},
            )
            add("block_base_one_vs_two_minimal_blocks", "control", count, 3_000, {})

        add(
            "startup_minimal_no_candidate_tx",
            "target",
            0,
            7_000,
            {"proposal_startup": 1, "block_base": 1, "tx_base": 0, "native_value_transfer": 0},
            startup=True,
        )
        add(
            "startup_minimal_one_no_code_tx",
            "target",
            0,
            8_000,
            {"proposal_startup": 1, "block_base": 1, "tx_base": 1, "native_value_transfer": 0},
            startup=True,
        )

        artifact = opcode_gas.fit_controlled_overheads(
            controlled_manifest(), rows, [], generator_max_count=8
        )
        self.assertEqual(
            artifact["o_p"],
            {
                "tx_base": "1000",
                "native_value_transfer": "2000",
                "block_base": "3000",
                "proposal_startup": "4000",
            },
            artifact,
        )
        self.assertEqual(
            artifact["o_s"],
            {
                "tx_base": "2000",
                "native_value_transfer": "4000",
                "block_base": "6000",
                "proposal_startup": "8000",
            },
            artifact,
        )

        rejected_rows = copy.deepcopy(rows)
        for row in rejected_rows:
            if (
                row["case"]
                in {"tx_base_no_code_no_value", "tx_base_minimal_contract_call"}
                and row["lane"] == "target"
                and row["target_count"] == 8
            ):
                row["gas"] = 100_000
        rejected = opcode_gas.fit_controlled_overheads(
            controlled_manifest(), rejected_rows, [], generator_max_count=8
        )
        native = next(
            result
            for result in rejected["case_results"]
            if result["case_id"] == "native_transfer_positive_vs_zero"
        )
        self.assertEqual(native["status"], "rejected")
        self.assertIn("unmeasured_overhead_dependency", native["reasons"])
        self.assertEqual(native["dependency_ids"], ["tx_base"])
        self.assertEqual(
            native["secondary"],
            {
                "status": "failed",
                "reason": "unresidualized_dependencies",
                "dependency_ids": ["tx_base"],
            },
        )
        self.assertEqual(
            opcode_gas.controlled_round_decision(rejected["case_results"], 8),
            "expand_next_round",
        )

    def test_overhead_round_runs_every_point_three_times_and_startup_only_once(self):
        calls = []

        def fake_run(cmd, *, check):
            self.assertTrue(check)
            spec_path = pathlib.Path(cmd[cmd.index("--input") + 1])
            report_path = pathlib.Path(cmd[cmd.index("--jsonl-out") + 1])
            spec = opcode_gas.json.loads(spec_path.read_text())
            calls.append((spec["target_count"], spec["include_startup"]))
            workload_spec = {
                "schema_version": 1,
                "case_id": "startup_minimal_no_candidate_tx" if spec["include_startup"] else "tx_base_no_code_no_value",
                "target_count": 1 if spec["include_startup"] else spec["target_count"],
            }
            workload_id = opcode_gas.controlled_workload_id(workload_spec)
            report_path.write_text(
                opcode_gas.json.dumps(
                    {
                        "guest_input_sha256": "0x" + "b" * 64,
                        "gas": 1,
                        "total_instruction_count": 2,
                        "exit_code": 0,
                        "public_values": "0x01",
                        "controlled_overhead": {
                            "status": "accepted",
                            "target_count": spec["target_count"],
                            "reasons": [],
                            "observation": {
                                "case_id": "startup_minimal_no_candidate_tx" if spec["include_startup"] else "tx_base_no_code_no_value",
                                "overhead_key_id": "proposal_startup" if spec["include_startup"] else "tx_base",
                                "lane": "target",
                                "baseline_kind": "mathematical_zero_baseline" if spec["include_startup"] else None,
                                "target_count": 1 if spec["include_startup"] else spec["target_count"],
                                "workload_id": workload_id,
                                "workload_spec": workload_spec,
                                "backend_input_sha256": "b" * 64,
                                "guest_input_sha256": "0x" + "b" * 64,
                                "guest_input_bincode_length": 1,
                                "public_output": "0x01",
                                "absolute_operation_counts": {},
                                "absolute_feature_counts": {},
                                "expected_operation_deltas": {},
                                "expected_feature_deltas": {},
                                "started_candidate_transaction_count": 0,
                                "committed_candidate_transaction_count": 0,
                                "unattempted_candidate_transaction_count": 0,
                            },
                        },
                    }
                )
                + "\n"
            )

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            opcode_gas.subprocess, "run", fake_run
        ):
            output = pathlib.Path(tmp) / "runs.jsonl"
            opcode_gas.run_controlled_overhead_round(
                guest_launcher=pathlib.Path("guest-launcher"),
                calibration_run_id="calibration",
                generator_max_count=8,
                include_startup=True,
                out=output,
            )
            rows = list(opcode_gas.iter_jsonl(output))

        self.assertEqual(len(calls), 5 * 3)
        self.assertEqual([call for call in calls if call[1]], [(0, True)] * 3)
        self.assertEqual(len(rows), 15)
        self.assertEqual(sorted({row["repeat_index"] for row in rows}), [0, 1, 2])

    def test_task4_step1_cli_entrypoints_are_executable(self):
        parser = opcode_gas.build_parser()
        commands = {
            "run-controlled": [
                "--fixtures", "fixtures",
                "--guest-launcher", "guest-launcher",
                "--calibration-run", "run",
                "--controlled-manifest", "manifest.toml",
                "--out", "runs.jsonl",
            ],
            "fit-controlled-costs": [
                "--runs", "runs.jsonl",
                "--controlled-manifest", "manifest.toml",
                "--out", "fit.json",
            ],
            "build-candidate": [
                "--run", "run",
                "--controlled-manifest", "manifest.toml",
                "--fit", "fit.json",
                "--overheads", "overheads.json",
                "--provenance", "provenance.json",
            ],
            "build-sp1-bridge": [
                "--run", "run",
                "--controlled-manifest", "manifest.toml",
                "--samples", "samples.json",
            ],
        }
        for command, tail in commands.items():
            with self.subTest(command=command):
                args = parser.parse_args([command, *tail])
                self.assertEqual(args.command, command)
                self.assertTrue(callable(args.func))

        run = parser.parse_args(["run-controlled", *commands["run-controlled"]])
        self.assertEqual(run.repeats, 3)
        self.assertEqual(run.opcode_stage, "revm-opcode-lab")

    def test_candidate_and_bridge_cli_join_marginal_controlled_artifacts(self):
        manifest = controlled_manifest()
        revision = "a" * 40
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overhead = {
            "schema_version": 1,
            "status": "accepted",
            "o_p": {key: "100" for key in opcode_gas.Q_FORMULA},
            "o_s": {key: "200" for key in opcode_gas.Q_FORMULA},
            "case_results": [],
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run, identity = persist_completed_controlled_run(
                root, manifest, rows, overhead, revision
            )
            bridge_dir = run / "bridge"
            fit_path = run / "controlled-fit.json"
            overhead_path = run / "controlled-overheads.json"
            provenance_path = run / "provenance.json"
            manifest_path = root / "manifest.toml"
            manifest_path.write_text("test fixture is supplied by mock\n")
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas,
                "verify_frozen_controlled_manifest",
                return_value=(manifest, identity),
            ), mock.patch.object(
                opcode_gas, "current_uzen_schedule", return_value=fixture_schedule()
            ):
                opcode_gas.cmd_build_candidate(
                    opcode_gas.argparse.Namespace(
                        run=run,
                        controlled_manifest=manifest_path,
                        fit=fit_path,
                        overheads=overhead_path,
                        provenance=provenance_path,
                    )
                )
                samples_path = run / "samples" / "controlled-cycle-cost-samples.json"
                sample_artifact = opcode_gas.json.loads(samples_path.read_text())
                self.assertEqual(
                    set(sample_artifact["samples"]), set(manifest.bridge_key_ids)
                )
                self.assertTrue(
                    all(
                        sample["status"] == "available"
                        for sample in sample_artifact["samples"].values()
                    )
                )
                opcode_gas.cmd_build_sp1_bridge(
                    opcode_gas.argparse.Namespace(
                        run=run,
                        controlled_manifest=manifest_path,
                        samples=samples_path,
                    )
                )
                tampered = {**sample_artifact, "candidate_sha256": "d" * 64}
                samples_path.write_text(opcode_gas.json.dumps(tampered) + "\n")
                with self.assertRaisesRegex(ValueError, "candidate/run identity"):
                    opcode_gas.cmd_build_sp1_bridge(
                        opcode_gas.argparse.Namespace(
                            run=run,
                            controlled_manifest=manifest_path,
                            samples=samples_path,
                        )
                    )
                samples_path.write_bytes(opcode_gas.canonical_json(sample_artifact))
            result = opcode_gas.json.loads(
                (bridge_dir / "controlled-bridge.json").read_text()
            )
            self.assertEqual(result["status"], "stable_controlled")
            self.assertEqual(result["kappa_sp1"], "0.5")
            bridge_root = opcode_gas.json.loads(
                (bridge_dir / "bridge-root.json").read_text()
            )
            self.assertEqual(
                bridge_root["candidate_sha256"],
                (run / "candidate" / "candidate.sha256").read_text().strip(),
            )
            self.assertEqual(
                bridge_root["components"]["controlled-cycle-cost-samples.json"],
                opcode_gas.sha256_file(samples_path),
            )
            candidate = opcode_gas.json.loads(
                (run / "candidate" / "candidate-manifest.json").read_text()
            )
            self.assertEqual(candidate["provenance"]["calibration_id"], run.name)
            self.assertEqual(
                candidate["provenance"]["controlled_decisions_sha256"],
                opcode_gas.sha256_file(run / "controlled-decisions.json"),
            )

    def test_candidate_cli_rejects_tampered_or_unbound_controlled_artifacts(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overhead = {
            "schema_version": 1,
            "status": "accepted",
            "o_p": {key: "100" for key in opcode_gas.Q_FORMULA},
            "o_s": {key: "200" for key in opcode_gas.Q_FORMULA},
            "case_results": [],
        }

        def invoke(root, run, identity, **overrides):
            args = {
                "run": run,
                "controlled_manifest": root / "manifest.toml",
                "fit": run / "controlled-fit.json",
                "overheads": run / "controlled-overheads.json",
                "provenance": run / "provenance.json",
                **overrides,
            }
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas,
                "verify_frozen_controlled_manifest",
                return_value=(manifest, identity),
            ), mock.patch.object(
                opcode_gas, "current_uzen_schedule", return_value=fixture_schedule()
            ):
                opcode_gas.cmd_build_candidate(opcode_gas.argparse.Namespace(**args))

        mutations = {
            "fit": lambda run: (run / "controlled-fit.json").write_text(
                opcode_gas.json.dumps({"case_results": rows}) + "\n"
            ),
            "overhead": lambda run: (run / "controlled-overheads.json").write_text(
                opcode_gas.json.dumps(
                    {
                        **overhead,
                        "o_p": {key: "101" for key in opcode_gas.Q_FORMULA},
                    }
                )
                + "\n"
            ),
            "decisions": lambda run: (run / "controlled-decisions.json").write_text(
                opcode_gas.json.dumps(
                    {
                        **opcode_gas.json.loads(
                            (run / "controlled-decisions.json").read_text()
                        ),
                        "rounds": [
                            {
                                **opcode_gas.json.loads(
                                    (run / "controlled-decisions.json").read_text()
                                )["rounds"][0],
                                "decision": "expand_next_round",
                            }
                        ],
                    }
                )
                + "\n"
            ),
            "provenance": lambda run: (run / "provenance.json").write_text(
                opcode_gas.json.dumps(
                    {
                        **opcode_gas.json.loads((run / "provenance.json").read_text()),
                        "implementation_revision": "d" * 40,
                    }
                )
                + "\n"
            ),
        }
        for name, mutate in mutations.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as tmp:
                root = pathlib.Path(tmp)
                run, identity = persist_completed_controlled_run(
                    root, manifest, rows, overhead
                )
                (root / "manifest.toml").write_text("mock\n")
                mutate(run)
                with self.assertRaises(ValueError):
                    invoke(root, run, identity)

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run, identity = persist_completed_controlled_run(
                root, manifest, rows, overhead
            )
            (root / "manifest.toml").write_text("mock\n")
            arbitrary = root / "arbitrary-accepted-fit.json"
            arbitrary.write_bytes((run / "controlled-fit.json").read_bytes())
            with self.assertRaisesRegex(ValueError, "canonical persisted"):
                invoke(root, run, identity, fit=arbitrary)

    def test_experiment_identity_rejects_stale_content_id_and_duplicate_fields(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overhead = {
            "schema_version": 1,
            "status": "accepted",
            "o_p": {key: "100" for key in opcode_gas.Q_FORMULA},
            "o_s": {key: "200" for key in opcode_gas.Q_FORMULA},
            "case_results": [],
        }
        with tempfile.TemporaryDirectory() as tmp:
            run, _ = persist_completed_controlled_run(
                pathlib.Path(tmp), manifest, rows, overhead
            )
            experiment = opcode_gas.json.loads((run / "experiment.json").read_text())

            stale = copy.deepcopy(experiment)
            stale["calibration_identity"]["guest_artifacts_sha256"] = "2" * 64
            stale["guest_artifacts_sha256"] = "2" * 64
            with self.assertRaisesRegex(ValueError, "content-addressed calibration_id"):
                opcode_gas.experiment_provenance_declaration(stale)

            divergent = copy.deepcopy(experiment)
            divergent["sp1_sdk_version"] = "different-sdk"
            with self.assertRaisesRegex(ValueError, "duplicate field sp1_sdk_version"):
                opcode_gas.experiment_provenance_declaration(divergent)

    def test_fit_controlled_costs_consumes_three_bound_repeats(self):
        rows = []
        workload_id = "a" * 64
        backend_hash = "b" * 64
        for count in [0, 1, 2, 4, 8]:
            for repeat_index in range(3):
                rows.append(
                    {
                        "case": "add",
                        "kind": "opcode",
                        "lane": "target",
                        "target_count": count,
                        "target_raw_gas": 3,
                        "generator_max_count": 8,
                        "workload_id": workload_id,
                        "backend_input_sha256": backend_hash,
                        "execution_row_id": opcode_gas.controlled_execution_row_id(
                            workload_id,
                            backend="sp1",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256=backend_hash,
                        ),
                        "repeat_index": repeat_index,
                        "gas": 10_000 + count * 1_200,
                        "total_instruction_count": 20_000 + count * 2_400,
                        "exit_code": 0,
                        "public_values": "0x01",
                        "isolation": {
                            "status": "passed",
                            "bytecode_size": 128,
                            "input_size": 256,
                            "non_target_counts": {"opcode:0x60": 16},
                            "non_target_raw_gas": 48,
                        },
                    }
                )

        result = {
            row["case_id"]: row
            for row in opcode_gas.fit_controlled_costs(controlled_manifest(), rows)
        }["add"]

        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["g_p"], "1200")
        self.assertEqual(result["c_p"], "400")
        self.assertEqual(result["generator_max_count"], 8)

    def test_fit_rejects_cross_batch_precompile_pairing(self):
        rows = []
        for lane, pair_id, workload_id, backend_hash in [
            ("target", "c" * 64, "a" * 64, "b" * 64),
            ("control", "d" * 64, "e" * 64, "f" * 64),
        ]:
            for repeat_index in range(3):
                rows.append(
                    {
                        "case": "identity",
                        "kind": "precompile",
                        "lane": lane,
                        "pair_id": pair_id,
                        "target_count": 0,
                        "target_raw_gas": 18,
                        "generator_max_count": 8,
                        "workload_id": workload_id,
                        "backend_input_sha256": backend_hash,
                        "execution_row_id": opcode_gas.controlled_execution_row_id(
                            workload_id,
                            backend="sp1",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256=backend_hash,
                        ),
                        "repeat_index": repeat_index,
                        "gas": 10_000,
                        "total_instruction_count": 20_000,
                        "exit_code": 0,
                        "public_values": "0x01",
                        "isolation": {
                            "status": "passed",
                            "input_size": 32,
                            "output_size": 32,
                            "folded_bytes_per_iteration": 40,
                        },
                    }
                )

        with self.assertRaisesRegex(ValueError, "pair_id mismatch"):
            opcode_gas.fit_controlled_costs(controlled_manifest(), rows)

    def test_adaptive_runner_regenerates_whole_prefix_without_mixing_footprints(self):
        generated = []
        fitted_footprints = []

        def fake_generate(_manifest, out, *, provenance, generator_max_count):
            generated.append(generator_max_count)
            out.mkdir(parents=True)
            self.assertRegex(provenance["calibration_id"], r"^[0-9a-f]{24}$")
            return []

        def fake_run(args):
            args.out.write_text(
                opcode_gas.json.dumps(
                    {"generator_max_count": generated[-1]}
                )
                + "\n"
            )

        def fake_fit(_manifest, rows):
            footprints = {row["generator_max_count"] for row in rows}
            fitted_footprints.append(footprints)
            return [{"status": "accepted"}]

        def fake_overhead_run(*, out, **_kwargs):
            out.write_text("{}\n")

        def fake_overhead_fit(
            _manifest,
            _rows,
            _results,
            *,
            generator_max_count,
            overhead_generator_max_count,
        ):
            if generator_max_count == 8:
                case_results = [
                    {
                        "status": "rejected",
                        "reasons": ["unmeasured_overhead_dependency"],
                        "dependency_ids": ["tx_base"],
                    }
                ]
                status = "rejected"
            else:
                case_results = [{"status": "accepted"}]
                status = "accepted"
            return {
                "schema_version": 1,
                "generator_max_count": generator_max_count,
                "overhead_generator_max_count": overhead_generator_max_count,
                "status": status,
                "o_p": {key: "1" for key in opcode_gas.Q_FORMULA},
                "case_results": case_results,
            }

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run, _ = persist_execution_identity(root, controlled_manifest())
            args = opcode_gas.argparse.Namespace(
                fixtures=root / "fixtures",
                guest_launcher=pathlib.Path("guest-launcher"),
                elf=pathlib.Path("opcode.elf"),
                precompile_elf=pathlib.Path("precompile.elf"),
                calibration_run=run,
                controlled_manifest=root / "manifest.toml",
                out=root / "runs.jsonl",
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas,
                "verify_frozen_controlled_manifest",
                return_value=(
                    controlled_manifest(),
                    {
                        "controlled_manifest_sha256": "a" * 64,
                        "controlled_manifest_rows_sha256": "b" * 64,
                    },
                ),
            ), mock.patch.object(opcode_gas, "generate_cases", fake_generate), mock.patch.object(
                opcode_gas, "cmd_run", fake_run
            ), mock.patch.object(opcode_gas, "fit_controlled_costs", fake_fit), mock.patch.object(
                opcode_gas, "run_controlled_overhead_round", fake_overhead_run
            ), mock.patch.object(opcode_gas, "fit_controlled_overheads", fake_overhead_fit):
                opcode_gas.cmd_run_controlled(args)
            decisions = opcode_gas.json.loads(
                (run / "controlled-decisions.json").read_text()
            )
            self.assertTrue((run / "controlled-fit.json").is_file())
            self.assertTrue((run / "controlled-overheads.json").is_file())

        self.assertEqual(generated, [8, 32])
        self.assertEqual(fitted_footprints, [{8}, {32}])
        self.assertEqual(
            [row["decision"] for row in decisions["rounds"]],
            ["expand_next_round", "complete"],
        )

    def test_operation_2048_reuses_sealed_512_overhead_raw_and_refits(self):
        generated = []
        overhead_runs = []
        overhead_fits = []

        def fake_generate(_manifest, out, *, provenance, generator_max_count):
            generated.append(generator_max_count)
            out.mkdir(parents=True)
            return []

        def fake_run(args):
            args.out.write_text(
                opcode_gas.json.dumps(
                    {"generator_max_count": generated[-1]}
                )
                + "\n"
            )

        def fake_fit(_manifest, rows):
            generator_max_count = next(iter(rows))["generator_max_count"]
            return [
                {
                    "status": (
                        "accepted" if generator_max_count == 2048 else "rejected"
                    ),
                    "reasons": (
                        [] if generator_max_count == 2048 else ["exhausted_sweep"]
                    ),
                    "generator_max_count": generator_max_count,
                }
            ]

        def fake_overhead_run(*, generator_max_count, out, **_kwargs):
            overhead_runs.append(generator_max_count)
            out.write_text(
                opcode_gas.json.dumps(
                    {
                        "overhead_key_id": "tx_base",
                        "generator_max_count": generator_max_count,
                    }
                )
                + "\n"
            )

        def fake_overhead_fit(
            _manifest,
            rows,
            results,
            *,
            generator_max_count,
            overhead_generator_max_count=None,
        ):
            overhead_fits.append(
                (
                    generator_max_count,
                    overhead_generator_max_count,
                    {row["generator_max_count"] for row in rows},
                    {row["generator_max_count"] for row in results},
                )
            )
            return {
                "schema_version": 1,
                "generator_max_count": generator_max_count,
                "overhead_generator_max_count": overhead_generator_max_count,
                "status": "accepted",
                "o_p": {key: "1" for key in opcode_gas.Q_FORMULA},
                "case_results": [{"status": "accepted"}],
            }

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run, _ = persist_execution_identity(root, controlled_manifest())
            args = opcode_gas.argparse.Namespace(
                fixtures=root / "fixtures",
                guest_launcher=pathlib.Path("guest-launcher"),
                elf=pathlib.Path("opcode.elf"),
                precompile_elf=pathlib.Path("precompile.elf"),
                calibration_run=run,
                controlled_manifest=root / "manifest.toml",
                out=root / "runs.jsonl",
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas,
                "verify_frozen_controlled_manifest",
                return_value=(
                    controlled_manifest(),
                    {
                        "controlled_manifest_sha256": "a" * 64,
                        "controlled_manifest_rows_sha256": "b" * 64,
                    },
                ),
            ), mock.patch.object(opcode_gas, "generate_cases", fake_generate), mock.patch.object(
                opcode_gas, "cmd_run", fake_run
            ), mock.patch.object(opcode_gas, "fit_controlled_costs", fake_fit), mock.patch.object(
                opcode_gas, "run_controlled_overhead_round", fake_overhead_run
            ), mock.patch.object(
                opcode_gas, "fit_controlled_overheads", fake_overhead_fit
            ):
                opcode_gas.cmd_run_controlled(args)

            decisions_path = run / "controlled-decisions.json"
            decisions = opcode_gas.json.loads(decisions_path.read_text())
            records = decisions["rounds"]
            self.assertEqual(generated, [8, 32, 128, 512, 2048])
            self.assertEqual(overhead_runs, [8, 32, 128, 512])
            self.assertEqual(
                overhead_fits[-1],
                (2048, 512, {512}, {2048}),
            )
            self.assertEqual(records[-1]["overhead_generator_max_count"], 512)
            self.assertEqual(records[-1]["overhead_runs"], records[-2]["overhead_runs"])
            self.assertEqual(
                records[-1]["overhead_runs_sha256"],
                records[-2]["overhead_runs_sha256"],
            )
            opcode_gas.validate_persisted_controlled_decisions(run, decisions)

            tampered = copy.deepcopy(decisions)
            tampered["rounds"][-1]["overhead_generator_max_count"] = 2048
            with self.assertRaisesRegex(ValueError, "overhead generator"):
                opcode_gas.validate_persisted_controlled_decisions(run, tampered)

            tampered_fit = copy.deepcopy(decisions)
            overhead_fit_path = run / records[-1]["overhead_fit"]
            overhead_payload = opcode_gas.json.loads(overhead_fit_path.read_text())
            overhead_payload["overhead_generator_max_count"] = 2048
            overhead_fit_path.write_text(opcode_gas.json.dumps(overhead_payload) + "\n")
            tampered_fit["rounds"][-1]["overhead_fit_sha256"] = (
                opcode_gas.sha256_file(overhead_fit_path)
            )
            with self.assertRaisesRegex(ValueError, "overhead footprint"):
                opcode_gas.validate_persisted_controlled_decisions(run, tampered_fit)

    def test_adaptive_resume_rejects_gap_duplicate_and_edited_decision(self):
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp)
            record_index = 0

            def record(generator_max_count, results, decision):
                nonlocal record_index
                record_index += 1
                raw = run / f"raw-{generator_max_count}-{record_index}.jsonl"
                fit = run / f"fit-{generator_max_count}-{record_index}.json"
                overhead_raw = run / f"overhead-raw-{generator_max_count}-{record_index}.jsonl"
                overhead_fit = run / f"overhead-fit-{generator_max_count}-{record_index}.json"
                raw.write_text("{}\n")
                overhead_raw.write_text("{}\n")
                fit.write_text(
                    opcode_gas.json.dumps(
                        {
                            "schema_version": 1,
                            "generator_max_count": generator_max_count,
                            "case_results": results,
                        }
                    )
                    + "\n"
                )
                overhead_fit.write_text(
                    opcode_gas.json.dumps(
                        {
                            "schema_version": 1,
                            "generator_max_count": generator_max_count,
                            "overhead_generator_max_count": generator_max_count,
                            "case_results": [],
                        }
                    )
                    + "\n"
                )
                return {
                    "generator_max_count": generator_max_count,
                    "raw_runs": raw.name,
                    "raw_runs_sha256": opcode_gas.sha256_file(raw),
                    "fit": fit.name,
                    "fit_sha256": opcode_gas.sha256_file(fit),
                    "overhead_runs": overhead_raw.name,
                    "overhead_runs_sha256": opcode_gas.sha256_file(overhead_raw),
                    "overhead_generator_max_count": generator_max_count,
                    "overhead_fit": overhead_fit.name,
                    "overhead_fit_sha256": opcode_gas.sha256_file(overhead_fit),
                    "decision": decision,
                }

            complete_8 = record(8, [{"status": "accepted"}], "complete")
            expand_8 = record(
                8,
                [{"status": "rejected", "reasons": ["exhausted_sweep"]}],
                "expand_next_round",
            )
            complete_32 = record(32, [{"status": "accepted"}], "complete")

            for rounds, message in [
                ([complete_8, complete_8], "unique contiguous prefix"),
                ([complete_32], "unique contiguous prefix"),
                ([{**complete_8, "decision": "expand_next_round"}], "decision changed"),
                ([complete_8, complete_32], "continue after completion"),
            ]:
                with self.subTest(message=message), self.assertRaisesRegex(
                    ValueError, message
                ):
                    opcode_gas.validate_persisted_controlled_decisions(
                        run, {"schema_version": 1, "rounds": rounds}
                    )

            validated = opcode_gas.validate_persisted_controlled_decisions(
                run,
                {"schema_version": 1, "rounds": [expand_8, complete_32]},
            )
            self.assertEqual(list(validated), [8, 32])

    def test_complete_component_ledger_distinguishes_evm_and_host_actions(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        measurements = opcode_gas.construct_measurement_values(manifest, rows)
        ledger = opcode_gas.build_component_cost_ledger(
            manifest,
            measurements,
            {key: "100" for key in opcode_gas.Q_FORMULA},
            fixture_schedule(),
        )
        by_id = {row["component_id"]: row for row in ledger["primary_rows"]}
        self.assertEqual(by_id["opcode:0x01"]["raw_gas_unit"], "interpreter_raw_gas")
        self.assertEqual(by_id["opcode:0x01"]["current_zkgas_multiplier"], 1)
        self.assertEqual(by_id["opcode:0x01"]["marginal_prover_gas"], "400")
        self.assertEqual(by_id["opcode:0x01"]["cost_index"], "1")
        self.assertEqual(by_id["opcode:0x20"]["status"], "unmeasured")
        self.assertEqual(by_id["opcode:0x40"]["status"], "unmeasured")
        self.assertEqual(by_id["tx_base"]["raw_gas_unit"], None)
        diagnostics = {
            row["component_id"]: row for row in ledger["diagnostic_rows"]
        }
        self.assertEqual(
            diagnostics["host_action:trie_merkle_hash"]["status"], "unmeasured"
        )
        self.assertEqual(
            diagnostics["host_action:trie_merkle_hash"]["reason"],
            "no_independent_controlled_observable_unit",
        )

    def test_residualized_overheads_subtract_each_descendant_once(self):
        manifest = controlled_manifest()
        costs = {
            "tx_base": Decimal("10"),
            "native_value_transfer": Decimal("5"),
            "block_base": Decimal("100"),
        }
        native = opcode_gas.residualize_overhead_delta(
            manifest,
            "native_value_transfer",
            target_minus_control="15",
            feature_deltas={"native_value_transfer": 1, "tx_base": 1},
            accepted_costs=costs,
        )
        self.assertEqual(native, Decimal("5"))

        block = opcode_gas.residualize_overhead_delta(
            manifest,
            "block_base",
            target_minus_control="130",
            feature_deltas={
                "block_base": 1,
                "native_value_transfer": 2,
                "tx_base": 2,
            },
            accepted_costs=costs,
        )
        self.assertEqual(block, Decimal("100"))

        startup = opcode_gas.fixed_startup_residual(
            [
                [Decimal("1000"), Decimal("1000"), Decimal("1000")],
                [Decimal("1002"), Decimal("1002"), Decimal("1002")],
            ]
        )
        self.assertEqual(startup, Decimal("1001"))
        with self.assertRaisesRegex(ValueError, "repeat-stable"):
            opcode_gas.fixed_startup_residual(
                [[Decimal("1000"), Decimal("1001"), Decimal("1000")]]
            )

    def test_exact_costs_required_consistency_and_add_normalization(self):
        add = accepted_case("add", "opcode:0x01", "raw_gas_slope", 10**40 + 1, 3)
        mul = accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2 * (10**40 + 1), 5)
        identity = accepted_case("identity", "precompile:0x04", "raw_gas_slope", 6 * (10**40 + 1), 18)
        values = opcode_gas.construct_measurement_values(
            controlled_manifest(), [add, mul, identity]
        )

        self.assertEqual(values["opcode:0x01"]["c_p"], f"{Decimal(10**40 + 1) / Decimal(3)}")
        self.assertEqual(values["opcode:0x02"]["m_p"], "1.2")
        self.assertEqual(values["precompile:0x04"]["m_p"], "1")
        self.assertEqual(getcontext().prec, 50)

    def test_missing_rejected_or_confounded_required_case_excludes_complete_key(self):
        manifest_data = controlled_manifest_data()
        manifest_data["cases"].append(copy.deepcopy(manifest_data["cases"][0]))
        manifest_data["cases"][-1]["name"] = "add_second"
        manifest_data["measurement_keys"][0]["required_case_ids"].append("add_second")
        manifest = opcode_gas.parse_controlled_manifest(
            manifest_data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        add = accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3)
        for second in [
            None,
            {**accepted_case("add_second", "opcode:0x01", "raw_gas_slope", 1200, 3), "status": "rejected"},
            {**accepted_case("add_second", "opcode:0x01", "raw_gas_slope", 1200, 3), "status": "confounded_template"},
        ]:
            with self.subTest(second=second):
                cases = [add] + ([] if second is None else [second])
                values = opcode_gas.construct_measurement_values(manifest, cases)
                self.assertEqual(values["opcode:0x01"]["status"], "required_case_incomplete")

        outside = accepted_case("add_second", "opcode:0x01", "raw_gas_slope", 1300, 3)
        values = opcode_gas.construct_measurement_values(manifest, [add, outside])
        self.assertEqual(values["opcode:0x01"]["status"], "scenario_dependent")

    def test_fixed_event_is_not_raw_gas_normalized(self):
        result = opcode_gas.case_primary_value(
            accepted_case("call_spawned", "opcode:0xf1:spawned", "fixed_per_event", 1234)
        )
        self.assertEqual(result, ("f_p", Decimal("1234")))

    def test_secondary_changes_do_not_change_candidate_digest(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overheads = {key: "100" for key in opcode_gas.Q_FORMULA}
        first = opcode_gas.build_candidate_components(manifest, rows, overheads, {"revision": "a" * 40})
        rows[0]["secondary"] = {"status": "failed", "reason": "noise"}
        second = opcode_gas.build_candidate_components(manifest, rows, overheads, {"revision": "a" * 40})
        self.assertEqual(first["candidate_sha256"], second["candidate_sha256"])
        self.assertNotEqual(first["cycle_sample_sha256"], second["cycle_sample_sha256"])

    def test_diagnostic_secondary_changes_do_not_change_candidate_digest(self):
        manifest_data = controlled_manifest_data()
        manifest_data["cases"].append(
            {
                **manifest_data["cases"][0],
                "name": "add_diagnostic",
                "scenario": "arithmetic diagnostic",
            }
        )
        manifest_data["measurement_keys"][0]["diagnostic_case_ids"] = [
            "add_diagnostic"
        ]
        manifest = opcode_gas.parse_controlled_manifest(
            manifest_data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
            accepted_case(
                "add_diagnostic", "opcode:0x01", "raw_gas_slope", 1200, 3
            ),
        ]
        overheads = {key: "100" for key in opcode_gas.Q_FORMULA}
        first = opcode_gas.build_candidate_components(
            manifest, rows, overheads, {"revision": "a" * 40}
        )
        rows[-1]["secondary"] = {"status": "failed", "reason": "noise"}
        second = opcode_gas.build_candidate_components(
            manifest, rows, overheads, {"revision": "a" * 40}
        )
        self.assertEqual(first["candidate_sha256"], second["candidate_sha256"])
        self.assertNotEqual(first["cycle_sample_sha256"], second["cycle_sample_sha256"])

    def test_candidate_requires_add_q_and_passing_checkpoint(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overheads = {key: "100" for key in opcode_gas.Q_FORMULA}
        for broken_rows, broken_overheads, message in [
            (rows[1:], overheads, "ADD"),
            (rows, {key: value for key, value in overheads.items() if key != "tx_base"}, "Q_formula"),
            ([{**rows[0], "checkpoint": {**rows[0]["checkpoint"], "status": "failed"}}, *rows[1:]], overheads, "checkpoint"),
        ]:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.build_candidate_components(
                    manifest, broken_rows, broken_overheads, {"revision": "a" * 40}
                )

    def test_bridge_exact_median_states_and_independent_digest(self):
        samples = {
            "opcode:0x01": {"prover_gas": "10", "instruction_count": "20"},
            "opcode:0x02": {"prover_gas": "20", "instruction_count": "50"},
            "precompile:0x04": {"prover_gas": "40", "instruction_count": "80"},
            **{key: {"prover_gas": "10", "instruction_count": "20"} for key in opcode_gas.Q_FORMULA},
        }
        bridge = opcode_gas.build_controlled_bridge(
            controlled_manifest(), samples
        )
        self.assertEqual(bridge["kappa_sp1"], "0.5")
        self.assertEqual(bridge["status"], "not_stable_controlled")
        self.assertEqual(bridge["keys"]["opcode:0x02"]["rho"], "0.4")
        self.assertEqual(
            bridge["keys"]["opcode:0x02"]["predicted_prover_gas"], "25"
        )
        self.assertEqual(bridge["keys"]["opcode:0x02"]["ape"], "0.25")

        missing = copy.deepcopy(samples)
        del missing["opcode:0x02"]
        insufficient = opcode_gas.build_controlled_bridge(controlled_manifest(), missing)
        self.assertEqual(insufficient["status"], "insufficient_data")
        self.assertIn("opcode:0x02", insufficient["missing_keys"])
        self.assertNotEqual(bridge["bridge_sha256"], insufficient["bridge_sha256"])

    def test_diagnostic_overheads_are_independently_hashed_and_never_enter_q(self):
        artifact = opcode_gas.build_diagnostic_overheads(
            controlled_manifest(),
            [
                {
                    "overhead_key_id": "witness_byte",
                    "case_id": "witness_bytes",
                    "status": "accepted",
                    "units": "32",
                    "o_p": "7.25",
                    "o_s": "11",
                    "controlled_evidence": {"target": "a" * 64, "control": "b" * 64},
                }
            ],
        )
        self.assertEqual(artifact["rows"][0]["overhead_key_id"], "witness_byte")
        self.assertNotIn("witness_byte", artifact["q_formula"])
        self.assertEqual(
            artifact["sha256"],
            opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {key: value for key, value in artifact.items() if key != "sha256"}
                )
            ),
        )


class IdentityAndValidationTests(unittest.TestCase):
    def test_controlled_workload_and_execution_identity_boundaries(self):
        spec = {
            "schema_version": 1,
            "measurement_key_id": "opcode:0x01",
            "case_id": "add",
            "target_count": 4,
            "lane": "target",
            "state": {"balance": "1"},
            "environment": {"timestamp": 2},
            "input": {"calldata": "0x"},
            "expected_operation_deltas": {"opcode:0x01": 4},
            "expected_feature_deltas": {},
        }
        workload_id = opcode_gas.controlled_workload_id(spec)
        for ignored in [
            {"backend": "sp1"},
            {"run_id": "other"},
            {"repeat_index": 9},
            {"sdk": "different"},
            {"elf": "different"},
            {"backend_encoding": "different"},
        ]:
            self.assertEqual(opcode_gas.controlled_workload_id(spec, **ignored), workload_id)

        for path, value in [
            (("state", "balance"), "2"),
            (("environment", "timestamp"), 3),
            (("input", "calldata"), "0x01"),
            (("target_count",), 5),
            (("lane",), "control"),
            (("expected_operation_deltas", "opcode:0x01"), 5),
            (("expected_feature_deltas", "tx_base"), 1),
        ]:
            changed = copy.deepcopy(spec)
            target = changed
            for part in path[:-1]:
                target = target[part]
            target[path[-1]] = value
            self.assertNotEqual(opcode_gas.controlled_workload_id(changed), workload_id)

        ids = {
            opcode_gas.controlled_execution_row_id(
                workload_id,
                backend=backend,
                run_id=run_id,
                repeat_index=repeat,
                backend_input_sha256=input_hash,
            )
            for backend, run_id, repeat, input_hash in [
                ("sp1", "run", 0, "a" * 64),
                ("risc0", "run", 0, "a" * 64),
                ("sp1", "other", 0, "a" * 64),
                ("sp1", "run", 1, "a" * 64),
                ("sp1", "run", 0, "b" * 64),
            ]
        }
        self.assertEqual(len(ids), 5)

    def test_event_resolution_inclusion_exclusion_and_selected_not_dispatched(self):
        manifest = controlled_manifest()
        operation = {
            "kind": "opcode",
            "opcode": "0x01",
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 3,
            "spawned": False,
            "dispatch_status": "not_applicable",
        }
        self.assertEqual(
            opcode_gas.resolve_measurement_key(manifest, operation),
            ("opcode:0x01", "measured"),
        )
        self.assertEqual(
            opcode_gas.resolve_measurement_key(
                manifest, {**operation, "opcode": "0xaa"}
            )[1],
            "no_measurement_key",
        )
        self.assertEqual(
            opcode_gas.resolve_measurement_key(
                manifest,
                {
                    **operation,
                    "opcode": "0xf1",
                    "spawned": True,
                    "pricing_basis": None,
                    "dispatch_status": "selected_not_dispatched",
                },
            )[1],
            "selected_not_dispatched",
        )

    def test_local_operation_trace_prediction_excludes_block_owned_system_work_once(self):
        manifest = controlled_manifest()
        candidate = {
            "c_p": {"opcode:0x01": "400", "precompile:0x04": "20"},
            "f_p": {},
        }
        result = opcode_gas.predict_operations_from_trace(
            manifest,
            candidate,
            [
                {
                    "kind": "opcode",
                    "opcode": "0x01",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                    "interpreter_raw_gas": 3,
                    "disposition": "attempted",
                },
                {
                    "kind": "precompile",
                    "address": "0x04",
                    "pricing_basis": "raw_gas_slope",
                    "native_gas": 18,
                    "disposition": "system",
                },
                {
                    "kind": "opcode",
                    "opcode": "0x01",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                    "interpreter_raw_gas": 999,
                    "phase": "transaction",
                    "is_anchor": True,
                    "disposition": "committed",
                },
                {
                    "kind": "opcode",
                    "opcode": "0xaa",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                    "interpreter_raw_gas": 5,
                    "disposition": "committed",
                },
                {
                    "kind": "opcode",
                    "opcode": "0x01",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                    "interpreter_raw_gas": 999,
                    "disposition": "unattempted",
                },
            ],
        )
        self.assertEqual(result["predicted_prover_gas"], "1200")
        self.assertEqual(result["measured_operation_count"], 1)
        self.assertEqual(result["block_owned_system_operation_count"], 2)
        self.assertEqual(result["unmeasured_operation_count"], 1)
        self.assertEqual(result["unmeasured"][0]["reason"], "no_measurement_key")

    def test_exact_prediction_mape_coverage_and_wrong_candidate_failure(self):
        candidate = {
            "c_p": {"opcode:0x01": "400", "precompile:0x04": "20"},
            "f_p": {},
            "o_p": {
                "proposal_startup": "100",
                "block_base": "10",
                "tx_base": "5",
                "native_value_transfer": "2",
            },
            "q_formula": list(opcode_gas.Q_FORMULA),
            "thresholds": {"proposal_ape_max": "0.10"},
            "candidate_sha256": "c" * 64,
        }
        proposal = {
            "network": "hoodi",
            "proposal_id": 1,
            "actual_prover_gas": "1327",
            "trace_ab_passed": True,
            "guest_input_join": True,
            "public_output_join": True,
            "difficulty_reconciled": True,
            "coverage_denominators": {
                "operation_count": 2,
                "raw_gas": 4,
                "spawned_event": 0,
            },
            "features": {
                "proposal_startup": 1,
                "block_base": 1,
                "tx_base": 1,
                "native_value_transfer": 1,
            },
            "operations": [
                {"measurement_key_id": "opcode:0x01", "basis": "raw_gas_slope", "raw_gas": 3, "disposition": "committed"},
                {"measurement_key_id": "precompile:0x04", "basis": "raw_gas_slope", "raw_gas": 1, "phase": "transaction", "is_anchor": True, "disposition": "committed"},
                {"measurement_key_id": "opcode:0x01", "basis": "raw_gas_slope", "raw_gas": 999, "disposition": "unattempted"},
            ],
        }
        report = opcode_gas.validate_sealed_candidate(candidate, [proposal])
        self.assertEqual(report["rows"][0]["predicted_prover_gas"], "1317")
        self.assertEqual(
            report["rows"][0]["ape"], str(Decimal(10) / Decimal(1327))
        )
        self.assertEqual(report["classification"], "candidate_table_validated_at_reported_coverage")

        wrong = copy.deepcopy(candidate)
        wrong["c_p"]["opcode:0x01"] = "800"
        failed = opcode_gas.validate_sealed_candidate(wrong, [proposal])
        self.assertEqual(failed["classification"], "candidate_table_not_validated")

    def test_bridge_truth_table(self):
        expected = {
            ("insufficient_data", "validated_on_proposals"): "inconclusive",
            ("not_stable_controlled", "validated_on_proposals"): "not_supported",
            ("not_stable_controlled", "not_evaluable_on_proposals"): "not_supported",
            ("stable_controlled", "not_evaluable_on_proposals"): "inconclusive",
            ("stable_controlled", "validated_on_proposals"): "supported",
            ("stable_controlled", "not_validated_on_proposals"): "not_supported",
        }
        for statuses, result in expected.items():
            with self.subTest(statuses=statuses):
                self.assertEqual(opcode_gas.bridge_validation_outcome(*statuses), result)

    def test_sealing_refuses_proposal_outputs_and_validation_is_read_only(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overheads = {key: "100" for key in opcode_gas.Q_FORMULA}
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp)
            (run / "proposal-results.json").write_text("{}")
            with self.assertRaisesRegex(ValueError, "proposal result"):
                opcode_gas.seal_candidate_directory(
                    run, manifest, rows, overheads, {"revision": "a" * 40}
                )

    def test_candidate_seal_transitively_verifies_components_and_tampering(self):
        manifest = controlled_manifest()
        rows = [
            accepted_case("add", "opcode:0x01", "raw_gas_slope", 1200, 3),
            accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2000, 5),
            accepted_case("identity", "precompile:0x04", "raw_gas_slope", 7200, 18),
        ]
        overheads = {key: "100" for key in opcode_gas.Q_FORMULA}
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp)
            sealed = opcode_gas.seal_candidate_directory(
                run, manifest, rows, overheads, {"revision": "a" * 40}
            )
            verified = opcode_gas.verify_candidate_directory(run)
            self.assertEqual(verified["candidate_sha256"], sealed["candidate_sha256"])
            self.assertTrue(verified["candidate_manifest"]["review_only"])
            self.assertFalse(verified["candidate_manifest"]["production_write"])
            self.assertFalse(verified["candidate_manifest"]["integer_schedule_emitted"])

            component = run / "candidate" / "normalized-primary.json"
            component.write_text("{}")
            with self.assertRaisesRegex(ValueError, "component digest"):
                opcode_gas.verify_candidate_directory(run)

    def test_bridge_seal_is_independent_and_insufficient_data_is_sealable(self):
        manifest = controlled_manifest()
        samples = {
            "opcode:0x01": {"prover_gas": "10", "instruction_count": "20"}
        }
        with tempfile.TemporaryDirectory() as tmp:
            run = pathlib.Path(tmp)
            (run / "bridge").mkdir()
            (run / "experiment.json").write_text(
                '{"implementation_revision":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}'
            )
            bridge_manifest = {
                "schema_version": 1,
                "implementation_revision": "a" * 40,
                "bridge_key_ids": list(manifest.bridge_key_ids),
                "model": manifest.bridge_model,
                "controlled_ape_max": "0.10",
                "proposal_ape_max": "0.10",
                "missing_data": "insufficient_data_is_sealable_and_non_gating",
            }
            (run / "bridge" / "bridge-manifest.json").write_text(
                __import__("json").dumps(bridge_manifest)
            )
            sealed = opcode_gas.seal_bridge_directory(run, manifest, samples)
            self.assertEqual(sealed["controlled_bridge"]["status"], "insufficient_data")
            self.assertTrue((run / "bridge" / "bridge.sha256").is_file())
            self.assertEqual(
                opcode_gas.verify_bridge_directory(run)["bridge_sha256"],
                sealed["bridge_sha256"],
            )
            candidate_refs = sealed["bridge_root"].get("candidate_sha256")
            self.assertIsNone(candidate_refs)


if __name__ == "__main__":
    unittest.main()
