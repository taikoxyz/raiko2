import copy
import hashlib
import pathlib
import sys
import tempfile
import types
import unittest
from dataclasses import replace
from unittest import mock
from decimal import Decimal, getcontext, localcontext

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import opcode_gas
from test_manifest import CONTROLLED_SCHEDULE_KEYS, controlled_manifest_data
from test_manifest import fixture_schedule


def controlled_manifest():
    return opcode_gas.parse_controlled_manifest(
        controlled_manifest_data(), schedule_keys=CONTROLLED_SCHEDULE_KEYS
    )


def formal_relation_manifest():
    return opcode_gas.load_manifest(
        ROOT
        / "experiments"
        / "opcode-gas"
        / "manifests"
        / "sp1-calibration-v1.toml",
        schedule=fixture_schedule(),
    )


def block_affine_model(manifest):
    opcode_keys = tuple(
        sorted(
            {
                key
                for row in manifest.block_calibration_rows
                for key in row.expected_raw_gas_by_key
            }
        )
    )
    return types.SimpleNamespace(
        opcode_keys=opcode_keys,
        anchor_keys=opcode_gas.OPCODE_RELATION_ANCHORS,
        anchor_basis={
            key: {
                anchor: opcode_gas.Fraction(int(key == anchor), 1)
                for anchor in opcode_gas.OPCODE_RELATION_ANCHORS
            }
            for key in opcode_keys
        },
    )


def formal_relation_rows(manifest, *, slope_overrides=None):
    slope_overrides = slope_overrides or {}
    rows = []
    provenance = {
        "calibration_id": "b" * 24,
        "calibration_identity_sha256": "b" * 64,
        "implementation_revision": "c" * 40,
        "controlled_manifest_sha256": "d" * 64,
        "controlled_manifest_rows_sha256": "e" * 64,
    }
    for index, relation in enumerate(manifest.opcode_relations):
        slope = Decimal(
            str(
                slope_overrides.get(
                    relation.id, 5000 + index if relation.signed_raw_gas_by_key else 0
                )
            )
        )
        pair_id = hashlib.sha256(relation.id.encode()).hexdigest()
        control_map = {
            key: value * 8 for key, value in relation.control_raw_gas_by_key.items()
        }
        for count in (0, 1, 2, 4, 8):
            target_map = {
                key: value * (8 - count)
                for key, value in relation.control_raw_gas_by_key.items()
            }
            for key, value in relation.target_raw_gas_by_key.items():
                target_map[key] = target_map.get(key, 0) + value * count
            for repeat_index in range(3):
                for lane, actual_map, prover_gas in (
                    ("target", target_map, Decimal(100_000) + slope * count),
                    ("control", control_map, Decimal(100_000)),
                ):
                    backend_input = hashlib.sha256(
                        f"{relation.id}:{count}:{lane}".encode()
                    ).hexdigest()
                    rows.append(
                        {
                            **provenance,
                            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
                            "signal_kind": opcode_gas.FORMAL_RELATION_SIGNAL_KIND,
                            "relation_id": relation.id,
                            "relation_split": relation.split,
                            "scenario_id": relation.scenario_id,
                            "dynamic_key": relation.dynamic_key,
                            "pair_id": pair_id,
                            "lane": lane,
                            "diagnostic_count": count,
                            "generator_max_count": 8,
                            "repeat_index": repeat_index,
                            "prover_gas": int(prover_gas),
                            "total_instruction_count": 200_000,
                            "exit_code": 0,
                            "public_values": "0x01",
                            "workload_id": hashlib.sha256(
                                f"workload:{relation.id}:{count}:{lane}".encode()
                            ).hexdigest(),
                            "backend_input_sha256": backend_input,
                            "sp1_execution_engine": "gas-estimator",
                            "sp1_gas_trace_chunk_threshold": 134_217_728,
                            "sp1_gas_trace_chunk_slots": 2,
                            "target_raw_gas_by_key": {
                                key: str(value)
                                for key, value in relation.target_raw_gas_by_key.items()
                            },
                            "control_raw_gas_by_key": {
                                key: str(value)
                                for key, value in relation.control_raw_gas_by_key.items()
                            },
                            "signed_raw_gas_by_key": {
                                key: str(value)
                                for key, value in relation.signed_raw_gas_by_key.items()
                            },
                            "actual_raw_gas_by_key": {
                                key: str(value) for key, value in actual_map.items() if value
                            },
                        }
                    )
    return rows


def formal_relation_provenance(rows):
    return {
        field: rows[0][field]
        for field in opcode_gas.FORMAL_RELATION_PROVENANCE_FIELDS
    }


def opcode_execution_provenance():
    return {
        "sp1_execution_engine": "gas-estimator",
        "sp1_gas_trace_chunk_threshold": 134_217_728,
        "sp1_gas_trace_chunk_slots": 2,
    }


def standard_execution_provenance():
    return {"sp1_execution_engine": "standard"}


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
            "tx_gas_limit": 1_000_024,
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
        "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
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

        rows = linear_observations()
        rows[1]["isolation"]["tx_gas_limit"] += 1
        result = opcode_gas.evaluate_controlled_sweep(
            rows,
            pricing_basis="raw_gas_slope",
            target_raw_gas=3,
            generator_max_count=8,
        )
        self.assertEqual(result["status"], "rejected")
        self.assertIn("confounded_template", result["reasons"])

        for invalid_limit in [None, True, 0]:
            rows = linear_observations()
            for row in rows:
                if invalid_limit is None:
                    row["isolation"].pop("tx_gas_limit")
                else:
                    row["isolation"]["tx_gas_limit"] = invalid_limit
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


