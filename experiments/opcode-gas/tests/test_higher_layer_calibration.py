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
    / "sp1-higher-layer-v2.json"
)
V1_MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "sp1-higher-layer-v1.json"
)
CALIBRATION_ID = "1" * 24
OPERATION_COVERAGE_V2_REF = {
    "path": "experiments/opcode-gas/manifests/operation-coverage-v2.json",
    "artifact_sha256": (
        "35fd25a7878dd407522ab676c8a9965c663c6885e3190df0fd51f6c497b84e77"
    ),
    "file_sha256": "75fec3c4307c59539cc6180e6511dd7fc1111197746c6b0abd9a872902c665a3",
}


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
        approximation = manifest.native_transfer_approximation
        self.assertEqual(approximation.overhead_key_id, "native_value_transfer")
        self.assertEqual(approximation.method, "frozen_max_observed_per_transfer")
        self.assertEqual(approximation.coefficient_prover_gas, "5017")
        self.assertEqual(approximation.materiality_budget, "0.002")
        self.assertEqual(approximation.required_counts, (1, 2, 4, 8, 16, 32, 64, 128))
        self.assertEqual(
            approximation.source,
            {
                "calibration_id": "999b91b91fd693899d09fa53",
                "identity_sha256": (
                    "999b91b91fd693899d09fa53c0502c19"
                    "e6c43d6f9a0ddcf4c38bd15ef9c0fccd"
                ),
                "raw_rows_sha256": (
                    "297d88799b069765481f9793645da504"
                    "c507b14193677109cf760803d491140e"
                ),
                "fit_sha256": (
                    "fc92696741c1b74503b4863f156936a6"
                    "b3ad00164222058d8db26f14f637b3e5"
                ),
            },
        )
        self.assertEqual(
            json.loads(V1_MANIFEST_PATH.read_text())["artifact_sha256"],
            "f531201bd93f98ba20e51474ae4d64ae1f444bdaad027ea88362a47fb45e9809",
        )
        self.assertEqual(
            dict(manifest.operation_coverage_ref), OPERATION_COVERAGE_V2_REF
        )
        self.assertEqual(
            opcode_gas._HIGHER_LAYER_COVERAGE_PATH,
            pathlib.Path(OPERATION_COVERAGE_V2_REF["path"]),
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

    def test_rejects_resealed_native_transfer_approximation_drift(self):
        mutations = (
            (
                lambda artifact: artifact["native_transfer_approximation"].__setitem__(
                    "coefficient_prover_gas", "5018"
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"].__setitem__(
                    "materiality_budget", "0.003"
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"].__setitem__(
                    "method", "mean_observed_per_transfer"
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"][
                    "required_counts"
                ].append(256),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"]["source"].__setitem__(
                    "calibration_id", "0" * 24
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"]["source"].__setitem__(
                    "identity_sha256", "0" * 64
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"]["source"].__setitem__(
                    "raw_rows_sha256", "0" * 64
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"]["source"].__setitem__(
                    "fit_sha256", "0" * 64
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"].__setitem__(
                    "unexpected", True
                ),
                "unknown fields",
            ),
            (
                lambda artifact: artifact["native_transfer_approximation"].__setitem__(
                    "materiality_budget", "0.0020"
                ),
                "native transfer approximation differs",
            ),
            (
                lambda artifact: artifact.__setitem__("schema_version", 1),
                "higher-layer manifest header differs",
            ),
        )
        for mutate, expected in mutations:
            with self.subTest(mutate=mutate):
                self.assert_rejected(mutate, expected)


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
            "native_value_transfer": Decimal("5017"),
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

        self.assertEqual(result["status"], "accepted_with_declared_approximation")
        self.assertEqual(result["decision"], "accepted")
        self.assertEqual(
            result["fixed_cost_statuses"],
            {
                "proposal_startup": "accepted",
                "block_base": "accepted",
                "tx_base": "accepted",
                "native_value_transfer": "declared_approximation",
            },
        )
        self.assertEqual(
            result["fixed_costs"],
            {
                "proposal_startup": "1000",
                "block_base": "2000",
                "tx_base": "300",
                "native_value_transfer": "5017",
            },
        )
        self.assertEqual(result["root_rejection_reasons"], [])
        self.assertEqual(
            opcode_gas.canonical_json(result), opcode_gas.canonical_json(result)
        )

    def test_native_approximation_uses_target_total_materiality_and_all_frozen_counts(self):
        result = self.evaluate(self.rows(128), 128)
        evidence = result["overhead_results"]["native_value_transfer"]
        self.assertEqual(evidence["status"], "declared_approximation")
        self.assertEqual(evidence["o_p"], "5017")
        self.assertEqual(evidence["materiality_budget"], "0.002")
        self.assertEqual(
            [point["count"] for point in evidence["count_evidence"]],
            [1, 2, 4, 8, 16, 32, 64, 128],
        )
        self.assertTrue(
            all(
                point["materiality"] == "0"
                for point in evidence["count_evidence"]
            )
        )

    def test_native_policy_expands_until_all_frozen_counts_exist(self):
        for bound in (8, 32):
            result = self.evaluate(self.rows(bound), bound)
            self.assertEqual(result["decision"], "expand_next_round")
            self.assertIn(
                "native_approximation_requires_bound_128",
                result["root_rejection_reasons"],
            )

    def test_zero_delta_dependency_does_not_block_block_or_startup(self):
        rows = self.rows(128)
        for row in rows:
            if (
                row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "target"
            ):
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) - 4977 * row["target_count"]
                )
        result = self.evaluate(rows, 128)
        self.assertEqual(
            result["overhead_results"]["native_value_transfer"]["status"],
            "rejected",
        )
        self.assertEqual(result["overhead_results"]["block_base"]["status"], "accepted")
        self.assertEqual(
            result["overhead_results"]["proposal_startup"]["status"], "accepted"
        )

    def test_nonzero_dependency_blocks_block_when_native_is_rejected(self):
        rows = self.rows(128)
        for row in rows:
            if (
                row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "target"
            ):
                row["prover_gas"] = str(
                    Decimal(row["prover_gas"]) - 4977 * row["target_count"]
                )
            if (
                row["case"] == "block_base_one_vs_two_minimal_blocks"
                and row["lane"] == "target"
            ):
                row["expected_feature_deltas"]["native_value_transfer"] = row[
                    "target_count"
                ]
                self.bind_identity(row)

        result = self.evaluate(rows, 128)
        native = result["overhead_results"]["native_value_transfer"]
        block = next(
            item
            for item in result["case_results"]
            if item["case_id"] == "block_base_one_vs_two_minimal_blocks"
        )
        self.assertEqual(native["status"], "rejected")
        self.assertEqual(block["status"], "rejected")
        self.assertEqual(block["reasons"], ["unmeasured_overhead_dependency"])
        self.assertEqual(block["dependency_ids"], ["native_value_transfer"])

    def test_native_approximation_rejects_invalid_measurement_evidence(self):
        def mutate_nonpositive_delta(rows):
            for row in rows:
                if (
                    row["case"] == "native_transfer_positive_vs_zero"
                    and row["lane"] == "target"
                    and row["target_count"] == 1
                ):
                    row["prover_gas"] = "10000"

        def mutate_target_repeat(rows):
            row = next(
                row
                for row in rows
                if row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "target"
                and row["target_count"] == 1
                and row["repeat_index"] == 2
            )
            row["prover_gas"] = str(Decimal(row["prover_gas"]) + 1)

        def mutate_control_repeat(rows):
            row = next(
                row
                for row in rows
                if row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "control"
                and row["target_count"] == 1
                and row["repeat_index"] == 2
            )
            row["prover_gas"] = "10001"

        def mutate_baseline_delta(rows):
            for row in rows:
                if (
                    row["case"] == "native_transfer_positive_vs_zero"
                    and row["lane"] == "target"
                    and row["target_count"] == 0
                ):
                    row["prover_gas"] = "10001"

        def mutate_target_total_zero(rows):
            for row in rows:
                if (
                    row["case"] == "native_transfer_positive_vs_zero"
                    and row["lane"] == "target"
                    and row["target_count"] == 1
                ):
                    row["prover_gas"] = "0"

        def mutate_unexpected_operation(rows):
            for row in rows:
                if (
                    row["case"] == "native_transfer_positive_vs_zero"
                    and row["lane"] == "target"
                    and row["target_count"] == 1
                ):
                    row["observed_operation_deltas"] = {
                        "opcode:0x5f": {
                            "pricing_basis": "raw_gas_slope",
                            "units": 1,
                            "event_count": 1,
                        }
                    }
                    self.bind_identity(row)

        mutations = {
            "nonpositive delta": mutate_nonpositive_delta,
            "target repeat": mutate_target_repeat,
            "control repeat": mutate_control_repeat,
            "baseline delta": mutate_baseline_delta,
            "target total zero": mutate_target_total_zero,
            "unexpected native operation deltas": mutate_unexpected_operation,
        }
        for label, mutate in mutations.items():
            rows = self.rows(128)
            mutate(rows)
            with self.subTest(label=label):
                result = self.evaluate(rows, 128)
                self.assertEqual(
                    result["overhead_results"]["native_value_transfer"]["status"],
                    "rejected",
                )

    def test_native_approximation_rejects_materiality_strictly_above_budget(self):
        rows = self.rows(128)
        for row in rows:
            if (
                row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "target"
                and row["target_count"] == 1
            ):
                row["prover_gas"] = "14986"

        result = self.evaluate(rows, 128)
        evidence = result["overhead_results"]["native_value_transfer"]
        self.assertEqual(evidence["status"], "rejected")
        self.assertEqual(evidence["reasons"], ["native_approximation_materiality"])
        first = evidence["count_evidence"][0]
        self.assertEqual(first["absolute_error"], "31")
        self.assertEqual(
            first["materiality"],
            "0.0020685973575336981182436941145068730815427732550380354997998131589483517950086748",
        )

    def test_native_approximation_rejects_missing_count_and_changed_lane_inventory(self):
        def remove_count(rows):
            rows[:] = [
                row
                for row in rows
                if not (
                    row["case"] == "native_transfer_positive_vs_zero"
                    and row["target_count"] == 128
                )
            ]

        def change_lane(rows):
            row = next(
                row
                for row in rows
                if row["case"] == "native_transfer_positive_vs_zero"
                and row["lane"] == "target"
                and row["target_count"] == 128
                and row["repeat_index"] == 0
            )
            row["lane"] = "control"
            self.bind_identity(row)

        for label, mutate in {
            "missing count": remove_count,
            "changed lane": change_lane,
        }.items():
            rows = self.rows(128)
            mutate(rows)
            with self.subTest(label=label), self.assertRaisesRegex(
                ValueError, "inventory"
            ):
                self.evaluate(rows, 128)

    def test_round_8_expands_for_incomplete_native_approximation(self):
        result = self.evaluate(self.rows(8), 8)

        self.assertEqual(result["decision"], "expand_next_round")
        self.assertEqual(
            result["root_rejection_reasons"],
            ["native_approximation_requires_bound_128"],
        )
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
        self.assertEqual(native["status"], "pending_required_counts")
        self.assertEqual(
            native["reasons"], ["native_approximation_requires_bound_128"]
        )
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

        with self.assertRaisesRegex(ValueError, "gas-estimator"):
            self.evaluate(rows, 128)

    def test_rejects_unused_or_incomplete_row_inventory(self):
        def bogus_lane(rows):
            row = copy.deepcopy(rows[0])
            row["lane"] = "ignored"
            self.bind_identity(row)
            rows.append(row)

        def startup_control(rows):
            row = copy.deepcopy(
                next(
                    row
                    for row in rows
                    if row["case"] == "startup_minimal_no_candidate_tx"
                )
            )
            row["lane"] = "control"
            self.bind_identity(row)
            rows.append(row)

        mutations = {
            "bogus lane": bogus_lane,
            "startup control": startup_control,
            "duplicate": lambda rows: rows.append(copy.deepcopy(rows[0])),
            "missing": lambda rows: rows.pop(0),
        }
        for label, mutate in mutations.items():
            rows = self.rows(128)
            mutate(rows)
            with self.subTest(label=label), self.assertRaisesRegex(
                ValueError, "inventory"
            ):
                self.evaluate(rows, 128)

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
                "experiments/opcode-gas/manifests/operation-coverage-v2.json",
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
        self.assertEqual(
            prepared.operation_coverage,
            pathlib.Path(OPERATION_COVERAGE_V2_REF["path"]),
        )
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


