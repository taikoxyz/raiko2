from __future__ import annotations

import copy
import dataclasses
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


EXPERIMENT_ROOT = pathlib.Path(__file__).resolve().parents[1]
FIXTURES = pathlib.Path(__file__).resolve().parent / "fixtures"
sys.path.insert(0, str(EXPERIMENT_ROOT))

import risc0_zkgas


def candidate_manifest(*, network: str = "taiko_mainnet", count: int = 1) -> dict:
    split = "holdout" if network == "taiko_mainnet" else "fit"
    return {
        "schema_version": 1,
        "network": network,
        "split_targets": {split: count},
        "candidates": [
            {"proposal_id": 42 + index, "split": split} for index in range(count)
        ],
    }


class ManifestAndFeatureTests(unittest.TestCase):
    def test_real_default_chain_spec_resolves_pinned_l1_and_unzen_for_both_networks(self):
        chain_spec_path = EXPERIMENT_ROOT.parents[1] / "config" / "chain_spec_list_default.json"
        expected = {
            "taiko_mainnet": ("ethereum", 1786021200),
            "taiko_hoodi": ("hoodi", 1781787600),
        }
        for network, (l1_network, unzen_timestamp) in expected.items():
            with self.subTest(network=network):
                self.assertEqual(
                    risc0_zkgas.SUPPORTED_NETWORKS[network]["l1_network"],
                    l1_network,
                )
                self.assertEqual(
                    risc0_zkgas._resolve_unzen_timestamp(chain_spec_path, network),
                    unzen_timestamp,
                )

    def test_manifest_is_finite_network_scoped_and_unique(self):
        selected = risc0_zkgas.validate_candidate_manifest(
            candidate_manifest(count=2), "taiko_mainnet", max_candidates=1
        )
        self.assertEqual([candidate.proposal_id for candidate in selected], [42])

        unsupported = candidate_manifest(network="taiko_devnet")
        with self.assertRaisesRegex(ValueError, "unsupported network"):
            risc0_zkgas.validate_candidate_manifest(
                unsupported, "taiko_devnet", max_candidates=1
            )

        duplicate = candidate_manifest(count=2)
        duplicate["candidates"][1]["proposal_id"] = 42
        with self.assertRaisesRegex(ValueError, "duplicate proposal_id"):
            risc0_zkgas.validate_candidate_manifest(
                duplicate, "taiko_mainnet", max_candidates=2
            )

    def test_split_targets_are_mandatory_exact_and_covered_by_candidates(self):
        manifest = {
            "schema_version": 1,
            "network": "taiko_hoodi",
            "split_targets": {"fit": 1, "calibration": 1},
            "candidates": [
                {"proposal_id": 1, "split": "fit"},
                {"proposal_id": 2, "split": "calibration"},
            ],
        }
        candidates = risc0_zkgas.validate_candidate_manifest(
            manifest, "taiko_hoodi", max_candidates=2
        )
        self.assertEqual(
            risc0_zkgas.validate_split_targets(
                manifest, "taiko_hoodi", target_count=2, candidates=candidates
            ),
            {"fit": 1, "calibration": 1},
        )

        missing = copy.deepcopy(manifest)
        missing.pop("split_targets")
        with self.assertRaisesRegex(ValueError, "split_targets"):
            risc0_zkgas.validate_split_targets(
                missing, "taiko_hoodi", target_count=2, candidates=candidates
            )

        wrong_total = copy.deepcopy(manifest)
        wrong_total["split_targets"]["fit"] = 2
        with self.assertRaisesRegex(ValueError, "target_count"):
            risc0_zkgas.validate_split_targets(
                wrong_total, "taiko_hoodi", target_count=2, candidates=candidates
            )

        with self.assertRaisesRegex(ValueError, "not enough calibration candidates"):
            risc0_zkgas.validate_split_targets(
                manifest, "taiko_hoodi", target_count=2, candidates=candidates[:1]
            )

    def test_exact_guest_input_features_use_post_unzen_nonzero_difficulty(self):
        path = FIXTURES / "guest-input-post-unzen.json"
        guest_input = json.loads(path.read_text())

        features = risc0_zkgas.extract_guest_input_features(
            guest_input,
            network="taiko_mainnet",
            proposal_id=42,
            l2_start=100,
            l2_end=101,
            guest_input_json_bytes=path.stat().st_size,
            unzen_timestamp=1786021200,
        )

        self.assertEqual(features["total_zkgas"], 300)
        self.assertEqual(features["min_block_zkgas"], 100)
        self.assertEqual(features["median_block_zkgas"], 150.0)
        self.assertEqual(features["p95_block_zkgas"], 200)
        self.assertEqual(features["max_block_zkgas"], 200)
        self.assertEqual(features["block_count"], 2)
        self.assertEqual(features["transaction_count"], 3)
        self.assertEqual(features["proposal_state_node_count"], 3)
        self.assertEqual(features["witness_state_index_count"], 3)
        self.assertEqual(features["witness_code_count"], 3)
        self.assertEqual(features["guest_input_json_bytes"], path.stat().st_size)

        pre_unzen = copy.deepcopy(guest_input)
        pre_unzen["witnesses"][0]["block"]["header"]["timestamp"] -= 1
        with self.assertRaisesRegex(risc0_zkgas.TerminalSampleError, "pre-Unzen"):
            risc0_zkgas.extract_guest_input_features(
                pre_unzen,
                network="taiko_mainnet",
                proposal_id=42,
                l2_start=100,
                l2_end=101,
                guest_input_json_bytes=1,
                unzen_timestamp=1786021200,
            )

        zero_difficulty = copy.deepcopy(guest_input)
        zero_difficulty["witnesses"][1]["block"]["header"]["difficulty"] = "0x0"
        with self.assertRaisesRegex(risc0_zkgas.TerminalSampleError, "non-zero difficulty"):
            risc0_zkgas.extract_guest_input_features(
                zero_difficulty,
                network="taiko_mainnet",
                proposal_id=42,
                l2_start=100,
                l2_end=101,
                guest_input_json_bytes=1,
                unzen_timestamp=1786021200,
            )

        wrong_manifest_identity = copy.deepcopy(guest_input)
        wrong_manifest_identity["taiko"]["chain_spec"]["is_taiko"] = False
        with self.assertRaisesRegex(
            risc0_zkgas.TerminalSampleError, "Taiko chain"
        ):
            risc0_zkgas.extract_guest_input_features(
                wrong_manifest_identity,
                network="taiko_mainnet",
                proposal_id=42,
                l2_start=100,
                l2_end=101,
                guest_input_json_bytes=1,
                unzen_timestamp=1786021200,
            )

    def test_launcher_report_requires_authoritative_risc0_frame_bytes(self):
        report = risc0_zkgas.parse_launcher_report(
            json.loads((FIXTURES / "launcher-report.json").read_text())
        )
        self.assertEqual(report["risc0_image_id"], "0ximage")
        self.assertEqual(report["risc0_input_bytes"], 12345)
        self.assertEqual(report["evaluated_mcycles_count"], 2501)
        self.assertEqual(report["current_quote_bucket_mcycles"], 3000)

        missing = json.loads((FIXTURES / "launcher-report.json").read_text())
        missing.pop("risc0_input_bytes")
        missing["guest_input_json_bytes"] = 999999
        with self.assertRaisesRegex(risc0_zkgas.TerminalSampleError, "risc0_input_bytes"):
            risc0_zkgas.parse_launcher_report(missing)

        missing_image = json.loads((FIXTURES / "launcher-report.json").read_text())
        missing_image.pop("risc0_image_id")
        with self.assertRaisesRegex(risc0_zkgas.TerminalSampleError, "risc0_image_id"):
            risc0_zkgas.parse_launcher_report(missing_image)


