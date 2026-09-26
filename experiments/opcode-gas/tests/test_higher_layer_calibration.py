import copy
import json
import pathlib
import sys
import tempfile
import unittest
from decimal import Decimal, localcontext


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


if __name__ == "__main__":
    unittest.main()