class FormalOpcodeRelationTests(unittest.TestCase):
    def test_relation_artifact_is_byte_identical_across_caller_decimal_contexts(self):
        manifest = formal_relation_manifest()
        rows = formal_relation_rows(manifest)
        relation_id = next(
            relation.id
            for relation in manifest.opcode_relations
            if relation.signed_raw_gas_by_key
        )
        for row in rows:
            if (
                row["relation_id"] == relation_id
                and row["lane"] == "target"
                and row["diagnostic_count"] == 1
            ):
                row["prover_gas"] += 1

        artifacts = []
        for precision in (28, 80):
            with localcontext() as caller:
                caller.prec = precision
                artifact = opcode_gas.fit_opcode_relations(
                    manifest, copy.deepcopy(rows)
                )
                opcode_gas.validate_opcode_relations_artifact(
                    manifest,
                    artifact,
                    rows,
                    formal_relation_provenance(rows),
                )
                artifacts.append(opcode_gas.canonical_json(artifact))

        self.assertEqual(artifacts[0], artifacts[1])

    def test_signed_signal_gate_uses_zero_delta_not_shared_lane_baseline(self):
        baseline = Decimal("1000000000")
        counts = {
            count: (
                baseline + Decimal(5000 * count),
                baseline,
                Decimal(5000 * count),
            )
            for count in (0, 1, 2, 4, 8)
        }

        result = opcode_gas._signed_relation_fit(counts, generator_max_count=8)

        self.assertEqual(result["slope"], Decimal(5000))
        self.assertEqual(result["signal"], Decimal(20000))

    def test_fits_rank_98_artifact_with_negative_slope_and_exact_serialization(self):
        manifest = formal_relation_manifest()
        negative = next(
            relation for relation in manifest.opcode_relations if relation.signed_raw_gas_by_key
        )
        rows = formal_relation_rows(
            manifest, slope_overrides={negative.id: -5000}
        )

        artifact = opcode_gas.fit_opcode_relations(manifest, rows)

        self.assertEqual(artifact["status"], "accepted")
        self.assertEqual(len(artifact["equations"]), 98)
        self.assertEqual(len(artifact["self_controls"]), 4)
        self.assertEqual(len(artifact["dynamic_holdouts"]), 12)
        self.assertEqual(artifact["affine_model"]["rank"], 98)
        self.assertEqual(artifact["affine_model"]["nullity"], 4)
        self.assertEqual(
            artifact["artifact_sha256"],
            opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in artifact.items()
                        if key != "artifact_sha256"
                    }
                )
            ),
        )
        self.assertEqual(
            artifact["affine_model"]["anchor_keys"],
            list(opcode_gas.OPCODE_RELATION_ANCHORS),
        )
        equation = next(
            row for row in artifact["equations"] if row["relation_id"] == negative.id
        )
        self.assertEqual(equation["slope_p"], "-5000")
        self.assertEqual(
            equation["signed_raw_gas_by_key"],
            {
                key: str(value)
                for key, value in sorted(negative.signed_raw_gas_by_key.items())
            },
        )
        for value in artifact["affine_model"]["mu_zero"].values():
            self.assertNotIn("e", value.lower())
        opcode_gas.validate_opcode_relations_artifact(
            manifest, artifact, rows, formal_relation_provenance(rows)
        )

    def test_rejects_signed_quality_trace_completeness_and_rank_failures(self):
        manifest = formal_relation_manifest()
        relation = next(
            relation for relation in manifest.opcode_relations if relation.signed_raw_gas_by_key
        )
        base_rows = formal_relation_rows(manifest)

        missing = [row for row in base_rows if row["relation_id"] != relation.id]
        with self.assertRaisesRegex(ValueError, "missing canonical relation"):
            opcode_gas.fit_opcode_relations(manifest, missing)

        stale_provenance = copy.deepcopy(base_rows)
        for row in stale_provenance:
            row["calibration_id"] = "a" * 24
        with self.assertRaisesRegex(ValueError, "content-addressed"):
            opcode_gas.fit_opcode_relations(manifest, stale_provenance)

        mixed_provenance = copy.deepcopy(base_rows)
        mixed_provenance[0]["controlled_manifest_sha256"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "share durable provenance"):
            opcode_gas.fit_opcode_relations(manifest, mixed_provenance)

        tiny = copy.deepcopy(base_rows)
        for row in tiny:
            if row["relation_id"] == relation.id and row["lane"] == "target":
                row["prover_gas"] = 100_000 + 10 * row["diagnostic_count"]
        with self.assertRaisesRegex(ValueError, "signal"):
            opcode_gas.fit_opcode_relations(manifest, tiny)

        noisy = copy.deepcopy(base_rows)
        noisy_row = next(
            row
            for row in noisy
            if row["relation_id"] == relation.id
            and row["lane"] == "target"
            and row["diagnostic_count"] == 1
            and row["repeat_index"] == 2
        )
        noisy_row["prover_gas"] += 1
        with self.assertRaisesRegex(ValueError, "repeat noise"):
            opcode_gas.fit_opcode_relations(manifest, noisy)

        wrong_sign = copy.deepcopy(base_rows)
        for row in wrong_sign:
            if (
                row["relation_id"] == relation.id
                and row["lane"] == "target"
                and row["diagnostic_count"] == 8
            ):
                row["prover_gas"] = 50_000
        with self.assertRaisesRegex(ValueError, "checkpoint sign"):
            opcode_gas.fit_opcode_relations(manifest, wrong_sign)

        wrong_units = copy.deepcopy(base_rows)
        unit_row = next(
            row
            for row in wrong_units
            if row["relation_id"] == relation.id
            and row["lane"] == "target"
            and row["diagnostic_count"] == 1
        )
        key = next(iter(unit_row["actual_raw_gas_by_key"]))
        for row in wrong_units:
            if (
                row["relation_id"] == relation.id
                and row["lane"] == "target"
                and row["diagnostic_count"] == 1
            ):
                row["actual_raw_gas_by_key"][key] = str(
                    int(row["actual_raw_gas_by_key"][key]) + 1
                )
        with self.assertRaisesRegex(ValueError, "raw-gas units"):
            opcode_gas.fit_opcode_relations(manifest, wrong_units)

        canonical = [
            item
            for item in manifest.opcode_relations
            if item.split == "canonical" and item.signed_raw_gas_by_key
        ]
        dependent = replace(
            canonical[1],
            target_raw_gas_by_key=canonical[0].target_raw_gas_by_key,
            control_raw_gas_by_key=canonical[0].control_raw_gas_by_key,
            signed_raw_gas_by_key=canonical[0].signed_raw_gas_by_key,
        )
        mutated_relations = tuple(
            dependent if item.id == dependent.id else item
            for item in manifest.opcode_relations
        )
        rank_97_manifest = replace(manifest, opcode_relations=mutated_relations)
        with self.assertRaisesRegex(ValueError, "rank"):
            opcode_gas.fit_opcode_relations(
                rank_97_manifest, formal_relation_rows(rank_97_manifest)
            )

        self_control = next(
            item for item in manifest.opcode_relations if not item.signed_raw_gas_by_key
        )
        truncated_self = [
            row
            for row in base_rows
            if row["relation_id"] != self_control.id or row["diagnostic_count"] == 0
        ]
        with self.assertRaisesRegex(ValueError, "self-control counts"):
            opcode_gas.fit_opcode_relations(manifest, truncated_self)

    def test_artifact_validation_rechecks_all_formal_evidence_after_rehash(self):
        manifest = formal_relation_manifest()
        rows = formal_relation_rows(manifest)
        artifact = opcode_gas.fit_opcode_relations(manifest, rows)

        mutations = []
        missing_self = copy.deepcopy(artifact)
        missing_self["self_controls"] = []
        mutations.append(("self controls", missing_self))
        missing_holdouts = copy.deepcopy(artifact)
        missing_holdouts["dynamic_holdouts"] = []
        mutations.append(("dynamic holdouts", missing_holdouts))
        wrong_gates = copy.deepcopy(artifact)
        wrong_gates["quality_gates"]["repeats"] = 2
        mutations.append(("quality gates", wrong_gates))
        stale_provenance = copy.deepcopy(artifact)
        stale_provenance["provenance"]["calibration_id"] = "f" * 24
        mutations.append(("provenance", stale_provenance))
        bad_evidence = copy.deepcopy(artifact)
        bad_evidence["dynamic_holdouts"][0]["r2_p"] = "0"
        mutations.append(("quality evidence", bad_evidence))
        noncanonical_count = copy.deepcopy(artifact)
        noncanonical_count["equations"][0]["selected_counts"][0] = False
        mutations.append(("quality evidence", noncanonical_count))
        arbitrary_raw_hash = copy.deepcopy(artifact)
        arbitrary_raw_hash["raw_rows_sha256"] = "f" * 64
        mutations.append(("raw rows hash", arbitrary_raw_hash))
        plausible_but_false_r2 = copy.deepcopy(artifact)
        plausible_but_false_r2["equations"][0]["r2_p"] = "0.999"
        mutations.append(("quality evidence", plausible_but_false_r2))

        for label, tampered in mutations:
            with self.subTest(label=label):
                tampered["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(
                        {
                            key: value
                            for key, value in tampered.items()
                            if key != "artifact_sha256"
                        }
                    )
                )
                with self.assertRaisesRegex(ValueError, label):
                    opcode_gas.validate_opcode_relations_artifact(
                        manifest,
                        tampered,
                        rows,
                        formal_relation_provenance(rows),
                    )

    def test_rejects_rounded_or_tampered_exact_basis_coefficients(self):
        manifest = formal_relation_manifest()
        artifact = opcode_gas.fit_opcode_relations(
            manifest, formal_relation_rows(manifest)
        )
        tampered = copy.deepcopy(artifact)
        opcode_key = next(
            key
            for key, row in tampered["affine_model"]["B"].items()
            if any(value != "0" for value in row.values())
        )
        anchor_key = next(
            key
            for key, value in tampered["affine_model"]["B"][opcode_key].items()
            if value != "0"
        )
        tampered["affine_model"]["B"][opcode_key][anchor_key] = "0.5"
        tampered["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(
                {
                    key: value
                    for key, value in tampered.items()
                    if key != "artifact_sha256"
                }
            )
        )

        with self.assertRaisesRegex(ValueError, "basis"):
            rows = formal_relation_rows(manifest)
            opcode_gas.validate_opcode_relations_artifact(
                manifest,
                tampered,
                rows,
                formal_relation_provenance(rows),
            )