class FakeRunner:
    def __init__(self, fixture_path: pathlib.Path):
        self.fixture_path = fixture_path
        self.commands: list[list[str]] = []

    def __call__(self, command, **kwargs):
        command = [str(part) for part in command]
        self.commands.append(command)
        if "--discover-only" in command:
            network = command[command.index("--network") + 1]
            proposal_id = int(command[command.index("--proposal-id") + 1])
            output = pathlib.Path(command[command.index("--proposal-out") + 1])
            output.write_text(
                json.dumps(
                    {
                        "proposals": [
                            {
                                "network": network,
                                "l1_network": risc0_zkgas.SUPPORTED_NETWORKS[network][
                                    "l1_network"
                                ],
                                "proposal_id": proposal_id,
                                "l1_inclusion_block_number": 500,
                                "last_anchor_block_number": 499,
                                "l2_start": 100,
                                "l2_end": 101,
                                "l2_block_numbers": [100, 101],
                            }
                        ]
                    }
                )
            )
        elif pathlib.Path(command[0]).name == "preflight":
            output = pathlib.Path(command[command.index("--output") + 1])
            guest_input = json.loads(self.fixture_path.read_text())
            network = command[command.index("--network") + 1]
            proposal_id = int(command[command.index("--proposal-id") + 1])
            activation = risc0_zkgas.SUPPORTED_NETWORKS[network]["unzen_timestamp"]
            guest_input["taiko"]["proposal_id"] = proposal_id
            guest_input["taiko"]["chain_spec"]["name"] = network
            guest_input["taiko"]["chain_spec"]["chain_id"] = (
                167000 if network == "taiko_mainnet" else 167013
            )
            for offset, witness in enumerate(guest_input["witnesses"]):
                witness["block"]["header"]["timestamp"] = activation + offset
            output.write_text(json.dumps(guest_input))
        elif pathlib.Path(command[0]).name == "guest-launcher":
            output = pathlib.Path(command[command.index("--json-out") + 1])
            output.write_bytes((FIXTURES / "launcher-report.json").read_bytes())
        else:
            raise AssertionError(f"unexpected command: {command}")
        return subprocess.CompletedProcess(command, 0, stdout="", stderr="")


