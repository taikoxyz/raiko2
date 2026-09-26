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
        self.assertEqual(models["opcode:0x15"]["parameters"]["body_per_raw_gas"], "30")
        self.assertEqual(models["opcode:0x1e"]["parameters"]["body_per_raw_gas"], "25")
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
        self.assertEqual(artifact["replayed_equations"][0]["signed_residual_p"], "0")
        self.assertEqual(artifact["replayed_equations"][1]["signed_residual_p"], "0")

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


class OsakaRunnerTests(unittest.TestCase):
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
            )
            with patches[0], patches[1], patches[2]:
                opcode_gas._run_osaka_canary_rounds(
                    calibration_run=root,
                    output_root=output,
                    manifest=manifest,
                    historical_observations=historical,
                    provenance=provenance,
                    version_identity=version_identity,
                    args=Namespace(guest_launcher=root / "guest", elf=root / "elf", controlled_manifest=CURRENT_MANIFEST),
                )
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
    def _seal(self, root):
        baseline, canary, supplement = minimal_augmentation_sources()
        derivation = seal(
            {
                "schema_version": 1,
                "purpose": "core_opcode_postprocess_derivation",
                "derivation_id": "baseline-test",
                "candidate_eligible": False,
            }
        )
        dynamic = seal(
            {
                "schema_version": 4,
                "purpose": "dynamic_opcode_models",
                "candidate_eligible": False,
            }
        )
        version_identity = {
            "taiko_fork": "Unzen",
            "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
            "ethereum_upgrade": "Fusaka",
            "revm_spec_id": "OSAKA",
            "proving_backend": "sp1",
            "primary_metric": "proverGas",
        }
        provenance = {
            "calibration_id": "a" * 24,
            "calibration_identity_sha256": "a" * 64,
            "implementation_revision": "b" * 40,
            "controlled_manifest_sha256": "c" * 64,
            "controlled_manifest_rows_sha256": "d" * 64,
            "complete_schedule_sha256": "e" * 64,
            "guest_elf_sha256": "f" * 64,
            "version_identity": version_identity,
        }
        paths = {}
        for name, payload in (
            ("derivation", derivation),
            ("dynamic", dynamic),
            ("core", baseline),
            ("canary", canary),
            ("supplement", supplement),
        ):
            path = root / f"{name}.json"
            path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            paths[name] = path
        return opcode_gas.seal_osaka_augmentation_directory(
            root / "derivations",
            baseline_derivation_path=paths["derivation"],
            baseline_dynamic_path=paths["dynamic"],
            baseline_core_path=paths["core"],
            canary_path=paths["canary"],
            supplement_path=paths["supplement"],
            provenance=provenance,
            historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
        )

    def test_create_only_seal_and_directory_only_replay(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            sealed = self._seal(root)
            verified = opcode_gas.verify_osaka_augmentation_directory(
                pathlib.Path(sealed["directory"]),
                expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
            )

            self.assertEqual(verified["augmentation_id"], sealed["augmentation_id"])
            self.assertEqual(
                sorted(path.name for path in pathlib.Path(sealed["directory"]).iterdir()),
                ["augmentation.json", "compatibility-canary.json", "core-opcode-submodel.json", "opcode-supplement.json"],
            )
            with self.assertRaisesRegex(ValueError, "already exists"):
                self._seal(root)

            canary_path = pathlib.Path(sealed["directory"]) / "compatibility-canary.json"
            changed = json.loads(canary_path.read_text())
            changed["status"] = "failed"
            canary_path.write_text(json.dumps(changed, indent=2, sort_keys=True) + "\n")
            with self.assertRaises(ValueError):
                opcode_gas.verify_osaka_augmentation_directory(
                    pathlib.Path(sealed["directory"]),
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
                        expected_historical_manifest_sha256=opcode_gas.HISTORICAL_CORE_MANIFEST_SHA256,
                    )

if __name__ == "__main__":
    unittest.main()
