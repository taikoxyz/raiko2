import copy
import json
import pathlib
import shutil
import sys
import tempfile
import unittest
from argparse import Namespace
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from test_sp1_candidate_report import canonical_formal_relation_round_rows


FIXTURE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "tests"
    / "fixtures"
    / "historical-core-102"
    / "controlled-manifest.toml"
)
DERIVATION = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "3e1d97c461cd2ef9a40e6a02"
)
CURRENT_MANIFEST = (
    ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-calibration-v1.toml"
)
HISTORICAL_EVIDENCE = FIXTURE.with_name("historical-evidence.json")


class HistoricalBaselineTests(unittest.TestCase):
    def test_historical_fixture_is_the_exact_tracked_schema(self):
        self.assertEqual(
            opcode_gas.sha256_file(FIXTURE),
            "4140fe1a0ccc533dbee8940a63da6db41be8a26955cc0022ccae5e1aedc3e01e",
        )
        result = opcode_gas.validate_historical_core_opcode_baseline(
            DERIVATION, FIXTURE
        )

        self.assertEqual(len(result["opcode_keys"]), 102)
        self.assertEqual(result["modeled_named_opcode_count"], 101)
        self.assertEqual(result["missing_core_opcode_keys"], ["opcode:0x15"])
        self.assertEqual(result["unsupported_named_opcode_count"], 49)
        self.assertNotIn("opcode:0x1e", result["opcode_keys"])

    def test_historical_fixture_checksum_record_is_repo_relative(self):
        checksum = FIXTURE.with_suffix(".sha256").read_text()
        self.assertEqual(
            checksum,
            "4140fe1a0ccc533dbee8940a63da6db41be8a26955cc0022ccae5e1aedc3e01e  "
            "experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml\n",
        )

    def test_historical_validator_uses_frozen_inventory_not_current_globals(self):
        changed_defaults = dict(opcode_gas.PURE_OPCODE_DEFAULTS)
        changed_defaults.pop(0x1E)
        changed_defaults[0xFE] = ("stack", "stack_push", 3)
        changed_names = dict(opcode_gas.UZEN_OPCODE_NAMES)
        changed_names[0x1E] = "different-clz"
        changed_names[0xFE] = "different-opcode"
        with mock.patch.object(opcode_gas, "PURE_OPCODE_DEFAULTS", changed_defaults), mock.patch.object(
            opcode_gas, "UZEN_OPCODE_NAMES", changed_names
        ):
            result = opcode_gas.validate_historical_core_opcode_baseline(
                DERIVATION, FIXTURE
            )
        self.assertEqual(result["opcode_keys"], opcode_gas.HISTORICAL_CORE_OPCODE_KEYS)

    def test_historical_evidence_rejects_resealed_identity_and_program_mutations(self):
        mutations = (
            lambda value: value["canaries"][0]["row_evidence"][0].__setitem__(
                "bytecode_sha256", "0" * 64
            ),
            lambda value: value["canaries"][0]["equation"].__setitem__(
                "slope_p", "0"
            ),
            lambda value: value["canaries"][0].__setitem__(
                "relation_id", "opcode:0x15:canonical"
            ),
            lambda value: value["canaries"][0]["row_evidence"][0].pop(
                "fixture_sha256"
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                root = pathlib.Path(tmp)
                fixture = root / FIXTURE.name
                evidence = root / HISTORICAL_EVIDENCE.name
                shutil.copy2(FIXTURE, fixture)
                shutil.copy2(FIXTURE.with_suffix(".sha256"), fixture.with_suffix(".sha256"))
                shutil.copy2(HISTORICAL_EVIDENCE, evidence)
                payload = json.loads(evidence.read_text())
                mutate(payload)
                payload["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(
                        {
                            key: value
                            for key, value in payload.items()
                            if key != "artifact_sha256"
                        }
                    )
                )
                evidence.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                evidence.with_suffix(".sha256").write_text(
                    f"{opcode_gas.sha256_file(evidence)}  historical-evidence.json\n"
                )
                with mock.patch.object(opcode_gas, "REPO_ROOT", root), self.assertRaises(
                    ValueError
                ):
                    opcode_gas.validate_historical_core_opcode_baseline(
                        DERIVATION, fixture
                    )

    def test_program_identity_hashes_actual_bytecode_and_shape_evidence(self):
        row = copy.deepcopy(json.loads(HISTORICAL_EVIDENCE.read_text())["canaries"][0]["row_evidence"][0])
        row.pop("bytecode_sha256")
        row["bytecode"] = "0x6000"
        original = opcode_gas._formal_relation_program_sha256([row])
        row["bytecode"] = "0x6001"
        self.assertNotEqual(original, opcode_gas._formal_relation_program_sha256([row]))

    def test_historical_validator_rejects_fixture_and_derivation_mutations(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            fixture = root / "controlled-manifest.toml"
            fixture.write_bytes(FIXTURE.read_bytes() + b"\n")
            fixture.with_suffix(".sha256").write_text(
                FIXTURE.with_suffix(".sha256").read_text()
            )
            with self.assertRaises(ValueError):
                opcode_gas.validate_historical_core_opcode_baseline(
                    DERIVATION, fixture
                )

        mutations = (
            ("derivation.json", "schema_version", 2),
            ("derivation.json", "artifact_sha256", "0" * 64),
            ("dynamic-opcode-models.json", "schema_version", 3),
            ("core-opcode-submodel.json", "schema_version", 4),
            ("core-opcode-submodel.json", "artifact_sha256", "0" * 64),
        )
        for filename, field, value in mutations:
            with self.subTest(filename=filename, field=field), tempfile.TemporaryDirectory() as tmp:
                target = pathlib.Path(tmp) / DERIVATION.name
                shutil.copytree(DERIVATION, target)
                path = target / filename
                payload = json.loads(path.read_text())
                payload[field] = value
                path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                with self.assertRaises(ValueError):
                    opcode_gas.validate_historical_core_opcode_baseline(
                        target, FIXTURE
                    )

        with tempfile.TemporaryDirectory() as tmp:
            target = pathlib.Path(tmp) / DERIVATION.name
            shutil.copytree(DERIVATION, target)
            path = target / "core-opcode-submodel.json"
            payload = json.loads(path.read_text())
            payload["registry"]["models"].pop("opcode:0x01")
            payload["artifact_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {key: value for key, value in payload.items() if key != "artifact_sha256"}
                )
            )
            path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            derivation = json.loads((target / "derivation.json").read_text())
            derivation["output_hashes"]["core_artifact_sha256"] = payload[
                "artifact_sha256"
            ]
            derivation["output_hashes"]["core_file_sha256"] = opcode_gas.sha256_file(
                path
            )
            derivation["artifact_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in derivation.items()
                        if key != "artifact_sha256"
                    }
                )
            )
            (target / "derivation.json").write_text(
                json.dumps(derivation, indent=2, sort_keys=True) + "\n"
            )
            with self.assertRaises(ValueError):
                opcode_gas.validate_historical_core_opcode_baseline(target, FIXTURE)

    def test_historical_validator_rejects_resealed_registry_coverage_drift(self):
        for mutate in (
            lambda core: core["registry"]["models"].update(
                {"unexpected": core["registry"]["models"].pop("invalid")}
            ),
            lambda core: core["unsupported_named_opcode_keys"].__setitem__(
                0, "opcode:0x01"
            ),
        ):
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                target = pathlib.Path(tmp) / DERIVATION.name
                shutil.copytree(DERIVATION, target)
                core_path = target / "core-opcode-submodel.json"
                core = json.loads(core_path.read_text())
                mutate(core)
                core["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(
                        {
                            key: value
                            for key, value in core.items()
                            if key != "artifact_sha256"
                        }
                    )
                )
                core_path.write_text(json.dumps(core, indent=2, sort_keys=True) + "\n")
                derivation_path = target / "derivation.json"
                derivation = json.loads(derivation_path.read_text())
                derivation["output_hashes"]["core_artifact_sha256"] = core[
                    "artifact_sha256"
                ]
                derivation["output_hashes"]["core_file_sha256"] = opcode_gas.sha256_file(
                    core_path
                )
                derivation["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(
                        {
                            key: value
                            for key, value in derivation.items()
                            if key != "artifact_sha256"
                        }
                    )
                )
                derivation_path.write_text(
                    json.dumps(derivation, indent=2, sort_keys=True) + "\n"
                )
                with self.assertRaises(ValueError):
                    opcode_gas.validate_historical_core_opcode_baseline(target, FIXTURE)


