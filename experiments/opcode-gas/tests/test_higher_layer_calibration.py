import copy
import json
import pathlib
import sys
import tempfile
import unittest
from decimal import Decimal, localcontext
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "sp1-higher-layer-v1.json"
)
CALIBRATION_ID = "1" * 24


def reseal(manifest):
    manifest.pop("artifact_sha256", None)
    manifest["artifact_sha256"] = opcode_gas.sha256_bytes(
        opcode_gas.canonical_json(manifest)
    )


class HigherLayerManifestTests(unittest.TestCase):
    def load_raw(self):
        opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        return json.loads(MANIFEST_PATH.read_text())

    def assert_rejected(self, mutate, expected):
        artifact = copy.deepcopy(self.load_raw())
        mutate(artifact)
        reseal(artifact)
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "manifest.json"
            path.write_bytes(opcode_gas._canonical_json_file_bytes(artifact))
            with self.assertRaisesRegex(ValueError, expected):
                opcode_gas.load_higher_layer_manifest(path)

    def test_committed_manifest_freezes_exact_campaign_contract(self):
        manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)

        self.assertEqual(
            manifest.q_formula,
            (
                "proposal_startup",
                "block_base",
                "tx_base",
                "native_value_transfer",
            ),
        )
        self.assertEqual(manifest.generator_rounds, (8, 32, 128))
        self.assertEqual(manifest.repeats, 3)
        self.assertEqual(
            manifest.overhead_case_ids,
            (
                "tx_base_no_code_no_value",
                "tx_base_minimal_contract_call",
                "native_transfer_positive_vs_zero",
                "block_base_one_vs_two_minimal_blocks",
                "startup_minimal_no_candidate_tx",
                "startup_minimal_one_no_code_tx",
            ),
        )
        self.assertEqual(
            tuple(pair.pair_id for pair in manifest.state_holdouts),
            (
                "witness_topology_1",
                "witness_topology_8",
                "witness_topology_32",
                "dirty_accounts_2",
                "dirty_accounts_8",
                "dirty_accounts_32",
            ),
        )
        self.assertEqual(
            tuple((pair.kind, pair.scale) for pair in manifest.state_holdouts),
            (
                ("witness_topology", 1),
                ("witness_topology", 8),
                ("witness_topology", 32),
                ("dirty_accounts", 2),
                ("dirty_accounts", 8),
                ("dirty_accounts", 32),
            ),
        )

    def test_committed_manifest_is_canonical_and_content_addressed(self):
        raw = self.load_raw()

        self.assertEqual(
            MANIFEST_PATH.read_bytes(), opcode_gas._canonical_json_file_bytes(raw)
        )
        unhashed = {
            key: value for key, value in raw.items() if key != "artifact_sha256"
        }
        self.assertEqual(
            raw["artifact_sha256"],
            opcode_gas.sha256_bytes(opcode_gas.canonical_json(unhashed)),
        )
        opcode_gas.validate_higher_layer_manifest(raw)

    def test_source_references_bind_exact_artifact_and_file_bytes(self):
        manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)

        for source_ref in (
            manifest.operation_coverage_ref,
            manifest.augmented_core_ref,
        ):
            source_path = ROOT / source_ref["path"]
            source = json.loads(source_path.read_text())
            self.assertEqual(source["artifact_sha256"], source_ref["artifact_sha256"])
            self.assertEqual(
                opcode_gas.sha256_file(source_path), source_ref["file_sha256"]
            )
            self.assertEqual(
                source_path.read_bytes(), opcode_gas._canonical_json_file_bytes(source)
            )

    def test_rejects_changed_source_artifact_hashes(self):
        self.assert_rejected(
            lambda artifact: artifact["operation_coverage_ref"].__setitem__(
                "artifact_sha256", "0" * 64
            ),
            "operation coverage reference differs",
        )
        self.assert_rejected(
            lambda artifact: artifact["augmented_core_ref"].__setitem__(
                "artifact_sha256", "0" * 64
            ),
            "augmented core reference differs",
        )

    def test_rejects_extra_fit_family_overhead_case_and_round_512(self):
        self.assert_rejected(
            lambda artifact: artifact["q_formula"].append("state_trie"),
            "Q formula differs",
        )
        self.assert_rejected(
            lambda artifact: artifact["overhead_case_ids"].append(
                "unreviewed_overhead_family"
            ),
            "overhead cases differ",
        )
        self.assert_rejected(
            lambda artifact: artifact["generator_rounds"].append(512),
            "generator rounds differ",
        )

    def test_rejects_missing_checkpoint_gate(self):
        self.assert_rejected(
            lambda artifact: artifact["gates"].pop("maximum_checkpoint_ape"),
            "gates differ",
        )

    def test_rejects_noncanonical_or_duplicate_state_holdout(self):
        self.assert_rejected(
            lambda artifact: artifact["state_holdouts"][0].__setitem__(
                "pair_id", "witness-topology-1"
            ),
            "state holdouts differ",
        )

        def duplicate(artifact):
            artifact["state_holdouts"][1] = copy.deepcopy(
                artifact["state_holdouts"][0]
            )

        self.assert_rejected(duplicate, "duplicate state holdout")

    def test_rejects_state_holdout_in_fit_set(self):
        self.assert_rejected(
            lambda artifact: artifact["overhead_case_ids"].append(
                artifact["state_holdouts"][0]["pair_id"]
            ),
            "state holdouts cannot enter the fit set",
        )

    def test_rejects_resealed_changed_source_path(self):
        self.assert_rejected(
            lambda artifact: artifact["operation_coverage_ref"].__setitem__(
                "path", artifact["augmented_core_ref"]["path"]
            ),
            "operation coverage reference differs",
        )

    def test_rejects_unknown_fields_at_each_schema_level(self):
        mutations = (
            lambda artifact: artifact.__setitem__("unexpected", True),
            lambda artifact: artifact["version_identity"].__setitem__(
                "unexpected", "value"
            ),
            lambda artifact: artifact["operation_coverage_ref"].__setitem__(
                "unexpected", "value"
            ),
            lambda artifact: artifact["state_holdouts"][0].__setitem__(
                "unexpected", "value"
            ),
            lambda artifact: artifact["state_holdouts"][0]["control"].__setitem__(
                "unexpected", 1
            ),
            lambda artifact: artifact["gates"].__setitem__(
                "unexpected", "0.1"
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                self.assert_rejected(mutate, "unknown fields|differs")


class StaticOperationDeltaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        cls.coverage = json.loads(
            (ROOT / manifest.operation_coverage_ref["path"]).read_text()
        )
        cls.core = json.loads((ROOT / manifest.augmented_core_ref["path"]).read_text())

    def test_resolves_exact_dispatch_and_raw_gas_body_terms(self):
        delta = {
            "pricing_basis": "raw_gas_slope",
            "units": 8,
            "event_count": 4,
        }
        with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
            expected = (
                Decimal(delta["event_count"])
                * Decimal(self.core["registry"]["common_dispatch"])
                + Decimal(delta["units"])
                * Decimal(self.core["body_scale"])
                * Decimal(
                    self.core["registry"]["models"]["opcode:0x5f"]["parameters"]
                    ["body_per_raw_gas"]
                )
            )
            negative_expected = -expected

        self.assertEqual(
            opcode_gas.resolve_static_operation_delta(
                self.core, self.coverage, "opcode:0x5f", delta
            ),
            expected,
        )
        self.assertEqual(
            opcode_gas.resolve_static_operation_delta(
                self.core,
                self.coverage,
                "opcode:0x5f",
                {**delta, "units": -8, "event_count": -4},
            ),
            negative_expected,
        )

    def test_rejects_missing_event_count_wrong_basis_and_impossible_signs(self):
        valid = {
            "pricing_basis": "raw_gas_slope",
            "units": 2,
            "event_count": 1,
        }
        invalid = (
            ({key: value for key, value in valid.items() if key != "event_count"}, "event_count"),
            ({**valid, "pricing_basis": "fixed_per_event"}, "pricing basis"),
            ({**valid, "event_count": -1}, "sign"),
            ({**valid, "units": -2}, "sign"),
        )
        for delta, expected in invalid:
            with self.subTest(delta=delta), self.assertRaisesRegex(ValueError, expected):
                opcode_gas.resolve_static_operation_delta(
                    self.core, self.coverage, "opcode:0x5f", delta
                )

    def test_resolves_measured_dispatch_only_static_operation(self):
        delta = {
            "pricing_basis": "raw_gas_slope",
            "units": 3,
            "event_count": 1,
        }

        self.assertEqual(
            opcode_gas.resolve_static_operation_delta(
                self.core, self.coverage, "opcode:0x19", delta
            ),
            Decimal(self.core["registry"]["common_dispatch"]),
        )

    def test_rejects_non_static_unmeasured_wrapper_and_absent_coverage(self):
        delta = {
            "pricing_basis": "raw_gas_slope",
            "units": 2,
            "event_count": 1,
        }
        rejected = (
            ("opcode:0x0a", "static_raw_gas"),
            ("opcode:0x00", "static_raw_gas"),
            ("precompile:0x01", "static_raw_gas"),
            ("opcode:0xf1:spawned", "coverage"),
            ("opcode:0xaa:absent", "coverage"),
        )
        for key, expected in rejected:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, expected):
                opcode_gas.resolve_static_operation_delta(
                    self.core, self.coverage, key, delta
                )

    def test_rejects_inputs_other_than_the_exact_pinned_coverage_and_core(self):
        delta = {
            "pricing_basis": "raw_gas_slope",
            "units": 2,
            "event_count": 1,
        }
        changed_coverage = copy.deepcopy(self.coverage)
        changed_coverage["artifact_sha256"] = "0" * 64
        changed_core = copy.deepcopy(self.core)
        changed_core["artifact_sha256"] = "0" * 64

        with self.assertRaisesRegex(ValueError, "pinned operation coverage"):
            opcode_gas.resolve_static_operation_delta(
                self.core, changed_coverage, "opcode:0x5f", delta
            )
        with self.assertRaisesRegex(ValueError, "pinned augmented core"):
            opcode_gas.resolve_static_operation_delta(
                changed_core, self.coverage, "opcode:0x5f", delta
            )


class HigherLayerFixedRoundTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        cls.coverage = json.loads(
            (ROOT / cls.manifest.operation_coverage_ref["path"]).read_text()
        )
        cls.core = json.loads(
            (ROOT / cls.manifest.augmented_core_ref["path"]).read_text()
        )

    def rows(self, bound, *, corrupt_case=None, corrupt_mode=None):
        costs = {
            "proposal_startup": Decimal("1000"),
            "block_base": Decimal("2000"),
            "tx_base": Decimal("300"),
            "native_value_transfer": Decimal("40"),
        }
        rows = []
        counts = opcode_gas.controlled_round_counts(bound)
        cases = (
            ("tx_base_no_code_no_value", "tx_base"),
            ("tx_base_minimal_contract_call", "tx_base"),
            ("native_transfer_positive_vs_zero", "native_value_transfer"),
            ("block_base_one_vs_two_minimal_blocks", "block_base"),
        )
        for case_id, key_id in cases:
            for count in counts:
                for lane in ("target", "control"):
                    feature_deltas = {}
                    if lane == "target":
                        feature_deltas = {
                            "tx_base": {"tx_base": count},
                            "native_value_transfer": {
                                "native_value_transfer": count,
                                "tx_base": 0,
                            },
                            "block_base": {
                                "block_base": count,
                                "native_value_transfer": 0,
                                "tx_base": 0,
                            },
                        }[key_id]
                    operation_deltas = {}
                    gas = Decimal("10000")
                    if lane == "target":
                        gas += costs[key_id] * count
                        if case_id == "tx_base_minimal_contract_call" and count:
                            operation_deltas = {
                                "opcode:0x5f": {
                                    "pricing_basis": "raw_gas_slope",
                                    "units": count,
                                    "event_count": count,
                                }
                            }
                            with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
                                gas += opcode_gas.resolve_static_operation_delta(
                                    self.core,
                                    self.coverage,
                                    "opcode:0x5f",
                                    operation_deltas["opcode:0x5f"],
                                )
                    for repeat in range(3):
                        row_gas = gas
                        if (
                            case_id == corrupt_case
                            and corrupt_mode == "repeat_noise"
                            and repeat == 2
                            and lane == "target"
                        ):
                            row_gas += 1
                        row = {
                            "case": case_id,
                            "overhead_key_id": key_id,
                            "lane": lane,
                            "baseline_kind": None,
                            "target_count": count,
                            "generator_max_count": bound,
                            "repeat_index": repeat,
                            "status": "accepted",
                            "prover_gas": str(row_gas),
                            "expected_feature_deltas": feature_deltas,
                            "observed_operation_deltas": operation_deltas,
                            "sp1_execution_engine": "gas-estimator",
                            "sp1_gas_trace_chunk_threshold": 134_217_728,
                            "sp1_gas_trace_chunk_slots": 2,
                        }
                        self.bind_identity(row)
                        rows.append(row)

        startup_cases = (
            ("startup_minimal_no_candidate_tx", 0),
            ("startup_minimal_one_no_code_tx", 1),
        )
        for case_id, tx_count in startup_cases:
            gas = (
                costs["proposal_startup"]
                + costs["block_base"]
                + costs["tx_base"] * tx_count
            )
            for repeat in range(3):
                row = {
                    "case": case_id,
                    "overhead_key_id": "proposal_startup",
                    "lane": "target",
                    "baseline_kind": "mathematical_zero_baseline",
                    "target_count": 1,
                    "generator_max_count": bound,
                    "repeat_index": repeat,
                    "status": "accepted",
                    "prover_gas": str(gas),
                    "expected_feature_deltas": {
                        "proposal_startup": 1,
                        "block_base": 1,
                        "tx_base": tx_count,
                        "native_value_transfer": 0,
                    },
                    "observed_operation_deltas": {},
                    "sp1_execution_engine": "gas-estimator",
                    "sp1_gas_trace_chunk_threshold": 134_217_728,
                    "sp1_gas_trace_chunk_slots": 2,
                }
                self.bind_identity(row)
                rows.append(row)
        return rows

    def bind_identity(self, row, *, calibration_id=CALIBRATION_ID):
        row.update(
            {
                "expected_operation_deltas": copy.deepcopy(
                    row["observed_operation_deltas"]
                ),
                "operation_phase_ownership": "transaction_non_anchor_only",
                "system_operation_ownership": "block_base",
                "anchor_operation_ownership": "block_base",
            }
        )
        backend_input_sha256 = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(
                {
                    "case": row["case"],
                    "lane": row["lane"],
                    "target_count": row["target_count"],
                }
            )
        )
        workload_spec = {
            "schema_version": 2,
            "overhead_key_id": row["overhead_key_id"],
            "case_id": row["case"],
            "lane": row["lane"],
            "target_count": row["target_count"],
            "guest_input_canonical_sha256": backend_input_sha256,
            "expected_operation_deltas": row["expected_operation_deltas"],
            "expected_feature_deltas": row["expected_feature_deltas"],
            "operation_phase_ownership": "transaction_non_anchor_only",
            "system_operation_ownership": "block_base",
            "anchor_operation_ownership": "block_base",
        }
        if row.get("baseline_kind") is not None:
            workload_spec["baseline_kind"] = row["baseline_kind"]
        workload_id = opcode_gas.controlled_workload_id(workload_spec)
        row.update(
            {
                "calibration_run_id": calibration_id,
                "workload_spec": workload_spec,
                "workload_id": workload_id,
                "backend_input_sha256": backend_input_sha256,
                "guest_input_sha256": "0x" + backend_input_sha256,
                "execution_row_id": opcode_gas.controlled_execution_row_id(
                    workload_id,
                    backend="sp1",
                    execution_engine="gas-estimator",
                    run_id=calibration_id,
                    repeat_index=row["repeat_index"],
                    backend_input_sha256=backend_input_sha256,
                ),
            }
        )

    def evaluate(self, rows, bound):
        return opcode_gas.evaluate_higher_layer_fixed_round(
            self.manifest,
            self.coverage,
            self.core,
            rows,
            bound,
            calibration_id=CALIBRATION_ID,
        )

    def test_exactly_recovers_known_costs_and_accepts_round_128(self):
        result = self.evaluate(self.rows(128), 128)

        self.assertEqual(result["status"], "accepted")
        self.assertEqual(result["decision"], "accepted")
        self.assertEqual(
            result["fixed_costs"],
            {
                "proposal_startup": "1000",
                "block_base": "2000",
                "tx_base": "300",
                "native_value_transfer": "40",
            },
        )
        self.assertEqual(result["root_rejection_reasons"], [])
        self.assertEqual(
            opcode_gas.canonical_json(result), opcode_gas.canonical_json(result)
        )

    def test_round_8_expands_only_for_exhausted_sweep(self):
        result = self.evaluate(self.rows(8), 8)

        self.assertEqual(result["decision"], "expand_next_round")
        self.assertEqual(result["root_rejection_reasons"], ["exhausted_sweep"])
        self.assertNotIn("unmeasured_overhead_dependency", result["root_rejection_reasons"])

    def test_checkpoint_bound_is_an_expandable_root_reason(self):
        real = opcode_gas.evaluate_controlled_sweep

        def checkpoint_once(observations, **kwargs):
            points = list(observations)
            if points and Decimal(points[-1]["prover_gas_repeats"][0]) < 10000:
                return {
                    "status": "rejected",
                    "selected_counts": [0, 1, 2, 4],
                    "reasons": ["checkpoint_generator_bound"],
                }
            return real(points, **kwargs)

        with mock.patch.object(
            opcode_gas, "evaluate_controlled_sweep", side_effect=checkpoint_once
        ):
            result = self.evaluate(self.rows(8), 8)
        self.assertEqual(result["decision"], "expand_next_round")
        self.assertIn("checkpoint_generator_bound", result["root_rejection_reasons"])

    def test_nonexpandable_failure_propagates_and_terminates(self):
        result = self.evaluate(
            self.rows(
                8,
                corrupt_case="tx_base_no_code_no_value",
                corrupt_mode="repeat_noise",
            ),
            8,
        )

        self.assertEqual(result["decision"], "terminal_failure")
        self.assertIn("repeat_noise_p", result["root_rejection_reasons"])
        native = result["overhead_results"]["native_value_transfer"]
        self.assertEqual(native["status"], "required_case_incomplete")
        self.assertIn("repeat_noise_p", native["root_rejection_reasons"])
        self.assertNotIn(
            "unmeasured_overhead_dependency", result["root_rejection_reasons"]
        )

    def test_expandable_failure_is_terminal_at_round_128(self):
        with mock.patch.object(
            opcode_gas,
            "evaluate_controlled_sweep",
            return_value={
                "status": "rejected",
                "reasons": ["exhausted_sweep"],
            },
        ):
            result = self.evaluate(self.rows(128), 128)
        self.assertEqual(result["decision"], "terminal_failure")
        self.assertEqual(result["root_rejection_reasons"], ["exhausted_sweep"])

    def test_rejects_noncanonical_generator_bound(self):
        with self.assertRaisesRegex(ValueError, "generator bound"):
            self.evaluate([], 512)

    def test_rejects_standard_engine_report(self):
        rows = self.rows(128)
        for row in rows:
            row["sp1_execution_engine"] = "standard"
            row["sp1_gas_trace_chunk_threshold"] = None
            row["sp1_gas_trace_chunk_slots"] = None

        result = self.evaluate(rows, 128)

        self.assertEqual(result["decision"], "terminal_failure")
        self.assertTrue(
            any("gas-estimator" in reason for reason in result["root_rejection_reasons"])
        )

    def test_rejects_foreign_or_mutated_raw_row_identity(self):
        def replace_backend_identity(row):
            backend_input_sha256 = "0" * 64
            row["backend_input_sha256"] = backend_input_sha256
            row["guest_input_sha256"] = "0x" + backend_input_sha256
            row["execution_row_id"] = opcode_gas.controlled_execution_row_id(
                row["workload_id"],
                backend="sp1",
                execution_engine="gas-estimator",
                run_id=CALIBRATION_ID,
                repeat_index=row["repeat_index"],
                backend_input_sha256=backend_input_sha256,
            )

        mutations = {
            "calibration": lambda row: row.__setitem__(
                "calibration_run_id", "2" * 24
            ),
            "execution row": lambda row: row.__setitem__(
                "execution_row_id", "0" * 64
            ),
            "workload ID": lambda row: row.__setitem__("workload_id", "0" * 64),
            "workload spec": lambda row: row["workload_spec"].__setitem__(
                "target_count", row["target_count"] + 1
            ),
            "repeat": lambda row: row.__setitem__("repeat_index", 7),
            "backend input": lambda row: row.__setitem__(
                "backend_input_sha256", "0" * 64
            ),
            "workload spec guest input": replace_backend_identity,
        }
        for expected, mutate in mutations.items():
            rows = self.rows(128)
            mutate(rows[0])
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                self.evaluate(rows, 128)

    def test_rejected_preexecution_row_is_bound_to_current_calibration(self):
        rejected = {
            "case": "block_base_one_vs_two_minimal_blocks",
            "overhead_key_id": "block_base",
            "lane": "target",
            "target_count": 8,
            "generator_max_count": 8,
            "repeat_index": 0,
            "status": "rejected",
            "calibration_run_id": "2" * 24,
            "sp1_execution_engine": "gas-estimator",
            "sp1_gas_trace_chunk_threshold": 134_217_728,
            "sp1_gas_trace_chunk_slots": 2,
            "reasons": ["generation_failure"],
        }

        with self.assertRaisesRegex(ValueError, "calibration"):
            self.evaluate([rejected], 8)


class HigherLayerCampaignInterfaceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        cls.coverage = json.loads(
            (ROOT / cls.manifest.operation_coverage_ref["path"]).read_text()
        )
        cls.core = json.loads(
            (ROOT / cls.manifest.augmented_core_ref["path"]).read_text()
        )

    def test_parser_exposes_only_bounded_campaign_commands(self):
        parser = opcode_gas.build_parser()
        prepared = parser.parse_args(
            [
                "prepare-higher-layer-calibration",
                "--manifest",
                str(MANIFEST_PATH.relative_to(ROOT)),
                "--operation-coverage",
                "experiments/opcode-gas/manifests/operation-coverage-v1.json",
                "--augmented-core",
                "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json",
                "--guest-launcher",
                "target/release/guest-launcher",
                "--elf",
                "crates/guests/elf/sp1_shasta_proposal.elf",
                "--out",
                "experiments/opcode-gas/runs",
                "--run-path-file",
                "higher-layer-run-path",
            ]
        )
        running = parser.parse_args(
            ["run-higher-layer-calibration", "--run", "experiments/opcode-gas/runs/id"]
        )

        self.assertIs(prepared.func, opcode_gas.cmd_prepare_higher_layer_calibration)
        self.assertIs(running.func, opcode_gas.cmd_run_higher_layer_calibration)

    def test_create_only_artifact_accepts_same_bytes_and_rejects_conflict(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "round.json"
            opcode_gas.persist_immutable_bytes(path, b"one\n")
            opcode_gas.persist_immutable_bytes(path, b"one\n")
            with self.assertRaisesRegex(ValueError, "different bytes"):
                opcode_gas.persist_immutable_bytes(path, b"two\n")

    def test_prepare_rejects_dirty_implementation_and_noncanonical_inputs(self):
        canonical = {
            "manifest_path": MANIFEST_PATH,
            "operation_coverage_path": ROOT
            / opcode_gas._HIGHER_LAYER_COVERAGE_PATH,
            "augmented_core_path": ROOT / opcode_gas._HIGHER_LAYER_CORE_PATH,
            "guest_launcher": ROOT / opcode_gas._HIGHER_LAYER_LAUNCHER_PATH,
            "elf": ROOT / opcode_gas._HIGHER_LAYER_ELF_PATH,
            "output_root": ROOT / opcode_gas._HIGHER_LAYER_RUN_ROOT,
        }
        with mock.patch.object(
            opcode_gas,
            "git_worktree_status",
            return_value=" M experiments/opcode-gas/opcode_gas.py\n",
        ), self.assertRaisesRegex(ValueError, "dirty implementation path"):
            opcode_gas.prepare_higher_layer_calibration(**canonical)

        wrongs = {
            "manifest_path": ROOT / "README.md",
            "operation_coverage_path": MANIFEST_PATH,
            "augmented_core_path": MANIFEST_PATH,
            "guest_launcher": ROOT / "Cargo.toml",
            "elf": ROOT / "Cargo.toml",
            "output_root": ROOT / "experiments/opcode-gas/other-runs",
        }
        for field, wrong in wrongs.items():
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, "not canonical"
            ):
                opcode_gas.prepare_higher_layer_calibration(
                    **{**canonical, field: wrong}
                )

    def test_identity_rejects_wrong_pinned_file_hash(self):
        coverage_path = ROOT / opcode_gas._HIGHER_LAYER_COVERAGE_PATH
        real_sha256_file = opcode_gas.sha256_file

        def changed_hash(path):
            if pathlib.Path(path).resolve() == coverage_path.resolve():
                return "0" * 64
            return real_sha256_file(path)

        with mock.patch.object(
            opcode_gas, "sha256_file", side_effect=changed_hash
        ), self.assertRaisesRegex(ValueError, "operation coverage file SHA256"):
            opcode_gas._higher_layer_identity_payload(
                implementation_revision="1" * 40,
                manifest_path=MANIFEST_PATH,
                operation_coverage_path=coverage_path,
                augmented_core_path=ROOT / opcode_gas._HIGHER_LAYER_CORE_PATH,
                guest_launcher=ROOT / opcode_gas._HIGHER_LAYER_LAUNCHER_PATH,
                elf=ROOT / opcode_gas._HIGHER_LAYER_ELF_PATH,
            )

    def test_identity_requires_production_proposal_gas_estimator(self):
        parameters = opcode_gas.higher_layer_sp1_execution_parameters()

        self.assertEqual(parameters["mode"], "execute")
        self.assertEqual(parameters["prover"], "local")
        self.assertEqual(parameters["primary_api"], "ExecutionReport::gas")
        self.assertEqual(parameters["engines"]["overhead"], "gas-estimator")
        self.assertEqual(
            parameters["gas_estimator"],
            {
                "gas_trace_chunk_threshold": 134_217_728,
                "gas_trace_chunk_slots": 2,
            },
        )

    def test_load_identity_rejects_symlinked_artifact(self):
        payload = {"implementation_revision": "1" * 40}
        identity_sha256 = opcode_gas.sha256_bytes(opcode_gas.canonical_json(payload))
        document = {
            "schema_version": opcode_gas._HIGHER_LAYER_IDENTITY_SCHEMA_VERSION,
            "calibration_id": identity_sha256[:24],
            "identity_sha256": identity_sha256,
            "identity": payload,
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / identity_sha256[:24]
            run.mkdir()
            external = root / "external-identity.json"
            external.write_bytes(opcode_gas._canonical_json_file_bytes(document))
            (run / "identity.json").symlink_to(external)

            with mock.patch.object(
                opcode_gas, "_HIGHER_LAYER_RUN_ROOT", root
            ), mock.patch.object(
                opcode_gas, "_resolve_repo_path", return_value=run
            ), self.assertRaisesRegex(ValueError, "non-symlink"):
                opcode_gas._load_higher_layer_identity(run)

    def test_higher_layer_executor_command_selects_gas_estimator(self):
        def inspect_command(command, **_kwargs):
            engine_index = command.index("--sp1-execution-engine")
            self.assertEqual(command[engine_index + 1], "gas-estimator")
            self.assertEqual(command[command.index("--mode") + 1], "execute")
            self.assertEqual(command[command.index("--sp1-prover") + 1], "local")
            raise RuntimeError("command inspected")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            opcode_gas.subprocess, "run", side_effect=inspect_command
        ), self.assertRaisesRegex(RuntimeError, "command inspected"):
            opcode_gas.run_controlled_overhead_round(
                guest_launcher=pathlib.Path("guest-launcher"),
                calibration_run_id="calibration",
                generator_max_count=8,
                include_startup=True,
                out=pathlib.Path(temporary) / "round.jsonl",
                execution_engine="gas-estimator",
            )

    def test_run_reuses_executor_for_8_32_128_and_never_writes_holdouts(self):
        fixture = HigherLayerFixedRoundTests()
        fixture.manifest = self.manifest
        fixture.coverage = self.coverage
        fixture.core = self.core
        bounds = []

        def fake_executor(**kwargs):
            bound = kwargs["generator_max_count"]
            self.assertEqual(kwargs["execution_engine"], "gas-estimator")
            bounds.append(bound)
            rows = fixture.rows(bound)
            kwargs["out"].write_bytes(
                b"".join(opcode_gas.canonical_json(row) + b"\n" for row in rows)
            )

        run_root = ROOT / opcode_gas._HIGHER_LAYER_RUN_ROOT
        run_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=run_root) as temporary:
            run = pathlib.Path(temporary)
            identity = {
                "calibration_id": CALIBRATION_ID,
                "identity_sha256": "1" * 64,
            }
            with mock.patch.object(
                opcode_gas,
                "_validate_current_higher_layer_identity",
                return_value=(identity, self.manifest, self.coverage, self.core),
            ), mock.patch.object(
                opcode_gas,
                "_load_higher_layer_decisions",
                return_value=(
                    {
                        "schema_version": 1,
                        "identity_sha256": identity["identity_sha256"],
                        "rounds": [],
                    },
                    [],
                ),
            ):
                result = opcode_gas.run_higher_layer_calibration(
                    run, executor=fake_executor
                )

            self.assertEqual(bounds, [8, 32, 128])
            self.assertEqual(result["decision"], "accepted")
            self.assertFalse(any("state" in path.name for path in run.rglob("*")))

    def test_missing_prior_decision_rejects_raw_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = pathlib.Path(temporary)
            (run / "raw").mkdir()
            with self.assertRaisesRegex(ValueError, "missing prior round decisions"):
                opcode_gas._load_higher_layer_decisions(
                    run,
                    self.manifest,
                    self.coverage,
                    self.core,
                    "b" * 64,
                )

    def test_load_decisions_rejects_symlinked_ledger_seal_and_parent(self):
        decisions = {
            "schema_version": opcode_gas._HIGHER_LAYER_DECISIONS_SCHEMA_VERSION,
            "identity_sha256": "1" * 64,
            "rounds": [],
        }
        decision_bytes = opcode_gas._higher_layer_decisions_bytes(decisions)
        seal_bytes = (opcode_gas.sha256_bytes(decision_bytes) + "\n").encode()
        for symlink_name in (
            "overhead-decisions.json",
            "overhead-decisions.sha256",
            "parent",
        ):
            with self.subTest(
                symlink_name=symlink_name
            ), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                external = root / "external"
                external.mkdir()
                (external / "overhead-decisions.json").write_bytes(decision_bytes)
                (external / "overhead-decisions.sha256").write_bytes(seal_bytes)
                if symlink_name == "parent":
                    run = root / "run"
                    run.symlink_to(external, target_is_directory=True)
                else:
                    run = root / "run"
                    run.mkdir()
                    other_name = (
                        "overhead-decisions.sha256"
                        if symlink_name == "overhead-decisions.json"
                        else "overhead-decisions.json"
                    )
                    (run / other_name).write_bytes(
                        seal_bytes
                        if other_name.endswith(".sha256")
                        else decision_bytes
                    )
                    (run / symlink_name).symlink_to(external / symlink_name)

                with self.assertRaisesRegex(ValueError, "non-symlink"):
                    opcode_gas._load_higher_layer_decisions(
                        run,
                        self.manifest,
                        self.coverage,
                        self.core,
                        "1" * 64,
                    )

    def round_artifacts(self, destination, *, row_calibration_id=CALIBRATION_ID):
        fixture = HigherLayerFixedRoundTests()
        fixture.manifest = self.manifest
        fixture.coverage = self.coverage
        fixture.core = self.core
        rows = fixture.rows(8)
        if row_calibration_id != CALIBRATION_ID:
            for row in rows:
                fixture.bind_identity(row, calibration_id=row_calibration_id)
        fit = opcode_gas.evaluate_higher_layer_fixed_round(
            self.manifest,
            self.coverage,
            self.core,
            rows,
            8,
            calibration_id=row_calibration_id,
        )
        raw_bytes = b"".join(
            opcode_gas.canonical_json(row) + b"\n" for row in rows
        )
        fit_bytes = opcode_gas._canonical_json_file_bytes(fit)
        raw_path = destination / "raw" / "overhead-round-8.jsonl"
        fit_path = destination / "fit" / "overhead-round-8.json"
        raw_path.parent.mkdir(parents=True, exist_ok=True)
        fit_path.parent.mkdir(parents=True, exist_ok=True)
        raw_path.write_bytes(raw_bytes)
        fit_path.write_bytes(fit_bytes)
        return {
            "schema_version": 1,
            "identity_sha256": "1" * 64,
            "rounds": [
                {
                    "generator_max_count": 8,
                    "raw_rows": "raw/overhead-round-8.jsonl",
                    "raw_rows_sha256": opcode_gas.sha256_bytes(raw_bytes),
                    "fit": "fit/overhead-round-8.json",
                    "fit_sha256": opcode_gas.sha256_bytes(fit_bytes),
                    "decision": fit["decision"],
                }
            ],
        }

    def test_persisted_replay_rejects_resealed_foreign_raw_rows(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = pathlib.Path(temporary) / "run"
            run.mkdir()
            decisions = self.round_artifacts(run, row_calibration_id="2" * 24)

            with self.assertRaisesRegex(ValueError, "calibration"):
                opcode_gas.validate_persisted_higher_layer_decisions(
                    run,
                    decisions,
                    self.manifest,
                    self.coverage,
                    self.core,
                    "1" * 64,
                )

    def test_persisted_replay_rejects_symlinked_round_files_and_parents(self):
        for symlink_parent in (False, True):
            with self.subTest(
                symlink_parent=symlink_parent
            ), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                external = root / "external"
                decisions = self.round_artifacts(external)
                run = root / "run"
                run.mkdir()
                if symlink_parent:
                    (run / "raw").symlink_to(external / "raw", target_is_directory=True)
                    (run / "fit").symlink_to(external / "fit", target_is_directory=True)
                else:
                    (run / "raw").mkdir()
                    (run / "fit").mkdir()
                    (run / "raw" / "overhead-round-8.jsonl").symlink_to(
                        external / "raw" / "overhead-round-8.jsonl"
                    )
                    (run / "fit" / "overhead-round-8.json").symlink_to(
                        external / "fit" / "overhead-round-8.json"
                    )

                with self.assertRaisesRegex(ValueError, "non-symlink"):
                    opcode_gas.validate_persisted_higher_layer_decisions(
                        run,
                        decisions,
                        self.manifest,
                        self.coverage,
                        self.core,
                        "1" * 64,
                    )


if __name__ == "__main__":
    unittest.main()