class HigherLayerTask5FixedCostTests(unittest.TestCase):
    fixed_cost_statuses = {
        "proposal_startup": "accepted",
        "block_base": "accepted",
        "tx_base": "accepted",
        "native_value_transfer": "declared_approximation",
    }

    @classmethod
    def setUpClass(cls):
        cls.manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        cls.coverage = json.loads(
            (ROOT / cls.manifest.operation_coverage_ref["path"]).read_text()
        )
        cls.core = json.loads(
            (ROOT / cls.manifest.augmented_core_ref["path"]).read_text()
        )

    def accepted_rounds(self):
        fixture = HigherLayerFixedRoundTests()
        fixture.manifest = self.manifest
        fixture.coverage = self.coverage
        fixture.core = self.core
        validated = []
        rounds = []
        for bound in self.manifest.generator_rounds:
            fit = fixture.evaluate(fixture.rows(bound), bound)
            record = {
                "generator_max_count": bound,
                "raw_rows": f"raw/overhead-round-{bound}.jsonl",
                "raw_rows_sha256": str(bound) * 64,
                "fit": f"fit/overhead-round-{bound}.json",
                "fit_sha256": str(bound + 1) * 64,
                "decision": fit["decision"],
            }
            rounds.append(record)
            validated.append({**record, "fit_payload": fit})
        decisions = {
            "schema_version": 1,
            "identity_sha256": "1" * 64,
            "rounds": rounds,
        }
        identity = {
            "schema_version": 1,
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
            "identity": {},
        }
        return identity, decisions, validated

    def fit(self, run, *, validated_mutator=None):
        identity, decisions, validated = self.accepted_rounds()
        if validated_mutator is not None:
            validated_mutator(validated)
        with mock.patch.object(
            opcode_gas,
            "_validate_current_higher_layer_identity",
            return_value=(
                identity,
                self.manifest,
                self.coverage,
                self.core,
            ),
        ), mock.patch.object(
            opcode_gas,
            "_load_higher_layer_decisions",
            return_value=(decisions, validated),
        ):
            return opcode_gas.fit_higher_layer_fixed_costs(run)

    def test_fit_replays_first_accepted_round_and_seals_exact_fixed_costs(self):
        with tempfile.TemporaryDirectory() as temporary:
            run = pathlib.Path(temporary)
            result = self.fit(run)

            self.assertEqual(
                result["status"], "accepted_with_declared_approximation"
            )
            self.assertEqual(result["selected_round"], 128)
            self.assertEqual(result["fixed_cost_rank"], 4)
            self.assertEqual(
                result["fixed_costs"],
                {
                    "proposal_startup": "1000",
                    "block_base": "2000",
                    "tx_base": "300",
                    "native_value_transfer": "5017",
                },
            )
            self.assertEqual(
                result["fixed_cost_statuses"], self.fixed_cost_statuses
            )
            fixed_path = run / "fixed-costs.json"
            self.assertEqual(
                fixed_path.read_bytes(), opcode_gas._canonical_json_file_bytes(result)
            )

    def test_fit_rejects_ordinary_accepted_or_nonaccepted_terminal_and_conflict(self):
        def ordinary_accepted(validated):
            validated[-1]["fit_payload"] = {
                **validated[-1]["fit_payload"],
                "status": "accepted",
            }

        def terminal_failure(validated):
            validated[-1]["decision"] = "terminal_failure"
            validated[-1]["fit_payload"] = {
                **validated[-1]["fit_payload"],
                "decision": "terminal_failure",
                "status": "rejected",
                "fixed_costs": {},
            }

        for setup, expected in (
            (ordinary_accepted, "accepted fixed costs"),
            (terminal_failure, "terminal accepted"),
            (None, "different bytes"),
        ):
            with self.subTest(expected=expected), tempfile.TemporaryDirectory() as temporary:
                run = pathlib.Path(temporary)
                if setup is None:
                    (run / "fixed-costs.json").write_text("{}\n")
                with self.assertRaisesRegex(ValueError, expected):
                    self.fit(run, validated_mutator=setup)

    def test_fixed_cost_values_rejects_inexact_status_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixed = self.fit(pathlib.Path(temporary))

        mutations = {
            "missing": lambda statuses: (
                statuses.pop("native_value_transfer"),
                None,
            )[1],
            "changed": lambda statuses: statuses.update(
                {"unexpected": "declared_approximation"}
            ),
            "reordered": lambda statuses: list(reversed(statuses.items())),
            "relabeled": lambda statuses: statuses.__setitem__(
                "native_value_transfer", "accepted"
            ),
        }
        for label, mutate in mutations.items():
            artifact = copy.deepcopy(fixed)
            statuses = artifact["fixed_cost_statuses"]
            replacement = mutate(statuses)
            if replacement is not None:
                artifact["fixed_cost_statuses"] = dict(replacement)
            with self.subTest(label=label), self.assertRaisesRegex(
                ValueError, "fixed-cost artifact"
            ):
                opcode_gas._higher_layer_fixed_cost_values(self.manifest, artifact)


