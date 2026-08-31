from __future__ import annotations

import argparse
import copy
import hashlib
import json
import importlib.util
import math
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "scripts" / "modeling" / "risc0_zkgas_model.py"
FIXTURE_DIR = (
    ROOT
    / "tests"
    / "fixtures"
    / "risc0-zkgas"
    / "2026-08-31-m2-global-cap-v2"
)
MODEL = ROOT / "crates" / "prover" / "models" / "risc0-zkgas.json"
MODULE_SPEC = importlib.util.spec_from_file_location("risc0_zkgas_model", SCRIPT)
assert MODULE_SPEC and MODULE_SPEC.loader
MODEL_TOOL = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(MODEL_TOOL)


class Risc0ZkGasModelCliTests(unittest.TestCase):
    def run_cli(self, *arguments: object) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, SCRIPT, *map(str, arguments)],
            cwd=ROOT,
            check=False,
            capture_output=True,
            text=True,
        )

    def write_collectors(self, directory: pathlib.Path, *, perturb_fit=False):
        hoodi = directory / "hoodi-samples.jsonl"
        mainnet = directory / "mainnet-samples.jsonl"
        config = json.loads((FIXTURE_DIR / "config.json").read_text())
        provenance = config["proposal"]["provenance"]
        approved_hashes = config["proposal"]["collector_artifact_hashes"]
        rows = [
            json.loads(line)
            for name in ("hoodi-fit.jsonl", "validation.jsonl")
            for line in (FIXTURE_DIR / name).read_text().splitlines()
        ]
        if perturb_fit:
            next(row for row in rows if row["split"] == "fit")["actual_mcycles"] += 100

        def collector_row(row):
            artifact_hashes = {
                **approved_hashes["shared"],
                "chain_spec_sha256": approved_hashes["chain_spec_sha256_by_network"][
                    row["network"]
                ],
            }
            return {
                **{key: value for key, value in row.items() if key != "actual_mcycles"},
                "split": "holdout" if row["network"] == "taiko_mainnet" else row["split"],
                "status": "success",
                "schema_version": 1,
                "attempt": 1,
                "stratum": None,
                "unzen_timestamp": MODEL_TOOL.UNZEN_TIMESTAMPS[row["network"]],
                "sample_key": (
                    f"{row['network']}:{row['proposal_id']}:{provenance['image_id']}"
                ),
                "evaluated_mcycles_count": row["actual_mcycles"],
                "risc0_user_cycles": row["actual_mcycles"] * 1_000_000,
                "source_revision": provenance["source_revision"],
                "image_id": provenance["image_id"],
                "risc0_image_id": provenance["image_id"],
                "risc0_version": provenance["risc0_version"],
                "execution_po2": provenance["min_execution_po2"],
                "artifact_hashes": artifact_hashes,
            }

        for path, network in (
            (hoodi, "taiko_hoodi"),
            (mainnet, "taiko_mainnet"),
        ):
            path.write_text(
                "".join(
                    json.dumps(collector_row(row), separators=(",", ":")) + "\n"
                    for row in rows
                    if row["network"] == network
                )
            )
        return hoodi, mainnet

    def test_check_reproduces_the_committed_runtime_model(self):
        result = self.run_cli("check")

        self.assertEqual(
            result.returncode,
            0,
            result.stderr or result.stdout,
        )
        self.assertIn("runtime model matches generated artifact", result.stdout)

    def test_fit_m2_reductions_are_explicit_left_folds(self):
        rows = [
            json.loads(line)
            for line in (FIXTURE_DIR / "hoodi-fit.jsonl").read_text().splitlines()
        ]
        expected = {
            name: Decimal(value)
            for name, value in json.loads(MODEL.read_text())["proposal"][
                "coefficients"
            ]["decimal"].items()
        }

        with mock.patch(
            "builtins.sum",
            side_effect=lambda values, start=0: math.fsum(values) + start,
        ):
            actual = MODEL_TOOL.fit_m2(rows)

        self.assertEqual(actual, expected)

    def test_artifact_bytes_are_canonical_across_key_order(self):
        first = {"z": {"b": 2, "a": 1}, "a": 0}
        reordered = {"a": 0, "z": {"a": 1, "b": 2}}

        self.assertEqual(
            MODEL_TOOL.artifact_bytes(first),
            MODEL_TOOL.artifact_bytes(reordered),
        )

    def test_check_rejects_noncanonical_fit_fixture_bytes(self):
        variants = {}
        original_rows = [
            json.loads(line)
            for line in (FIXTURE_DIR / "hoodi-fit.jsonl").read_text().splitlines()
        ]
        whitespace = (FIXTURE_DIR / "hoodi-fit.jsonl").read_bytes().replace(
            b'{"network"', b' {"network"', 1
        )
        variants["whitespace"] = whitespace
        junk_rows = copy.deepcopy(original_rows)
        junk_rows[0]["junk"] = True
        variants["junk field"] = b"".join(
            (json.dumps(row, separators=(",", ":")) + "\n").encode()
            for row in junk_rows
        )
        variants["failure row"] = (
            (FIXTURE_DIR / "hoodi-fit.jsonl").read_bytes()
            + b'{"status":"failure"}\n'
        )

        for label, payload in variants.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = pathlib.Path(directory) / "fixture"
                shutil.copytree(FIXTURE_DIR, fixture)
                (fixture / "hoodi-fit.jsonl").write_bytes(payload)

                result = self.run_cli(
                    "check", "--fixture-dir", fixture, "--model", MODEL
                )

                self.assertEqual(result.returncode, 2, result.stderr or result.stdout)
                self.assertNotIn("Traceback", result.stderr)

    def test_check_binds_both_canonical_fixture_payloads_to_raw_input_hash(self):
        artifact = json.loads(MODEL.read_text())
        expected_hash = hashlib.sha256(
            (FIXTURE_DIR / "hoodi-fit.jsonl").read_bytes()
            + (FIXTURE_DIR / "validation.jsonl").read_bytes()
        ).hexdigest()
        self.assertEqual(
            artifact["proposal"]["raw_input_rows_sha256"], expected_hash
        )

        with tempfile.TemporaryDirectory() as directory:
            fixture = pathlib.Path(directory) / "fixture"
            shutil.copytree(FIXTURE_DIR, fixture)
            validation = fixture / "validation.jsonl"
            validation.write_bytes(b" " + validation.read_bytes())

            result = self.run_cli(
                "check", "--fixture-dir", fixture, "--model", MODEL
            )

        self.assertEqual(result.returncode, 2, result.stderr or result.stdout)
        self.assertNotIn("Traceback", result.stderr)

    def test_aggregation_prediction_must_equal_checked_linear_formula(self):
        aggregation = copy.deepcopy(
            json.loads((FIXTURE_DIR / "config.json").read_text())["aggregation"]
        )
        aggregation["provenance"]["image_id"] = "0x" + "1" * 64
        aggregation["measurements"] = [
            {
                "child_count": count,
                "actual_mcycles": 180 * count,
                "predicted_mcycles": 180 * count,
                "enabled": True,
            }
            for count in range(1, 6)
        ]
        aggregation["calibrated_counts"] = list(range(1, 6))
        MODEL_TOOL.validate_aggregation(aggregation)

        aggregation["measurements"][2]["predicted_mcycles"] += 1
        with self.assertRaisesRegex(
            MODEL_TOOL.ModelError, "predicted_mcycles must equal"
        ):
            MODEL_TOOL.validate_aggregation(aggregation)

    def test_aggregation_prediction_multiplication_rejects_u64_overflow(self):
        aggregation = copy.deepcopy(
            json.loads((FIXTURE_DIR / "config.json").read_text())["aggregation"]
        )
        aggregation["per_child_mcycles"] = 1 << 63
        aggregation["provenance"]["image_id"] = "0x" + "1" * 64
        aggregation["measurements"] = [
            {
                "child_count": 2,
                "actual_mcycles": 1,
                "predicted_mcycles": 1,
                "enabled": False,
            }
        ]

        with self.assertRaisesRegex(MODEL_TOOL.ModelError, "prediction must fit u64"):
            MODEL_TOOL.validate_aggregation(aggregation)

    def test_aggregation_u32_limit_applies_only_to_enabled_measurements(self):
        per_child_mcycles = (1 << 32) + 1
        aggregation = copy.deepcopy(
            json.loads((FIXTURE_DIR / "config.json").read_text())["aggregation"]
        )
        aggregation["per_child_mcycles"] = per_child_mcycles
        aggregation["provenance"]["image_id"] = "0x" + "1" * 64
        aggregation["measurements"] = [
            {
                "child_count": count,
                "actual_mcycles": per_child_mcycles * count * 2,
                "predicted_mcycles": per_child_mcycles * count,
                "enabled": False,
            }
            for count in range(1, 6)
        ]
        MODEL_TOOL.validate_aggregation(aggregation)

        for measurement in aggregation["measurements"]:
            measurement["actual_mcycles"] = measurement["predicted_mcycles"]
            measurement["enabled"] = True
        aggregation["calibrated_counts"] = list(range(1, 6))

        with self.assertRaisesRegex(
            MODEL_TOOL.ModelError,
            "enabled aggregation prediction must fit u32",
        ):
            MODEL_TOOL.validate_aggregation(aggregation)

    def test_validation_rejects_unbounded_diagnostic_precision_without_traceback(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture = temporary / "fixture"
            shutil.copytree(FIXTURE_DIR, fixture)
            config = json.loads((fixture / "config.json").read_text())
            config["proposal"]["cohorts"]["hoodi"][
                "diagnostic_decimal_places"
            ]["continuous"] = 10_000
            (fixture / "config.json").write_text(json.dumps(config))

            result = self.run_cli(
                "check", "--fixture-dir", fixture, "--model", MODEL
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("diagnostic decimal places", result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def test_compact_fixture_numerics_must_fit_rust_u32(self):
        base = {
            "network": "taiko_hoodi",
            "split": "fit",
            "proposal_id": 1,
            "block_count": 1,
            "total_zkgas": 1,
            "actual_mcycles": 1,
        }
        for field in ("proposal_id", "block_count", "total_zkgas", "actual_mcycles"):
            with self.subTest(field=field):
                row = {**base, field: 1 << 32}
                with self.assertRaisesRegex(MODEL_TOOL.ModelError, "must fit u32"):
                    MODEL_TOOL.compact_row(row, "fixture row")

    def test_aggregation_schema_errors_are_model_errors(self):
        with self.assertRaises(MODEL_TOOL.ModelError):
            MODEL_TOOL.validate_aggregation(None)

    def test_cli_schema_validation_errors_exit_two_without_traceback(self):
        cases = (
            ("config is not a mapping", []),
            ("aggregation is not a mapping", {"aggregation": None}),
        )
        for label, replacement in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                fixture = temporary / "fixture"
                shutil.copytree(FIXTURE_DIR, fixture)
                if isinstance(replacement, list):
                    config = replacement
                else:
                    config = json.loads((fixture / "config.json").read_text())
                    config.update(replacement)
                (fixture / "config.json").write_text(json.dumps(config))

                result = self.run_cli(
                    "check", "--fixture-dir", fixture, "--model", MODEL
                )

                self.assertEqual(result.returncode, 2)
                self.assertNotIn("Traceback", result.stderr)

    def test_update_recovers_an_incomplete_fixture_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture = temporary / "fixture"
            fixture.mkdir()
            shutil.copy2(FIXTURE_DIR / "config.json", fixture / "config.json")
            shutil.copy2(
                FIXTURE_DIR / "hoodi-fit.jsonl", fixture / "hoodi-fit.jsonl"
            )
            hoodi, mainnet = self.write_collectors(temporary)
            model = temporary / "risc0-zkgas.json"

            result = self.run_cli(
                "update",
                "--config",
                fixture / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                fixture,
                "--model",
                model,
            )

            self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
            self.assertEqual(model.read_bytes(), MODEL.read_bytes())
            self.assertTrue((fixture / "validation.jsonl").is_file())

    def test_update_self_heals_a_complete_noncanonical_existing_fixture(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture = temporary / "fixture"
            shutil.copytree(FIXTURE_DIR, fixture)
            fit_path = fixture / "hoodi-fit.jsonl"
            fit_path.write_bytes(b" " + fit_path.read_bytes())
            validation_path = fixture / "validation.jsonl"
            validation_rows = [
                json.loads(line) for line in validation_path.read_text().splitlines()
            ]
            validation_rows[0]["legacy_junk"] = True
            validation_path.write_text(
                "".join(
                    json.dumps(row, separators=(",", ":")) + "\n"
                    for row in validation_rows
                )
            )
            hoodi, mainnet = self.write_collectors(temporary)
            model = temporary / "risc0-zkgas.json"

            result = self.run_cli(
                "update",
                "--config",
                fixture / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                fixture,
                "--model",
                model,
            )

            self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
            self.assertEqual(
                fit_path.read_bytes(),
                (FIXTURE_DIR / "hoodi-fit.jsonl").read_bytes(),
            )
            self.assertEqual(
                validation_path.read_bytes(),
                (FIXTURE_DIR / "validation.jsonl").read_bytes(),
            )
            self.assertEqual(model.read_bytes(), MODEL.read_bytes())

    def test_update_rejects_existing_data_without_config_before_writing(self):
        data_sets = (
            ("fit only", ("hoodi-fit.jsonl",)),
            ("validation only", ("validation.jsonl",)),
            ("both", ("hoodi-fit.jsonl", "validation.jsonl")),
        )
        for label, names in data_sets:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                fixture = temporary / "fixture"
                fixture.mkdir()
                original = {}
                for name in names:
                    payload = (FIXTURE_DIR / name).read_bytes()
                    (fixture / name).write_bytes(payload)
                    original[name] = payload
                hoodi, mainnet = self.write_collectors(temporary)
                model = temporary / "risc0-zkgas.json"

                result = self.run_cli(
                    "update",
                    "--config",
                    FIXTURE_DIR / "config.json",
                    "--hoodi-samples",
                    hoodi,
                    "--mainnet-samples",
                    mainnet,
                    "--fixture-dir",
                    fixture,
                    "--model",
                    model,
                )

                self.assertEqual(result.returncode, 2)
                self.assertIn("existing fixture data requires config.json", result.stderr)
                self.assertFalse((fixture / "config.json").exists())
                self.assertFalse(model.exists())
                for name, payload in original.items():
                    self.assertEqual((fixture / name).read_bytes(), payload)

    def test_interrupted_atomic_write_preserves_the_existing_target(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture = temporary / "fixture"
            shutil.copytree(FIXTURE_DIR, fixture)
            model = temporary / "risc0-zkgas.json"
            shutil.copy2(MODEL, model)
            hoodi, mainnet = self.write_collectors(temporary)
            original_config = (fixture / "config.json").read_bytes()
            args = argparse.Namespace(
                fixture_dir=fixture,
                config=fixture / "config.json",
                model=model,
                hoodi_samples=hoodi,
                mainnet_samples=mainnet,
            )
            original_write = pathlib.Path.write_bytes

            def torn_write(path, payload):
                original_write(path, payload[:10])
                raise OSError("simulated interrupted write")

            with mock.patch.object(pathlib.Path, "write_bytes", torn_write):
                with self.assertRaisesRegex(OSError, "simulated interrupted write"):
                    MODEL_TOOL.update(args)

            self.assertEqual((fixture / "config.json").read_bytes(), original_config)

    def test_check_rejects_runtime_model_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            drifted_model = pathlib.Path(directory) / "risc0-zkgas.json"
            artifact = json.loads(MODEL.read_text())
            artifact["proposal"]["coefficients"]["scaled"]["intercept"] += 1
            drifted_model.write_text(json.dumps(artifact))

            result = self.run_cli("check", "--model", drifted_model)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runtime model differs from generated artifact", result.stderr)

    def test_update_projects_collector_rows_and_rebuilds_exact_artifact(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture_output = temporary / "fixture"
            model_output = temporary / "risc0-zkgas.json"
            hoodi, mainnet = self.write_collectors(temporary)

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                fixture_output,
                "--model",
                model_output,
            )

            self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
            self.assertEqual(
                (fixture_output / "hoodi-fit.jsonl").read_bytes(),
                (FIXTURE_DIR / "hoodi-fit.jsonl").read_bytes(),
            )
            self.assertEqual(
                (fixture_output / "config.json").read_bytes(),
                (FIXTURE_DIR / "config.json").read_bytes(),
            )
            self.assertEqual(
                (fixture_output / "validation.jsonl").read_bytes(),
                (FIXTURE_DIR / "validation.jsonl").read_bytes(),
            )
            self.assertEqual(model_output.read_bytes(), MODEL.read_bytes())

    def test_update_refits_coefficients_under_a_new_model_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary, perturb_fit=True)
            changed_model = temporary / "risc0-zkgas.json"
            fixture_output = temporary / "2026-09-01-m2-v2"
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["model_id"] = "risc0-zkgas-m2-auto"
            fixture_output.mkdir()
            config_path = fixture_output / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "update",
                "--config",
                config_path,
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                fixture_output,
                "--model",
                changed_model,
            )

            self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
            changed = json.loads(changed_model.read_text())
            current = json.loads(MODEL.read_text())
            self.assertRegex(
                changed["model_id"], r"^risc0-zkgas-m2-[0-9a-f]{16}$"
            )
            self.assertEqual(
                json.loads((fixture_output / "config.json").read_text())["model_id"],
                changed["model_id"],
            )
            self.assertNotEqual(
                changed["proposal"]["coefficients"]["decimal"],
                current["proposal"]["coefficients"]["decimal"],
            )

    def test_update_rejects_changed_fit_under_the_current_model_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary, perturb_fit=True)

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match model content", result.stderr)

    def test_update_rejects_collector_provenance_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            for row in rows:
                row["source_revision"] = "0" * 40
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source_revision does not match config provenance", result.stderr)

    def test_update_rejects_mixed_collector_cohort(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0]["artifact_hashes"]["proposal_elf_sha256"] = "0" * 64
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("artifact_hashes", result.stderr)

    def test_update_rejects_actual_guest_image_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in mainnet.read_text().splitlines()]
            for row in rows:
                row["risc0_image_id"] = "0x" + "0" * 64
            mainnet.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("risc0_image_id does not match config provenance", result.stderr)

    def test_update_rejects_a_collector_row_without_status(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0].pop("status")
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing status", result.stderr)

    def test_update_rejects_unapproved_shared_artifact_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            for path in (hoodi, mainnet):
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                for row in rows:
                    row["artifact_hashes"]["guest_launcher_binary_sha256"] = "0" * 64
                path.write_text(
                    "".join(
                        json.dumps(row, separators=(",", ":")) + "\n"
                        for row in rows
                    )
                )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("guest_launcher_binary_sha256 does not match config", result.stderr)

    def test_update_rejects_rebinding_current_id_to_new_collector_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            changed_hash = "0" * 64
            for path in (hoodi, mainnet):
                rows = [json.loads(line) for line in path.read_text().splitlines()]
                for row in rows:
                    row["artifact_hashes"]["guest_launcher_binary_sha256"] = changed_hash
                path.write_text(
                    "".join(
                        json.dumps(row, separators=(",", ":")) + "\n"
                        for row in rows
                    )
                )
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["collector_artifact_hashes"]["shared"][
                "guest_launcher_binary_sha256"
            ] = changed_hash
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "update",
                "--config",
                config_path,
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "copied-v1",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match model content", result.stderr)

    def test_update_rejects_reusing_a_new_model_identity_for_changed_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            fixture_output = temporary / "fixtures" / "2026-09-01-m2-v2-a"
            model_output = temporary / "models" / "risc0-zkgas-a.json"
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["model_id"] = "risc0-zkgas-m2-auto"
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))
            hoodi, mainnet = self.write_collectors(temporary)

            first = self.run_cli(
                "update",
                "--config",
                config_path,
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                fixture_output,
                "--model",
                model_output,
            )
            self.assertEqual(first.returncode, 0, first.stderr or first.stdout)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0]["evaluated_mcycles_count"] += 100
            rows[0]["risc0_user_cycles"] += 100_000_000
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )
            second = self.run_cli(
                "update",
                "--config",
                fixture_output / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixtures" / "2026-09-01-m2-v2-b",
                "--model",
                temporary / "models" / "risc0-zkgas-b.json",
            )

        self.assertNotEqual(second.returncode, 0)
        self.assertIn("does not match model content", second.stderr)

    def test_update_rejects_aggregation_config_that_runtime_would_reject(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["model_id"] = "risc0-zkgas-m2-auto"
            config["aggregation"]["per_child_mcycles"] = 0
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "update",
                "--config",
                config_path,
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "2026-09-01-m2-v2",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("aggregation per_child_mcycles must be non-zero", result.stderr)

    def test_update_rejects_incomplete_successful_collector_schema(self):
        for missing_field in (
            "schema_version",
            "sample_key",
            "evaluated_mcycles_count",
            "risc0_user_cycles",
        ):
            with self.subTest(
                missing_field=missing_field
            ), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                hoodi, mainnet = self.write_collectors(temporary)
                rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
                removed = rows[0].pop(missing_field)
                if missing_field == "evaluated_mcycles_count":
                    rows[0]["actual_mcycles"] = removed
                hoodi.write_text(
                    "".join(
                        json.dumps(row, separators=(",", ":")) + "\n"
                        for row in rows
                    )
                )

                result = self.run_cli(
                    "update",
                    "--config",
                    FIXTURE_DIR / "config.json",
                    "--hoodi-samples",
                    hoodi,
                    "--mainnet-samples",
                    mainnet,
                    "--fixture-dir",
                    temporary / "fixture-output",
                    "--model",
                    temporary / "risc0-zkgas.json",
                )

                self.assertNotEqual(result.returncode, 0)
                self.assertIn(missing_field, result.stderr)

    def test_update_rejects_inconsistent_authoritative_cycle_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0]["risc0_user_cycles"] += 1_000_000
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match risc0_user_cycles", result.stderr)

    def test_update_rejects_compact_actual_mcycles_in_raw_collector_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0]["actual_mcycles"] = rows[0]["evaluated_mcycles_count"]
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("must not provide compact actual_mcycles", result.stderr)

    def test_update_rejects_wrong_successful_row_count(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()][1:]
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("expected 80 Hoodi fit rows, found 79", result.stderr)

    def test_update_accepts_ceil_mcycles_for_nonintegral_million_cycles(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            rows[0]["risc0_user_cycles"] = (
                rows[0]["evaluated_mcycles_count"] * 1_000_000 - 1
            )
            hoodi.write_text(
                "".join(json.dumps(row, separators=(",", ":")) + "\n" for row in rows)
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)

    def test_check_rejects_config_values_that_runtime_schema_rejects(self):
        cases = (
            (
                "boolean schema",
                lambda config: config.__setitem__("schema_version", True),
                "schema_version must be a u32 integer",
            ),
            (
                "unexpected provenance field",
                lambda config: config["proposal"]["provenance"].__setitem__(
                    "unexpected", 1
                ),
                "proposal provenance has an unsupported schema",
            ),
            (
                "u32 execution overflow",
                lambda config: config["proposal"]["provenance"].__setitem__(
                    "min_execution_po2", 1 << 32
                ),
                "proposal min_execution_po2 must fit u32",
            ),
            (
                "zero minimum execution po2",
                lambda config: config["proposal"]["provenance"].__setitem__(
                    "min_execution_po2", 0
                ),
                "proposal min_execution_po2 must be non-zero",
            ),
            (
                "u64 coefficient overflow",
                lambda config: config["proposal"]["coefficients"].__setitem__(
                    "scale", 1 << 64
                ),
                "coefficient scale must fit u64",
            ),
            (
                "u64 aggregation overflow",
                lambda config: config["aggregation"].__setitem__(
                    "per_child_mcycles", 1 << 64
                ),
                "aggregation per_child_mcycles must fit u64",
            ),
            (
                "invalid mainnet provenance",
                lambda config: config["proposal"]["cohorts"]["mainnet"].__setitem__(
                    "influenced_model_selection", False
                ),
                "Mainnet evaluation provenance is inconsistent",
            ),
            (
                "mutable error budget",
                lambda config: config["proposal"].__setitem__(
                    "error_budget_percent", 100
                ),
                "error_budget_percent must be exactly 10",
            ),
        )
        for label, mutate, expected_error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                temporary = pathlib.Path(directory)
                config = json.loads((FIXTURE_DIR / "config.json").read_text())
                mutate(config)
                config_path = temporary / "config.json"
                config_path.write_text(json.dumps(config))

                result = self.run_cli(
                    "check", "--config", config_path, "--model", MODEL
                )

                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected_error, result.stderr)

    def test_proposal_policy_uses_a_global_cap_and_calibrated_minimum_po2(self):
        config = json.loads((FIXTURE_DIR / "config.json").read_text())

        MODEL_TOOL.validate_config(config)
        self.assertEqual(config["schema_version"], 2)
        self.assertEqual(config["proposal"]["max_total_zkgas"], 500_000_000)
        self.assertNotIn("domains", config["proposal"])
        self.assertEqual(
            config["proposal"]["provenance"]["min_execution_po2"], 20
        )
        self.assertNotIn(
            "execution_po2", config["proposal"]["provenance"]
        )

    def test_scaled_coefficients_must_fit_nonzero_rust_u64_fields(self):
        config = {"proposal": {"coefficients": {"scale": 1}}}
        with self.assertRaisesRegex(
            MODEL_TOOL.ModelError,
            "scaled total_zkgas coefficient must fit u64",
        ):
            MODEL_TOOL.scaled_coefficients(
                config,
                {
                    "intercept": Decimal(1),
                    "total_zkgas": Decimal(-1),
                    "block_count": Decimal(1),
                },
            )

    def test_schema_v2_rejects_the_legacy_v1_model_id(self):
        config = json.loads((FIXTURE_DIR / "config.json").read_text())
        config["model_id"] = "risc0-zkgas-m2-v1"

        with self.assertRaisesRegex(
            MODEL_TOOL.ModelError,
            "new model_id must be content-addressed or use risc0-zkgas-m2-auto",
        ):
            MODEL_TOOL.validate_config(config)

    def test_check_rejects_a_zero_global_zkgas_cap(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["max_total_zkgas"] = 0
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "check",
                "--config",
                config_path,
                "--model",
                MODEL,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("proposal max_total_zkgas must be non-zero", result.stderr)

    def test_check_rejects_a_cap_that_admits_an_over_budget_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["max_total_zkgas"] = 562_107_601
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "check",
                "--config",
                config_path,
                "--model",
                MODEL,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exceeds the configured 10% error budget", result.stderr)

    def test_check_rejects_a_cap_that_excludes_a_whole_cohort(self):
        # 369_558_585 is one below the lowest Hoodi total_zkgas, so every admitted row is Mainnet
        # and the 120-row cohort M2 is fitted on contributes no evidence to the budget check.
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["max_total_zkgas"] = 369_558_585
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "check",
                "--config",
                config_path,
                "--model",
                MODEL,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "proposal max_total_zkgas admits no taiko_hoodi observations", result.stderr
        )

    def test_update_rejects_a_collector_cohort_po2_that_differs_from_the_minimum(self):
        # `min_execution_po2` names a runtime floor, but every collected observation must have been
        # measured at exactly that po2 -- a lower or higher cohort is not the calibrated cohort.
        for cohort_po2 in (19, 21):
            with self.subTest(cohort_po2=cohort_po2):
                with tempfile.TemporaryDirectory() as directory:
                    temporary = pathlib.Path(directory)
                    hoodi, mainnet = self.write_collectors(temporary)
                    rows = [
                        json.loads(line) for line in hoodi.read_text().splitlines()
                    ]
                    for row in rows:
                        row["execution_po2"] = cohort_po2
                    hoodi.write_text(
                        "".join(
                            json.dumps(row, separators=(",", ":")) + "\n"
                            for row in rows
                        )
                    )

                    result = self.run_cli(
                        "update",
                        "--config",
                        FIXTURE_DIR / "config.json",
                        "--hoodi-samples",
                        hoodi,
                        "--mainnet-samples",
                        mainnet,
                        "--fixture-dir",
                        temporary / "fixture-output",
                        "--model",
                        temporary / "risc0-zkgas.json",
                    )

                self.assertNotEqual(result.returncode, 0)
                self.assertIn(
                    "execution_po2 does not match config provenance", result.stderr
                )

    def test_check_rejects_a_cap_that_admits_no_observations(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["max_total_zkgas"] = 216_314_229
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "check",
                "--config",
                config_path,
                "--model",
                MODEL,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "proposal max_total_zkgas admits no observations", result.stderr
        )

    def test_update_rejects_collector_rows_built_from_another_proposal_elf(self):
        # `validate_collector_cohort` is the only automated binding between packaged samples and a
        # guest ELF identity: the runtime deliberately does not compare model provenance with the
        # running binary, so a cohort measured on another ELF must never package silently.
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            hoodi, mainnet = self.write_collectors(temporary)
            rows = [json.loads(line) for line in hoodi.read_text().splitlines()]
            for row in rows:
                row["artifact_hashes"]["proposal_elf_sha256"] = "a" * 64
            hoodi.write_text(
                "".join(
                    json.dumps(row, separators=(",", ":")) + "\n" for row in rows
                )
            )

            result = self.run_cli(
                "update",
                "--config",
                FIXTURE_DIR / "config.json",
                "--hoodi-samples",
                hoodi,
                "--mainnet-samples",
                mainnet,
                "--fixture-dir",
                temporary / "fixture-output",
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("proposal ELF does not match config provenance", result.stderr)

    def test_check_rejects_a_config_whose_collector_elf_differs_from_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["provenance"]["elf_sha256"] = "a" * 64
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli(
                "check",
                "--config",
                config_path,
                "--model",
                MODEL,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "collector proposal ELF must match proposal provenance", result.stderr
        )

    def test_check_rejects_unsupported_model_id_family(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["model_id"] = "other-model-v1"
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli("check", "--config", config_path, "--model", MODEL)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unsupported model_id family", result.stderr)


if __name__ == "__main__":
    unittest.main()