class CandidateConstructionTests(unittest.TestCase):
    def test_manifest_exposes_six_static_count_controls_outside_formal_rows(self):
        manifest = formal_relation_manifest()

        self.assertEqual(len(manifest.block_calibration_rows), 48)
        self.assertEqual(
            [row.program.count for row in manifest.static_count_control_rows],
            [1, 2, 4, 8, 16, 32],
        )
        self.assertTrue(
            all(
                row.workload_family == "static_count_control"
                and row.split == "diagnostic"
                and row.program.kind == "opcode_loop"
                and row.program.family == "static_count_control"
                and row.program.scenario == "push3_pop_fixed_pop"
                for row in manifest.static_count_control_rows
            )
        )

    def _run_block_calibration_with_drift(
        self, drift_field, *, reject_first_control=False, capture_error=False
    ):
        manifest = formal_relation_manifest()
        first_row_id = manifest.block_calibration_rows[0].row_id
        repeats_by_row = {}

        def fake_run(cmd, *, check):
            self.assertTrue(check)
            spec_path = pathlib.Path(cmd[cmd.index("--input") + 1])
            report_path = pathlib.Path(cmd[cmd.index("--jsonl-out") + 1])
            spec = opcode_gas.json.loads(spec_path.read_text())
            is_parity_gate = spec_path.name == "parity-spec.json"
            repeat_index = repeats_by_row.get(spec["row_id"], 0)
            if not is_parity_gate:
                repeats_by_row[spec["row_id"]] = repeat_index + 1
            drift = (
                (spec["row_id"] == first_row_id and repeat_index == 1)
                or (
                    drift_field == "control_prover_gas"
                    and spec["workload_family"] == "static_count_control"
                    and repeat_index == 1
                )
            )
            execution_engine = cmd[cmd.index("--sp1-execution-engine") + 1]
            parity_backend_drift = (
                is_parity_gate
                and execution_engine == "standard"
                and drift_field == "parity_backend_input"
            )
            parity_guest_drift = (
                is_parity_gate
                and execution_engine == "standard"
                and drift_field == "parity_guest_input"
            )
            reject_control = (
                reject_first_control
                and not is_parity_gate
                and spec["workload_family"] == "static_count_control"
                and repeat_index == 0
            )
            if reject_control:
                report_path.write_text(
                    opcode_gas.json.dumps(
                        {
                            "controlled_block": {
                                "status": "rejected",
                                "reasons": ["host_trace_mismatch"],
                                "error": "synthetic rejection",
                            }
                        }
                    )
                    + "\n"
                )
                return
            backend_input = (
                "c" if drift and drift_field == "backend_input" or parity_backend_drift else "b"
            ) * 64
            guest_input = (
                "d" if drift and drift_field == "guest_input" or parity_guest_drift else "b"
            ) * 64
            public_values = "0x02" if drift and drift_field == "public_values" else "0x01"
            public_output = "0x03" if drift and drift_field == "public_output" else "0x01"
            prover_gas = (
                101 if drift and drift_field in {"prover_gas", "control_prover_gas"} else 100
            )
            reported_row_id = (
                "f" * 64 if drift and drift_field == "reported_row_id" else spec["row_id"]
            )
            observation_row_id = (
                "e" * 64
                if drift and drift_field == "observation_row_id"
                else spec["row_id"]
            )
            report_path.write_text(
                opcode_gas.json.dumps(
                    {
                        "guest_input_sha256": "0x" + guest_input,
                        "sp1_execution_engine": execution_engine,
                        "sp1_gas_trace_chunk_threshold": (
                            None if execution_engine == "standard" else 134_217_728
                        ),
                        "sp1_gas_trace_chunk_slots": (
                            None if execution_engine == "standard" else 2
                        ),
                        "gas": prover_gas,
                        "total_instruction_count": 200,
                        "total_syscall_count": 20,
                        "exit_code": 0,
                        "public_values": public_values,
                        "controlled_block": {
                            "status": "accepted",
                            "row_id": reported_row_id,
                            "reasons": [],
                            "observation": {
                                "row_id": observation_row_id,
                                "backend_input_sha256": backend_input,
                                "public_output": public_output,
                                "actual_final_state_root": spec[
                                    "expected_final_state_root"
                                ],
                                "actual_raw_gas_by_key": spec["expected_raw_gas_by_key"],
                                "actual_features": spec["expected_features"],
                                "actual_diagnostics": spec["expected_diagnostics"],
                            },
                        },
                    }
                )
                + "\n"
            )

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            opcode_gas.subprocess, "run", fake_run
        ):
            output = pathlib.Path(tmp) / "block-calibration-rows.jsonl"
            error = None
            try:
                rows = opcode_gas.run_block_calibration_rows(
                    manifest=manifest,
                    affine_model=block_affine_model(manifest),
                    guest_launcher=pathlib.Path("guest-launcher"),
                    calibration_run_id="calibration",
                    relation_artifact_sha256="a" * 64,
                    relation_raw_rows_sha256="b" * 64,
                    out=output,
                )
            except ValueError as caught:
                if not capture_error:
                    raise
                rows = None
                error = caught
            raw_rows = list(opcode_gas.iter_jsonl(output)) if output.exists() else []
        if capture_error:
            return first_row_id, rows, raw_rows, error
        return first_row_id, rows, raw_rows

    def test_block_calibration_rejects_nondeterministic_prover_gas_as_one_row(self):
        row_id, rows, _ = self._run_block_calibration_with_drift("prover_gas")
        matching = [row for row in rows if row["row_id"] == row_id]
        self.assertEqual(len(matching), 1)
        self.assertEqual(matching[0]["status"], "rejected")
        self.assertIn("repeat_instability", matching[0]["reasons"])
        self.assertIn("prover_gas", matching[0]["repeat_mismatches"])

    def test_block_calibration_rejects_public_or_input_identity_drift(self):
        for field in (
            "public_values",
            "public_output",
            "backend_input",
            "guest_input",
            "reported_row_id",
            "observation_row_id",
        ):
            with self.subTest(field=field):
                row_id, rows, _ = self._run_block_calibration_with_drift(field)
                matching = [row for row in rows if row["row_id"] == row_id]
                self.assertEqual(len(matching), 1)
                self.assertEqual(matching[0]["status"], "rejected")
                self.assertIn("repeat_instability", matching[0]["reasons"])

    def test_block_calibration_keeps_controls_in_raw_evidence_but_not_formal_return(self):
        _, formal_rows, raw_rows = self._run_block_calibration_with_drift(None)

        controls = [
            row for row in raw_rows if row["purpose"] == "static_count_control"
        ]
        self.assertEqual(len(controls), 18)
        self.assertEqual(
            {row["row_id"] for row in controls},
            {
                row.row_id
                for row in formal_relation_manifest().static_count_control_rows
            },
        )
        self.assertEqual({row["cross_input_data_floor_p"] for row in raw_rows}, {0})
        self.assertTrue(
            all(row["purpose"] == "block_calibration" for row in formal_rows)
        )
        self.assertEqual(len(formal_rows), 144)

    def test_block_calibration_rejects_parity_input_identity_drift(self):
        for field in ("parity_backend_input", "parity_guest_input"):
            with self.subTest(field=field):
                with self.assertRaisesRegex(ValueError, "parity"):
                    self._run_block_calibration_with_drift(field)

    def test_block_calibration_persists_rejected_control_before_formal_abort(self):
        _, rows, raw_rows, error = self._run_block_calibration_with_drift(
            None, reject_first_control=True, capture_error=True
        )

        self.assertIsNone(rows)
        self.assertIsNotNone(error)
        self.assertRegex(str(error), "raw evidence was preserved")
        self.assertEqual(len(raw_rows), 1)
        self.assertEqual(raw_rows[0]["purpose"], "static_count_control")
        self.assertEqual(raw_rows[0]["status"], "rejected")

    def test_block_calibration_persists_unstable_control_raw_repeats_before_abort(self):
        _, rows, raw_rows, error = self._run_block_calibration_with_drift(
            "control_prover_gas", capture_error=True
        )

        self.assertIsNone(rows)
        self.assertIsNotNone(error)
        self.assertRegex(str(error), "raw evidence was preserved")
        self.assertEqual(len(raw_rows), 4)
        self.assertEqual(
            [row["status"] for row in raw_rows],
            ["accepted", "accepted", "accepted", "rejected"],
        )
        self.assertEqual(
            {row["purpose"] for row in raw_rows}, {"static_count_control"}
        )
        self.assertEqual(raw_rows[-1]["reasons"], ["repeat_instability"])

    def test_block_calibration_control_validator_rejects_second_repeat_trace_and_bool_floor(self):
        _, _, raw_rows = self._run_block_calibration_with_drift(None)
        relation_artifact = {
            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
            "status": "accepted",
            "artifact_sha256": "a" * 64,
            "raw_rows_sha256": "b" * 64,
            "provenance": {"calibration_id": "calibration"},
        }
        manifest = formal_relation_manifest()

        changed_repeat = copy.deepcopy(raw_rows)
        control = next(
            row for row in changed_repeat if row["purpose"] == "static_count_control"
        )
        sibling = next(
            row
            for row in changed_repeat
            if row["purpose"] == "static_count_control"
            and row["row_id"] == control["row_id"]
            and row["repeat_index"] == 1
        )
        sibling["actual_features"]["block_base"] = 2
        with self.assertRaisesRegex(ValueError, "control (trace|repeats)"):
            opcode_gas._validated_block_calibration_rows(
                manifest, relation_artifact, changed_repeat
            )

        bool_floor = copy.deepcopy(raw_rows)
        control_ids = sorted(
            {row["row_id"] for row in bool_floor if row["purpose"] == "static_count_control"}
        )
        for row in bool_floor:
            if row["row_id"] == control_ids[-1]:
                row["prover_gas"] = 101
            row["cross_input_data_floor_p"] = True
        with self.assertRaisesRegex(ValueError, "data-control floor"):
            opcode_gas._validated_block_calibration_rows(
                manifest, relation_artifact, bool_floor
            )

    def test_block_calibration_validator_rejects_legacy_standard_rows_without_controls(self):
        _, _, raw_rows = self._run_block_calibration_with_drift(None)
        relation_artifact = {
            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
            "status": "accepted",
            "artifact_sha256": "a" * 64,
            "raw_rows_sha256": "b" * 64,
            "provenance": {"calibration_id": "calibration"},
        }
        legacy_rows = [
            copy.deepcopy(row)
            for row in raw_rows
            if row["purpose"] == "block_calibration"
        ]
        self.assertEqual(len(legacy_rows), 144)
        for row in legacy_rows:
            row["sp1_execution_engine"] = "standard"
            row["sp1_gas_trace_chunk_threshold"] = None
            row["sp1_gas_trace_chunk_slots"] = None
            del row["cross_input_data_floor_p"]
        with self.assertRaisesRegex(ValueError, "control rows are incomplete"):
            opcode_gas._validated_block_calibration_rows(
                formal_relation_manifest(), relation_artifact, legacy_rows
            )

    def test_block_calibration_validator_rejects_appended_unknown_raw_purpose(self):
        _, _, raw_rows = self._run_block_calibration_with_drift(None)
        relation_artifact = {
            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
            "status": "accepted",
            "artifact_sha256": "a" * 64,
            "raw_rows_sha256": "b" * 64,
            "provenance": {"calibration_id": "calibration"},
        }
        raw_rows.append({"purpose": "unknown", "garbage": True})

        with self.assertRaisesRegex(ValueError, "raw row purpose"):
            opcode_gas._validated_block_calibration_rows(
                formal_relation_manifest(), relation_artifact, raw_rows
            )

    def test_overhead_residual_uses_raw_gas_units_not_operation_event_count(self):
        rows = []
        for count in [0, 1, 2, 4, 8]:
            for lane in ["target", "control"]:
                workload_id = opcode_gas.sha256_bytes(f"{lane}:{count}".encode())
                for repeat_index in range(3):
                    rows.append(
                        {
                            "case": "tx_base_no_code_no_value",
                            "overhead_key_id": "tx_base",
                            "lane": lane,
                            "target_count": count,
                            "workload_id": workload_id,
                            "backend_input_sha256": "b" * 64,
                            "execution_row_id": opcode_gas.controlled_execution_row_id(
                                workload_id,
                                backend="sp1",
                                execution_engine="standard",
                                run_id="calibration",
                                repeat_index=repeat_index,
                                backend_input_sha256="b" * 64,
                            ),
                            "repeat_index": repeat_index,
                            **standard_execution_provenance(),
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
                        "overhead_key_id": "test-overhead",
                        "lane": lane,
                        "target_count": 1 if startup else count,
                        "generator_max_count": 8,
                        "workload_id": workload_id,
                        "backend_input_sha256": "b" * 64,
                        "execution_row_id": opcode_gas.controlled_execution_row_id(
                            workload_id,
                            backend="sp1",
                            execution_engine="standard",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256="b" * 64,
                        ),
                        "repeat_index": repeat_index,
                        **standard_execution_provenance(),
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
                        "sp1_execution_engine": "standard",
                        "sp1_gas_trace_chunk_threshold": None,
                        "sp1_gas_trace_chunk_slots": None,
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
            "run-block-calibration": [
                "--guest-launcher", "guest-launcher",
                "--calibration-run", "run",
                "--controlled-manifest", "manifest.toml",
                "--relations", "run/opcode-relations.json",
                "--out", "run/block-calibration-rows.jsonl",
            ],
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
        block = parser.parse_args(
            ["run-block-calibration", *commands["run-block-calibration"]]
        )
        self.assertEqual(block.repeats, 3)

    def test_fit_block_calibration_serializes_canonical_evidence(self):
        anchors = ("A0", "A1", "A2", "A3")
        features = tuple(opcode_gas.Q_FORMULA)
        opcode_keys = (*anchors, "DERIVED")
        affine_model = opcode_gas.AffineOpcodeModel(
            opcode_keys=opcode_keys,
            anchor_keys=anchors,
            rank=1,
            nullity=4,
            mu_zero={**{key: Decimal(0) for key in anchors}, "DERIVED": Decimal("5")},
            anchor_basis={
                key: {
                    anchor: opcode_gas.Fraction(int(key == anchor))
                    for anchor in anchors
                }
                for key in opcode_keys
            },
        )
        parameters = tuple(map(Decimal, ("2", "3", "4", "5", "6", "7", "8", "9")))
        specs = []
        raw_rows = []
        for family_index in range(2):
            for parameter_index, parameter in enumerate(parameters):
                raw = {key: 0 for key in opcode_keys}
                q = {key: 0 for key in features}
                if parameter_index < 4:
                    raw[anchors[parameter_index]] = 1
                else:
                    q[features[parameter_index - 4]] = 1
                row_id = f"family-{family_index}-parameter-{parameter_index}"
                program = (
                    types.SimpleNamespace(
                        kind="opcode_loop",
                        family="pop_family",
                        count=1,
                        scenario="canonical",
                    )
                    if family_index == 0 and parameter_index == 0
                    else types.SimpleNamespace(kind="empty")
                )
                spec = types.SimpleNamespace(
                    row_id=row_id,
                    workload_family=f"family-{family_index}",
                    split="fit",
                    block_count=1,
                    transaction_count=1,
                    program=program,
                    expected_final_state_root="0x" + "1" * 64,
                    expected_raw_gas_by_key=raw,
                    expected_features=q,
                    expected_diagnostics={"witness_node_count": 1},
                )
                specs.append(spec)
                payload = opcode_gas._controlled_block_row_payload(spec)
                for repeat_index in range(3):
                    raw_rows.append(
                        {
                            **payload,
                            "schema_version": 1,
                            "purpose": "block_calibration",
                            "status": "accepted",
                            "repeat_index": repeat_index,
                            "calibration_id": "b" * 24,
                            "relation_artifact_sha256": "a" * 64,
                            "relation_raw_rows_sha256": "c" * 64,
                            "preflight_fit_rank": 8,
                            "backend_input_sha256": "d" * 64,
                            "guest_input_sha256": "0x" + "d" * 64,
                            "execution_row_id": opcode_gas.controlled_execution_row_id(
                                row_id,
                                backend="sp1",
                                execution_engine="standard",
                                run_id="b" * 24,
                                repeat_index=repeat_index,
                                backend_input_sha256="d" * 64,
                            ),
                            "reported_row_id": row_id,
                            "observation_row_id": row_id,
                            "backend": "sp1",
                            "mode": "execute",
                            "sp1_prover": "local",
                            "primary_api": "ExecutionReport::gas",
                            "sp1_execution_engine": "standard",
                            "sp1_gas_trace_chunk_threshold": None,
                            "sp1_gas_trace_chunk_slots": None,
                            "exit_code": 0,
                            "total_instruction_count": 100,
                            "total_syscall_count": 10,
                            "public_values": "0x01",
                            "host_public_output": "0x01",
                            "prover_gas": int(parameter),
                            "actual_raw_gas_by_key": raw,
                            "actual_features": q,
                            "actual_diagnostics": spec.expected_diagnostics,
                            "actual_final_state_root": spec.expected_final_state_root,
                        }
                    )
        holdout_raw = {**{key: 1 for key in anchors}, "DERIVED": 0}
        holdout_q = {key: 1 for key in features}
        holdout_spec = types.SimpleNamespace(
            row_id="holdout",
            workload_family="holdout-family",
            split="holdout",
            block_count=1,
            transaction_count=1,
            program=types.SimpleNamespace(kind="empty"),
            expected_final_state_root="0x" + "2" * 64,
            expected_raw_gas_by_key=holdout_raw,
            expected_features=holdout_q,
            expected_diagnostics={"witness_node_count": 0},
        )
        specs.append(holdout_spec)
        holdout_payload = opcode_gas._controlled_block_row_payload(holdout_spec)
        for repeat_index in range(3):
            raw_rows.append(
                {
                    **holdout_payload,
                    "schema_version": 1,
                    "purpose": "block_calibration",
                    "status": "accepted",
                    "repeat_index": repeat_index,
                    "calibration_id": "b" * 24,
                    "relation_artifact_sha256": "a" * 64,
                    "relation_raw_rows_sha256": "c" * 64,
                    "preflight_fit_rank": 8,
                    "backend_input_sha256": "d" * 64,
                    "guest_input_sha256": "0x" + "d" * 64,
                    "execution_row_id": opcode_gas.controlled_execution_row_id(
                        "holdout",
                        backend="sp1",
                        execution_engine="standard",
                        run_id="b" * 24,
                        repeat_index=repeat_index,
                        backend_input_sha256="d" * 64,
                    ),
                    "reported_row_id": "holdout",
                    "observation_row_id": "holdout",
                    "backend": "sp1",
                    "mode": "execute",
                    "sp1_prover": "local",
                    "primary_api": "ExecutionReport::gas",
                    "sp1_execution_engine": "standard",
                    "sp1_gas_trace_chunk_threshold": None,
                    "sp1_gas_trace_chunk_slots": None,
                    "exit_code": 0,
                    "total_instruction_count": 100,
                    "total_syscall_count": 10,
                    "public_values": "0x01",
                    "host_public_output": "0x01",
                    "prover_gas": int(sum(parameters)),
                    "actual_raw_gas_by_key": holdout_raw,
                    "actual_features": holdout_q,
                    "actual_diagnostics": holdout_spec.expected_diagnostics,
                    "actual_final_state_root": holdout_spec.expected_final_state_root,
                }
            )
        dynamic_rows = []
        for scenario_id, split, raw_units in (
            ("canonical", "canonical", 1),
            ("medium", "dynamic_holdout", 2),
            ("large", "dynamic_holdout", 4),
        ):
            dynamic_rows.append(
                {
                    "relation_id": scenario_id,
                    "scenario_id": scenario_id,
                    "split": split,
                    "dynamic_key": "A0",
                    "signed_raw_gas_by_key": {
                        "A0": str(raw_units),
                        "A1": "-1",
                    },
                    "slope_p": str(raw_units * 2 - 3),
                }
            )
        relation_artifact = {
            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
            "status": "accepted",
            "artifact_sha256": "a" * 64,
            "raw_rows_sha256": "c" * 64,
            "provenance": {"calibration_id": "b" * 24},
            "equations": [dynamic_rows[0]],
            "dynamic_holdouts": dynamic_rows[1:],
        }
        for raw_row in raw_rows:
            raw_row["sp1_execution_engine"] = "gas-estimator"
            raw_row["sp1_gas_trace_chunk_threshold"] = 134_217_728
            raw_row["sp1_gas_trace_chunk_slots"] = 2
            raw_row["execution_row_id"] = opcode_gas.controlled_execution_row_id(
                raw_row["row_id"],
                backend="sp1",
                execution_engine="gas-estimator",
                run_id="b" * 24,
                repeat_index=raw_row["repeat_index"],
                backend_input_sha256="d" * 64,
            )
            raw_row["cross_input_data_floor_p"] = 0
        static_count_controls = []
        control_features = {key: 0 for key in features}
        control_raw = {key: 0 for key in anchors}
        for control_index, count in enumerate((1, 2, 4, 8, 16, 32)):
            control = types.SimpleNamespace(
                row_id=f"static-control-{control_index}",
                workload_family="static_count_control",
                split="diagnostic",
                block_count=1,
                transaction_count=1,
                program=types.SimpleNamespace(
                    kind="opcode_loop",
                    family="static_count_control",
                    count=count,
                    scenario="push3_pop_fixed_pop",
                ),
                expected_final_state_root="0x" + "3" * 64,
                expected_raw_gas_by_key=control_raw,
                expected_features=control_features,
                expected_diagnostics={"witness_node_count": 0},
            )
            static_count_controls.append(control)
            control_payload = opcode_gas._controlled_block_row_payload(control)
            for repeat_index in range(3):
                raw_rows.append(
                    {
                        **control_payload,
                        "schema_version": 1,
                        "purpose": "static_count_control",
                        "status": "accepted",
                        "repeat_index": repeat_index,
                        "calibration_id": "b" * 24,
                        "relation_artifact_sha256": "a" * 64,
                        "relation_raw_rows_sha256": "c" * 64,
                        "preflight_fit_rank": 8,
                        "backend_input_sha256": "d" * 64,
                        "guest_input_sha256": "0x" + "d" * 64,
                        "execution_row_id": opcode_gas.controlled_execution_row_id(
                            control.row_id,
                            backend="sp1",
                            execution_engine="gas-estimator",
                            run_id="b" * 24,
                            repeat_index=repeat_index,
                            backend_input_sha256="d" * 64,
                        ),
                        "reported_row_id": control.row_id,
                        "observation_row_id": control.row_id,
                        "backend": "sp1",
                        "mode": "execute",
                        "sp1_prover": "local",
                        "primary_api": "ExecutionReport::gas",
                        "sp1_execution_engine": "gas-estimator",
                        "sp1_gas_trace_chunk_threshold": 134_217_728,
                        "sp1_gas_trace_chunk_slots": 2,
                        "exit_code": 0,
                        "total_instruction_count": 100,
                        "total_syscall_count": 10,
                        "public_values": "0x01",
                        "host_public_output": "0x01",
                        "prover_gas": 100,
                        "actual_raw_gas_by_key": control_raw,
                        "actual_features": control_features,
                        "actual_diagnostics": control.expected_diagnostics,
                        "actual_final_state_root": control.expected_final_state_root,
                        "cross_input_data_floor_p": 0,
                    }
                )
        manifest = types.SimpleNamespace(
            block_calibration_rows=tuple(specs),
            static_count_control_rows=tuple(static_count_controls),
            dynamic_raw_gas_keys=("A0",),
            normalization_reference_key="A0",
        )

        artifact = opcode_gas.fit_block_calibration_artifact(
            manifest, affine_model, relation_artifact, raw_rows
        )

        self.assertEqual(artifact["status"], "accepted")
        self.assertEqual(artifact["parameter_order"], [*anchors, *features])
        self.assertEqual(artifact["exact_fit_rank"], 8)
        self.assertEqual(artifact["opcode_multipliers_add_normalized"]["A0"], "1")
        self.assertEqual(len(artifact["dynamic_holdouts"]["A0"]["observations"]), 3)
        self.assertEqual(
            artifact["raw_block_rows_sha256"],
            opcode_gas.sha256_bytes(opcode_gas.canonical_json(raw_rows)),
        )
        self.assertEqual(
            artifact["artifact_sha256"],
            opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {key: value for key, value in artifact.items() if key != "artifact_sha256"}
                )
            ),
        )

        tampered_rows = copy.deepcopy(raw_rows)
        tampered_rows[0]["execution_row_id"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "execution row identity"):
            opcode_gas.fit_block_calibration_artifact(
                manifest, affine_model, relation_artifact, tampered_rows
            )

        def assert_bool_mutation_rejected(label, mutate):
            with self.subTest(bool_alias=label):
                mutated = copy.deepcopy(raw_rows)
                mutate(mutated)
                with self.assertRaises(ValueError):
                    opcode_gas.fit_block_calibration_artifact(
                        manifest, affine_model, relation_artifact, mutated
                    )

        first_row_id = "family-0-parameter-0"
        first_repeats = lambda rows: [
            row for row in rows if row["row_id"] == first_row_id
        ]

        assert_bool_mutation_rejected(
            "schema_version",
            lambda rows: first_repeats(rows)[0].__setitem__("schema_version", True),
        )
        assert_bool_mutation_rejected(
            "exit_code",
            lambda rows: first_repeats(rows)[0].__setitem__("exit_code", False),
        )
        assert_bool_mutation_rejected(
            "preflight_fit_rank",
            lambda rows: first_repeats(rows)[0].__setitem__("preflight_fit_rank", True),
        )
        assert_bool_mutation_rejected(
            "block_count",
            lambda rows: first_repeats(rows)[0].__setitem__("block_count", True),
        )
        assert_bool_mutation_rejected(
            "transaction_count",
            lambda rows: first_repeats(rows)[0].__setitem__("transaction_count", True),
        )
        assert_bool_mutation_rejected(
            "instruction_count",
            lambda rows: first_repeats(rows)[0].__setitem__(
                "total_instruction_count", True
            ),
        )

        def mutate_repeat_index(rows):
            repeat = first_repeats(rows)[0]
            repeat["repeat_index"] = False
            repeat["execution_row_id"] = opcode_gas.controlled_execution_row_id(
                first_row_id,
                backend="sp1",
                execution_engine="standard",
                run_id="b" * 24,
                repeat_index=False,
                backend_input_sha256="d" * 64,
            )

        assert_bool_mutation_rejected("repeat_index", mutate_repeat_index)

        def mutate_integer_map(rows, row_id, expected_field, actual_field, key):
            for repeat in (row for row in rows if row["row_id"] == row_id):
                repeat[expected_field][key] = True
                repeat[actual_field][key] = True

        assert_bool_mutation_rejected(
            "raw_gas",
            lambda rows: mutate_integer_map(
                rows,
                first_row_id,
                "expected_raw_gas_by_key",
                "actual_raw_gas_by_key",
                "A0",
            ),
        )
        assert_bool_mutation_rejected(
            "feature_count",
            lambda rows: mutate_integer_map(
                rows,
                "family-0-parameter-4",
                "expected_features",
                "actual_features",
                features[0],
            ),
        )
        assert_bool_mutation_rejected(
            "diagnostic_count",
            lambda rows: mutate_integer_map(
                rows,
                first_row_id,
                "expected_diagnostics",
                "actual_diagnostics",
                "witness_node_count",
            ),
        )

        def mutate_nested_program_count(rows):
            for repeat in first_repeats(rows):
                repeat["program"]["count"] = True

        assert_bool_mutation_rejected(
            "nested_semantic_count", mutate_nested_program_count
        )

        for label, value in (
            ("integer_valued_float", 2.0),
            ("numeric_string", "2"),
            ("bool", True),
        ):
            with self.subTest(prover_gas_type=label):
                mutated = copy.deepcopy(raw_rows)
                for repeat in first_repeats(mutated):
                    repeat["prover_gas"] = value
                with self.assertRaises(ValueError):
                    opcode_gas.fit_block_calibration_artifact(
                        manifest, affine_model, relation_artifact, mutated
                    )

        for field in (
            "block_count",
            "prover_gas",
            "public_values",
            "actual_diagnostics",
            "actual_final_state_root",
            "total_instruction_count",
            "sp1_gas_trace_chunk_threshold",
        ):
            with self.subTest(missing=field):
                truncated = copy.deepcopy(raw_rows)
                del truncated[0][field]
                with self.assertRaisesRegex(ValueError, "schema"):
                    opcode_gas.fit_block_calibration_artifact(
                        manifest, affine_model, relation_artifact, truncated
                    )

        alterations = {
            "block_count": 2,
            "prover_gas": "999",
            "public_values": "0x02",
            "actual_diagnostics": {"witness_node_count": 2},
            "actual_final_state_root": "0x" + "f" * 64,
            "total_instruction_count": 101,
            "sp1_gas_trace_chunk_threshold": 1,
        }
        for field, value in alterations.items():
            with self.subTest(altered=field):
                altered = copy.deepcopy(raw_rows)
                altered[0][field] = value
                with self.assertRaises(ValueError):
                    opcode_gas.fit_block_calibration_artifact(
                        manifest, affine_model, relation_artifact, altered
                    )

    def test_task5_fit_block_calibration_cli_is_executable(self):
        args = opcode_gas.build_parser().parse_args(
            [
                "fit-block-calibration",
                "--relations", "run/opcode-relations.json",
                "--runs", "run/block-calibration-rows.jsonl",
                "--controlled-manifest", "manifest.toml",
                "--out", "run/block-calibration.json",
            ]
        )
        self.assertEqual(args.command, "fit-block-calibration")
        self.assertTrue(callable(args.func))

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
                            execution_engine="gas-estimator",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256=backend_hash,
                        ),
                        "repeat_index": repeat_index,
                        **opcode_execution_provenance(),
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
                            "tx_gas_limit": 1_000_024,
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

    def test_controlled_fit_rejects_missing_or_mixed_execution_provenance(self):
        base = {
            "case": "add",
            "kind": "opcode",
            "target_count": 0,
            "workload_id": "a" * 64,
            "backend_input_sha256": "b" * 64,
            "gas": 10_000,
            "total_instruction_count": 20_000,
            "exit_code": 0,
            "public_values": "0x01",
            "isolation": {"status": "passed", "tx_gas_limit": 1_000_024},
        }
        missing = [
            {
                **base,
                "repeat_index": repeat,
                "execution_row_id": str(repeat + 1) * 64,
            }
            for repeat in range(3)
        ]
        mixed = [
            {
                **row,
                **(
                    opcode_execution_provenance()
                    if repeat < 2
                    else standard_execution_provenance()
                ),
            }
            for repeat, row in enumerate(missing)
        ]

        for rows in [missing, mixed]:
            with self.subTest(rows=rows), self.assertRaisesRegex(
                ValueError, "SP1 execution provenance"
            ):
                opcode_gas._controlled_repeat_point(rows)

    def test_formal_fit_and_candidate_reject_matched_control_diagnostics(self):
        row = {
            "purpose": "matched_control_diagnostic",
            "case": "add__matched_t",
        }

        with self.assertRaisesRegex(ValueError, "matched-control diagnostic"):
            opcode_gas.fit_controlled_costs(controlled_manifest(), [row])
        with self.assertRaisesRegex(ValueError, "matched-control diagnostic"):
            opcode_gas.build_candidate_components(
                controlled_manifest(),
                [row],
                {},
                {},
            )

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
                            execution_engine="standard",
                            run_id="calibration",
                            repeat_index=repeat_index,
                            backend_input_sha256=backend_hash,
                        ),
                        "repeat_index": repeat_index,
                        **standard_execution_provenance(),
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

    def test_later_operation_rounds_reuse_sealed_128_overhead_raw_and_refit(self):
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
            run, identity = persist_execution_identity(root, controlled_manifest())
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
            self.assertEqual(overhead_runs, [8, 32, 128])
            self.assertEqual(
                overhead_fits[-2],
                (512, 128, {128}, {512}),
            )
            self.assertEqual(
                overhead_fits[-1],
                (2048, 128, {128}, {2048}),
            )
            self.assertEqual(records[-1]["overhead_generator_max_count"], 128)
            for record in records[3:]:
                self.assertEqual(record["overhead_generator_max_count"], 128)
                self.assertEqual(record["overhead_runs"], records[2]["overhead_runs"])
                self.assertEqual(
                    record["overhead_runs_sha256"],
                    records[2]["overhead_runs_sha256"],
                )
            artifacts = opcode_gas.load_terminal_controlled_artifacts(
                run,
                identity,
            )
            self.assertEqual(artifacts["overhead_generator_max_count"], 128)
            self.assertEqual(
                opcode_gas._sealed_candidate_provenance(artifacts)[
                    "terminal_overhead_generator_max_count"
                ],
                128,
            )
            opcode_gas.validate_persisted_controlled_decisions(run, decisions)

            tampered = copy.deepcopy(decisions)
            tampered["rounds"][-1]["overhead_generator_max_count"] = 2048
            with self.assertRaisesRegex(ValueError, "overhead generator"):
                opcode_gas.validate_persisted_controlled_decisions(run, tampered)

            tampered_raw = copy.deepcopy(decisions)
            sealed_overhead_raw = run / records[2]["overhead_runs"]
            alternate_overhead_raw = run / "alternate-overhead-runs.jsonl"
            alternate_overhead_raw.write_bytes(
                sealed_overhead_raw.read_bytes() + sealed_overhead_raw.read_bytes()
            )
            tampered_raw["rounds"][-1]["overhead_runs"] = str(
                alternate_overhead_raw.relative_to(run)
            )
            tampered_raw["rounds"][-1]["overhead_runs_sha256"] = (
                opcode_gas.sha256_file(alternate_overhead_raw)
            )
            with self.assertRaisesRegex(ValueError, "does not reuse the sealed 128 round"):
                opcode_gas.validate_persisted_controlled_decisions(run, tampered_raw)

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
        caller_precision = getcontext().prec
        add = accepted_case("add", "opcode:0x01", "raw_gas_slope", 10**40 + 1, 3)
        mul = accepted_case("mul", "opcode:0x02", "raw_gas_slope", 2 * (10**40 + 1), 5)
        identity = accepted_case("identity", "precompile:0x04", "raw_gas_slope", 6 * (10**40 + 1), 18)
        values = opcode_gas.construct_measurement_values(
            controlled_manifest(), [add, mul, identity]
        )

        self.assertEqual(
            values["opcode:0x01"]["c_p"],
            "3333333333333333333333333333333333333333.6666666666666666666666666666666666666667",
        )
        self.assertEqual(values["opcode:0x02"]["m_p"], "1.2")
        self.assertEqual(values["precompile:0x04"]["m_p"], "1")
        self.assertEqual(getcontext().prec, caller_precision)

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
    def test_block_calibration_preflight_requires_exact_rank_and_frozen_scope(self):
        manifest = formal_relation_manifest()
        opcode_keys = tuple(
            f"opcode:0x{case.opcode:02x}"
            for case in manifest.cases
            if case.kind == "opcode"
            and case.opcode is not None
            and case.opcode in opcode_gas.PURE_OPCODE_DEFAULTS
            and opcode_gas.PURE_OPCODE_DEFAULTS[case.opcode][1] == case.template
        )
        equations = tuple(
            opcode_gas.RelationEquation(
                relation.id,
                {
                    key: opcode_gas.Fraction(value)
                    for key, value in relation.signed_raw_gas_by_key.items()
                },
                Decimal(0),
            )
            for relation in manifest.opcode_relations
            if relation.split == "canonical" and relation.signed_raw_gas_by_key
        )
        model = opcode_gas.derive_affine_opcode_model(
            opcode_keys=opcode_keys,
            equations=equations,
            anchor_keys=opcode_gas.OPCODE_RELATION_ANCHORS,
        )

        result = opcode_gas.preflight_block_calibration_rows(manifest, model)
        self.assertEqual(result["fit_row_count"], 40)
        self.assertEqual(result["holdout_row_count"], 8)
        self.assertEqual(result["fit_rank"], 8)
        self.assertEqual(
            set(result["holdout_families"]),
            {
                "pop_family",
                "push_family",
                "dup_family",
                "swap_family",
                "proposal_startup",
                "block_base",
                "tx_base",
                "native_value_transfer",
            },
        )

        fit_rows = tuple(
            row for row in manifest.block_calibration_rows if row.split == "fit"
        )
        with self.assertRaisesRegex(ValueError, "40 fit rows"):
            opcode_gas.preflight_block_calibration_rows(
                replace(manifest, block_calibration_rows=fit_rows[:-1]), model
            )

        dynamic = replace(
            manifest.block_calibration_rows[0],
            expected_raw_gas_by_key={
                **manifest.block_calibration_rows[0].expected_raw_gas_by_key,
                "opcode:0x0a": 10,
            },
        )
        with self.assertRaisesRegex(ValueError, "dynamic raw-gas"):
            opcode_gas.preflight_block_calibration_rows(
                replace(
                    manifest,
                    block_calibration_rows=(dynamic, *manifest.block_calibration_rows[1:]),
                ),
                model,
            )

        precompile = replace(
            manifest.block_calibration_rows[0],
            expected_raw_gas_by_key={"precompile:0x04": 18},
        )
        with self.assertRaisesRegex(ValueError, "precompile or spawned"):
            opcode_gas.preflight_block_calibration_rows(
                replace(
                    manifest,
                    block_calibration_rows=(precompile, *manifest.block_calibration_rows[1:]),
                ),
                model,
            )

        rounded_basis = {
            key: dict(coefficients)
            for key, coefficients in model.anchor_basis.items()
        }
        used_key = next(
            iter(manifest.block_calibration_rows[0].expected_raw_gas_by_key)
        )
        rounded_basis[used_key][opcode_gas.OPCODE_RELATION_ANCHORS[0]] = 0.0
        with self.assertRaisesRegex(ValueError, "rounded basis"):
            opcode_gas.preflight_block_calibration_rows(
                manifest,
                types.SimpleNamespace(
                    opcode_keys=model.opcode_keys,
                    anchor_keys=model.anchor_keys,
                    anchor_basis=rounded_basis,
                ),
            )

        missing_family = replace(
            manifest.block_calibration_rows[-1],
            workload_family="pop_family",
        )
        with self.assertRaisesRegex(ValueError, "does not have one holdout row"):
            opcode_gas.preflight_block_calibration_rows(
                replace(
                    manifest,
                    block_calibration_rows=(
                        *manifest.block_calibration_rows[:-1],
                        missing_family,
                    ),
                ),
                model,
            )

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
                execution_engine=engine,
                run_id=run_id,
                repeat_index=repeat,
                backend_input_sha256=input_hash,
            )
            for backend, engine, run_id, repeat, input_hash in [
                ("sp1", "gas-estimator", "run", 0, "a" * 64),
                ("sp1", "standard", "run", 0, "a" * 64),
                ("risc0", "standard", "run", 0, "a" * 64),
                ("sp1", "gas-estimator", "other", 0, "a" * 64),
                ("sp1", "gas-estimator", "run", 1, "a" * 64),
                ("sp1", "gas-estimator", "run", 0, "b" * 64),
            ]
        }
        self.assertEqual(len(ids), 6)

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
