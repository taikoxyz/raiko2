import copy
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from decimal import Decimal, localcontext


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import block_calibration
import opcode_gas


class BlockCalibrationTests(unittest.TestCase):
    CANDIDATE = "c" * 64
    MANIFEST = "d" * 64

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.launcher = self.root / "guest-launcher"
        self.launcher.write_bytes(b"bound launcher")
        self.runner_source = self.root / "block_calibration.py"
        self.runner_source.write_bytes(b"runner source")
        self.manifest_path = self.root / "manifest.json"
        self.source_paths = {}
        source_names = ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder")
        for name in source_names:
            path = self.launcher if name == "launcher" else self.root / name
            if name != "launcher":
                path.write_bytes(f"{name} bytes".encode())
            self.source_paths[name] = path
        self.rows = []
        for index in range(32):
            partition = "calibration" if index < 12 else "validation"
            row_id = f"{index + 1:064x}"
            backend = f"{index + 101:064x}"
            public_output = f"0x{index + 201:064x}"
            observation = {
                "row_id": row_id,
                "backend_input_sha256": backend,
                "guest_input_bincode_length": 1000 + index,
                "public_output": public_output,
                "finalized_block_zkgas": 100 + index,
            }
            self.rows.append(
                {
                    "partition": partition,
                    "category": "arithmetic",
                    "candidate_identity": self.CANDIDATE,
                    "row_id": row_id,
                    "workload_id": f"{index + 1001:064x}",
                    "backend_input_sha256": backend,
                    "candidate_coverage_complete": True,
                    "candidate_predicted_prover_gas": str(1000 + index),
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
            "candidate_identity": self.CANDIDATE,
            "artifact_sha256": self.MANIFEST,
            "sources": {
                name: {
                    "path": str(path.name),
                    "sha256": opcode_gas.sha256_bytes(path.read_bytes()),
                    **({"logical_identity": self.CANDIDATE} if name == "candidate" else {}),
                }
                for name, path in self.source_paths.items()
            },
            "rows": self.rows,
        }
        self.manifest_path.write_bytes(opcode_gas.canonical_json(self.manifest) + b"\n")
        self.events = []

    def tearDown(self):
        self.temporary.cleanup()

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

    def run_calibration(self, *, command_runner=None):
        def verify(*args, **kwargs):
            self.events.append(("verify", None))
            return self.manifest

        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            side_effect=verify,
        ), mock.patch.object(
            block_calibration,
            "_require_eligible_manifest",
        ), mock.patch.object(
            block_calibration,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_runner_source_path",
            return_value=self.runner_source,
        ):
            return block_calibration.run_calibration(
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                launcher_path=self.launcher,
                run_dir=self.root / "run",
                command_runner=command_runner or self.command_runner,
            )

    def implementation_identity(self):
        identities = {
            path: f"{index + 5001:064x}"
            for index, path in enumerate(
                block_calibration.IMPLEMENTATION_SOURCE_PATHS
            )
        }
        identities[block_calibration.RUNNER_SOURCE_RELATIVE_PATH] = (
            opcode_gas.sha256_bytes(self.runner_source.read_bytes())
        )
        return (
            "a" * 40,
            identities,
        )

    def test_run_verifies_before_sp1_and_executes_only_frozen_calibration_rows(self):
        rows = self.run_calibration()
        self.assertEqual(self.events[0][0], "verify")
        commands = [event[1] for event in self.events if event[0] == "command"]
        self.assertEqual(len(commands), 12)
        self.assertEqual(len(rows), 12)
        validation_ids = {row["row_id"] for row in self.rows[12:]}
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
            spec = json.loads(input_path.read_bytes())
            self.assertNotIn(spec["row_id"], validation_ids)

    def test_resume_partial_failure_and_corrupt_rows_fail_closed(self):
        calls = 0

        def fail_fourth(command, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 4:
                return subprocess.CompletedProcess(command, 9, b"", b"failed")
            return self.command_runner(command, **kwargs)

        with self.assertRaisesRegex(ValueError, "launcher failed"):
            self.run_calibration(command_runner=fail_fourth)
        stale_partials = [
            self.root / "run" / ".identity.json.partial.999999.orphan",
            self.root / "run" / "inputs" / ".input.json.partial.999999.orphan",
            self.root / "run" / "rows" / ".row.json.partial.999999.orphan",
        ]
        for path in stale_partials:
            path.write_bytes(b"partial")
        self.events.clear()
        rows = self.run_calibration()
        self.assertEqual(len(rows), 12)
        self.assertEqual(
            len([event for event in self.events if event[0] == "command"]), 9
        )
        self.assertFalse(any(path.exists() for path in stale_partials))
        row_path = self.root / "run" / "rows" / f"{self.rows[0]['row_id']}.json"
        row_path.chmod(0o644)
        row_path.write_text("{}\n")
        with self.assertRaises(ValueError):
            self.run_calibration()

    def test_report_join_rejects_wrong_execution_provenance_and_shape(self):
        row = self.rows[0]
        valid = self.report(row)
        block_calibration.validate_report(valid, row, self.manifest)
        mutations = {
            "standard": lambda report: report.__setitem__("sp1_execution_engine", "standard"),
            "mock": lambda report: report["primary_workload_metric"].__setitem__("label", "cycles"),
            "input": lambda report: report.__setitem__("input", self.rows[1]["row_id"]),
            "public": lambda report: report.__setitem__("public_values", "0x00"),
            "elf": lambda report: report.__setitem__("sp1_proposal_elf_sha256", "0" * 64),
            "launcher": lambda report: report.__setitem__("guest_launcher_sha256", "0" * 64),
            "exit": lambda report: report.__setitem__("exit_code", 1),
            "duplicate": lambda report: report["workload_metrics"].append(
                {"label": "prover_gas", "count": report["gas"]}
            ),
            "missing": lambda report: report.pop("controlled_block"),
            "validation": lambda report: None,
        }
        for name, mutate in mutations.items():
            candidate = copy.deepcopy(valid)
            mutate(candidate)
            selected = copy.deepcopy(row)
            if name == "validation":
                selected["partition"] = "validation"
            with self.subTest(name=name), self.assertRaises(ValueError):
                block_calibration.validate_report(candidate, selected, self.manifest)

    def test_run_rejects_runner_source_toctou_before_publishing_row(self):
        changed = False

        def mutate_runner(command, **kwargs):
            nonlocal changed
            result = self.command_runner(command, **kwargs)
            if not changed:
                self.runner_source.write_bytes(b"changed runner source")
                changed = True
            return result

        with self.assertRaisesRegex(ValueError, "source changed"):
            self.run_calibration(command_runner=mutate_runner)
        rows_dir = self.root / "run" / "rows"
        self.assertEqual(list(rows_dir.iterdir()), [])

    def test_run_rejects_launcher_toctou_and_duplicate_or_missing_output(self):
        changed = False

        def mutate_launcher(command, **kwargs):
            nonlocal changed
            result = self.command_runner(command, **kwargs)
            if not changed:
                self.launcher.write_bytes(b"changed launcher")
                changed = True
            return result

        with self.assertRaisesRegex(ValueError, "launcher changed"):
            self.run_calibration(command_runner=mutate_launcher)
        self.assertEqual(list((self.root / "run" / "rows").iterdir()), [])

        self.launcher.write_bytes(b"bound launcher")
        for line_count in (0, 2):
            with self.subTest(line_count=line_count):
                run_dir = self.root / f"run-{line_count}"

                def wrong_count(command, **kwargs):
                    output_path = pathlib.Path(
                        command[command.index("--jsonl-out") + 1]
                    )
                    row = self.rows[0]
                    line = opcode_gas.canonical_json(self.report(row)) + b"\n"
                    output_path.write_bytes(line * line_count)
                    return subprocess.CompletedProcess(command, 0, b"", b"")

                with mock.patch.object(
                    block_calibration.block_comparison,
                    "verify_sealed_manifest",
                    return_value=self.manifest,
                ), mock.patch.object(
                    block_calibration,
                    "_require_eligible_manifest",
                ), mock.patch.object(
                    block_calibration,
                    "_current_implementation_identity",
                    return_value=self.implementation_identity(),
                ), mock.patch.object(
                    block_calibration,
                    "_runner_source_path",
                    return_value=self.runner_source,
                ), self.assertRaisesRegex(ValueError, "duplicate or missing"):
                    block_calibration.run_calibration(
                        manifest_path=self.manifest_path,
                        source_paths=self.source_paths,
                        launcher_path=self.launcher,
                        run_dir=run_dir,
                        command_runner=wrong_count,
                    )
                self.assertEqual(list((run_dir / "rows").iterdir()), [])

    def test_launcher_timeout_is_frozen_and_does_not_publish_a_row(self):
        def timeout(command, **kwargs):
            self.assertEqual(kwargs["timeout"], 600)
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])

        with self.assertRaisesRegex(ValueError, "timed out"):
            self.run_calibration(command_runner=timeout)
        self.assertEqual(list((self.root / "run" / "rows").iterdir()), [])

    def test_atomic_create_survives_interruption_collision_and_hardened_umask(self):
        target = self.root / "checkpoint.json"
        with mock.patch.object(os, "fsync", side_effect=OSError("interrupted")):
            with self.assertRaisesRegex(OSError, "interrupted"):
                block_calibration._durable_create(target, b"first\n")
        self.assertFalse(target.exists())

        previous_umask = os.umask(0o077)
        try:
            block_calibration._durable_create(target, b"complete\n")
        finally:
            os.umask(previous_umask)
        self.assertEqual(target.read_bytes(), b"complete\n")
        self.assertEqual(target.stat().st_mode & 0o777, 0o444)
        with self.assertRaises(FileExistsError):
            block_calibration._durable_create(target, b"replacement\n")
        self.assertEqual(target.read_bytes(), b"complete\n")

    def test_run_rejects_symlink_root(self):
        actual = self.root / "actual-run"
        actual.mkdir()
        linked = self.root / "linked-run"
        linked.symlink_to(actual, target_is_directory=True)
        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            return_value=self.manifest,
        ), mock.patch.object(
            block_calibration,
            "_require_eligible_manifest",
        ), mock.patch.object(
            block_calibration,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_runner_source_path",
            return_value=self.runner_source,
        ), self.assertRaisesRegex(ValueError, "run root"):
            block_calibration.run_calibration(
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                launcher_path=self.launcher,
                run_dir=linked,
                command_runner=self.command_runner,
            )

    def test_run_rejects_child_symlink_before_cleanup_or_execution(self):
        run_dir = self.root / "child-link-run"
        run_dir.mkdir()
        external = self.root / "external"
        external.mkdir()
        sentinel = external / ".row.json.partial.999999.orphan"
        sentinel.write_bytes(b"must remain")
        (run_dir / "inputs").symlink_to(external, target_is_directory=True)
        commands = mock.Mock()
        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            return_value=self.manifest,
        ), mock.patch.object(
            block_calibration,
            "_require_eligible_manifest",
        ), mock.patch.object(
            block_calibration,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_runner_source_path",
            return_value=self.runner_source,
        ), self.assertRaisesRegex(ValueError, "row inventory"):
            block_calibration.run_calibration(
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                launcher_path=self.launcher,
                run_dir=run_dir,
                command_runner=commands,
            )
        self.assertEqual(sentinel.read_bytes(), b"must remain")
        commands.assert_not_called()

    def test_implementation_revision_survives_data_commit_but_rejects_source_drift(self):
        repository = self.root / "git-repository"
        repository.mkdir()
        subprocess.run(["git", "init", "-q"], cwd=repository, check=True)
        subprocess.run(
            ["git", "config", "user.email", "fixture@example.invalid"],
            cwd=repository,
            check=True,
        )
        subprocess.run(
            ["git", "config", "user.name", "Fixture User"],
            cwd=repository,
            check=True,
        )
        for index, relative in enumerate(block_calibration.IMPLEMENTATION_SOURCE_PATHS):
            path = repository / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(f"source {index}\n")
        subprocess.run(["git", "add", "."], cwd=repository, check=True)
        subprocess.run(
            ["git", "commit", "-qm", "implementation"], cwd=repository, check=True
        )
        with mock.patch.object(block_calibration, "REPO_ROOT", repository):
            revision, hashes = block_calibration._current_implementation_identity()
            data = repository / "data.json"
            data.write_text("{}\n")
            subprocess.run(["git", "add", "data.json"], cwd=repository, check=True)
            subprocess.run(
                ["git", "commit", "-qm", "data"], cwd=repository, check=True
            )
            self.assertEqual(
                block_calibration._implementation_identity_at_revision(revision),
                (revision, hashes),
            )
            source = repository / block_calibration.IMPLEMENTATION_SOURCE_PATHS[0]
            source.write_text("dirty\n")
            with self.assertRaisesRegex(ValueError, "clean tracked"):
                block_calibration._implementation_identity_at_revision(revision)
            subprocess.run(
                ["git", "checkout", "--", block_calibration.IMPLEMENTATION_SOURCE_PATHS[0]],
                cwd=repository,
                check=True,
            )
            subprocess.run(
                ["git", "rm", "--cached", block_calibration.IMPLEMENTATION_SOURCE_PATHS[0]],
                cwd=repository,
                check=True,
                stdout=subprocess.PIPE,
            )
            with self.assertRaisesRegex(ValueError, "clean tracked"):
                block_calibration._implementation_identity_at_revision(revision)
            (repository / ".gitignore").write_text(
                block_calibration.IMPLEMENTATION_SOURCE_PATHS[0] + "\n"
            )
            with self.assertRaisesRegex(ValueError, "clean tracked"):
                block_calibration._implementation_identity_at_revision(revision)

    def test_only_final_candidate_manifest_pair_is_eligible(self):
        manifest_path = (
            ROOT
            / block_calibration.ELIGIBLE_MANIFEST_RELATIVE_PATH
        )
        manifest = json.loads(manifest_path.read_bytes())
        block_calibration._require_eligible_manifest(manifest_path, manifest)
        mutations = {
            "candidate": ("candidate_identity", "0" * 64),
            "manifest": ("artifact_sha256", "0" * 64),
        }
        for name, (field, value) in mutations.items():
            changed = copy.deepcopy(manifest)
            changed[field] = value
            with self.subTest(name=name), self.assertRaises(ValueError):
                block_calibration._require_eligible_manifest(manifest_path, changed)
        alternate = self.root / "manifest.json"
        alternate.write_bytes(manifest_path.read_bytes())
        with self.assertRaisesRegex(ValueError, "eligible"):
            block_calibration._require_eligible_manifest(alternate, manifest)
        exact_verify = mock.Mock()
        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            exact_verify,
        ):
            rejected_calls = (
                lambda: block_calibration.run_calibration(
                    manifest_path=alternate,
                    source_paths=self.source_paths,
                    launcher_path=self.launcher,
                    run_dir=self.root / "ineligible-run",
                    command_runner=self.command_runner,
                ),
                lambda: block_calibration.seal_calibration_artifact(
                    manifest_path=alternate,
                    source_paths=self.source_paths,
                    run_dir=self.root / "missing-run",
                    output_root=self.root / "calibration-output",
                ),
                lambda: block_calibration.verify_calibration_artifact(
                    self.root / "missing-calibration.json",
                    manifest_path=alternate,
                    source_paths=self.source_paths,
                ),
                lambda: block_calibration.seal_normalization_artifact(
                    calibration_path=self.root / "missing-calibration.json",
                    manifest_path=alternate,
                    source_paths=self.source_paths,
                    output_root=self.root / "normalization-output",
                ),
            )
            for operation in rejected_calls:
                with self.assertRaisesRegex(ValueError, "eligible"):
                    operation()
        exact_verify.assert_not_called()

    def test_calibration_and_normalization_artifacts_are_exact_portable_and_tamper_evident(self):
        self.assertEqual(
            block_calibration.ACCEPTANCE_THRESHOLDS,
            block_calibration.context_approximation.BLOCK_VALIDATION_ACCEPTANCE_THRESHOLDS,
        )
        reports = self.run_calibration()
        calibration_root = self.root / "calibration-artifacts"
        normalization_root = self.root / "normalization-artifacts"
        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            return_value=self.manifest,
        ), mock.patch.object(
            block_calibration,
            "_require_eligible_manifest",
        ), mock.patch.object(
            block_calibration,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_implementation_identity_at_revision",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_runner_source_path",
            return_value=self.runner_source,
        ):
            sealed_calibration = block_calibration.seal_calibration_artifact(
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                run_dir=self.root / "run",
                output_root=calibration_root,
            )
            calibration = json.loads(sealed_calibration.read_bytes())
            self.assertEqual(len(calibration["rows"]), 12)
            self.assertFalse(
                any("validation" in json.dumps(row) for row in calibration["rows"])
            )
            self.assertEqual(sealed_calibration.stat().st_mode & 0o777, 0o444)
            self.assertEqual(
                block_calibration.verify_calibration_artifact(
                    sealed_calibration,
                    manifest_path=self.manifest_path,
                    source_paths=self.source_paths,
                ),
                calibration,
            )
            sealed_calibration.chmod(0o644)
            self.assertEqual(
                block_calibration.verify_calibration_artifact(
                    sealed_calibration,
                    manifest_path=self.manifest_path,
                    source_paths=self.source_paths,
                ),
                calibration,
            )
            sealed_normalization = block_calibration.seal_normalization_artifact(
                calibration_path=sealed_calibration,
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                output_root=normalization_root,
            )
            normalization = json.loads(sealed_normalization.read_bytes())
            self.assertEqual(
                block_calibration.verify_normalization_artifact(
                    sealed_normalization,
                    calibration_path=sealed_calibration,
                    manifest_path=self.manifest_path,
                    source_paths=self.source_paths,
                ),
                normalization,
            )
        with localcontext() as context:
            context.prec = 100
            expected_ratios = [
                Decimal(reports[index]["gas"])
                / Decimal(
                    self.rows[index]["frozen_identity"]["observation"][
                        "finalized_block_zkgas"
                    ]
                )
                for index in range(12)
            ]
            expected_ratios.sort()
            expected_median = +(expected_ratios[5] + expected_ratios[6]) / 2
        self.assertEqual(Decimal(normalization["kappa_unzen"]), expected_median)
        self.assertEqual(normalization["status"], "sealed_before_validation")
        self.assertTrue(
            all(
                row["partition"] == "calibration"
                for row in normalization["calibration_rows"]
            )
        )
        self.assertTrue(
            {row["row_id"] for row in normalization["calibration_rows"]}.isdisjoint(
                {row["row_id"] for row in self.rows[12:]}
            )
        )

        for mode in (0o600, 0o666, 0o755):
            sealed_calibration.chmod(mode)
            with self.subTest(mode=oct(mode)), self.assertRaises(ValueError):
                block_calibration._verify_document(
                    sealed_calibration,
                    filename="calibration.json",
                    validator=block_calibration.validate_calibration_artifact,
                )
        sealed_calibration.chmod(0o444)
        with mock.patch.object(
            block_calibration.block_comparison,
            "verify_sealed_manifest",
            return_value=self.manifest,
        ), mock.patch.object(
            block_calibration,
            "_require_eligible_manifest",
        ), mock.patch.object(
            block_calibration,
            "_current_implementation_identity",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_implementation_identity_at_revision",
            return_value=self.implementation_identity(),
        ), mock.patch.object(
            block_calibration,
            "_runner_source_path",
            return_value=self.runner_source,
        ), self.assertRaises(FileExistsError):
            block_calibration.seal_calibration_artifact(
                manifest_path=self.manifest_path,
                source_paths=self.source_paths,
                run_dir=self.root / "run",
                output_root=calibration_root,
            )
        tampered = copy.deepcopy(normalization)
        tampered["kappa_unzen"] = "999"
        tampered["artifact_sha256"] = block_calibration._content_address(tampered)
        with self.assertRaises(ValueError):
            block_calibration.validate_normalization_artifact(tampered)
        sealed_normalization.parent.chmod(0o755)
        (sealed_normalization.parent / "extra").write_text("tamper")
        with self.assertRaises(ValueError):
            block_calibration._verify_document(
                sealed_normalization,
                filename="normalization.json",
                validator=block_calibration.validate_normalization_artifact,
            )

        linked_root = self.root / "linked-artifact"
        linked_root.symlink_to(sealed_calibration.parent, target_is_directory=True)
        with self.assertRaises(ValueError):
            block_calibration._verify_document(
                linked_root / "calibration.json",
                filename="calibration.json",
                validator=block_calibration.validate_calibration_artifact,
            )


if __name__ == "__main__":
    unittest.main()