class RetryOnceRunner(FakeRunner):
    def __init__(self, fixture_path: pathlib.Path):
        super().__init__(fixture_path)
        self.failed = False

    def __call__(self, command, **kwargs):
        if not self.failed:
            self.failed = True
            self.commands.append([str(part) for part in command])
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])
        return super().__call__(command, **kwargs)


class PartialDiscoveryTimeoutRunner(FakeRunner):
    def __init__(self, fixture_path: pathlib.Path):
        super().__init__(fixture_path)
        self.failed = False

    def __call__(self, command, **kwargs):
        command = [str(part) for part in command]
        if "--discover-only" in command and not self.failed:
            self.failed = True
            self.commands.append(command)
            output = pathlib.Path(command[command.index("--proposal-out") + 1])
            output.write_text('{"proposals":')
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])
        return super().__call__(command, **kwargs)


class PartialPreflightTimeoutRunner(FakeRunner):
    def __init__(self, fixture_path: pathlib.Path):
        super().__init__(fixture_path)
        self.failed = False

    def __call__(self, command, **kwargs):
        command = [str(part) for part in command]
        if pathlib.Path(command[0]).name == "preflight" and not self.failed:
            self.failed = True
            self.commands.append(command)
            output = pathlib.Path(command[command.index("--output") + 1])
            output.write_text('{"taiko":')
            raise subprocess.TimeoutExpired(command, kwargs["timeout"])
        return super().__call__(command, **kwargs)


class TerminalRunner(FakeRunner):
    def __call__(self, command, **kwargs):
        command = [str(part) for part in command]
        self.commands.append(command)
        return subprocess.CompletedProcess(
            command, 1, stdout="", stderr="invalid proposal tuple"
        )


class NondeterministicRunner(FakeRunner):
    def __init__(self, fixture_path: pathlib.Path):
        super().__init__(fixture_path)
        self.launch_count = 0

    def __call__(self, command, **kwargs):
        result = super().__call__(command, **kwargs)
        command = [str(part) for part in command]
        if pathlib.Path(command[0]).name == "guest-launcher":
            self.launch_count += 1
            if self.launch_count == 2:
                output = pathlib.Path(command[command.index("--json-out") + 1])
                report = json.loads(output.read_text())
                report["risc0_user_cycles"] += 1
                output.write_text(json.dumps(report))
        return result


class RepeatTimeoutOnceRunner(FakeRunner):
    def __init__(self, fixture_path: pathlib.Path):
        super().__init__(fixture_path)
        self.launch_count = 0
        self.failed = False

    def __call__(self, command, **kwargs):
        command = [str(part) for part in command]
        if pathlib.Path(command[0]).name == "guest-launcher":
            self.launch_count += 1
            if self.launch_count == 2 and not self.failed:
                self.failed = True
                self.commands.append(command)
                raise subprocess.TimeoutExpired(command, kwargs["timeout"])
        return super().__call__(command, **kwargs)


class WrongImageRunner(FakeRunner):
    def __call__(self, command, **kwargs):
        result = super().__call__(command, **kwargs)
        command = [str(part) for part in command]
        if pathlib.Path(command[0]).name == "guest-launcher":
            output = pathlib.Path(command[command.index("--json-out") + 1])
            report = json.loads(output.read_text())
            report["risc0_image_id"] = "0xwrong"
            output.write_text(json.dumps(report))
        return result


class MalformedReportRunner(FakeRunner):
    def __call__(self, command, **kwargs):
        result = super().__call__(command, **kwargs)
        command = [str(part) for part in command]
        if pathlib.Path(command[0]).name == "guest-launcher":
            output = pathlib.Path(command[command.index("--json-out") + 1])
            output.write_text("not JSON")
        return result


