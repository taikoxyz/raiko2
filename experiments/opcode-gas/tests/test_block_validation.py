import copy
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import block_validation
import opcode_gas


class BlockValidationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.launcher = self.root / "guest-launcher"
        self.launcher.write_bytes(b"bound launcher")
        self.source_paths = {}
        for name in (
            "candidate", "launcher", "elf", "vk", "trace", "fixture", "builder"
        ):
            path = self.launcher if name == "launcher" else self.root / name
            if name != "launcher":
                path.write_bytes(f"{name} bytes".encode())
            self.source_paths[name] = path
        self.rows = []
        for index in range(32):
            partition = "calibration" if index < 12 else "validation"
            row_id = f"{index + 1:064x}"
            backend = f"{index + 101:064x}"
            observation = {
                "row_id": row_id,
                "backend_input_sha256": backend,
                "guest_input_bincode_length": 1000 + index,
                "public_output": f"0x{index + 201:064x}",
                "finalized_block_zkgas": 100 + index,
            }
            self.rows.append(
                {
                    "partition": partition,
                    "category": "arithmetic",
                    "candidate_identity": block_validation.ELIGIBLE_CANDIDATE_IDENTITY,
                    "row_id": row_id,
                    "workload_id": f"{index + 1001:064x}",
                    "backend_input_sha256": backend,
                    "candidate_coverage_complete": True,
                    "candidate_predicted_prover_gas": str(1200 + index),
                    "bundle_sha256": f"{index + 2001:064x}",
                    "trace_sha256": f"{index + 3001:064x}",
                    "frozen_identity": {
                        "schema_version": 1,
                        "fixture_spec_sha256": f"{index + 4001:064x}",
                        "spec": {"row_id": row_id, "synthetic": index},
                        "observation": observation,
                    },
                }
            )
        self.manifest = {
            "schema_version": 2,
            "candidate_identity": block_validation.ELIGIBLE_CANDIDATE_IDENTITY,
            "artifact_sha256": block_validation.ELIGIBLE_MANIFEST_IDENTITY,
            "sources": {
                name: {
                    "path": str(path.name),
                    "sha256": opcode_gas.sha256_bytes(path.read_bytes()),
                    **(
                        {"logical_identity": block_validation.ELIGIBLE_CANDIDATE_IDENTITY}
                        if name == "candidate"
                        else {}
                    ),
                }
                for name, path in self.source_paths.items()
            },
            "rows": self.rows,
        }
        self.calibration = {
            "artifact_sha256": block_validation.ELIGIBLE_CALIBRATION_IDENTITY,
            "candidate_identity": block_validation.ELIGIBLE_CANDIDATE_IDENTITY,
            "manifest_identity": block_validation.ELIGIBLE_MANIFEST_IDENTITY,
        }
        self.normalization = {
            "artifact_sha256": block_validation.ELIGIBLE_NORMALIZATION_IDENTITY,
            "candidate_identity": block_validation.ELIGIBLE_CANDIDATE_IDENTITY,
            "manifest_identity": block_validation.ELIGIBLE_MANIFEST_IDENTITY,
            "calibration_identity": block_validation.ELIGIBLE_CALIBRATION_IDENTITY,
            "kappa_unzen": "1",
            "acceptance_thresholds": copy.deepcopy(
                block_validation.block_calibration.ACCEPTANCE_THRESHOLDS
            ),
        }
        self.manifest_path = self.root / "manifest.json"
        self.calibration_path = self.root / "calibration.json"
        self.normalization_path = self.root / "normalization.json"
        for path, value in (
            (self.manifest_path, self.manifest),
            (self.calibration_path, self.calibration),
            (self.normalization_path, self.normalization),
        ):
            path.write_bytes(opcode_gas.canonical_json(value) + b"\n")
        self.events = []

    def tearDown(self):
        self.temporary.cleanup()

    def implementation_identity(self):
        identities = {
            path: f"{index + 5001:064x}"
            for index, path in enumerate(block_validation.IMPLEMENTATION_SOURCE_PATHS)
        }
        identities[block_validation.RUNNER_SOURCE_RELATIVE_PATH] = (
            opcode_gas.sha256_bytes(
                (ROOT / block_validation.RUNNER_SOURCE_RELATIVE_PATH).read_bytes()
            )
        )
        return "a" * 40, identities

    def report(self, row, *, gas=None):
        observation = copy.deepcopy(row["frozen_identity"]["observation"])
        prover_gas = gas if gas is not None else 1200 + int(row["row_id"], 16)
        return {
            "stage": "controlled-block",
            "mode": "execute",
            "proof_mode": "compressed",
            "sp1_execution_engine": "gas-estimator",
            "sp1_gas_trace_chunk_threshold": 134217728,
            "sp1_gas_trace_chunk_slots": 2,
            "input": row["row_id"],
            "guest_input_sha256": f"0x{row['backend_input_sha256']}",
            "guest_input_bincode_length": observation["guest_input_bincode_length"],
            "sp1_proposal_elf_sha256": self.manifest["sources"]["elf"]["sha256"],
            "guest_launcher_sha256": self.manifest["sources"]["launcher"]["sha256"],
            "public_values": observation["public_output"],
            "wall_time_ms": 1,
            "primary_workload_metric": {"label": "prover_gas", "count": prover_gas},
            "workload_metrics": [
                {"label": "prover_gas", "count": prover_gas},
                {"label": "sp1_total_instruction_count", "count": 77},
                {"label": "sp1_total_syscall_count", "count": 3},
                {"label": "sp1_touched_memory_addresses", "count": 9},
            ],
            "exit_code": 0,
            "gas": prover_gas,
            "total_instruction_count": 77,
            "total_syscall_count": 3,
            "touched_memory_addresses": 9,
            "risc0_image_id": None,
            "risc0_input_bytes": None,
            "risc0_user_cycles": None,
            "risc0_padded_cycles": None,
            "risc0_segment_count": None,
            "risc0_po2_counts": [],
            "cycle_tracker": [],
            "invocation_tracker": [],
            "opcode_counts": [],
            "syscall_counts": [],
            "memory_snapshots": [],
            "controlled_trace": None,
            "controlled_overhead": None,
            "controlled_block": {
                "status": "accepted",
                "row_id": row["row_id"],
                "reasons": [],
                "observation": observation,
            },
            "controlled_state_holdout": None,
        }

    def command_runner(self, command, **kwargs):
        self.assertEqual(kwargs["timeout"], 600)
        self.events.append(("command", list(command)))
        input_path = pathlib.Path(command[command.index("--input") + 1])
        output_path = pathlib.Path(command[command.index("--jsonl-out") + 1])
        spec = json.loads(input_path.read_bytes())
        row = next(row for row in self.rows if row["row_id"] == spec["row_id"])
        output_path.write_bytes(opcode_gas.canonical_json(self.report(row)) + b"\n")
        return subprocess.CompletedProcess(command, 0, b"", b"")

    def run_validation(self, *, command_runner=None):
        def verify(**kwargs):
            self.events.append(("verify", None))
            return self.manifest, self.calibration, self.normalization

        with mock.patch.object(
            block_validation, "_verify_frozen_inputs", side_effect=verify
        ), mock.patch.object(
            block_validation,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ):
            return block_validation.run_validation(
                manifest_path=self.manifest_path,
                calibration_path=self.calibration_path,
                normalization_path=self.normalization_path,
                source_paths=self.source_paths,
                launcher_path=self.launcher,
                run_dir=self.root / "run",
                command_runner=command_runner or self.command_runner,
            )

    def test_run_verifies_before_sp1_and_executes_only_twenty_validation_rows(self):
        reports = self.run_validation()
        self.assertEqual(self.events[0][0], "verify")
        commands = [event[1] for event in self.events if event[0] == "command"]
        self.assertEqual(len(commands), 20)
        self.assertEqual(len(reports), 20)
        calibration_ids = {row["row_id"] for row in self.rows[:12]}
        for command in commands:
            self.assertEqual(
                command[1:11],
                [
                    "--stage", "controlled-block", "--proof-type", "sp1",
                    "--mode", "execute", "--sp1-prover", "local",
                    "--sp1-execution-engine", "gas-estimator",
                ],
            )
            input_path = pathlib.Path(command[command.index("--input") + 1])
            self.assertNotIn(json.loads(input_path.read_bytes())["row_id"], calibration_ids)

    def test_partial_failure_resumes_only_missing_rows_and_rejects_corruption(self):
        calls = 0

        def fail_fourth(command, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 4:
                return subprocess.CompletedProcess(command, 9, b"", b"failed")
            return self.command_runner(command, **kwargs)

        with self.assertRaisesRegex(ValueError, "launcher failed"):
            self.run_validation(command_runner=fail_fourth)
        completed = list((self.root / "run" / "rows").glob("*.json"))
        self.assertEqual(len(completed), 3)
        self.events.clear()
        reports = self.run_validation()
        self.assertEqual(len(reports), 20)
        self.assertEqual(
            len([event for event in self.events if event[0] == "command"]), 17
        )
        completed[0].chmod(0o644)
        completed[0].write_text("{}\n")
        with self.assertRaisesRegex(ValueError, "report schema"):
            self.run_validation()

    def test_timeout_and_launcher_change_do_not_publish_a_row(self):
        def timeout(command, **kwargs):
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])

        with self.assertRaisesRegex(ValueError, "timed out"):
            self.run_validation(command_runner=timeout)
        self.assertEqual(list((self.root / "run" / "rows").glob("*.json")), [])

        def mutate_launcher(command, **kwargs):
            completed = self.command_runner(command, **kwargs)
            self.launcher.write_bytes(b"changed")
            return completed

        with self.assertRaisesRegex(ValueError, "launcher changed"):
            self.run_validation(command_runner=mutate_launcher)
        self.assertEqual(list((self.root / "run" / "rows").glob("*.json")), [])

    def test_symlink_run_root_fails_before_external_mutation(self):
        external = self.root / "external"
        external.mkdir()
        sentinel = external / "sentinel"
        sentinel.write_text("preserve")
        os.symlink(external, self.root / "run")
        with self.assertRaisesRegex(ValueError, "run root"):
            self.run_validation()
        self.assertEqual(sentinel.read_text(), "preserve")
        self.assertEqual(list(external.iterdir()), [sentinel])

    def test_stale_partial_child_symlinks_fail_before_cleanup_mutation(self):
        external = self.root / "external"
        external.mkdir()
        sentinel = external / "sentinel"
        sentinel.write_text("preserve")
        run_dir = self.root / "run"
        run_dir.mkdir()
        trap = run_dir / ".trap.partial.99999999.dead"
        os.symlink(external, trap, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "run inventory"):
            self.run_validation()
        self.assertTrue(trap.is_symlink())
        self.assertEqual(sentinel.read_text(), "preserve")

        trap.unlink()
        inputs = run_dir / "inputs"
        inputs.mkdir()
        nested = inputs / ".trap.partial.99999999.dead"
        os.symlink(external, nested, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "row inventory"):
            self.run_validation()
        self.assertTrue(nested.is_symlink())
        self.assertEqual(sentinel.read_text(), "preserve")

    def test_wrong_manifest_row_or_partition_cannot_be_validated(self):
        validation = self.rows[12]
        report = self.report(validation)
        block_validation.validate_report(report, validation, self.manifest)
        wrong = copy.deepcopy(validation)
        wrong["row_id"] = "f" * 64
        with self.assertRaisesRegex(ValueError, "not a validation row"):
            block_validation.validate_report(report, wrong, self.manifest)
        with self.assertRaisesRegex(ValueError, "not a validation row"):
            block_validation.validate_report(
                self.report(self.rows[0]), self.rows[0], self.manifest
            )

    def test_validation_and_comparison_artifacts_replay_and_detect_tampering(self):
        reports = self.run_validation()
        revision, identities = self.implementation_identity()
        validation = block_validation.build_validation_artifact(
            manifest=self.manifest,
            calibration=self.calibration,
            normalization=self.normalization,
            reports=reports,
            source_revision=revision,
            implementation_sources=identities,
        )
        block_validation.validate_validation_artifact(validation)
        self.assertEqual(len(validation["rows"]), 20)
        self.assertEqual(
            {row["partition"] for row in validation["rows"]}, {"validation"}
        )
        comparison = block_validation.build_comparison_artifact(
            validation=validation, normalization=self.normalization
        )
        block_validation.validate_comparison_artifact(comparison)
        self.assertEqual(comparison["comparison"]["row_count"], 20)
        self.assertEqual(
            comparison["status"],
            "block_validated_against_unzen"
            if comparison["comparison"]["accepted"]
            else "block_validation_failed",
        )

        tampered = copy.deepcopy(comparison)
        tampered["comparison"]["accepted"] = not tampered["comparison"]["accepted"]
        tampered["artifact_sha256"] = (
            block_validation.block_calibration._content_address(tampered)
        )
        with self.assertRaisesRegex(ValueError, "exact replay"):
            block_validation.validate_comparison_artifact(tampered)

        changed_validation = copy.deepcopy(validation)
        changed_validation["rows"][0]["candidate_predicted_prover_gas"] = "9999"
        changed_validation["row_digest"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(changed_validation["rows"])
        )
        changed_validation["artifact_sha256"] = (
            block_validation.block_calibration._content_address(changed_validation)
        )
        rebuilt = block_validation.build_comparison_artifact(
            validation=changed_validation, normalization=self.normalization
        )
        self.assertNotEqual(
            rebuilt["validation_identity"], comparison["validation_identity"]
        )

    def test_public_seal_and_verify_replay_the_exact_run_and_comparison(self):
        self.run_validation()
        frozen = (self.manifest, self.calibration, self.normalization)
        identity = self.implementation_identity()
        validation_root = self.root / "validation-artifacts"
        comparison_root = self.root / "comparison-artifacts"
        with mock.patch.object(
            block_validation, "_verify_frozen_inputs", return_value=frozen
        ), mock.patch.object(
            block_validation,
            "_implementation_identity_at_revision",
            return_value=identity,
        ):
            validation_path = block_validation.seal_validation_artifact(
                manifest_path=self.manifest_path,
                calibration_path=self.calibration_path,
                normalization_path=self.normalization_path,
                source_paths=self.source_paths,
                run_dir=self.root / "run",
                output_root=validation_root,
            )
            validation = block_validation.verify_validation_artifact(
                validation_path,
                manifest_path=self.manifest_path,
                calibration_path=self.calibration_path,
                normalization_path=self.normalization_path,
                source_paths=self.source_paths,
            )
            comparison_path = block_validation.seal_comparison_artifact(
                validation_path=validation_path,
                manifest_path=self.manifest_path,
                calibration_path=self.calibration_path,
                normalization_path=self.normalization_path,
                source_paths=self.source_paths,
                output_root=comparison_root,
            )
            comparison = block_validation.verify_comparison_artifact(
                comparison_path,
                validation_path=validation_path,
                manifest_path=self.manifest_path,
                calibration_path=self.calibration_path,
                normalization_path=self.normalization_path,
                source_paths=self.source_paths,
            )
        self.assertEqual(
            comparison["validation_identity"], validation["artifact_sha256"]
        )
        self.assertEqual(validation_path.stat().st_mode & 0o777, 0o444)
        self.assertEqual(comparison_path.stat().st_mode & 0o777, 0o444)
        with self.assertRaises(FileExistsError):
            with mock.patch.object(
                block_validation, "_verify_frozen_inputs", return_value=frozen
            ), mock.patch.object(
                block_validation,
                "_implementation_identity_at_revision",
                return_value=identity,
            ):
                block_validation.seal_comparison_artifact(
                    validation_path=validation_path,
                    manifest_path=self.manifest_path,
                    calibration_path=self.calibration_path,
                    normalization_path=self.normalization_path,
                    source_paths=self.source_paths,
                    output_root=comparison_root,
                )

    def test_ineligible_input_fails_before_native_replay_or_sp1(self):
        bad = self.root / "bad.json"
        bad.write_text("{}\n")
        native = mock.Mock(side_effect=AssertionError("native replay must not run"))
        command = mock.Mock(side_effect=AssertionError("SP1 must not run"))
        common = {
            "manifest_path": bad,
            "calibration_path": self.calibration_path,
            "normalization_path": self.normalization_path,
            "source_paths": self.source_paths,
        }
        with mock.patch.object(
            block_validation.block_calibration,
            "verify_normalization_artifact",
            native,
        ):
            with self.assertRaisesRegex(ValueError, "eligible frozen manifest"):
                block_validation.run_validation(
                    **common,
                    launcher_path=self.launcher,
                    run_dir=self.root / "never-created",
                    command_runner=command,
                )
            for function, extra in (
                (
                    block_validation.seal_validation_artifact,
                    {"run_dir": self.root / "missing", "output_root": self.root / "out"},
                ),
                (
                    block_validation.verify_validation_artifact,
                    {"path": self.root / "missing.json"},
                ),
                (
                    block_validation.seal_comparison_artifact,
                    {
                        "validation_path": self.root / "missing.json",
                        "output_root": self.root / "out",
                    },
                ),
                (
                    block_validation.verify_comparison_artifact,
                    {
                        "path": self.root / "missing.json",
                        "validation_path": self.root / "missing-validation.json",
                    },
                ),
            ):
                with self.subTest(function=function.__name__), self.assertRaisesRegex(
                    ValueError, "eligible frozen manifest"
                ):
                    function(**common, **extra)
        native.assert_not_called()
        command.assert_not_called()

    def test_each_wrong_source_path_fails_before_native_replay_or_sp1(self):
        documents = {
            "frozen manifest": self.manifest,
            "calibration artifact": self.calibration,
            "normalization artifact": self.normalization,
        }

        def pinned(*args, label, **kwargs):
            return documents[label]

        for name in block_validation.SOURCE_NAMES:
            with self.subTest(name=name):
                wrong = self.root / f"wrong-{name}"
                wrong.write_bytes(self.source_paths[name].read_bytes())
                sources = dict(self.source_paths)
                sources[name] = wrong
                native = mock.Mock(
                    side_effect=AssertionError("native replay must not run")
                )
                command = mock.Mock(side_effect=AssertionError("SP1 must not run"))
                with mock.patch.object(
                    block_validation, "REPO_ROOT", self.root
                ), mock.patch.object(
                    block_validation,
                    "_preflight_pinned_document",
                    side_effect=pinned,
                ), mock.patch.object(
                    block_validation.block_calibration,
                    "verify_normalization_artifact",
                    native,
                ):
                    with self.assertRaisesRegex(ValueError, f"{name} source"):
                        block_validation.run_validation(
                            manifest_path=self.manifest_path,
                            calibration_path=self.calibration_path,
                            normalization_path=self.normalization_path,
                            source_paths=sources,
                            launcher_path=(
                                wrong if name == "launcher" else self.launcher
                            ),
                            run_dir=self.root / f"never-created-{name}",
                            command_runner=command,
                        )
                native.assert_not_called()
                command.assert_not_called()


if __name__ == "__main__":
    unittest.main()