class HigherLayerTask5StateVerdictTests(unittest.TestCase):
    fixed_cost_statuses = {
        "proposal_startup": "accepted",
        "block_base": "accepted",
        "tx_base": "accepted",
        "native_value_transfer": "declared_approximation",
    }

    @classmethod
    def setUpClass(cls):
        cls.manifest = opcode_gas.load_higher_layer_manifest(MANIFEST_PATH)
        cls.coverage = json.loads(
            (ROOT / cls.manifest.operation_coverage_ref["path"]).read_text()
        )
        cls.core = json.loads(
            (ROOT / cls.manifest.augmented_core_ref["path"]).read_text()
        )
        cls.fixed_costs = {
            "proposal_startup": "1000",
            "block_base": "2000",
            "tx_base": "300",
            "native_value_transfer": "5017",
        }
        cls.fixed = {
            "schema_version": 2,
            "status": "accepted_with_declared_approximation",
            "identity_sha256": "1" * 64,
            "calibration_id": CALIBRATION_ID,
            "selected_round": 128,
            "fixed_cost_rank": 4,
            "fixed_costs": cls.fixed_costs,
            "fixed_cost_statuses": cls.fixed_cost_statuses,
            "fixed_costs_sha256": opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(cls.fixed_costs)
            ),
            "source": {},
            "fit": {},
        }

    @staticmethod
    def pair_spec(pair):
        return {
            "kind": pair.kind,
            "pair_id": pair.pair_id,
            "scale": pair.scale,
            "control": dict(pair.control),
            "target": dict(pair.target),
        }

    def rows(self):
        rows = []
        for pair_index, pair in enumerate(self.manifest.state_holdouts, start=1):
            tx_count = pair.scale if pair.kind == "dirty_accounts" else 0
            features = {
                "proposal_startup": 1,
                "block_base": 1,
                "tx_base": tx_count,
                "native_value_transfer": tx_count,
            }
            observed_gas = 3000 + 5317 * tx_count
            spec = self.pair_spec(pair)
            for lane_index, lane in enumerate(("control", "target")):
                backend_input_sha256 = f"{pair_index * 2 + lane_index:064x}"
                workload_spec = {
                    "schema_version": 1,
                    "pair_id": pair.pair_id,
                    "lane": lane,
                    "pair_spec": spec,
                    "guest_input_canonical_sha256": backend_input_sha256,
                }
                workload_id = opcode_gas.controlled_workload_id(workload_spec)
                observation = {
                    "pair_id": pair.pair_id,
                    "lane": lane,
                    "backend_input_sha256": backend_input_sha256,
                    "guest_input_sha256": "0x" + backend_input_sha256,
                    "guest_input_bincode_length": 1000 + pair_index,
                    "public_output": "0x" + f"{pair_index * 2 + lane_index + 20:064x}",
                    "actual_final_state_root": "0x"
                    + f"{pair_index * 2 + lane_index + 40:064x}",
                    "actual_raw_gas_by_key": {},
                    "actual_features": features,
                    "actual_diagnostics": {
                        "guest_input_bincode_length": 1000 + pair_index,
                        "witness_node_count": 10 + lane_index,
                        "witness_byte_count": 100 + lane_index,
                        "blob_count": 0,
                        "kzg_invocation_count": 0,
                        "calldata_length": 0,
                        "bytecode_length": 0,
                        "touched_state_key_count": tx_count + lane_index,
                    },
                    "started_candidate_transaction_count": tx_count,
                    "committed_candidate_transaction_count": tx_count,
                    "unattempted_candidate_transaction_count": 0,
                    "operation_phase_ownership": "transaction_non_anchor_only",
                    "system_operation_ownership": "block_base",
                    "anchor_operation_ownership": "block_base",
                }
                for repeat_index in range(3):
                    rows.append(
                        {
                            "schema_version": 1,
                            "calibration_run_id": CALIBRATION_ID,
                            "pair_id": pair.pair_id,
                            "lane": lane,
                            "repeat_index": repeat_index,
                            "status": "accepted",
                            "spec": spec,
                            "workload_spec": workload_spec,
                            "workload_id": workload_id,
                            "backend_input_sha256": backend_input_sha256,
                            "guest_input_sha256": "0x" + backend_input_sha256,
                            "guest_input_bincode_length": 1000 + pair_index,
                            "execution_row_id": opcode_gas.controlled_execution_row_id(
                                workload_id,
                                backend="sp1",
                                execution_engine="gas-estimator",
                                run_id=CALIBRATION_ID,
                                repeat_index=repeat_index,
                                backend_input_sha256=backend_input_sha256,
                            ),
                            "prover_gas": str(observed_gas),
                            "public_values": observation["public_output"],
                            "sp1_execution_engine": "gas-estimator",
                            "sp1_gas_trace_chunk_threshold": 134_217_728,
                            "sp1_gas_trace_chunk_slots": 2,
                            "observation": copy.deepcopy(observation),
                        }
                    )
        return rows

    def evaluate(self, rows):
        return opcode_gas.evaluate_higher_layer_state_holdouts(
            self.manifest,
            self.coverage,
            self.core,
            self.fixed,
            rows,
        )

    def verified_bundle(self):
        fixture = HigherLayerFixedRoundTests()
        fixture.manifest = self.manifest
        fixture.coverage = self.coverage
        fixture.core = self.core
        rounds = []
        evidence = []
        for bound in self.manifest.generator_rounds:
            rows = fixture.rows(bound)
            fit = fixture.evaluate(rows, bound)
            raw_bytes = b"".join(
                opcode_gas.canonical_json(row) + b"\n" for row in rows
            )
            fit_bytes = opcode_gas._canonical_json_file_bytes(fit)
            record = {
                "generator_max_count": bound,
                "raw_rows": f"raw/overhead-round-{bound}.jsonl",
                "raw_rows_sha256": opcode_gas.sha256_bytes(raw_bytes),
                "fit": f"fit/overhead-round-{bound}.json",
                "fit_sha256": opcode_gas.sha256_bytes(fit_bytes),
                "decision": fit["decision"],
            }
            rounds.append(record)
            evidence.append({"record": record, "rows": rows, "fit": fit})
        decisions = {
            "schema_version": 1,
            "identity_sha256": "1" * 64,
            "rounds": rounds,
        }
        fixed = {
            **self.fixed,
            "source": {
                "decision_ledger_sha256": opcode_gas.sha256_bytes(
                    opcode_gas._higher_layer_decisions_bytes(decisions)
                ),
                "selected_fit_sha256": rounds[-1]["fit_sha256"],
                "higher_layer_manifest_artifact_sha256": self.manifest.artifact_sha256,
                "operation_coverage_artifact_sha256": self.manifest.operation_coverage_ref[
                    "artifact_sha256"
                ],
                "augmented_core_artifact_sha256": self.manifest.augmented_core_ref[
                    "artifact_sha256"
                ],
            },
            "fit": evidence[-1]["fit"],
        }
        state_rows = self.rows()
        return {
            "identity": {
                "schema_version": 1,
                "calibration_id": CALIBRATION_ID,
                "identity_sha256": "1" * 64,
                "identity": {
                    "guest_launcher": {
                        "path": "target/release/guest-launcher",
                        "file_sha256": "a" * 64,
                    }
                },
            },
            "decisions": decisions,
            "round_evidence": evidence,
            "fixed": fixed,
            "state_rows": state_rows,
            "final": opcode_gas.evaluate_higher_layer_state_holdouts(
                self.manifest,
                self.coverage,
                self.core,
                fixed,
                state_rows,
            ),
        }

    def trace_executor(self, rows):
        def execute(**kwargs):
            pair_id = kwargs["pair_spec"]["pair_id"]
            return [
                {
                    "schema_version": 1,
                    "pair_id": row["pair_id"],
                    "lane": row["lane"],
                    "spec": row["spec"],
                    "observation": row["observation"],
                }
                for row in rows
                if row["pair_id"] == pair_id and row["repeat_index"] == 0
            ]

        return execute

    def write_live_run(self, root):
        verified = self.verified_bundle()
        run = root / "run"
        (run / "raw").mkdir(parents=True)
        (run / "fit").mkdir()
        for packaged in verified["round_evidence"]:
            record = packaged["record"]
            (run / record["raw_rows"]).write_bytes(
                b"".join(
                    opcode_gas.canonical_json(row) + b"\n"
                    for row in packaged["rows"]
                )
            )
            (run / record["fit"]).write_bytes(
                opcode_gas._canonical_json_file_bytes(packaged["fit"])
            )
        opcode_gas._write_higher_layer_decisions(run, verified["decisions"])
        (run / "fixed-costs.json").write_bytes(
            opcode_gas._canonical_json_file_bytes(verified["fixed"])
        )
        (run / "raw" / "state-holdouts.jsonl").write_bytes(
            b"".join(
                opcode_gas.canonical_json(row) + b"\n"
                for row in verified["state_rows"]
            )
        )
        (run / "higher-layer-calibration.json").write_bytes(
            opcode_gas._canonical_json_file_bytes(verified["final"])
        )
        run_path = root / "run-path"
        run_path.write_text(str(run) + "\n")
        return run, run_path, verified

    @staticmethod
    def reseal_live_terminal_fit(run, mutate):
        decisions = json.loads((run / "overhead-decisions.json").read_text())
        record = decisions["rounds"][-1]
        fit_path = run / record["fit"]
        fit = json.loads(fit_path.read_text())
        mutate(fit)
        fit_path.write_bytes(opcode_gas._canonical_json_file_bytes(fit))
        record["fit_sha256"] = opcode_gas.sha256_file(fit_path)
        opcode_gas._write_higher_layer_decisions(run, decisions)

        fixed_path = run / "fixed-costs.json"
        fixed = json.loads(fixed_path.read_text())
        fixed["status"] = fit["status"]
        fixed["fixed_costs"] = copy.deepcopy(fit["fixed_costs"])
        fixed["fixed_cost_statuses"] = copy.deepcopy(fit["fixed_cost_statuses"])
        fixed["fixed_costs_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(fixed["fixed_costs"])
        )
        fixed["source"]["selected_fit_sha256"] = record["fit_sha256"]
        fixed["source"]["decision_ledger_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas._higher_layer_decisions_bytes(decisions)
        )
        fixed["fit"] = fit
        fixed_path.write_bytes(opcode_gas._canonical_json_file_bytes(fixed))

    def test_live_replay_rejects_resealed_approximation_evidence_tamper(self):
        native = lambda fit: fit["overhead_results"]["native_value_transfer"]
        tamper_cases = {
            "policy status": lambda fit: native(fit).__setitem__(
                "status", "accepted"
            ),
            "per-count materiality": lambda fit: native(fit)["count_evidence"][
                0
            ].__setitem__("materiality", "0.001"),
            "coefficient": lambda fit: native(fit).__setitem__("o_p", "5018"),
            "budget": lambda fit: native(fit).__setitem__(
                "materiality_budget", "0.003"
            ),
            "fixed statuses": lambda fit: fit["fixed_cost_statuses"].__setitem__(
                "native_value_transfer", "accepted"
            ),
            "source identity": lambda fit: native(fit)["source"].__setitem__(
                "identity_sha256", "0" * 64
            ),
            "source raw": lambda fit: native(fit)["source"].__setitem__(
                "raw_rows_sha256", "0" * 64
            ),
            "source fit": lambda fit: native(fit)["source"].__setitem__(
                "fit_sha256", "0" * 64
            ),
            "target total": lambda fit: native(fit)["count_evidence"][0].__setitem__(
                "target_prover_gas", "15018"
            ),
            "observed delta": lambda fit: native(fit)["count_evidence"][0].__setitem__(
                "observed_native_delta", "5018"
            ),
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            _run, run_path, verified = self.write_live_run(root)
            with mock.patch.object(
                opcode_gas,
                "_validate_current_higher_layer_identity",
                return_value=(
                    verified["identity"],
                    self.manifest,
                    self.coverage,
                    self.core,
                ),
            ):
                replay = opcode_gas.verify_higher_layer_calibration(
                    run_path,
                    trace_executor=self.trace_executor(verified["state_rows"]),
                )
            self.assertEqual(replay["fixed"], verified["fixed"])
            self.assertEqual(replay["final"], verified["final"])

        for label, mutate in tamper_cases.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                run, run_path, verified = self.write_live_run(root)
                self.reseal_live_terminal_fit(run, mutate)
                with mock.patch.object(
                    opcode_gas,
                    "_validate_current_higher_layer_identity",
                    return_value=(
                        verified["identity"],
                        self.manifest,
                        self.coverage,
                        self.core,
                    ),
                ), self.assertRaisesRegex(ValueError, "replay|accepted fixed costs"):
                    opcode_gas.verify_higher_layer_calibration(
                        run_path,
                        trace_executor=self.trace_executor(verified["state_rows"]),
                    )

    def test_all_state_statuses_use_decimal_metrics_and_preserve_fixed_digest(self):
        accepted = self.evaluate(self.rows())
        failed_rows = self.rows()
        for row in failed_rows:
            if row["pair_id"] == "witness_topology_1" and row["lane"] == "target":
                row["prover_gas"] = "3600"
        failed = self.evaluate(failed_rows)

        rejected_rows = self.rows()
        rejected = rejected_rows[0]
        rejected_identity = {
            key: rejected[key]
            for key in (
                "schema_version",
                "calibration_run_id",
                "pair_id",
                "lane",
                "repeat_index",
                "spec",
            )
        }
        rejected.clear()
        rejected.update(
            {
                **rejected_identity,
                "status": "rejected",
                "reasons": ["state_holdout_io_failure"],
            }
        )
        inconclusive = self.evaluate(rejected_rows)

        self.assertEqual(
            accepted["coarse_state_trie"]["status"], "coarse_model_accepted"
        )
        self.assertEqual(accepted["schema_version"], 2)
        self.assertEqual(
            accepted["fixed_model_status"],
            "accepted_with_declared_approximation",
        )
        self.assertEqual(
            accepted["fixed_cost_statuses"], self.fixed_cost_statuses
        )
        self.assertEqual(failed["coarse_state_trie"]["status"], "needs_state_split")
        self.assertEqual(inconclusive["coarse_state_trie"]["status"], "inconclusive")
        self.assertEqual(
            failed["fixed_costs_sha256"], accepted["fixed_costs_sha256"]
        )
        self.assertEqual(failed["fixed_costs"], accepted["fixed_costs"])
        failed_pair = next(
            pair
            for pair in failed["state_holdouts"]
            if pair["pair_id"] == "witness_topology_1"
        )
        with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
            expected_target_ape = Decimal(1) / Decimal(6)
        self.assertEqual(Decimal(failed_pair["target_ape"]), expected_target_ape)
        self.assertEqual(Decimal(failed_pair["effect_ratio"]), Decimal("0.2"))

        for pair in accepted["state_holdouts"]:
            if pair["kind"] != "dirty_accounts":
                continue
            expected_total = 3000 + 5317 * pair["scale"]
            self.assertEqual(pair["control_prediction"], str(expected_total))
            self.assertEqual(pair["target_prediction"], str(expected_total))
            self.assertEqual(pair["predicted_delta"], "0")
            self.assertEqual(pair["observed_delta"], "0")

    def test_missing_inventory_rejects_but_complete_invalid_evidence_is_inconclusive(self):
        missing = self.rows()
        missing.pop()
        with self.assertRaisesRegex(ValueError, "inventory"):
            self.evaluate(missing)

        unstable = self.rows()
        unstable[0]["prover_gas"] = str(int(unstable[0]["prover_gas"]) + 1)
        self.assertEqual(
            self.evaluate(unstable)["coarse_state_trie"]["status"], "inconclusive"
        )

        coverage_invalid = self.rows()
        for row in coverage_invalid:
            if row["pair_id"] == "witness_topology_1":
                row["observation"]["actual_raw_gas_by_key"] = {
                    "opcode:0x00": {
                        "pricing_basis": "raw_gas_slope",
                        "units": 1,
                        "event_count": 1,
                    }
                }
        self.assertEqual(
            self.evaluate(coverage_invalid)["coarse_state_trie"]["status"],
            "inconclusive",
        )

    def test_structural_state_identity_tamper_raises_instead_of_becoming_inconclusive(self):
        mutations = {
            "execution": lambda row: row.__setitem__("execution_row_id", "0" * 64),
            "gas-estimator": lambda row: row.__setitem__(
                "sp1_execution_engine", "standard"
            ),
            "spec": lambda row: row["spec"]["control"].__setitem__(
                "extra_account_count", 99
            ),
        }
        for expected, mutate in mutations.items():
            rows = self.rows()
            mutate(rows[0])
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                self.evaluate(rows)

    def test_state_rows_enforce_rust_numeric_and_hash_domains(self):
        u64_max = (1 << 64) - 1
        i64_max = (1 << 63) - 1

        boundary_rows = self.rows()
        for row in boundary_rows:
            row["prover_gas"] = str(u64_max)
            row["observation"]["actual_features"]["block_base"] = i64_max
            row["observation"]["actual_features"]["tx_base"] = i64_max
            row["observation"]["started_candidate_transaction_count"] = i64_max
            row["observation"]["committed_candidate_transaction_count"] = i64_max
            row["guest_input_bincode_length"] = i64_max
            row["observation"]["guest_input_bincode_length"] = i64_max
            row["observation"]["actual_diagnostics"][
                "guest_input_bincode_length"
            ] = i64_max
            row["observation"]["actual_diagnostics"][
                "witness_node_count"
            ] = i64_max
        self.evaluate(boundary_rows)

        def mutate_all(mutate):
            rows = self.rows()
            for row in rows:
                mutate(row)
            return rows

        invalid = {
            "proverGas.*u64": mutate_all(
                lambda row: row.__setitem__("prover_gas", str(u64_max + 1))
            ),
            "feature.*i64": mutate_all(
                lambda row: row["observation"]["actual_features"].__setitem__(
                    "block_base", i64_max + 1
                )
            ),
            "diagnostic.*i64": mutate_all(
                lambda row: row["observation"]["actual_diagnostics"].__setitem__(
                    "witness_node_count", i64_max + 1
                )
            ),
            "bincode.*u64": mutate_all(
                lambda row: (
                    row.__setitem__("guest_input_bincode_length", u64_max + 1),
                    row["observation"].__setitem__(
                        "guest_input_bincode_length", u64_max + 1
                    ),
                    row["observation"]["actual_diagnostics"].__setitem__(
                        "guest_input_bincode_length", u64_max + 1
                    ),
                )
            ),
            "counter.*u64": mutate_all(
                lambda row: row["observation"].__setitem__(
                    "committed_candidate_transaction_count", u64_max + 1
                )
            ),
            "canonical B256": mutate_all(
                lambda row: row["observation"].__setitem__(
                    "actual_final_state_root", "not-a-b256"
                )
            ),
            "canonical B256 public": mutate_all(
                lambda row: (
                    row.__setitem__("public_values", "0xAB" + "00" * 31),
                    row["observation"].__setitem__(
                        "public_output", "0xAB" + "00" * 31
                    ),
                )
            ),
            "GuestInput identity": mutate_all(
                lambda row: (
                    row.__setitem__(
                        "guest_input_sha256", row["guest_input_sha256"].upper()
                    ),
                    row["observation"].__setitem__(
                        "guest_input_sha256",
                        row["observation"]["guest_input_sha256"].upper(),
                    ),
                )
            ),
        }
        for expected, rows in invalid.items():
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                self.evaluate(rows)

    def test_state_row_counter_consistency_is_structural(self):
        mutations = (
            lambda observation: observation.__setitem__(
                "started_candidate_transaction_count",
                observation["started_candidate_transaction_count"] + 1,
            ),
            lambda observation: observation.__setitem__(
                "committed_candidate_transaction_count",
                observation["started_candidate_transaction_count"] + 1,
            ),
        )
        for mutate in mutations:
            inconsistent = self.rows()
            for row in inconsistent:
                mutate(row["observation"])
            with self.assertRaisesRegex(ValueError, "counter consistency"):
                self.evaluate(inconsistent)

    def test_malformed_operation_ledgers_raise_instead_of_becoming_inconclusive(self):
        malformed = {
            "nonempty": {
                "": {
                    "pricing_basis": "raw_gas_slope",
                    "units": 1,
                    "event_count": 1,
                }
            },
            "zero row": {
                "opcode:0x5f": {
                    "pricing_basis": "raw_gas_slope",
                    "units": 0,
                    "event_count": 0,
                }
            },
            "positive units": {
                "opcode:0x5f": {
                    "pricing_basis": "raw_gas_slope",
                    "units": 0,
                    "event_count": 1,
                }
            },
            "positive event_count": {
                "opcode:0x5f": {
                    "pricing_basis": "raw_gas_slope",
                    "units": 1,
                    "event_count": 0,
                }
            },
            "i64": {
                "opcode:0x5f": {
                    "pricing_basis": "raw_gas_slope",
                    "units": 1 << 63,
                    "event_count": 1,
                }
            },
        }
        for expected, ledger in malformed.items():
            rows = self.rows()
            for row in rows:
                if row["pair_id"] == "witness_topology_1":
                    row["observation"]["actual_raw_gas_by_key"] = copy.deepcopy(
                        ledger
                    )
            with self.subTest(expected=expected), self.assertRaisesRegex(
                ValueError, expected
            ):
                self.evaluate(rows)

    @staticmethod
    def report(row):
        return {
            "stage": "controlled-state-holdout",
            "mode": "execute",
            "sp1_execution_engine": "gas-estimator",
            "sp1_gas_trace_chunk_threshold": 134_217_728,
            "sp1_gas_trace_chunk_slots": 2,
            "guest_input_sha256": row["guest_input_sha256"],
            "guest_input_bincode_length": row["guest_input_bincode_length"],
            "public_values": row["public_values"],
            "gas": int(row["prover_gas"]),
            "controlled_state_holdout": {
                "status": "accepted",
                "pair_id": row["pair_id"],
                "lane": row["lane"],
                "spec": row["spec"],
                "reasons": [],
                "observation": row["observation"],
            },
        }

    def test_state_runner_is_fixed_gated_and_persists_exact_inventory(self):
        rows = self.rows()
        calls = []

        def fake_executor(**kwargs):
            pair_id = kwargs["pair_spec"]["pair_id"]
            repeat_index = kwargs["repeat_index"]
            calls.append((pair_id, repeat_index))
            return [
                self.report(row)
                for row in rows
                if row["pair_id"] == pair_id
                and row["repeat_index"] == repeat_index
            ]

        identity = {
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
        }
        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            opcode_gas,
            "_validate_current_higher_layer_identity",
            return_value=(identity, self.manifest, self.coverage, self.core),
        ), mock.patch.object(
            opcode_gas,
            "fit_higher_layer_fixed_costs",
            return_value=self.fixed,
        ):
            run = pathlib.Path(temporary)
            with self.assertRaisesRegex(ValueError, "fixed-cost artifact"):
                opcode_gas.run_higher_layer_state_holdouts(
                    run, executor=fake_executor
                )
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            persisted = opcode_gas.run_higher_layer_state_holdouts(
                run, executor=fake_executor
            )

            self.assertEqual(len(persisted), 36)
            self.assertEqual(len({row["execution_row_id"] for row in persisted}), 36)
            self.assertEqual(
                calls,
                [
                    (pair.pair_id, repeat)
                    for pair in self.manifest.state_holdouts
                    for repeat in range(3)
                ],
            )
            raw_path = run / "raw" / "state-holdouts.jsonl"
            self.assertEqual(
                raw_path.read_bytes(),
                b"".join(
                    opcode_gas.canonical_json(row) + b"\n" for row in persisted
                ),
            )
            copied = copy.deepcopy(persisted)
            copied[0]["calibration_run_id"] = "2" * 24
            raw_path.write_bytes(
                b"".join(opcode_gas.canonical_json(row) + b"\n" for row in copied)
            )
            with self.assertRaisesRegex(ValueError, "calibration identity"):
                opcode_gas.run_higher_layer_state_holdouts(
                    run, executor=fake_executor
                )

    def test_state_runner_sanitizes_execution_failure_paths_for_sealing(self):
        identity = {
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
        }

        def fail_with_path(**_kwargs):
            raise OSError("failed to open /home/sample_user/private-input.json")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            opcode_gas,
            "_validate_current_higher_layer_identity",
            return_value=(identity, self.manifest, self.coverage, self.core),
        ), mock.patch.object(
            opcode_gas,
            "fit_higher_layer_fixed_costs",
            return_value=self.fixed,
        ):
            root = pathlib.Path(temporary)
            run = root / "run"
            run.mkdir()
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            rows = opcode_gas.run_higher_layer_state_holdouts(
                run, executor=fail_with_path
            )
            self.assertEqual(len(rows), 36)
            self.assertTrue(all(row["status"] == "rejected" for row in rows))
            self.assertTrue(
                all(row["reasons"] == ["state_holdout_io_failure"] for row in rows)
            )
            raw_text = (run / "raw" / "state-holdouts.jsonl").read_text()
            self.assertNotIn("/home/", raw_text)
            final = self.evaluate(rows)
            self.assertEqual(final["coarse_state_trie"]["status"], "inconclusive")

            verified = self.verified_bundle()
            verified["state_rows"] = rows
            verified["final"] = final
            destination = opcode_gas.seal_higher_layer_calibration(
                run,
                root / "sealed",
                root / "derivation-path",
                verifier=lambda _path: verified,
            )
            self.assertNotIn(
                "/home/", (destination / "state-holdout-evidence.json").read_text()
            )

    def test_state_report_requires_positive_integer_prover_gas(self):
        pair = self.manifest.state_holdouts[0]
        report = self.report(self.rows()[0])
        del report["gas"]
        with self.assertRaisesRegex(ValueError, "proverGas"):
            opcode_gas._higher_layer_state_row_from_report(
                report,
                pair=pair,
                repeat_index=0,
                calibration_id=CALIBRATION_ID,
            )

        report = self.report(self.rows()[0])
        report["gas"] = 1 << 64
        with self.assertRaisesRegex(ValueError, "proverGas.*u64"):
            opcode_gas._higher_layer_state_row_from_report(
                report,
                pair=pair,
                repeat_index=0,
                calibration_id=CALIBRATION_ID,
            )

    def test_normal_state_executor_uses_only_production_sp1_gas_estimator(self):
        pair_spec = self.pair_spec(self.manifest.state_holdouts[0])

        def inspect(command, **_kwargs):
            self.assertEqual(
                command[command.index("--stage") + 1], "controlled-state-holdout"
            )
            self.assertEqual(command[command.index("--proof-type") + 1], "sp1")
            self.assertEqual(command[command.index("--mode") + 1], "execute")
            self.assertEqual(command[command.index("--sp1-prover") + 1], "local")
            self.assertEqual(
                command[command.index("--sp1-execution-engine") + 1],
                "gas-estimator",
            )
            raise RuntimeError("state command inspected")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            opcode_gas.subprocess, "run", side_effect=inspect
        ), self.assertRaisesRegex(RuntimeError, "state command inspected"):
            opcode_gas.run_controlled_state_holdout_pair(
                guest_launcher=pathlib.Path("guest-launcher"),
                pair_spec=pair_spec,
                repeat_index=0,
                out=pathlib.Path(temporary) / "state.jsonl",
            )

    def test_finalize_persists_byte_exact_verdict_and_rejects_missing_slot(self):
        identity = {
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
        }
        with tempfile.TemporaryDirectory() as temporary:
            run = pathlib.Path(temporary)
            raw_path = run / "raw" / "state-holdouts.jsonl"
            raw_path.parent.mkdir()
            rows = self.rows()
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            raw_path.write_bytes(
                b"".join(opcode_gas.canonical_json(row) + b"\n" for row in rows)
            )
            with mock.patch.object(
                opcode_gas,
                "_validate_current_higher_layer_identity",
                return_value=(identity, self.manifest, self.coverage, self.core),
            ), mock.patch.object(
                opcode_gas,
                "fit_higher_layer_fixed_costs",
                return_value=self.fixed,
            ):
                result = opcode_gas.finalize_higher_layer_calibration(run)
                self.assertEqual(
                    (run / "higher-layer-calibration.json").read_bytes(),
                    opcode_gas._canonical_json_file_bytes(result),
                )

                raw_path.write_bytes(
                    b"".join(
                        opcode_gas.canonical_json(row) + b"\n" for row in rows[:-1]
                    )
                )
                with self.assertRaisesRegex(ValueError, "inventory"):
                    opcode_gas.finalize_higher_layer_calibration(run)

    def test_verifier_uses_one_host_trace_per_pair_and_rejects_diagnostic_tamper(self):
        identity = {
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
        }
        rows = self.rows()
        calls = []

        def fake_trace(**kwargs):
            pair_id = kwargs["pair_spec"]["pair_id"]
            calls.append(pair_id)
            return [
                {
                    "schema_version": 1,
                    "pair_id": row["pair_id"],
                    "lane": row["lane"],
                    "spec": row["spec"],
                    "observation": row["observation"],
                }
                for row in rows
                if row["pair_id"] == pair_id and row["repeat_index"] == 0
            ]

        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / "run"
            (run / "raw").mkdir(parents=True)
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            (run / "raw" / "state-holdouts.jsonl").write_bytes(
                b"".join(opcode_gas.canonical_json(row) + b"\n" for row in rows)
            )
            final = self.evaluate(rows)
            (run / "higher-layer-calibration.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(final)
            )
            run_path_file = root / "run-path"
            run_path_file.write_text(str(run) + "\n")
            with mock.patch.object(
                opcode_gas,
                "_validate_current_higher_layer_identity",
                return_value=(identity, self.manifest, self.coverage, self.core),
            ), mock.patch.object(
                opcode_gas,
                "fit_higher_layer_fixed_costs",
                return_value=self.fixed,
            ), mock.patch.object(
                opcode_gas,
                "_load_higher_layer_decisions",
                return_value=({"schema_version": 1, "rounds": []}, []),
            ):
                verified = opcode_gas.verify_higher_layer_calibration(
                    run_path_file, trace_executor=fake_trace
                )
                self.assertEqual(verified["final"], final)
                self.assertEqual(
                    calls, [pair.pair_id for pair in self.manifest.state_holdouts]
                )

                tampered = copy.deepcopy(rows)
                tampered[0]["observation"]["actual_diagnostics"][
                    "witness_node_count"
                ] += 1
                (run / "raw" / "state-holdouts.jsonl").write_bytes(
                    b"".join(
                        opcode_gas.canonical_json(row) + b"\n" for row in tampered
                    )
                )
                with self.assertRaisesRegex(ValueError, "trace"):
                    opcode_gas.verify_higher_layer_calibration(
                        run_path_file, trace_executor=fake_trace
                    )

    def test_verifier_packages_the_validated_round_snapshot_without_rereading(self):
        identity = {
            "calibration_id": CALIBRATION_ID,
            "identity_sha256": "1" * 64,
        }
        state_rows = self.rows()
        original_round_rows = [{"snapshot": "validated"}]
        record = {
            "generator_max_count": 8,
            "raw_rows": "raw/overhead-round-8.jsonl",
            "raw_rows_sha256": "2" * 64,
            "fit": "fit/overhead-round-8.json",
            "fit_sha256": "3" * 64,
            "decision": "expand_next_round",
        }
        validated = [
            {
                **record,
                "fit_payload": {"decision": "expand_next_round"},
                "raw_rows_payload": original_round_rows,
            }
        ]
        decisions = {
            "schema_version": 1,
            "identity_sha256": "1" * 64,
            "rounds": [record],
        }
        final = self.evaluate(state_rows)
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / "run"
            (run / "raw").mkdir(parents=True)
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            state_path = run / "raw" / "state-holdouts.jsonl"
            state_path.write_bytes(
                b"".join(
                    opcode_gas.canonical_json(row) + b"\n" for row in state_rows
                )
            )
            (run / "higher-layer-calibration.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(final)
            )
            run_path_file = root / "run-path"
            run_path_file.write_text(str(run) + "\n")

            def read_rows(path):
                if pathlib.Path(path) == state_path:
                    return state_rows
                return [{"snapshot": "mutated-second-read"}]

            with mock.patch.object(
                opcode_gas,
                "_validate_current_higher_layer_identity",
                return_value=(identity, self.manifest, self.coverage, self.core),
            ), mock.patch.object(
                opcode_gas,
                "fit_higher_layer_fixed_costs",
                return_value=self.fixed,
            ), mock.patch.object(
                opcode_gas,
                "_load_higher_layer_decisions",
                return_value=(decisions, validated),
            ), mock.patch.object(
                opcode_gas, "_read_higher_layer_rows", side_effect=read_rows
            ):
                verified = opcode_gas.verify_higher_layer_calibration(
                    run_path_file,
                    trace_executor=self.trace_executor(state_rows),
                )
            self.assertEqual(
                verified["round_evidence"][0]["rows"], original_round_rows
            )

    def test_host_trace_verifier_command_is_native_and_has_no_sp1_flags(self):
        pair_spec = self.pair_spec(self.manifest.state_holdouts[0])

        def inspect(command, **_kwargs):
            self.assertEqual(
                command[command.index("--stage") + 1],
                "controlled-state-holdout-trace",
            )
            self.assertEqual(command[command.index("--proof-type") + 1], "native")
            self.assertEqual(command[command.index("--mode") + 1], "execute")
            self.assertNotIn("--sp1-prover", command)
            self.assertNotIn("--sp1-execution-engine", command)
            self.assertNotIn("--elf", command)
            raise RuntimeError("trace command inspected")

        with tempfile.TemporaryDirectory() as temporary, mock.patch.object(
            opcode_gas.subprocess, "run", side_effect=inspect
        ), self.assertRaisesRegex(RuntimeError, "trace command inspected"):
            opcode_gas.run_controlled_state_holdout_trace(
                guest_launcher=pathlib.Path("guest-launcher"),
                pair_spec=pair_spec,
                out=pathlib.Path(temporary) / "trace.jsonl",
            )

    def test_task5_parser_exposes_gated_command_order(self):
        parser = opcode_gas.build_parser()
        commands = {
            "fit-higher-layer-fixed-costs": opcode_gas.cmd_fit_higher_layer_fixed_costs,
            "run-higher-layer-state-holdouts": opcode_gas.cmd_run_higher_layer_state_holdouts,
            "finalize-higher-layer-calibration": opcode_gas.cmd_finalize_higher_layer_calibration,
            "verify-higher-layer-calibration": opcode_gas.cmd_verify_higher_layer_calibration,
            "seal-higher-layer-calibration": opcode_gas.cmd_seal_higher_layer_calibration,
        }
        for command, expected in commands.items():
            arguments = [command]
            if command == "verify-higher-layer-calibration":
                arguments += ["--run-path-file", "run-path"]
            elif command == "seal-higher-layer-calibration":
                arguments += [
                    "--run",
                    "run",
                    "--out-root",
                    "out",
                    "--derivation-path-file",
                    "derivation-path",
                ]
            else:
                arguments += ["--run", "run"]
            with self.subTest(command=command):
                self.assertIs(parser.parse_args(arguments).func, expected)

    def test_verifier_rejects_symlinked_state_or_final_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / "run"
            external = root / "external"
            (run / "raw").mkdir(parents=True)
            external.mkdir()
            (run / "fixed-costs.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(self.fixed)
            )
            rows = self.rows()
            raw_bytes = b"".join(
                opcode_gas.canonical_json(row) + b"\n" for row in rows
            )
            final_bytes = opcode_gas._canonical_json_file_bytes(self.evaluate(rows))
            (external / "state.jsonl").write_bytes(raw_bytes)
            (external / "final.json").write_bytes(final_bytes)
            run_path_file = root / "run-path"
            run_path_file.write_text(str(run) + "\n")
            identity = {
                "calibration_id": CALIBRATION_ID,
                "identity_sha256": "1" * 64,
            }
            for artifact in ("state", "final"):
                with self.subTest(artifact=artifact):
                    raw_path = run / "raw" / "state-holdouts.jsonl"
                    final_path = run / "higher-layer-calibration.json"
                    for path in (raw_path, final_path):
                        if path.exists() or path.is_symlink():
                            path.unlink()
                    raw_path.symlink_to(external / "state.jsonl") if artifact == "state" else raw_path.write_bytes(raw_bytes)
                    final_path.symlink_to(external / "final.json") if artifact == "final" else final_path.write_bytes(final_bytes)
                    with mock.patch.object(
                        opcode_gas,
                        "_validate_current_higher_layer_identity",
                        return_value=(identity, self.manifest, self.coverage, self.core),
                    ), mock.patch.object(
                        opcode_gas,
                        "fit_higher_layer_fixed_costs",
                        return_value=self.fixed,
                    ), mock.patch.object(
                        opcode_gas,
                        "_load_higher_layer_decisions",
                        return_value=({"schema_version": 1, "rounds": []}, []),
                    ), self.assertRaisesRegex(ValueError, "non-symlink"):
                        opcode_gas.verify_higher_layer_calibration(
                            run_path_file, trace_executor=lambda **_kwargs: []
                        )
            symlink_run = root / "symlink-run"
            symlink_run.symlink_to(run, target_is_directory=True)
            run_path_file.write_text(str(symlink_run) + "\n")
            with self.assertRaisesRegex(ValueError, "symlink"):
                opcode_gas._higher_layer_run_from_path_file(run_path_file)

    def test_seal_is_create_only_exact_four_and_cleans_partial_publish(self):
        verified = self.verified_bundle()
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / "run"
            run.mkdir()
            output = root / "sealed"
            path_file = root / "derivation-path"
            path_file.touch()
            destination = opcode_gas.seal_higher_layer_calibration(
                run,
                output,
                path_file,
                verifier=lambda _path: verified,
            )
            self.assertEqual(
                {path.name for path in destination.iterdir()},
                {
                    "identity.json",
                    "overhead-evidence.json",
                    "state-holdout-evidence.json",
                    "higher-layer-calibration.json",
                },
            )
            self.assertEqual(path_file.read_text(), str(destination) + "\n")
            with self.assertRaisesRegex(ValueError, "already exists"):
                opcode_gas.seal_higher_layer_calibration(
                    run,
                    output,
                    path_file,
                    verifier=lambda _path: verified,
                )

            failing_output = root / "failing"
            real_write = opcode_gas._write_derivation_json
            writes = 0

            def fail_second(path, value):
                nonlocal writes
                writes += 1
                if writes == 2:
                    raise OSError("injected publish failure")
                real_write(path, value)

            with mock.patch.object(
                opcode_gas, "_write_derivation_json", side_effect=fail_second
            ), self.assertRaisesRegex(OSError, "injected"):
                opcode_gas.seal_higher_layer_calibration(
                    run,
                    failing_output,
                    root / "unused-path",
                    verifier=lambda _path: verified,
                )
            self.assertEqual(list(failing_output.iterdir()), [])

            for label, prepare_handoff, expected in (
                (
                    "nonempty",
                    lambda path: path.write_text("occupied\n"),
                    "non-empty",
                ),
                (
                    "symlink",
                    lambda path: path.symlink_to(root / "empty-external"),
                    "cannot be claimed",
                ),
            ):
                with self.subTest(handoff=label):
                    (root / "empty-external").touch(exist_ok=True)
                    handoff = root / f"{label}-handoff"
                    prepare_handoff(handoff)
                    rejected_output = root / f"{label}-output"
                    with self.assertRaisesRegex(ValueError, expected):
                        opcode_gas.seal_higher_layer_calibration(
                            run,
                            rejected_output,
                            handoff,
                            verifier=lambda _path: verified,
                        )
                    self.assertEqual(list(rejected_output.iterdir()), [])

    def test_seal_rejects_absolute_user_specific_persisted_path(self):
        verified = self.verified_bundle()
        verified["identity"]["identity"]["guest_launcher"][
            "path"
        ] = "/home/sample_user/tool"
        with tempfile.TemporaryDirectory() as temporary, self.assertRaisesRegex(
            ValueError, "portable"
        ):
            root = pathlib.Path(temporary)
            run = root / "run"
            run.mkdir()
            opcode_gas.seal_higher_layer_calibration(
                run,
                root / "sealed",
                root / "derivation-path",
                verifier=lambda _path: verified,
            )

    def test_portable_package_replays_without_live_run_and_rejects_tamper(self):
        verified = self.verified_bundle()
        rows = verified["state_rows"]

        def validate_source(identity):
            if not opcode_gas._exact_json_equal(identity, verified["identity"]):
                raise ValueError("sealed source identity differs")
            return self.manifest, self.coverage, self.core

        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            run = root / "run"
            run.mkdir()
            destination = opcode_gas.seal_higher_layer_calibration(
                run,
                root / "sealed",
                root / "derivation-path",
                verifier=lambda _path: verified,
            )
            copied = root / "copied-four-files"
            copied.mkdir()
            for source in destination.iterdir():
                (copied / source.name).write_bytes(source.read_bytes())

            replay = opcode_gas.verify_sealed_higher_layer_calibration(
                copied,
                trace_executor=self.trace_executor(rows),
                source_validator=validate_source,
            )
            self.assertEqual(replay["fixed"], verified["fixed"])
            self.assertEqual(replay["final"], verified["final"])
            copied_path_file = root / "copied-path"
            copied_path_file.write_text(str(copied) + "\n")
            replay_from_path = opcode_gas.verify_higher_layer_calibration(
                copied_path_file,
                trace_executor=self.trace_executor(rows),
                source_validator=validate_source,
            )
            self.assertEqual(replay_from_path["final"], verified["final"])

            external_state = root / "external-state.json"
            external_state.write_bytes(
                (destination / "state-holdout-evidence.json").read_bytes()
            )
            symlinked_file = root / "symlinked-file-package"
            symlinked_file.mkdir()
            for source in destination.iterdir():
                target = symlinked_file / source.name
                if source.name == "state-holdout-evidence.json":
                    target.symlink_to(external_state)
                else:
                    target.write_bytes(source.read_bytes())
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                opcode_gas.verify_sealed_higher_layer_calibration(
                    symlinked_file,
                    trace_executor=self.trace_executor(rows),
                    source_validator=validate_source,
                )
            symlinked_package = root / "symlinked-package"
            symlinked_package.symlink_to(destination, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "exactly four"):
                opcode_gas.verify_sealed_higher_layer_calibration(
                    symlinked_package,
                    trace_executor=self.trace_executor(rows),
                    source_validator=validate_source,
                )

            rejected_rows = copy.deepcopy(rows)
            rejected = rejected_rows[0]
            rejected_identity = {
                key: rejected[key]
                for key in (
                    "schema_version",
                    "calibration_run_id",
                    "pair_id",
                    "lane",
                    "repeat_index",
                    "spec",
                )
            }
            rejected.clear()
            rejected.update(
                {
                    **rejected_identity,
                    "status": "rejected",
                    "reasons": ["state_holdout_io_failure"],
                }
            )
            rejected_final = self.evaluate(rejected_rows)

            def mutate_terminal_fit(overhead, mutate):
                packaged = overhead["rounds"][-1]
                fit = packaged["fit"]
                mutate(fit)
                fit_sha256 = opcode_gas.sha256_bytes(
                    opcode_gas._canonical_json_file_bytes(fit)
                )
                packaged["record"]["fit_sha256"] = fit_sha256
                ledger_record = overhead["decision_ledger"]["rounds"][-1]
                ledger_record["fit_sha256"] = fit_sha256
                fixed = overhead["fixed_costs"]
                fixed["status"] = fit["status"]
                fixed["fixed_costs"] = copy.deepcopy(fit["fixed_costs"])
                fixed["fixed_cost_statuses"] = copy.deepcopy(
                    fit["fixed_cost_statuses"]
                )
                fixed["fixed_costs_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(fixed["fixed_costs"])
                )
                fixed["source"]["selected_fit_sha256"] = fit_sha256
                fixed["source"]["decision_ledger_sha256"] = (
                    opcode_gas.sha256_bytes(
                        opcode_gas._higher_layer_decisions_bytes(
                            overhead["decision_ledger"]
                        )
                    )
                )
                fixed["fit"] = copy.deepcopy(fit)

            def native_result(fit):
                return fit["overhead_results"]["native_value_transfer"]

            tamper_cases = {
                "source": lambda overhead, state, final: overhead["fixed_costs"][
                    "source"
                ].__setitem__("selected_fit_sha256", "0" * 64),
                "raw repeat": lambda overhead, state, final: overhead["rounds"][
                    -1
                ]["rows"][0].__setitem__("prover_gas", "999999"),
                "event count": lambda overhead, state, final: overhead["rounds"][
                    -1
                ]["rows"][0].setdefault("observed_operation_deltas", {}).setdefault(
                    "opcode:0x5f",
                    {
                        "pricing_basis": "raw_gas_slope",
                        "units": 0,
                        "event_count": 0,
                    },
                ).__setitem__("event_count", 1),
                "diagnostic": lambda overhead, state, final: state["rows"][0][
                    "observation"
                ]["actual_diagnostics"].__setitem__("witness_node_count", 99),
                "rejected URI": lambda overhead, state, final: (
                    state["rows"][0].clear(),
                    state["rows"][0].update(
                        {
                            "schema_version": 1,
                            "calibration_run_id": CALIBRATION_ID,
                            "pair_id": "witness_topology_1",
                            "lane": "control",
                            "repeat_index": 0,
                            "status": "rejected",
                            "spec": self.pair_spec(self.manifest.state_holdouts[0]),
                            "reasons": [
                                "failed:file:///home/sample_user/private-input.json"
                            ],
                        }
                    ),
                    final.clear(),
                    final.update(copy.deepcopy(rejected_final)),
                ),
                "verdict": lambda overhead, state, final: final[
                    "coarse_state_trie"
                ].__setitem__("status", "needs_state_split"),
                "policy status": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit).__setitem__(
                        "status", "accepted"
                    ),
                ),
                "per-count materiality": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["count_evidence"][0].__setitem__(
                        "materiality", "0.001"
                    ),
                ),
                "coefficient": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit).__setitem__("o_p", "5018"),
                ),
                "budget": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit).__setitem__(
                        "materiality_budget", "0.003"
                    ),
                ),
                "fixed statuses": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: fit["fixed_cost_statuses"].__setitem__(
                        "native_value_transfer", "accepted"
                    ),
                ),
                "source identity": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["source"].__setitem__(
                        "identity_sha256", "0" * 64
                    ),
                ),
                "source raw": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["source"].__setitem__(
                        "raw_rows_sha256", "0" * 64
                    ),
                ),
                "source fit": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["source"].__setitem__(
                        "fit_sha256", "0" * 64
                    ),
                ),
                "target total": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["count_evidence"][0].__setitem__(
                        "target_prover_gas", "15018"
                    ),
                ),
                "observed delta": lambda overhead, state, final: mutate_terminal_fit(
                    overhead,
                    lambda fit: native_result(fit)["count_evidence"][0].__setitem__(
                        "observed_native_delta", "5018"
                    ),
                ),
                "drop approximation status": lambda overhead, state, final: final.pop(
                    "fixed_model_status"
                ),
                "relabel approximation status": lambda overhead, state, final: final[
                    "fixed_cost_statuses"
                ].__setitem__("native_value_transfer", "accepted"),
            }
            for label, mutate in tamper_cases.items():
                with self.subTest(label=label):
                    overhead = json.loads(
                        (destination / "overhead-evidence.json").read_text()
                    )
                    state = json.loads(
                        (destination / "state-holdout-evidence.json").read_text()
                    )
                    final = json.loads(
                        (destination / "higher-layer-calibration.json").read_text()
                    )
                    mutate(overhead, state, final)
                    tampered = root / f"tampered-{label.replace(' ', '-')}"
                    tampered.mkdir()
                    for name, value in (
                        ("overhead-evidence.json", overhead),
                        ("state-holdout-evidence.json", state),
                        ("higher-layer-calibration.json", final),
                    ):
                        (tampered / name).write_bytes(
                            opcode_gas._canonical_json_file_bytes(value)
                        )
                    sealed_identity = json.loads(
                        (destination / "identity.json").read_text()
                    )
                    for name in (
                        "overhead-evidence.json",
                        "state-holdout-evidence.json",
                        "higher-layer-calibration.json",
                    ):
                        sealed_identity["identity"]["file_sha256s"][name] = (
                            opcode_gas.sha256_file(tampered / name)
                        )
                    identity_sha256 = opcode_gas.sha256_bytes(
                        opcode_gas.canonical_json(sealed_identity["identity"])
                    )
                    sealed_identity["identity_sha256"] = identity_sha256
                    sealed_identity["derivation_id"] = identity_sha256[:24]
                    (tampered / "identity.json").write_bytes(
                        opcode_gas._canonical_json_file_bytes(sealed_identity)
                    )
                    with self.assertRaises(ValueError):
                        opcode_gas.verify_sealed_higher_layer_calibration(
                            tampered,
                            trace_executor=self.trace_executor(rows),
                            source_validator=validate_source,
                        )

            changed_source = root / "tampered-sealed-source-identity"
            changed_source.mkdir()
            for source in destination.iterdir():
                (changed_source / source.name).write_bytes(source.read_bytes())
            sealed_identity = json.loads(
                (changed_source / "identity.json").read_text()
            )
            sealed_identity["identity"]["source_identity"]["identity"][
                "guest_launcher"
            ]["file_sha256"] = "b" * 64
            identity_sha256 = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(sealed_identity["identity"])
            )
            sealed_identity["identity_sha256"] = identity_sha256
            sealed_identity["derivation_id"] = identity_sha256[:24]
            (changed_source / "identity.json").write_bytes(
                opcode_gas._canonical_json_file_bytes(sealed_identity)
            )
            with self.assertRaisesRegex(ValueError, "source identity"):
                opcode_gas.verify_sealed_higher_layer_calibration(
                    changed_source,
                    trace_executor=self.trace_executor(rows),
                    source_validator=validate_source,
                )


if __name__ == "__main__":
    unittest.main()