class RelationSubsetTests(unittest.TestCase):
    def setUp(self):
        self.current = opcode_gas.load_manifest(CURRENT_MANIFEST)
        self.historical = opcode_gas.load_manifest(FIXTURE)

    def test_exact_osaka_relation_subsets_are_accepted(self):
        opcode_gas.validate_osaka_relation_subsets(
            self.current,
            self.historical,
            opcode_gas.OSAKA_CANARY_RELATION_IDS,
            opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS,
        )
        self.assertEqual(
            opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS,
            ("opcode:0x15:canonical", "opcode:0x1e:canonical"),
        )

    def test_canary_subset_rejects_identity_drift(self):
        expected = list(opcode_gas.OSAKA_CANARY_RELATION_IDS)
        mutations = (
            expected[:-1],
            list(reversed(expected)),
            expected + [expected[0]],
            expected + ["opcode:0x03:canonical"],
            ["opcode:0x03:canonical", *expected[1:]],
            ["opcode:0xff:unknown", *expected[1:]],
        )
        for relation_ids in mutations:
            with self.subTest(relation_ids=relation_ids), self.assertRaises(ValueError):
                opcode_gas.validate_osaka_relation_subsets(
                    self.current,
                    self.historical,
                    relation_ids,
                    opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS,
                )

    def test_supplement_subset_rejects_any_other_identity(self):
        expected = list(opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS)
        mutations = (
            expected[:-1],
            list(reversed(expected)),
            expected + [expected[0]],
            expected + ["opcode:0x01:canonical"],
            ["opcode:0x16:canonical", expected[1]],
        )
        for relation_ids in mutations:
            with self.subTest(relation_ids=relation_ids), self.assertRaises(ValueError):
                opcode_gas.validate_osaka_relation_subsets(
                    self.current,
                    self.historical,
                    opcode_gas.OSAKA_CANARY_RELATION_IDS,
                    relation_ids,
                )


def canary_rows(*, drifts=None):
    drifts = drifts or ["0"] * len(opcode_gas.OSAKA_CANARY_RELATION_IDS)
    baseline = []
    osaka = []
    for index, (relation_id, drift) in enumerate(
        zip(opcode_gas.OSAKA_CANARY_RELATION_IDS, drifts)
    ):
        relation = {
            "relation_id": relation_id,
            "status": "accepted",
            "slope_p": "100",
            "signed_raw_gas_by_key": {f"opcode:0x{index + 1:02x}": "3"},
            "target_raw_gas_by_key": {f"opcode:0x{index + 1:02x}": "3"},
            "control_raw_gas_by_key": {"opcode:0x90": "3"},
            "program_sha256": f"{index + 1:064x}",
            "raw_rows_sha256": f"{index + 101:064x}",
            "repeat_count": 3,
        }
        baseline.append(relation)
        current = copy.deepcopy(relation)
        current["slope_p"] = opcode_gas._decimal_text(
            opcode_gas.Decimal("100") * (opcode_gas.Decimal("1") + opcode_gas.Decimal(drift))
        )
        osaka.append(current)
    return baseline, osaka


class CompatibilityCanaryTests(unittest.TestCase):
    baseline_hash = "a" * 64

    def evaluate(self, drifts=None):
        baseline, current = canary_rows(drifts=drifts)
        return opcode_gas.build_osaka_compatibility_canary(
            baseline,
            current,
            baseline_artifact_sha256=self.baseline_hash,
            expected_baseline_artifact_sha256=self.baseline_hash,
        )

    def test_identical_and_exact_threshold_slopes_pass(self):
        identical = self.evaluate()
        at_relation_limit = self.evaluate(["0.10", *(["0"] * 10)])
        at_aggregate_limit = self.evaluate(
            [*(["0.10"] * 5), "0.05", *(["0"] * 5)]
        )

        self.assertEqual(identical["status"], "passed")
        self.assertEqual(at_relation_limit["relations"][0]["drift_ape"], "0.1")
        self.assertEqual(at_relation_limit["status"], "passed")
        self.assertEqual(at_aggregate_limit["drift_mape"], "0.05")
        self.assertEqual(at_aggregate_limit["status"], "passed")
        self.assertNotIn("fitted_coefficients", json.dumps(at_aggregate_limit))

    def test_relation_and_aggregate_threshold_excess_fail_closed(self):
        over_relation = self.evaluate(["0.1000000001", *(["0"] * 10)])
        over_aggregate = self.evaluate([*(["0.10"] * 6), *(["0"] * 5)])

        self.assertEqual(over_relation["status"], "failed")
        self.assertIn("relation_drift_ape_exceeds_0.10", over_relation["reasons"])
        self.assertEqual(over_aggregate["status"], "failed")
        self.assertIn("drift_mape_exceeds_0.05", over_aggregate["reasons"])

    def test_canary_records_signed_residual_ape_mape_and_reasons(self):
        artifact = self.evaluate(["-0.05", *(["0"] * 10)])

        self.assertEqual(artifact["relations"][0]["baseline_slope_p"], "100")
        self.assertEqual(artifact["relations"][0]["osaka_slope_p"], "95")
        self.assertEqual(artifact["relations"][0]["signed_residual_p"], "-5")
        self.assertEqual(artifact["relations"][0]["drift_ape"], "0.05")
        self.assertEqual(
            opcode_gas.Decimal(artifact["drift_mape"]) * 11,
            opcode_gas.Decimal("0.05"),
        )
        self.assertEqual(artifact["reasons"], [])
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

    def test_canary_rejects_identity_repeat_and_numeric_failures(self):
        mutation_cases = []
        for label, mutate in (
            ("sign", lambda row: row.__setitem__("slope_p", "-1")),
            ("zero", lambda row: row.__setitem__("slope_p", "0")),
            ("non-finite", lambda row: row.__setitem__("slope_p", "NaN")),
            (
                "raw map",
                lambda row: row["signed_raw_gas_by_key"].__setitem__("opcode:0xff", "1"),
            ),
            ("program", lambda row: row.__setitem__("program_sha256", "f" * 64)),
            ("repeat", lambda row: row.__setitem__("repeat_count", 2)),
            ("status", lambda row: row.__setitem__("status", "quality_rejected")),
        ):
            baseline, current = canary_rows()
            mutate(current[0])
            mutation_cases.append((label, baseline, current))

        for label, baseline, current in mutation_cases:
            with self.subTest(label=label), self.assertRaises(ValueError):
                opcode_gas.build_osaka_compatibility_canary(
                    baseline,
                    current,
                    baseline_artifact_sha256=self.baseline_hash,
                    expected_baseline_artifact_sha256=self.baseline_hash,
                )

        baseline, current = canary_rows()
        with self.assertRaises(ValueError):
            opcode_gas.build_osaka_compatibility_canary(
                baseline,
                current,
                baseline_artifact_sha256=self.baseline_hash,
                expected_baseline_artifact_sha256="b" * 64,
            )


def seal(payload):
    payload = copy.deepcopy(payload)
    payload["artifact_sha256"] = opcode_gas.sha256_bytes(
        opcode_gas.canonical_json(payload)
    )
    return payload