class CollectorTests(unittest.TestCase):
    def make_config(self, root: pathlib.Path, *, target_count: int = 1):
        root.mkdir(parents=True, exist_ok=True)
        manifest_path = root / "candidates.json"
        manifest_path.write_text(json.dumps(candidate_manifest()))
        proposal_elf = root / "artifacts" / "risc0_shasta_proposal.elf"
        preflight_bin = root / "bin" / "preflight"
        guest_launcher_bin = root / "bin" / "guest-launcher"
        stress_script = root / "scripts" / "stress_shasta_proposal.py"
        chain_spec_list = root / "config" / "chain_spec_list.json"
        for path, contents in (
            (proposal_elf, b"proposal elf"),
            (preflight_bin, b"preflight binary"),
            (guest_launcher_bin, b"guest launcher binary"),
            (stress_script, b"# discovery script\n"),
            (
                chain_spec_list,
                json.dumps(
                    [
                        {"name": "taiko_mainnet"},
                        {"name": "ethereum"},
                        {"name": "taiko_hoodi"},
                        {"name": "hoodi"},
                    ]
                ).encode(),
            ),
        ):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(contents)
        (root / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "risc0-zkvm"\nversion = "3.0.5"\n'
        )
        return risc0_zkgas.CollectorConfig(
            network="taiko_mainnet",
            candidate_manifest=manifest_path,
            target_count=target_count,
            max_candidates=10,
            output_dir=root / "run",
            source_revision="abc123",
            image_id="0ximage",
            risc0_version="3.0.5",
            execution_po2=20,
            proposal_elf=proposal_elf,
            preflight_bin=preflight_bin,
            guest_launcher_bin=guest_launcher_bin,
            stress_script=stress_script,
            chain_spec_list=chain_spec_list,
            repo_root=root,
            revision_resolver=lambda _root: "abc123",
        )

    def test_collector_is_resumable_idempotent_and_execute_only(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = FakeRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)
            first_commands = list(runner.commands)
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)
            self.assertEqual(runner.commands, first_commands)

            rows = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["status"], "success")
            self.assertEqual(rows[0]["risc0_input_bytes"], 12345)
            cached_input = next((config.output_dir / "inputs").glob("*.json"))
            self.assertEqual(rows[0]["guest_input_json_bytes"], cached_input.stat().st_size)
            launcher_command = next(
                command
                for command in runner.commands
                if pathlib.Path(command[0]).name == "guest-launcher"
            )
            self.assertIn("execute", launcher_command)
            self.assertNotIn("prove", launcher_command)
            self.assertNotIn("boundless", " ".join(launcher_command).lower())
            discovery_command = next(
                command for command in runner.commands if "--discover-only" in command
            )
            preflight_command = next(
                command
                for command in runner.commands
                if pathlib.Path(command[0]).name == "preflight"
            )
            self.assertEqual(
                discovery_command[discovery_command.index("--chain-spec-list") + 1],
                preflight_command[preflight_command.index("--chain-spec-file") + 1],
            )

            run_manifest = json.loads(
                (config.output_dir / "run-manifest.json").read_text()
            )
            self.assertEqual(
                run_manifest["artifact_hashes"]["proposal_elf_sha256"],
                hashlib.sha256(config.proposal_elf.read_bytes()).hexdigest(),
            )
            self.assertEqual(run_manifest["resolved_risc0_version"], "3.0.5")

            progress = json.loads((config.output_dir / "progress.json").read_text())
            self.assertEqual(progress["successful_samples"], 1)
            self.assertEqual(progress["shortfall"], 0)
            self.assertTrue(progress["target_reached"])

    def test_collector_repairs_only_a_torn_final_jsonl_record(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = FakeRunner(FIXTURES / "guest-input-post-unzen.json")
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)
            command_count = len(runner.commands)
            samples_path = config.output_dir / "samples.jsonl"
            with samples_path.open("ab") as handle:
                handle.write(b'{"partial":')
                handle.flush()

            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            self.assertEqual(len(risc0_zkgas.read_jsonl(samples_path)), 1)
            self.assertEqual(len(runner.commands), command_count)
            self.assertTrue(samples_path.read_bytes().endswith(b"\n"))

    def test_jsonl_repair_rejects_newline_terminated_and_interior_malformed_rows(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            newline_tail = root / "newline-tail.jsonl"
            newline_tail.write_bytes(b'{}\n{"partial":\n')
            with self.assertRaises(json.JSONDecodeError):
                risc0_zkgas.read_jsonl(
                    newline_tail, repair_truncated_tail=True
                )

            interior = root / "interior.jsonl"
            interior.write_bytes(b'{}\n{"partial":\n{}\n')
            with self.assertRaises(json.JSONDecodeError):
                risc0_zkgas.read_jsonl(interior, repair_truncated_tail=True)

    def test_jsonl_repair_terminates_a_valid_final_record_before_next_append(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = pathlib.Path(tmpdir) / "samples.jsonl"
            path.write_bytes(b'{"proposal_id":1}')

            self.assertEqual(
                risc0_zkgas.read_jsonl(path, repair_truncated_tail=True),
                [{"proposal_id": 1}],
            )
            self.assertTrue(path.read_bytes().endswith(b"\n"))

            risc0_zkgas.append_jsonl_fsync(path, {"proposal_id": 2})
            self.assertEqual(
                risc0_zkgas.read_jsonl(path),
                [{"proposal_id": 1}, {"proposal_id": 2}],
            )

    def test_rebuilt_binary_and_false_caller_identity_cannot_reuse_cohort(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = FakeRunner(FIXTURES / "guest-input-post-unzen.json")
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            config.preflight_bin.write_bytes(b"rebuilt preflight")
            with self.assertRaisesRegex(ValueError, "run manifest"):
                risc0_zkgas.collect(config, runner=runner)

            wrong_revision = dataclasses.replace(
                self.make_config(root / "revision"), source_revision="wrong"
            )
            with self.assertRaisesRegex(ValueError, "source_revision"):
                risc0_zkgas.collect(wrong_revision, runner=runner)

            wrong_version = dataclasses.replace(
                self.make_config(root / "version"), risc0_version="9.9.9"
            )
            with self.assertRaisesRegex(ValueError, "risc0_version"):
                risc0_zkgas.collect(wrong_version, runner=runner)

            wrong_schedule = self.make_config(root / "schedule")
            wrong_schedule.chain_spec_list.write_text(
                json.dumps(
                    [
                        {
                            "name": "taiko_mainnet",
                            "hard_forks": {
                                "UNZEN": {"Timestamp": 1786021199}
                            },
                        },
                        {"name": "ethereum"},
                    ]
                )
            )
            with self.assertRaisesRegex(ValueError, "UNZEN timestamp"):
                risc0_zkgas.collect(wrong_schedule, runner=runner)

    def test_chain_spec_identity_requires_unique_selected_l1_and_l2_entries(self):
        cases = {
            "missing L1": [{"name": "taiko_mainnet"}],
            "missing L2": [{"name": "ethereum"}],
            "duplicate L1": [
                {"name": "taiko_mainnet"},
                {"name": "ethereum"},
                {"name": "ethereum"},
            ],
            "duplicate L2": [
                {"name": "taiko_mainnet"},
                {"name": "taiko_mainnet"},
                {"name": "ethereum"},
            ],
        }
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            for label, entries in cases.items():
                with self.subTest(label=label):
                    config = self.make_config(root / label.replace(" ", "-"))
                    config.chain_spec_list.write_text(json.dumps(entries))
                    with self.assertRaisesRegex(ValueError, label):
                        risc0_zkgas.collect(
                            config,
                            runner=FakeRunner(
                                FIXTURES / "guest-input-post-unzen.json"
                            ),
                        )
                    self.assertFalse(config.output_dir.exists())

    def test_manifest_exhaustion_records_shortfall_and_returns_nonzero(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = TerminalRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)

            progress = json.loads((config.output_dir / "progress.json").read_text())
            self.assertEqual(progress["successful_samples"], 0)
            self.assertEqual(progress["shortfall"], 1)
            self.assertTrue(progress["manifest_exhausted"])

    def test_collector_requires_each_split_quota_before_success(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root, target_count=2)
            config.candidate_manifest.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "network": "taiko_hoodi",
                        "split_targets": {"fit": 1, "calibration": 1},
                        "candidates": [
                            {"proposal_id": 42, "split": "fit"},
                            {"proposal_id": 43, "split": "fit"},
                            {"proposal_id": 44, "split": "calibration"},
                        ],
                    }
                )
            )
            config = dataclasses.replace(config, network="taiko_hoodi")
            runner = FakeRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            rows = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")
            self.assertEqual(
                [(row["proposal_id"], row["split"]) for row in rows],
                [(42, "fit"), (44, "calibration")],
            )
            progress = json.loads((config.output_dir / "progress.json").read_text())
            self.assertEqual(
                progress["successful_samples_by_split"],
                {"fit": 1, "calibration": 1},
            )
            self.assertEqual(
                progress["target_counts_by_split"],
                {"fit": 1, "calibration": 1},
            )
            self.assertEqual(
                progress["shortfall_by_split"], {"fit": 0, "calibration": 0}
            )

    def test_retryable_failure_is_retried_but_terminal_failure_is_not(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            retry_config = self.make_config(root / "retry")
            retry_runner = RetryOnceRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(retry_config, runner=retry_runner), 0)
            self.assertEqual(
                risc0_zkgas.read_jsonl(retry_config.output_dir / "samples.jsonl")[0]["status"],
                "retryable_failure",
            )
            self.assertEqual(risc0_zkgas.collect(retry_config, runner=retry_runner), 0)
            self.assertEqual(
                [
                    row["status"]
                    for row in risc0_zkgas.read_jsonl(
                        retry_config.output_dir / "samples.jsonl"
                    )
                ],
                ["retryable_failure", "success"],
            )

            terminal_config = self.make_config(root / "terminal")
            terminal_runner = TerminalRunner(FIXTURES / "guest-input-post-unzen.json")
            self.assertNotEqual(
                risc0_zkgas.collect(terminal_config, runner=terminal_runner), 0
            )
            command_count = len(terminal_runner.commands)
            self.assertNotEqual(
                risc0_zkgas.collect(terminal_config, runner=terminal_runner), 0
            )
            self.assertEqual(len(terminal_runner.commands), command_count)
            terminal_row = risc0_zkgas.read_jsonl(
                terminal_config.output_dir / "samples.jsonl"
            )[0]
            self.assertEqual(terminal_row["status"], "terminal_failure")
            self.assertEqual(terminal_row["failure_stage"], "discovery")

    def test_partial_discovery_timeout_is_not_published_and_resume_succeeds(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config = self.make_config(pathlib.Path(tmpdir))
            runner = PartialDiscoveryTimeoutRunner(
                FIXTURES / "guest-input-post-unzen.json"
            )

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            rows = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")
            self.assertEqual(
                [row["status"] for row in rows], ["retryable_failure", "success"]
            )

    def test_partial_preflight_timeout_is_not_published_and_resume_succeeds(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config = self.make_config(pathlib.Path(tmpdir))
            runner = PartialPreflightTimeoutRunner(
                FIXTURES / "guest-input-post-unzen.json"
            )

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            rows = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")
            self.assertEqual(
                [row["status"] for row in rows], ["retryable_failure", "success"]
            )

    def test_malformed_published_cache_is_observable_and_retryable(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config = self.make_config(pathlib.Path(tmpdir))
            discovery_dir = config.output_dir / "discovery"
            discovery_dir.mkdir(parents=True)
            image_stem = risc0_zkgas._safe_stem(config.image_id)
            cache = discovery_dir / f"taiko_mainnet-42-{image_stem}.json"
            cache.write_text('{"proposals":')
            runner = FakeRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)
            first_row = risc0_zkgas.read_jsonl(
                config.output_dir / "samples.jsonl"
            )[0]
            self.assertEqual(first_row["status"], "retryable_failure")
            self.assertEqual(first_row["failure_class"], "cache")
            self.assertIn("removed malformed cached discovery", first_row["error"])

            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

    def test_determinism_mismatch_is_a_terminal_result(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = dataclasses.replace(
                self.make_config(root), determinism_rate=1.0
            )
            runner = NondeterministicRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)

            row = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")[0]
            self.assertEqual(row["status"], "terminal_failure")
            self.assertIn("deterministic repeat", row["error"])
            self.assertTrue(row["determinism_checked"])
            self.assertFalse(row["determinism_match"])

    def test_repeat_timeout_is_retryable_but_not_a_completed_determinism_check(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            config = dataclasses.replace(
                self.make_config(pathlib.Path(tmpdir)), determinism_rate=1.0
            )
            runner = RepeatTimeoutOnceRunner(
                FIXTURES / "guest-input-post-unzen.json"
            )

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)
            self.assertEqual(risc0_zkgas.collect(config, runner=runner), 0)

            rows = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")
            self.assertEqual(
                [row["status"] for row in rows], ["retryable_failure", "success"]
            )
            self.assertIsNot(rows[0].get("determinism_checked"), True)
            self.assertTrue(rows[1]["determinism_checked"])
            self.assertTrue(rows[1]["determinism_match"])

    def test_actual_elf_image_id_mismatch_is_a_terminal_cohort_failure(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = WrongImageRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)

            row = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")[0]
            self.assertEqual(row["status"], "terminal_failure")
            self.assertEqual(row["failure_stage"], "execution")
            self.assertIn("image_id", row["error"])

    def test_malformed_tool_output_is_preserved_as_terminal_result(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            config = self.make_config(root)
            runner = MalformedReportRunner(FIXTURES / "guest-input-post-unzen.json")

            self.assertNotEqual(risc0_zkgas.collect(config, runner=runner), 0)

            row = risc0_zkgas.read_jsonl(config.output_dir / "samples.jsonl")[0]
            self.assertEqual(row["status"], "terminal_failure")
            self.assertEqual(row["failure_class"], "invalid_output")


class ModelTests(unittest.TestCase):
    def test_m3_fit_calibration_and_quote_gates(self):
        fit_rows = []
        for index, (zkgas, blocks, input_bytes) in enumerate(
            [(10, 1, 100), (20, 3, 110), (30, 2, 160), (40, 5, 130), (50, 4, 190), (60, 7, 170)]
        ):
            actual = 100 + 2 * zkgas + 10 * blocks + 0.5 * input_bytes
            fit_rows.append(
                {
                    "network": "taiko_hoodi",
                    "split": "fit",
                    "proposal_id": index,
                    "total_zkgas": zkgas,
                    "block_count": blocks,
                    "risc0_input_bytes": input_bytes,
                    "evaluated_mcycles_count": actual,
                }
            )

        model = risc0_zkgas.fit_model("M3", fit_rows)
        self.assertAlmostEqual(model.coefficients["intercept"], 100)
        self.assertAlmostEqual(model.coefficients["total_zkgas"], 2)
        self.assertAlmostEqual(model.coefficients["block_count"], 10)
        self.assertAlmostEqual(model.coefficients["risc0_input_bytes"], 0.5)

        calibration_rows = [
            dict(fit_rows[0], split="calibration", evaluated_mcycles_count=185),
            dict(fit_rows[1], split="calibration", evaluated_mcycles_count=230),
        ]
        margin = risc0_zkgas.largest_positive_residual(model, calibration_rows)
        self.assertAlmostEqual(margin, 5)
        self.assertEqual(risc0_zkgas.quote_bucket(2001), 3000)
        self.assertEqual(risc0_zkgas.quote_bucket(1), 2000)

        holdout = [
            dict(
                fit_rows[2],
                network="taiko_mainnet",
                split="holdout",
                evaluated_mcycles_count=2500,
            ),
        ]
        evaluation = risc0_zkgas.evaluate_holdout(model, margin, holdout)
        self.assertEqual(evaluation["underquote_count"], 1)
        self.assertFalse(evaluation["gates"]["zero_underquotes"])

    def test_analysis_selects_on_hoodi_then_writes_holdout_decision(self):
        rows = []
        proposal_id = 1
        for zkgas in (10, 20, 30):
            for blocks in (1, 3):
                for input_bytes in (100, 200):
                    actual = 500 + 2 * zkgas + 50 * blocks + 0.4 * input_bytes
                    rows.append(
                        self.sample_row(
                            proposal_id,
                            network="taiko_hoodi",
                            split="fit",
                            zkgas=zkgas,
                            blocks=blocks,
                            input_bytes=input_bytes,
                            actual=actual,
                        )
                    )
                    proposal_id += 1
        for index, (zkgas, blocks, input_bytes) in enumerate(
            [(15, 2, 120), (25, 4, 180), (35, 2, 220), (45, 4, 140)]
        ):
            predicted = 500 + 2 * zkgas + 50 * blocks + 0.4 * input_bytes
            rows.append(
                self.sample_row(
                    proposal_id + index,
                    network="taiko_hoodi",
                    split="calibration",
                    zkgas=zkgas,
                    blocks=blocks,
                    input_bytes=input_bytes,
                    actual=predicted + 20,
                    determinism_checked=index == 0,
                )
            )
        proposal_id += 4
        for index, (zkgas, blocks, input_bytes) in enumerate(
            [(12, 2, 130), (22, 4, 170), (32, 2, 210), (42, 4, 150), (52, 2, 190)]
        ):
            predicted = 500 + 2 * zkgas + 50 * blocks + 0.4 * input_bytes
            rows.append(
                self.sample_row(
                    proposal_id + index,
                    network="taiko_mainnet",
                    split="holdout",
                    zkgas=zkgas,
                    blocks=blocks,
                    input_bytes=input_bytes,
                    actual=predicted + 20,
                    determinism_checked=index == 0,
                )
            )
        rows.append(
            {
                "status": "terminal_failure",
                "network": "taiko_hoodi",
                "failure_stage": "preflight",
                "failure_class": "invalid_sample",
                "error": "fixture exclusion",
            }
        )

        analysis = risc0_zkgas.analyze_experiment(
            rows,
            expected_fit_count=12,
            expected_calibration_count=4,
            expected_holdout_count=5,
        )

        self.assertEqual(analysis["selected_model"], "M3")
        self.assertAlmostEqual(analysis["calibration_margin_mcycles"], 20)
        self.assertTrue(analysis["recommend_shadow_mode"])
        self.assertEqual(analysis["accounted_failures"]["terminal_failure:preflight"], 1)
        for model_name in ("M1", "M2", "M3"):
            self.assertIn("mainnet_holdout", analysis["candidate_models"][model_name])

        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            model_path = root / "model.json"
            report_path = root / "report.md"
            risc0_zkgas.write_analysis_outputs(analysis, model_path, report_path)
            artifact = json.loads(model_path.read_text())
            report = report_path.read_text()

        self.assertEqual(artifact["selected_model"], "M3")
        self.assertEqual(artifact["calibration_method"], "largest_positive_residual")
        self.assertIn("Mainnet Holdout Gates", report)
        self.assertIn("Mainnet underquotes", report)
        self.assertIn("proposal_elf_sha256", report)
        self.assertIn("Proceed to the 1,000-proposal shadow stage", report)
        self.assertIn("terminal_failure:preflight", report)

        determinism_failure = {
            "status": "terminal_failure",
            "network": "taiko_mainnet",
            "failure_stage": "execution",
            "failure_class": "invalid_sample",
            "error": "RISC0 deterministic repeat did not match",
            "determinism_checked": True,
            "determinism_match": False,
        }
        failed_analysis = risc0_zkgas.analyze_experiment(
            [*rows, determinism_failure],
            expected_fit_count=12,
            expected_calibration_count=4,
            expected_holdout_count=5,
        )
        self.assertFalse(failed_analysis["recommend_shadow_mode"])
        self.assertFalse(
            failed_analysis["mainnet_holdout"]["gates"][
                "deterministic_repeats_match"
            ]
        )
        self.assertEqual(failed_analysis["sample_counts"]["determinism_repeats"], 3)
        self.assertEqual(failed_analysis["sample_counts"]["determinism_mismatches"], 1)
        self.assertIn(
            "Determinism mismatches: 1",
            risc0_zkgas.render_markdown_report(failed_analysis),
        )

        mixed_identity_rows = copy.deepcopy(rows)
        mixed_identity_rows[0]["artifact_hashes"]["proposal_elf_sha256"] = "different"
        with self.assertRaisesRegex(ValueError, "artifact_hashes"):
            risc0_zkgas.analyze_experiment(
                mixed_identity_rows,
                expected_fit_count=12,
                expected_calibration_count=4,
                expected_holdout_count=5,
            )

        with tempfile.TemporaryDirectory() as tmpdir:
            root = pathlib.Path(tmpdir)
            hoodi_path = root / "hoodi.jsonl"
            mainnet_path = root / "mainnet.jsonl"
            hoodi_path.write_text(
                "".join(
                    json.dumps(row) + "\n"
                    for row in rows
                    if row.get("network") == "taiko_hoodi"
                )
            )
            mainnet_path.write_text(
                "".join(
                    json.dumps(row) + "\n"
                    for row in rows
                    if row.get("network") == "taiko_mainnet"
                )
            )
            self.assertEqual(
                risc0_zkgas.main(
                    [
                        "fit",
                        "--samples",
                        str(hoodi_path),
                        "--samples",
                        str(mainnet_path),
                        "--model-out",
                        str(root / "model.json"),
                        "--report-out",
                        str(root / "report.md"),
                        "--expected-fit-count",
                        "12",
                        "--expected-calibration-count",
                        "4",
                        "--expected-holdout-count",
                        "5",
                    ]
                ),
                0,
            )

    @staticmethod
    def sample_row(
        proposal_id,
        *,
        network,
        split,
        zkgas,
        blocks,
        input_bytes,
        actual,
        determinism_checked=False,
    ):
        return {
            "status": "success",
            "sample_key": f"{network}:{proposal_id}:0ximage",
            "network": network,
            "split": split,
            "proposal_id": proposal_id,
            "source_revision": "abc123",
            "image_id": "0ximage",
            "risc0_version": "3.0.5",
            "execution_po2": 20,
            "unzen_timestamp": risc0_zkgas.SUPPORTED_NETWORKS[network][
                "unzen_timestamp"
            ],
            "artifact_hashes": {
                "collector_script_sha256": "collector",
                "proposal_elf_sha256": "proposal",
                "preflight_binary_sha256": "preflight",
                "guest_launcher_binary_sha256": "launcher",
                "stress_discovery_script_sha256": "stress",
                "cargo_lock_sha256": "lock",
                "chain_spec_sha256": "chain",
            },
            "total_zkgas": zkgas,
            "block_count": blocks,
            "risc0_input_bytes": input_bytes,
            "evaluated_mcycles_count": actual,
            "determinism_checked": determinism_checked,
            "determinism_match": True if determinism_checked else None,
        }


if __name__ == "__main__":
    unittest.main()
