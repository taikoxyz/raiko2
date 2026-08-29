from __future__ import annotations

import json
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest
from decimal import Decimal


ROOT = pathlib.Path(__file__).resolve().parents[3]
SCRIPT = ROOT / "scripts" / "modeling" / "risc0_zkgas_model.py"
FIXTURE_DIR = (
    ROOT / "tests" / "fixtures" / "risc0-zkgas" / "2026-08-28-m2-v1"
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
                "execution_po2": provenance["execution_po2"],
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
                FIXTURE_DIR,
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("legacy model_id is reserved", result.stderr)

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
                FIXTURE_DIR,
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
                FIXTURE_DIR,
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
                FIXTURE_DIR,
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
                FIXTURE_DIR,
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
                FIXTURE_DIR,
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("guest_launcher_binary_sha256 does not match config", result.stderr)

    def test_update_rejects_rebinding_legacy_id_to_new_collector_provenance(self):
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
        self.assertIn("legacy model_id is reserved", result.stderr)

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
                    FIXTURE_DIR,
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
                FIXTURE_DIR,
                "--model",
                temporary / "risc0-zkgas.json",
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match risc0_user_cycles", result.stderr)

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
                    "execution_po2", 1 << 32
                ),
                "proposal execution_po2 must fit u32",
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

    def test_legacy_model_id_is_reserved_for_the_approved_v1_content(self):
        artifact = json.loads(MODEL.read_text())
        artifact["proposal"]["generator_config_sha256"] = "0" * 64

        with self.assertRaisesRegex(
            MODEL_TOOL.ModelError,
            "legacy model_id is reserved for the approved v1 content",
        ):
            MODEL_TOOL.resolve_model_id(artifact, "risc0-zkgas-m2-v1")

    def test_update_rejects_an_explicit_domain_that_admits_over_budget_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["domains"][1]["total_zkgas"]["maximum"] = 562_107_601
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

    def test_check_rejects_a_domain_boundary_without_an_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["domains"][0]["total_zkgas"]["minimum"] += 1
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
        self.assertIn("must equal an observed value", result.stderr)

    def test_check_rejects_duplicate_domains(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary = pathlib.Path(directory)
            config = json.loads((FIXTURE_DIR / "config.json").read_text())
            config["proposal"]["domains"].append(config["proposal"]["domains"][0])
            config_path = temporary / "config.json"
            config_path.write_text(json.dumps(config))

            result = self.run_cli("check", "--config", config_path, "--model", MODEL)

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("exactly once", result.stderr)

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