def minimal_augmentation_sources(*, iszero_slope="30", clz_slope="65"):
    baseline = seal(
        {
            "schema_version": 3,
            "purpose": "core_opcode_submodel",
            "status": "supported_core_submodel",
            "candidate_eligible": False,
            "body_scale": "1.25",
            "modeled_named_opcode_count": 101,
            "unsupported_named_opcode_count": 49,
            "unsupported_named_opcode_keys": [
                "opcode:0x15",
                "opcode:0x1e",
                *[f"opcode:0x{i + 32:02x}" for i in range(47)],
            ],
            "unsupported_opcode_reasons": {
                "opcode:0x15": "only_dispatch_dependent_evidence",
                "opcode:0x1e": "outside_core_opcode_scope",
            },
            "registry": {
                "common_dispatch": "12.5",
                "invalid_model_id": "invalid",
                "named_opcode_keys": ["opcode:0x01", "opcode:0x15", "opcode:0x1e", "opcode:0x90"],
                "opcode_model_ids": [None] * 256,
                "models": {
                    "invalid": {"kind": "invalid", "parameters": {}},
                    "opcode:0x01": {
                        "kind": "static_raw_gas",
                        "parameters": {"body_per_raw_gas": "7"},
                    },
                    "opcode:0x20": {
                        "kind": "dynamic_keccak",
                        "parameters": {"constant": "11", "permutation": "13"},
                    },
                    "opcode:0x90": {
                        "kind": "static_raw_gas",
                        "parameters": {"body_per_raw_gas": "20"},
                    },
                },
                "shared_memory_parameters": {"linear": "2", "quadratic": "3"},
            },
        }
    )
    baseline["registry"]["opcode_model_ids"][0x01] = "opcode:0x01"
    baseline["registry"]["opcode_model_ids"][0x90] = "opcode:0x90"
    baseline["artifact_sha256"] = opcode_gas.sha256_bytes(
        opcode_gas.canonical_json(
            {key: value for key, value in baseline.items() if key != "artifact_sha256"}
        )
    )
    canary = seal(
        {
            "schema_version": 1,
            "purpose": "osaka_opcode_compatibility_canary",
            "status": "passed",
            "candidate_eligible": False,
            "relation_ids": list(opcode_gas.OSAKA_CANARY_RELATION_IDS),
            "relations": [
                {
                    "relation_id": relation_id,
                    "historical_raw_rows_sha256": f"{index + 1:064x}",
                    "osaka_raw_rows_sha256": f"{index + 101:064x}",
                }
                for index, relation_id in enumerate(
                    opcode_gas.OSAKA_CANARY_RELATION_IDS
                )
            ],
        }
    )
    supplement = seal(
        {
            "schema_version": 1,
            "purpose": "osaka_opcode_supplement",
            "status": "accepted",
            "candidate_eligible": False,
            "relation_ids": list(opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS),
            "decisions_sha256": "d" * 64,
            "raw_rows_sha256": "e" * 64,
            "relations": [
                {
                    "relation_id": "opcode:0x15:canonical",
                    "slope_p": iszero_slope,
                    "signed_raw_gas_by_key": {"opcode:0x15": "3", "opcode:0x90": "-3"},
                    "control_opcode_key": "opcode:0x90",
                    "raw_rows_sha256": "1" * 64,
                },
                {
                    "relation_id": "opcode:0x1e:canonical",
                    "slope_p": clz_slope,
                    "signed_raw_gas_by_key": {"opcode:0x1e": "5", "opcode:0x90": "-3"},
                    "control_opcode_key": "opcode:0x90",
                    "raw_rows_sha256": "2" * 64,
                },
            ],
        }
    )
    return baseline, canary, supplement


class AugmentedCoreTests(unittest.TestCase):
    def test_builds_corrected_successor_from_immutable_legacy_package(self):
        legacy = ROOT / "experiments" / "opcode-gas" / "derivations" / "f945e67bb2c38c9c8ef50530"
        envelope = json.loads((legacy / "augmentation.json").read_text())
        canary = json.loads((legacy / "compatibility-canary.json").read_text())
        supplement = json.loads((legacy / "opcode-supplement.json").read_text())
        legacy_core = json.loads((legacy / "core-opcode-submodel.json").read_text())

        successor_envelope, corrected_core = (
            opcode_gas.build_corrected_osaka_augmentation_successor(
                envelope, legacy_core, canary, supplement
            )
        )

        self.assertNotEqual(successor_envelope["augmentation_id"], envelope["augmentation_id"])
        self.assertEqual(
            successor_envelope["augmentation_identity"]["analysis_schema_version"],
            opcode_gas.OSAKA_AUGMENTATION_ANALYSIS_SCHEMA_VERSION,
        )
        self.assertEqual(
            successor_envelope["augmentation_identity"]["recovery_formula"],
            opcode_gas.OSAKA_RECOVERY_FORMULA,
        )
        self.assertEqual(
            corrected_core["registry"]["models"]["opcode:0x15"]["parameters"]["body_per_raw_gas"],
            "8.5522130597052856748166153991501006514725859932557162966215147005349215656510073",
        )
        self.assertEqual(
            corrected_core["registry"]["models"]["opcode:0x1e"]["parameters"]["body_per_raw_gas"],
            "14.042917344604667734091737851524231802545087365319455755021222255471935718915789",
        )
        self.assertEqual(
            legacy_core["artifact_sha256"],
            "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b",
        )

    def test_solves_only_iszero_and_clz_and_preserves_baseline_registry(self):
        baseline, canary, supplement = minimal_augmentation_sources()
        original_registry = copy.deepcopy(baseline["registry"])
        artifact = opcode_gas.build_osaka_augmented_core_artifact(
            baseline,
            canary,
            supplement,
            augmentation_provenance={"implementation_revision": "a" * 40},
        )

        models = artifact["registry"]["models"]
        # Relation slopes are measured in the synthetic lab-body basis, while
        # registry bodies are production-scaled.  The fixture deliberately
        # uses body_scale != 1 so omitting that conversion cannot pass.
        self.assertEqual(models["opcode:0x15"]["parameters"]["body_per_raw_gas"], "32.5")
        self.assertEqual(models["opcode:0x1e"]["parameters"]["body_per_raw_gas"], "28.25")
        self.assertEqual(
            {key: value for key, value in models.items() if key not in {"opcode:0x15", "opcode:0x1e"}},
            original_registry["models"],
        )
        self.assertEqual(artifact["registry"]["shared_memory_parameters"], original_registry["shared_memory_parameters"])
        self.assertEqual(artifact["registry"]["common_dispatch"], original_registry["common_dispatch"])
        self.assertEqual(artifact["body_scale"], baseline["body_scale"])
        self.assertEqual(artifact["modeled_named_opcode_count"], 103)
        self.assertEqual(artifact["unsupported_named_opcode_count"], 47)
        self.assertEqual(artifact["candidate_eligible"], False)
        self.assertEqual(
            artifact["osaka_augmentation"]["recovery_formula"],
            opcode_gas.OSAKA_RECOVERY_FORMULA,
        )
        self.assertEqual(artifact["replayed_equations"][0]["signed_residual_p"], "0")
        self.assertEqual(artifact["replayed_equations"][1]["signed_residual_p"], "0")

    def test_legacy_formula_remains_replayable_without_mutating_old_artifacts(self):
        baseline, canary, supplement = minimal_augmentation_sources()
        artifact = opcode_gas.build_osaka_augmented_core_artifact(
            baseline,
            canary,
            supplement,
            augmentation_provenance={"implementation_revision": "a" * 40},
            recovery_formula_version=1,
        )

        models = artifact["registry"]["models"]
        self.assertEqual(models["opcode:0x15"]["parameters"]["body_per_raw_gas"], "30")
        self.assertEqual(models["opcode:0x1e"]["parameters"]["body_per_raw_gas"], "25")
        self.assertNotIn("recovery_formula", artifact["osaka_augmentation"])

    def test_rejects_invalid_solved_bodies_and_relation_identity(self):
        for iszero_slope, clz_slope in (("-61", "65"), ("30", "-101"), ("NaN", "65")):
            baseline, canary, supplement = minimal_augmentation_sources(
                iszero_slope=iszero_slope, clz_slope=clz_slope
            )
            with self.subTest(iszero=iszero_slope, clz=clz_slope), self.assertRaises(ValueError):
                opcode_gas.build_osaka_augmented_core_artifact(
                    baseline,
                    canary,
                    supplement,
                    augmentation_provenance={"implementation_revision": "a" * 40},
                )

    def test_nonterminating_decimal_solution_preserves_exact_fraction_replay(self):
        baseline, canary, supplement = minimal_augmentation_sources(
            iszero_slope="1", clz_slope="1"
        )
        artifact = opcode_gas.build_osaka_augmented_core_artifact(
            baseline,
            canary,
            supplement,
            augmentation_provenance={"implementation_revision": "a" * 40},
        )
        exact = artifact["osaka_augmentation"]["exact_solved_bodies"]
        self.assertEqual(exact["opcode:0x15"]["denominator"], "12")
        self.assertEqual(
            artifact["replayed_equations"][0]["exact_signed_residual_fraction"],
            "0",
        )
        self.assertLess(
            abs(opcode_gas.Decimal(artifact["replayed_equations"][0]["signed_residual_p"])),
            opcode_gas.Decimal("1e-75"),
        )

        for label, mutate in (
            ("equation", lambda supplement: supplement["relations"][0]["signed_raw_gas_by_key"].__setitem__("opcode:0x90", "-2")),
            ("control", lambda supplement: supplement["relations"][1].__setitem__("control_opcode_key", "opcode:0x80")),
        ):
            baseline, canary, supplement = minimal_augmentation_sources()
            mutate(supplement)
            supplement["artifact_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {key: value for key, value in supplement.items() if key != "artifact_sha256"}
                )
            )
            with self.subTest(label=label), self.assertRaises(ValueError):
                opcode_gas.build_osaka_augmented_core_artifact(
                    baseline,
                    canary,
                    supplement,
                    augmentation_provenance={"implementation_revision": "a" * 40},
                )


class OsakaCliContractTests(unittest.TestCase):
    def test_parser_exposes_exact_osaka_supplement_contracts(self):
        parser = opcode_gas.build_parser()
        run = parser.parse_args(
            [
                "run-osaka-opcode-supplement",
                "--run-path-file", "/tmp/run-path",
                "--controlled-manifest", "current.toml",
                "--baseline-derivation", "baseline",
                "--historical-manifest", "historical.toml",
                "--guest-launcher", "guest-launcher",
                "--elf", "opcode.elf",
            ]
        )
        verify = parser.parse_args(
            [
                "verify-osaka-opcode-supplement",
                "--run-path-file", "/tmp/run-path",
                "--controlled-manifest", "current.toml",
                "--baseline-derivation", "baseline",
                "--historical-manifest", "historical.toml",
            ]
        )
        seal_args = parser.parse_args(
            [
                "seal-osaka-opcode-augmentation",
                "--run-path-file", "/tmp/run-path",
                "--baseline-derivation", "baseline",
                "--historical-manifest", "historical.toml",
                "--out-root", "derivations",
                "--augmentation-path-file", "/tmp/augmentation-path",
            ]
        )
        verify_augmentation = parser.parse_args(
            [
                "verify-osaka-opcode-augmentation",
                "--augmentation-path-file", "/tmp/augmentation-path",
                "--historical-manifest", "historical.toml",
            ]
        )
        seal_corrected = parser.parse_args(
            [
                "seal-corrected-osaka-opcode-augmentation",
                "--predecessor-augmentation", "derivations/legacy",
                "--historical-manifest", "historical.toml",
                "--out-root", "derivations",
                "--augmentation-path-file", "/tmp/corrected-augmentation-path",
            ]
        )

        self.assertIs(run.func, opcode_gas.cmd_run_osaka_opcode_supplement)
        self.assertEqual(
            set(vars(run)) - {"command", "func"},
            {"run_path_file", "controlled_manifest", "baseline_derivation", "historical_manifest", "guest_launcher", "elf"},
        )
        self.assertIs(verify.func, opcode_gas.cmd_verify_osaka_opcode_supplement)
        self.assertEqual(
            set(vars(verify)) - {"command", "func"},
            {"run_path_file", "controlled_manifest", "baseline_derivation", "historical_manifest"},
        )
        self.assertIs(seal_args.func, opcode_gas.cmd_seal_osaka_opcode_augmentation)
        self.assertEqual(
            set(vars(seal_args)) - {"command", "func"},
            {"run_path_file", "baseline_derivation", "historical_manifest", "out_root", "augmentation_path_file"},
        )
        self.assertIs(
            verify_augmentation.func,
            opcode_gas.cmd_verify_osaka_opcode_augmentation,
        )
        self.assertEqual(
            set(vars(verify_augmentation)) - {"command", "func"},
            {"augmentation_path_file", "historical_manifest"},
        )
        self.assertIs(
            seal_corrected.func,
            opcode_gas.cmd_seal_corrected_osaka_opcode_augmentation,
        )
        self.assertEqual(
            set(vars(seal_corrected)) - {"command", "func"},
            {
                "predecessor_augmentation",
                "historical_manifest",
                "out_root",
                "augmentation_path_file",
            },
        )

    def test_seal_rejects_conflicting_pointer_before_publication(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            pointer = root / "augmentation-path"
            pointer.write_text("existing\n")
            args = Namespace(
                run_path_file=root / "run-path",
                baseline_derivation=root / "baseline",
                historical_manifest=root / "historical.toml",
                out_root=root / "derivations",
                augmentation_path_file=pointer,
            )
            with mock.patch.object(
                opcode_gas, "_read_durable_directory_path", return_value=root / ("a" * 24)
            ), mock.patch.object(
                opcode_gas,
                "_resolve_repo_path",
                side_effect=[root / "baseline", root / "historical.toml"],
            ), mock.patch.object(
                opcode_gas, "verify_osaka_opcode_supplement_run"
            ) as verify:
                with self.assertRaisesRegex(ValueError, "path file already exists"):
                    opcode_gas.cmd_seal_osaka_opcode_augmentation(args)
            verify.assert_not_called()


class OsakaRunnerTests(unittest.TestCase):
    def test_supplement_verifier_rejects_symlinked_output_root_before_replay(self):
        manifest = opcode_gas.load_manifest(CURRENT_MANIFEST)
        identity = {
            "implementation_revision": "a" * 40,
            "controlled_manifest_sha256": "b" * 64,
            "controlled_manifest_rows_sha256": "c" * 64,
            "complete_schedule_sha256": "d" * 64,
            "guest_artifacts": {
                "crates/guests/elf/sp1_revm_opcode_lab.elf": "e" * 64,
            },
            "version_identity": {
                field: f"value-{field}"
                for field in opcode_gas.CALIBRATION_VERSION_IDENTITY_FIELDS
            },
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = root / ("a" * 24)
            run.mkdir()
            outside = root / "outside"
            outside.mkdir()
            (run / "osaka-opcode-supplement").symlink_to(outside, target_is_directory=True)
            args = Namespace(
                run_path_file=root / "run-path",
                controlled_manifest=CURRENT_MANIFEST,
                baseline_derivation=DERIVATION,
                historical_manifest=FIXTURE,
            )
            with mock.patch.object(opcode_gas, "_read_durable_directory_path", return_value=run), mock.patch.object(
                opcode_gas, "validate_calibration_execution_identity", return_value=identity
            ), mock.patch.object(
                opcode_gas, "_resolve_repo_path", side_effect=[CURRENT_MANIFEST, DERIVATION, FIXTURE]
            ), mock.patch.object(
                opcode_gas, "verify_frozen_controlled_manifest", return_value=(manifest, identity)
            ), mock.patch.object(
                opcode_gas, "validate_calibration_version_identity", return_value=identity["version_identity"]
            ), mock.patch.object(
                opcode_gas, "validate_historical_core_opcode_baseline", return_value={"manifest": manifest}
            ), mock.patch.object(opcode_gas, "_run_osaka_canary_rounds") as replay:
                with self.assertRaisesRegex(ValueError, "output is missing"):
                    opcode_gas.verify_osaka_opcode_supplement_run(args)
            replay.assert_not_called()

    def test_bounded_campaign_replays_without_full_dynamic_preflight(self):
        current = opcode_gas.load_manifest(CURRENT_MANIFEST)
        manifest = opcode_gas._osaka_relation_manifest(
            current, opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS
        )
        provenance = {
            "calibration_id": "a" * 24,
            "calibration_identity_sha256": "a" * 64,
            "implementation_revision": "b" * 40,
            "controlled_manifest_sha256": "c" * 64,
            "controlled_manifest_rows_sha256": "d" * 64,
        }

        def fake_run(args):
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(
                "".join(
                    json.dumps(
                        {
                            "relation_id": relation.id,
                            "generator_max_count": 8,
                        }
                    )
                    + "\n"
                    for relation in manifest.opcode_relations
                )
            )

        def fake_fit(_manifest, _rows, relation_ids, bound, *, expected_provenance):
            self.assertEqual(expected_provenance, provenance)
            return [
                {
                    "relation_id": relation_id,
                    "generator_max_count": bound,
                    "status": "accepted",
                    "decision": "accepted",
                    "fit": {"slope_p": "1"},
                }
                for relation_id in relation_ids
            ]

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            opcode_gas, "cmd_run", side_effect=fake_run
        ), mock.patch.object(
            opcode_gas, "fit_formal_relation_round", side_effect=fake_fit
        ):
            root = pathlib.Path(tmp)
            fixtures = root / "fixtures"
            fixtures.mkdir()
            kwargs = dict(
                calibration_run=root,
                artifact_root=root,
                manifest=manifest,
                fixtures_root=fixtures,
                final_runs=root / "raw" / "formal-relations.jsonl",
                decisions_path=root / "decisions.json",
                decisions_seal_path=root / "decisions.sha256",
                args=Namespace(),
                provenance=provenance,
                validate_dynamic_preflight=False,
            )
            first = opcode_gas.run_formal_relation_adaptive_campaign(**kwargs)
            second = opcode_gas.run_formal_relation_adaptive_campaign(**kwargs)

        self.assertEqual(
            [row["relation_id"] for row in first["rows"]],
            list(opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS),
        )
        self.assertEqual(second["rows"], first["rows"])

    def test_top_level_runner_selects_only_bounded_relations(self):
        current = opcode_gas.load_manifest(CURRENT_MANIFEST)
        historical = opcode_gas.load_manifest(FIXTURE)
        version_identity = {
            field: f"value-{field}"
            for field in opcode_gas.CALIBRATION_VERSION_IDENTITY_FIELDS
        }
        elf = ROOT / "crates" / "guests" / "elf" / "sp1_revm_opcode_lab.elf"
        identity = {
            "implementation_revision": "a" * 40,
            "controlled_manifest_sha256": "b" * 64,
            "controlled_manifest_rows_sha256": "c" * 64,
            "complete_schedule_sha256": "e" * 64,
            "guest_artifacts": {
                "crates/guests/elf/sp1_revm_opcode_lab.elf": opcode_gas.sha256_file(elf)
            },
        }
        baseline = {
            "manifest": historical,
            "relation_artifact": {"artifact_sha256": "d" * 64},
            "derivation": {
                "derivation_identity": {
                    "source": {"source_hashes": {"relation_artifact_sha256": "d" * 64}}
                }
            },
        }
        canary = seal(
            {
                "schema_version": 1,
                "purpose": "osaka_opcode_compatibility_canary",
                "status": "passed",
                "candidate_eligible": False,
            }
        )
        supplement = seal(
            {
                "schema_version": 1,
                "purpose": "osaka_opcode_supplement",
                "status": "accepted",
                "candidate_eligible": False,
            }
        )
        canary_calls = []
        campaign_calls = []

        def fake_canaries(**kwargs):
            canary_calls.append(kwargs)
            return [{"relation_id": relation_id} for relation_id in opcode_gas.OSAKA_CANARY_RELATION_IDS]

        def fake_campaign(**kwargs):
            campaign_calls.append(kwargs)
            kwargs["decisions_path"].write_text("{}\n")
            return {"state": {"accepted": {}}, "rows": []}

        runs_root = ROOT / "experiments" / "opcode-gas" / "runs"
        with tempfile.TemporaryDirectory(dir=runs_root) as run_tmp, tempfile.TemporaryDirectory() as path_tmp:
            run = pathlib.Path(run_tmp)
            run_path = pathlib.Path(path_tmp) / "run.path"
            run_path.write_text(str(run) + "\n")
            args = Namespace(
                run_path_file=run_path,
                controlled_manifest=CURRENT_MANIFEST,
                baseline_derivation=DERIVATION,
                historical_manifest=FIXTURE,
                guest_launcher=ROOT / "target" / "release" / "guest-launcher",
                elf=elf,
            )
            with mock.patch.object(opcode_gas, "validate_calibration_execution_identity", return_value=identity), \
                mock.patch.object(opcode_gas, "verify_frozen_controlled_manifest", return_value=(current, identity)), \
                mock.patch.object(opcode_gas, "validate_calibration_version_identity", return_value=version_identity), \
                mock.patch.object(opcode_gas, "validate_calibration_guest_launcher"), \
                mock.patch.object(opcode_gas, "validate_historical_core_opcode_baseline", return_value=baseline), \
                mock.patch.object(opcode_gas, "_historical_osaka_canary_observations", return_value=[]), \
                mock.patch.object(opcode_gas, "_run_osaka_canary_rounds", side_effect=fake_canaries), \
                mock.patch.object(opcode_gas, "build_osaka_compatibility_canary", return_value=canary), \
                mock.patch.object(opcode_gas, "generate_relation_cases"), \
                mock.patch.object(opcode_gas, "run_formal_relation_adaptive_campaign", side_effect=fake_campaign), \
                mock.patch.object(opcode_gas, "_build_osaka_supplement_artifact", return_value=supplement), \
                mock.patch.object(opcode_gas, "cmd_run_proposal") as proposal, \
                mock.patch.object(opcode_gas, "cmd_run_controlled") as controlled:
                opcode_gas.cmd_run_osaka_opcode_supplement(args)

            self.assertEqual(
                tuple(relation.id for relation in canary_calls[0]["manifest"].opcode_relations),
                tuple(relation.id for relation in current.opcode_relations),
            )
            self.assertEqual(
                tuple(relation.id for relation in campaign_calls[0]["manifest"].opcode_relations),
                opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS,
            )
            self.assertEqual(
                campaign_calls[0]["final_runs"],
                run / "osaka-opcode-supplement" / "raw" / "formal-relations.jsonl",
            )
            self.assertNotEqual(campaign_calls[0]["final_runs"], run / "raw" / "formal-relations.jsonl")
            self.assertEqual(
                campaign_calls[0]["args"].elf,
                pathlib.Path("crates/guests/elf/sp1_revm_opcode_lab.elf"),
            )
            proposal.assert_not_called()
            controlled.assert_not_called()

    def test_fixed_canaries_reuse_historical_counts_and_resume_by_replay(self):
        manifest = opcode_gas.load_manifest(CURRENT_MANIFEST)
        bounds = {
            relation_id: bound
            for relation_id, bound in zip(
                opcode_gas.OSAKA_CANARY_RELATION_IDS,
                (128, 128, 128, 32, 8, 128, 128, 8, 8, 32, 512),
            )
        }
        historical = [
            {
                "relation_id": relation_id,
                "generator_max_count": bounds[relation_id],
            }
            for relation_id in opcode_gas.OSAKA_CANARY_RELATION_IDS
        ]
        provenance = {
            "calibration_id": "a" * 24,
            "calibration_identity_sha256": "a" * 64,
            "implementation_revision": "b" * 40,
            "controlled_manifest_sha256": "c" * 64,
            "controlled_manifest_rows_sha256": "d" * 64,
        }
        version_identity = {
            field: f"value-{field}"
            for field in opcode_gas.CALIBRATION_VERSION_IDENTITY_FIELDS
        }
        generated = []
        executed = []

        def fake_generate(_manifest, fixtures, *, provenance, generator_max_count, relation_ids):
            fixtures.mkdir(parents=True)
            generated.append((generator_max_count, tuple(relation_ids)))
            return []

        def fake_run(args):
            executed.append((args.out, args.repeats, args.expected_purpose))
            relation_ids = next(ids for bound, ids in generated if f"max-{bound}" in str(args.fixtures))
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(
                "".join(json.dumps({"relation_id": relation_id}) + "\n" for relation_id in relation_ids)
            )

        def fake_fit(_manifest, rows, relation_ids, bound, *, expected_provenance):
            self.assertEqual({row["relation_id"] for row in rows}, set(relation_ids))
            self.assertEqual(expected_provenance, provenance)
            return [
                {
                    "relation_id": relation_id,
                    "generator_max_count": bound,
                    "status": "accepted",
                    "decision": "accepted",
                    "reasons": [],
                    "fit": {"slope_p": "100"},
                }
                for relation_id in relation_ids
            ]

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            opcode_gas, "generate_relation_cases", side_effect=fake_generate
        ), mock.patch.object(opcode_gas, "cmd_run", side_effect=fake_run), mock.patch.object(
            opcode_gas, "fit_formal_relation_round", side_effect=fake_fit
        ), mock.patch.object(
            opcode_gas, "_formal_relation_program_sha256", return_value="f" * 64
        ):
            root = pathlib.Path(tmp)
            kwargs = dict(
                calibration_run=root,
                output_root=root / "osaka-opcode-supplement",
                manifest=manifest,
                historical_observations=historical,
                provenance=provenance,
                version_identity=version_identity,
                args=Namespace(
                    guest_launcher=root / "guest-launcher",
                    elf=root / "opcode.elf",
                    controlled_manifest=CURRENT_MANIFEST,
                ),
            )
            first = opcode_gas._run_osaka_canary_rounds(**kwargs)
            second = opcode_gas._run_osaka_canary_rounds(**kwargs)

        self.assertEqual([row["relation_id"] for row in first], list(opcode_gas.OSAKA_CANARY_RELATION_IDS))
        self.assertEqual(first, second)
        self.assertEqual([bound for bound, _ids in generated], [8, 32, 128, 512])
        self.assertEqual({item for _bound, ids in generated for item in ids}, set(opcode_gas.OSAKA_CANARY_RELATION_IDS))
        self.assertTrue(all(repeats == 3 for _path, repeats, _purpose in executed))
        self.assertTrue(all(purpose == opcode_gas.FORMAL_RELATION_PURPOSE for _path, _repeats, purpose in executed))
        self.assertEqual(len(executed), 4)

    def test_canary_resume_rejects_changed_persisted_raw_hash(self):
        manifest = opcode_gas.load_manifest(CURRENT_MANIFEST)
        historical = [
            {"relation_id": relation_id, "generator_max_count": 8}
            for relation_id in opcode_gas.OSAKA_CANARY_RELATION_IDS
        ]
        provenance = {
            "calibration_id": "a" * 24,
            "calibration_identity_sha256": "a" * 64,
            "implementation_revision": "b" * 40,
            "controlled_manifest_sha256": "c" * 64,
            "controlled_manifest_rows_sha256": "d" * 64,
        }
        version_identity = {
            field: f"value-{field}"
            for field in opcode_gas.CALIBRATION_VERSION_IDENTITY_FIELDS
        }
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            output = root / "osaka-opcode-supplement"

            def fake_generate(_manifest, fixtures, **_kwargs):
                fixtures.mkdir(parents=True)

            def fake_run(args):
                args.out.parent.mkdir(parents=True, exist_ok=True)
                args.out.write_text(
                    "".join(
                        json.dumps({"relation_id": relation_id}) + "\n"
                        for relation_id in opcode_gas.OSAKA_CANARY_RELATION_IDS
                    )
                )

            fit = [
                {"relation_id": relation_id, "status": "accepted", "decision": "accepted", "fit": {"slope_p": "100"}}
                for relation_id in opcode_gas.OSAKA_CANARY_RELATION_IDS
            ]
            patches = (
                mock.patch.object(opcode_gas, "generate_relation_cases", side_effect=fake_generate),
                mock.patch.object(opcode_gas, "cmd_run", side_effect=fake_run),
                mock.patch.object(opcode_gas, "fit_formal_relation_round", return_value=fit),
                mock.patch.object(opcode_gas, "_formal_relation_program_sha256", return_value="f" * 64),
            )
            with patches[0], patches[1], patches[2], patches[3]:
                opcode_gas._run_osaka_canary_rounds(
                    calibration_run=root,
                    output_root=output,
                    manifest=manifest,
                    historical_observations=historical,
                    provenance=provenance,
                    version_identity=version_identity,
                    args=Namespace(guest_launcher=root / "guest", elf=root / "elf", controlled_manifest=CURRENT_MANIFEST),
                )
                def replay(candidate):
                    return opcode_gas._run_osaka_canary_rounds(
                        calibration_run=root,
                        output_root=candidate,
                        manifest=manifest,
                        historical_observations=historical,
                        provenance=provenance,
                        version_identity=version_identity,
                        args=Namespace(guest_launcher=root / "guest", elf=root / "elf", controlled_manifest=CURRENT_MANIFEST),
                        allow_execution=False,
                    )

                for label, path_value in (("absolute", "/tmp/escape.jsonl"), ("parent", "../escape.jsonl")):
                    candidate = root / f"bad-{label}"
                    shutil.copytree(output, candidate)
                    decisions_path = candidate / "canary-decisions.json"
                    decisions = json.loads(decisions_path.read_text())
                    decisions["rounds"][0]["raw_runs"] = path_value
                    decisions_path.write_text(json.dumps(decisions, indent=2, sort_keys=True) + "\n")
                    (candidate / "canary-decisions.sha256").write_text(
                        opcode_gas.sha256_file(decisions_path) + "\n"
                    )
                    with self.subTest(path=label), self.assertRaisesRegex(ValueError, "source differs"):
                        replay(candidate)

                candidate = root / "bad-symlink"
                shutil.copytree(output, candidate)
                raw_link = candidate / "raw" / "canary.generator-max-8.jsonl"
                bypass = root / "outside.jsonl"
                shutil.copy2(raw_link, bypass)
                raw_link.unlink()
                raw_link.symlink_to(bypass)
                with self.assertRaisesRegex(ValueError, "source differs"):
                    replay(candidate)
                raw = output / "raw" / "canary.generator-max-8.jsonl"
                raw.write_bytes(raw.read_bytes() + b"{}\n")
                with self.assertRaisesRegex(ValueError, "source differs"):
                    opcode_gas._run_osaka_canary_rounds(
                        calibration_run=root,
                        output_root=output,
                    manifest=manifest,
                        historical_observations=historical,
                        provenance=provenance,
                        version_identity=version_identity,
                        args=Namespace(guest_launcher=root / "guest", elf=root / "elf", controlled_manifest=CURRENT_MANIFEST),
                        allow_execution=False,
                    )


class AugmentationSealTests(unittest.TestCase):
    def _build_current_campaign(self, root):
        output = root / "current-source"
        identity_path = root / "execution-identity.json"
        if identity_path.is_file():
            identity = json.loads(identity_path.read_text())
            provenance = {
                "calibration_id": opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(identity)
                )[:24],
                "calibration_identity_sha256": opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(identity)
                ),
                "implementation_revision": identity["implementation_revision"],
                "controlled_manifest_sha256": identity[
                    "controlled_manifest_sha256"
                ],
                "controlled_manifest_rows_sha256": identity[
                    "controlled_manifest_rows_sha256"
                ],
                "complete_schedule_sha256": identity["complete_schedule_sha256"],
                "guest_elf_sha256": identity["guest_artifacts"][
                    "crates/guests/elf/sp1_revm_opcode_lab.elf"
                ],
                "version_identity": identity["version_identity"],
            }
            return output, provenance, identity

        output.mkdir()
        controlled_manifest = output / "controlled-manifest.toml"
        shutil.copy2(CURRENT_MANIFEST, controlled_manifest)
        version_identity = {
            "taiko_fork": "Unzen",
            "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
            "ethereum_upgrade": "Fusaka",
            "revm_spec_id": "OSAKA",
            "proving_backend": "sp1",
            "primary_metric": "proverGas",
        }
        guest_artifacts = {
            str(path.relative_to(ROOT)): opcode_gas.sha256_file(path)
            for path in sorted((ROOT / "crates" / "guests" / "elf").glob("sp1*"))
            if path.is_file()
            and (path.name.endswith(".elf") or path.name.endswith(".vk.bin"))
        }
        identity = {
            "implementation_revision": "b" * 40,
            "alethia_reth_revision": "c" * 40,
            "rust_version": "rustc test",
            "sp1_sdk_version": "test-sdk",
            "controlled_manifest_sha256": opcode_gas.sha256_file(
                controlled_manifest
            ),
            "controlled_manifest_rows_sha256": opcode_gas.controlled_manifest_rows_sha256(
                controlled_manifest
            ),
            "complete_schedule_sha256": "e" * 64,
            "guest_artifacts": guest_artifacts,
            "guest_artifacts_sha256": opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(guest_artifacts)
            ),
            "guest_launcher_sha256": "9" * 64,
            "normalization_reference_key": "opcode:0x01",
            "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
            "primary_metric": "proverGas",
            "sp1_instruction_count": "secondary_non_gating",
            "workload_identity_schema_version": 1,
            "workload_canonicalization": "sha256(canonical_json(workload_spec))",
            "primary_formulas": {"candidate_cost": "g_p(k) / r(k)"},
            "q_formula": list(opcode_gas.Q_FORMULA),
            "out_of_fit_checkpoint": {
                "mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS
            },
            "quality_gates": {"checkpoint_ape_max": 0.10},
            "bridge": {"model": "through_origin_equal_key_median"},
            "version_identity": version_identity,
        }
        calibration_identity_sha256 = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(identity)
        )
        provenance = {
            "calibration_id": calibration_identity_sha256[:24],
            "calibration_identity_sha256": calibration_identity_sha256,
            "implementation_revision": identity["implementation_revision"],
            "controlled_manifest_sha256": identity[
                "controlled_manifest_sha256"
            ],
            "controlled_manifest_rows_sha256": identity[
                "controlled_manifest_rows_sha256"
            ],
            "complete_schedule_sha256": identity["complete_schedule_sha256"],
            "guest_elf_sha256": guest_artifacts[
                "crates/guests/elf/sp1_revm_opcode_lab.elf"
            ],
            "version_identity": version_identity,
        }
        formal_provenance = {
            field: provenance[field]
            for field in opcode_gas.FORMAL_RELATION_PROVENANCE_FIELDS
        }
        manifest = opcode_gas.load_manifest(controlled_manifest)
        baseline = opcode_gas.validate_historical_core_opcode_baseline(
            DERIVATION, FIXTURE
        )
        historical_rows = opcode_gas._historical_osaka_canary_observations(
            baseline
        )
        historical_by_id = {
            row["relation_id"]: row for row in historical_rows
        }
        canary_rows_by_bound = {}
        for bound in sorted(
            {row["generator_max_count"] for row in historical_rows}
        ):
            relation_ids = [
                relation.id
                for relation in manifest.opcode_relations
                if historical_by_id.get(relation.id, {}).get(
                    "generator_max_count"
                )
                == bound
            ]
            subset = opcode_gas._osaka_relation_manifest(manifest, relation_ids)
            canary_rows_by_bound[bound] = canonical_formal_relation_round_rows(
                subset,
                slope_overrides={
                    relation_id: historical_by_id[relation_id]["slope_p"]
                    for relation_id in relation_ids
                },
                generator_max_count=bound,
                provenance=formal_provenance,
            )

        def fake_generate(_manifest, fixtures, **_kwargs):
            fixtures.mkdir(parents=True)

        def fake_canary_run(args):
            bound = int(args.out.stem.rsplit("-", 1)[1])
            opcode_gas._write_canonical_jsonl(
                args.out, canary_rows_by_bound[bound]
            )

        with mock.patch.object(
            opcode_gas, "generate_relation_cases", side_effect=fake_generate
        ), mock.patch.object(opcode_gas, "cmd_run", side_effect=fake_canary_run):
            current_rows = opcode_gas._run_osaka_canary_rounds(
                calibration_run=root,
                output_root=output,
                manifest=manifest,
                historical_observations=historical_rows,
                provenance=formal_provenance,
                version_identity=version_identity,
                args=Namespace(
                    guest_launcher=root / "guest-launcher",
                    elf=root / "opcode.elf",
                    controlled_manifest=controlled_manifest,
                ),
            )
        relation_artifact_sha256 = baseline["relation_artifact"][
            "artifact_sha256"
        ]
        canary = opcode_gas.build_osaka_compatibility_canary(
            historical_rows,
            current_rows,
            baseline_artifact_sha256=relation_artifact_sha256,
            expected_baseline_artifact_sha256=relation_artifact_sha256,
        )
        canary["provenance"] = copy.deepcopy(provenance)
        canary["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(
                {
                    key: value
                    for key, value in canary.items()
                    if key != "artifact_sha256"
                }
            )
        )
        opcode_gas._atomic_write_json(output / "compatibility-canary.json", canary)

        supplement_manifest = opcode_gas._osaka_relation_manifest(
            manifest, opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS
        )
        supplement_rows = canonical_formal_relation_round_rows(
            supplement_manifest,
            slope_overrides={
                "opcode:0x15:canonical": "5000",
                "opcode:0x1e:canonical": "6000",
            },
            generator_max_count=8,
            provenance=formal_provenance,
        )
        fixtures_root = output / "fixtures" / "supplement"
        fixtures_root.mkdir(parents=True)

        def fake_supplement_run(args):
            opcode_gas._write_canonical_jsonl(args.out, supplement_rows)

        with mock.patch.object(opcode_gas, "cmd_run", side_effect=fake_supplement_run):
            campaign = opcode_gas.run_formal_relation_adaptive_campaign(
                calibration_run=root,
                artifact_root=output,
                manifest=supplement_manifest,
                fixtures_root=fixtures_root,
                final_runs=output / "raw" / "formal-relations.jsonl",
                decisions_path=output / "decisions.json",
                decisions_seal_path=output / "decisions.sha256",
                args=Namespace(),
                provenance=formal_provenance,
                validate_dynamic_preflight=False,
            )
        supplement = opcode_gas._build_osaka_supplement_artifact(
            supplement_manifest,
            campaign,
            formal_provenance,
            provenance,
            opcode_gas.sha256_file(output / "decisions.json"),
        )
        opcode_gas._atomic_write_json(output / "opcode-supplement.json", supplement)
        identity_path.write_text(json.dumps(identity, indent=2, sort_keys=True) + "\n")
        return output, provenance, identity

    def _seal(
        self,
        root,
        mutate_canary=None,
        mutate_supplement=None,
        mutate_identity=None,
    ):
        output, provenance, identity = self._build_current_campaign(root)
        canary_path = output / "compatibility-canary.json"
        supplement_path = output / "opcode-supplement.json"
        if mutate_canary is not None:
            canary = json.loads(canary_path.read_text())
            mutate_canary(canary)
            canary["artifact_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in canary.items()
                        if key != "artifact_sha256"
                    }
                )
            )
            canary_path.write_bytes(opcode_gas._canonical_json_file_bytes(canary))
        if mutate_supplement is not None:
            supplement = json.loads(supplement_path.read_text())
            mutate_supplement(supplement)
            supplement["artifact_sha256"] = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in supplement.items()
                        if key != "artifact_sha256"
                    }
                )
            )
            supplement_path.write_bytes(
                opcode_gas._canonical_json_file_bytes(supplement)
            )
        if mutate_identity is not None:
            mutate_identity(identity)
        return opcode_gas.seal_osaka_augmentation_directory(
            root / "derivations",
            baseline_derivation_path=DERIVATION / "derivation.json",
            baseline_dynamic_path=DERIVATION / "dynamic-opcode-models.json",
            baseline_core_path=DERIVATION / "core-opcode-submodel.json",
            canary_path=canary_path,
            supplement_path=supplement_path,
            provenance=provenance,
            historical_manifest=FIXTURE,
            historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
            controlled_manifest=output / "controlled-manifest.toml",
            execution_identity=identity,
        )

    def test_create_only_seal_and_directory_only_replay(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sealed = self._seal(root)
            verified = opcode_gas.verify_osaka_augmentation_directory(
                pathlib.Path(sealed["directory"]),
                historical_manifest=FIXTURE,
                expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
            )

            self.assertEqual(verified["augmentation_id"], sealed["augmentation_id"])
            self.assertEqual(
                sorted(path.name for path in pathlib.Path(sealed["directory"]).iterdir()),
                ["augmentation.json", "compatibility-canary.json", "core-opcode-submodel.json", "opcode-supplement.json"],
            )
            # If publication succeeded but the separate path-pointer write
            # failed, replaying the exact sources must recover the sealed
            # directory rather than making the operator regenerate it.
            self.assertEqual(self._seal(root), sealed)

            canary_path = pathlib.Path(sealed["directory"]) / "compatibility-canary.json"
            changed = json.loads(canary_path.read_text())
            changed["status"] = "failed"
            canary_path.write_text(json.dumps(changed, indent=2, sort_keys=True) + "\n")
            with self.assertRaises(ValueError):
                opcode_gas.verify_osaka_augmentation_directory(
                    pathlib.Path(sealed["directory"]),
                    historical_manifest=FIXTURE,
                    expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
                )

    def test_directory_verifier_rejects_symlinked_directory_and_json_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sealed = self._seal(root)
            directory = pathlib.Path(sealed["directory"])
            outside = root / "outside-package"
            directory.rename(outside)
            directory.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "directory inventory"):
                opcode_gas.verify_osaka_augmentation_directory(
                    directory,
                    historical_manifest=FIXTURE,
                    expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
                )

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sealed = self._seal(root)
            directory = pathlib.Path(sealed["directory"])
            child = directory / "compatibility-canary.json"
            outside = root / "outside-canary.json"
            shutil.copy2(child, outside)
            child.unlink()
            child.symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "directory inventory"):
                opcode_gas.verify_osaka_augmentation_directory(
                    directory,
                    historical_manifest=FIXTURE,
                    expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
                )

    def test_replay_rejects_every_bound_source_and_version_mutation(self):
        mutations = (
            ("augmentation.json", lambda value: value["replay_inputs"]["baseline_core"].__setitem__("body_scale", "2")),
            ("augmentation.json", lambda value: value["replay_inputs"]["baseline_core"]["registry"].__setitem__("common_dispatch", "99")),
            ("augmentation.json", lambda value: value["replay_inputs"]["baseline_core"]["registry"]["models"]["opcode:0x90"]["parameters"].__setitem__("body_per_raw_gas", "21")),
            ("augmentation.json", lambda value: value["replay_inputs"]["baseline_core"]["registry"]["models"]["opcode:0x01"]["parameters"].__setitem__("body_per_raw_gas", "8")),
            ("augmentation.json", lambda value: value["augmentation_identity"]["osaka_calibration"].__setitem__("historical_manifest_sha256", "0" * 64)),
            ("augmentation.json", lambda value: value["augmentation_identity"]["osaka_calibration"].__setitem__("guest_elf_sha256", "0" * 64)),
            ("augmentation.json", lambda value: value["augmentation_identity"]["osaka_calibration"].__setitem__("implementation_revision", "0" * 40)),
            ("augmentation.json", lambda value: value["augmentation_identity"]["osaka_calibration"]["version_identity"].pop("ethereum_upgrade")),
            ("compatibility-canary.json", lambda value: value.__setitem__("status", "failed")),
            ("opcode-supplement.json", lambda value: value["relations"][0].__setitem__("slope_p", "31")),
            ("opcode-supplement.json", lambda value: value["relations"][0]["signed_raw_gas_by_key"].__setitem__("opcode:0x90", "-2")),
            ("opcode-supplement.json", lambda value: value["relations"][1].__setitem__("control_opcode_key", "opcode:0x80")),
            ("opcode-supplement.json", lambda value: value.__setitem__("raw_rows_sha256", "0" * 64)),
        )
        for filename, mutate in mutations:
            with self.subTest(filename=filename), tempfile.TemporaryDirectory() as tmp:
                sealed = self._seal(pathlib.Path(tmp))
                directory = pathlib.Path(sealed["directory"])
                path = directory / filename
                payload = json.loads(path.read_text())
                mutate(payload)
                path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                with self.assertRaises(ValueError):
                    opcode_gas.verify_osaka_augmentation_directory(
                        directory,
                        historical_manifest=FIXTURE,
                        expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
                    )

    def test_seal_rejects_resealed_historical_canary_slope_and_program_drift(self):
        def mutate_slope(canary):
            row = canary["relations"][0]
            row["baseline_slope_p"] = "999"
            row["osaka_slope_p"] = "999"
            row["signed_residual_p"] = "0"
            row["drift_ape"] = "0"

        mutations = (
            mutate_slope,
            lambda canary: canary["relations"][0].__setitem__("program_sha256", "0" * 64),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    self._seal(pathlib.Path(tmp), mutate_canary=mutate)

    def test_seal_rejects_fully_resealed_current_canary_contract_drift(self):
        mutations = (
            lambda value: value.__setitem__("schema_version", 2),
            lambda value: value.__setitem__("baseline_artifact_sha256", "0" * 64),
            lambda value: value["quality_gates"].__setitem__(
                "per_relation_drift_ape_max", "0.20"
            ),
            lambda value: value["relations"][0].__setitem__(
                "osaka_raw_rows_sha256", "0" * 64
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    self._seal(pathlib.Path(tmp), mutate_canary=mutate)

    def test_seal_rejects_fully_resealed_supplement_contract_drift(self):
        mutations = (
            lambda value: value.__setitem__("schema_version", 2),
            lambda value: value.__setitem__("decisions_sha256", "0" * 64),
            lambda value: value.__setitem__("raw_rows_sha256", "0" * 64),
            lambda value: value["relations"][0].__setitem__(
                "raw_rows_sha256", "0" * 64
            ),
            lambda value: value["relations"][0].__setitem__("slope_p", "5001"),
            lambda value: value["relations"][0].__setitem__(
                "program_sha256", "0" * 64
            ),
            lambda value: value["relations"][0].__setitem__(
                "generator_max_count", 32
            ),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    self._seal(pathlib.Path(tmp), mutate_supplement=mutate)

    def test_seal_rejects_execution_identity_map_and_launcher_drift(self):
        mutations = (
            lambda value: value["guest_artifacts"].__setitem__(
                "crates/guests/elf/sp1_revm_opcode_lab.elf", "0" * 64
            ),
            lambda value: value.__setitem__("guest_launcher_sha256", "0" * 64),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate), tempfile.TemporaryDirectory() as tmp:
                with self.assertRaises(ValueError):
                    self._seal(pathlib.Path(tmp), mutate_identity=mutate)

if __name__ == "__main__":
    unittest.main()
