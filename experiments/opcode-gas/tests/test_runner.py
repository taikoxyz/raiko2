import pathlib
import sys
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class RunnerTests(unittest.TestCase):
    def test_runner_uses_guest_launcher_directly(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)

        with tempfile.TemporaryDirectory() as tmp:
            report_path = pathlib.Path(tmp) / "report.json"
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_guest_input(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    elf_path=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
                    input_path=pathlib.Path("/tmp/input.json"),
                    json_out=report_path,
                )

        self.assertEqual(calls[0][0], "target/release/guest-launcher")
        self.assertIn("--stage", calls[0])
        self.assertIn("opcode-lab", calls[0])
        self.assertIn("--elf", calls[0])
        self.assertIn("--sp1-prover", calls[0])
        self.assertIn("local", calls[0])
        self.assertNotIn("cargo", calls[0][0])

    def test_batch_runner_uses_one_guest_launcher_process(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = pathlib.Path(tmp)
            input_paths = [tmp_path / "a.json", tmp_path / "b.json"]
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_guest_inputs(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    elf_path=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
                    input_paths=input_paths,
                    reports_jsonl=tmp_path / "reports.jsonl",
                )

            input_list_path = tmp_path / "opcode-lab-inputs.json"
            self.assertEqual(len(calls), 1)
            self.assertEqual(calls[0][0], "target/release/guest-launcher")
            self.assertIn("--input-list", calls[0])
            self.assertIn(str(input_list_path), calls[0])
            self.assertIn("--jsonl-out", calls[0])
            self.assertEqual(
                opcode_gas.json.loads(input_list_path.read_text()),
                [str(path) for path in input_paths],
            )

    def test_batch_runner_accepts_revm_opcode_lab_stage(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = pathlib.Path(tmp)
            input_paths = [tmp_path / "a.json"]
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_guest_inputs(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    elf_path=pathlib.Path("crates/guests/elf/sp1_revm_opcode_lab.elf"),
                    input_paths=input_paths,
                    reports_jsonl=tmp_path / "reports.jsonl",
                    stage="revm-opcode-lab",
                )

        self.assertIn("--stage", calls[0])
        self.assertIn("revm-opcode-lab", calls[0])
        self.assertIn("crates/guests/elf/sp1_revm_opcode_lab.elf", calls[0])

    def test_raw_run_normalizes_guest_launcher_gas_to_prover_gas(self):
        case = {
            "case": "add",
            "target_count": 2,
            "target_raw_gas": 3,
        }
        report = {
            "gas": 160,
            "wall_time_ms": 9,
            "exit_code": 0,
        }

        raw_run = opcode_gas.raw_run_from_report(case, report)

        self.assertEqual(raw_run["prover_gas"], 160)
        self.assertEqual(raw_run["gas"], 160)
        self.assertEqual(raw_run["case"], "add")

    def test_raw_run_preserves_guest_launcher_primary_workload_metric(self):
        case = {
            "case": "proposal-170",
            "target_count": 1,
            "target_raw_gas": 30_000_000,
        }
        report = {
            "primary_workload_metric": {
                "label": "risc0_padded_cycles",
                "count": 4096,
            },
            "risc0_user_cycles": 2200,
            "risc0_padded_cycles": 4096,
        }

        raw_run = opcode_gas.raw_run_from_report(case, report)

        self.assertEqual(raw_run["workload_metric"], "risc0_padded_cycles")
        self.assertEqual(raw_run["workload_value"], 4096)
        self.assertEqual(raw_run["risc0_padded_cycles"], 4096)

    def test_raw_run_promotes_real_host_trace_to_isolation_evidence(self):
        case = {
            "case": "add",
            "opcode": "0x01",
            "target_count": 1,
            "target_raw_gas": 3,
        }
        report = {
            "gas": 160,
            "controlled_trace": {
                "schema_version": 1,
                "workload_id": "a" * 64,
                "backend_input_sha256": "b" * 64,
                "backend_input_len": 48,
                "target_opcode": 1,
                "declared_target_count": 1,
                "declared_target_raw_gas": 3,
                "executed_target_count": 1,
                "executed_target_raw_gas": 3,
                "non_target_counts": {"opcode:0x60": 2, "opcode:0x00": 1},
                "non_target_raw_gas": 6,
                "total_raw_gas": 9,
                "bytecode_len": 6,
            },
        }

        raw_run = opcode_gas.raw_run_from_report(case, report)

        self.assertEqual(raw_run["workload_id"], "a" * 64)
        self.assertEqual(raw_run["backend_input_sha256"], "b" * 64)
        self.assertEqual(
            raw_run["isolation"],
            {
                "status": "passed",
                "bytecode_size": 6,
                "input_size": 48,
                "non_target_counts": {"opcode:0x60": 2, "opcode:0x00": 1},
                "non_target_raw_gas": 6,
            },
        )

    def test_raw_run_rejects_host_trace_that_does_not_match_case_identity(self):
        case = {
            "case": "add",
            "opcode": "0x01",
            "target_count": 1,
            "target_raw_gas": 3,
        }
        report = {
            "controlled_trace": {
                "workload_id": "a" * 64,
                "backend_input_sha256": "b" * 64,
                "target_opcode": 2,
                "declared_target_count": 1,
                "declared_target_raw_gas": 3,
            }
        }

        with self.assertRaisesRegex(ValueError, "controlled trace identity"):
            opcode_gas.raw_run_from_report(case, report)

    def test_raw_run_promotes_typed_paired_precompile_identity_and_shape(self):
        case = {
            "case": "identity",
            "kind": "precompile",
            "address": "0x04",
            "lane": "control",
            "pair_id": "c" * 64,
            "target_count": 2,
            "target_raw_gas": 18,
            "input_size": 4,
            "expected_output_size": 4,
        }
        report = {
            "gas": 160,
            "controlled_trace": {
                "kind": "precompile",
                "workload_id": "a" * 64,
                "pair_id": "c" * 64,
                "backend_input_sha256": "b" * 64,
                "backend_input_len": 96,
                "address": 4,
                "target_count": 2,
                "target_raw_gas": 18,
                "lane": "control",
                "input_len": 4,
                "output_len": 4,
                "loop_iterations": 2,
                "folded_bytes_per_iteration": 12,
            },
        }

        raw_run = opcode_gas.raw_run_from_report(case, report)

        self.assertEqual(raw_run["workload_id"], "a" * 64)
        self.assertEqual(raw_run["backend_input_sha256"], "b" * 64)
        self.assertEqual(raw_run["pair_id"], "c" * 64)
        self.assertEqual(
            raw_run["isolation"],
            {
                "status": "passed",
                "input_size": 4,
                "output_size": 4,
                "loop_iterations": 2,
                "folded_bytes_per_iteration": 12,
            },
        )

    def test_raw_run_rejects_precompile_pair_or_lane_mismatch(self):
        case = {
            "case": "identity",
            "kind": "precompile",
            "address": "0x04",
            "lane": "target",
            "pair_id": "c" * 64,
            "target_count": 2,
            "target_raw_gas": 18,
        }
        trace = {
            "kind": "precompile",
            "workload_id": "a" * 64,
            "pair_id": "d" * 64,
            "backend_input_sha256": "b" * 64,
            "address": 4,
            "target_count": 2,
            "target_raw_gas": 18,
            "lane": "control",
        }

        with self.assertRaisesRegex(ValueError, "controlled trace identity"):
            opcode_gas.raw_run_from_report(case, {"controlled_trace": trace})

    def test_parser_accepts_damage_command(self):
        args = opcode_gas.build_parser().parse_args(
            [
                "damage",
                "--fit",
                "/tmp/fit.json",
                "--manifest",
                "experiments/opcode-gas/manifests/sp1-smoke.toml",
                "--eth-gas-limit",
                "30000000",
                "--zk-gas-limit",
                "100000000",
                "--out",
                "/tmp/damage",
            ]
        )

        self.assertEqual(args.command, "damage")
        self.assertEqual(args.fit, pathlib.Path("/tmp/fit.json"))
        self.assertEqual(args.eth_gas_limit, 30_000_000)
        self.assertEqual(args.zk_gas_limit, 100_000_000)

    def test_parser_accepts_revm_opcode_lab_run_stage(self):
        args = opcode_gas.build_parser().parse_args(
            [
                "run",
                "--fixtures",
                "/tmp/fixtures",
                "--guest-launcher",
                "target/release/guest-launcher",
                "--opcode-stage",
                "revm-opcode-lab",
                "--calibration-run",
                "/tmp/calibration",
                "--controlled-manifest",
                "/tmp/controlled.toml",
                "--out",
                "/tmp/runs.jsonl",
            ]
        )

        self.assertEqual(args.command, "run")
        self.assertEqual(args.opcode_stage, "revm-opcode-lab")

    def test_run_command_uses_revm_elf_for_revm_opcode_stage(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)
            reports_path = pathlib.Path(cmd[cmd.index("--jsonl-out") + 1])
            reports_path.write_text(
                opcode_gas.json.dumps(
                    {
                        "input": str(input_path),
                        "gas": 10,
                    }
                )
                + "\n"
            )

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = pathlib.Path(tmp)
            case_dir = tmp_path / "fixtures" / "add"
            case_dir.mkdir(parents=True)
            input_path = case_dir / "guest-input.json"
            case_path = case_dir / "case.json"
            input_path.write_text("{}\n")
            case_path.write_text(
                opcode_gas.json.dumps(
                    {
                        "kind": "opcode",
                        "case": "add",
                        "target_count": 1,
                        "target_raw_gas": 3,
                        "calibration_id": "calibration",
                        "controlled_manifest_sha256": "a" * 64,
                        "controlled_manifest_rows_sha256": "b" * 64,
                        "fixture_sha256": opcode_gas.sha256_file(input_path),
                    }
                )
                + "\n"
            )
            out_path = tmp_path / "runs.jsonl"
            args = opcode_gas.build_parser().parse_args(
                [
                    "run",
                    "--fixtures",
                    str(tmp_path / "fixtures"),
                    "--guest-launcher",
                    "target/release/guest-launcher",
                    "--opcode-stage",
                    "revm-opcode-lab",
                    "--calibration-run",
                    str(tmp_path / "calibration"),
                    "--controlled-manifest",
                    str(tmp_path / "controlled.toml"),
                    "--out",
                    str(out_path),
                ]
            )

            with mock.patch.object(opcode_gas, "REPO_ROOT", tmp_path), mock.patch.object(
                opcode_gas, "verify_frozen_controlled_manifest",
                return_value=(None, {
                    "controlled_manifest_sha256": "a" * 64,
                    "controlled_manifest_rows_sha256": "b" * 64,
                }),
            ), mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.cmd_run(args)

        self.assertIn("revm-opcode-lab", calls[0])
        self.assertIn("crates/guests/elf/sp1_revm_opcode_lab.elf", calls[0])

    def test_run_proposal_writes_normalized_risc0_raw_run(self):
        calls = []

        def fake_run(cmd, check):
            calls.append(cmd)
            json_out = pathlib.Path(cmd[cmd.index("--json-out") + 1])
            json_out.write_text(
                opcode_gas.json.dumps(
                    {
                        "primary_workload_metric": {
                            "label": "risc0_padded_cycles",
                            "count": 4096,
                        },
                        "risc0_user_cycles": 2200,
                        "risc0_padded_cycles": 4096,
                    }
                )
            )

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = pathlib.Path(tmp)
            out_path = tmp_path / "runs.jsonl"
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_proposal_guest_input(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    guest_input=tmp_path / "guest-input.json",
                    proof_type="risc0",
                    case_name="proposal-170",
                    target_raw_gas=30_000_000,
                    target_count=1,
                    out=out_path,
                    risc0_execution_po2=20,
                )

            raw_run = opcode_gas.json.loads(out_path.read_text())

        self.assertEqual(calls[0][0], "target/release/guest-launcher")
        self.assertIn("--proof-type", calls[0])
        self.assertIn("risc0", calls[0])
        self.assertIn("--risc0-execution-po2", calls[0])
        self.assertEqual(raw_run["case"], "proposal-170")
        self.assertEqual(raw_run["target_raw_gas"], 30_000_000)
        self.assertEqual(raw_run["workload_metric"], "risc0_padded_cycles")
        self.assertEqual(raw_run["workload_value"], 4096)

    def test_join_proposal_trace_requires_guest_input_hash_and_public_output(self):
        trace = {
            "status": "complete",
            "guest_input_sha256": "0x" + "ab" * 32,
            "public_output": "0x1234",
            "parity_passed": True,
            "block_count": 1,
            "partial_block_count": 0,
        }
        report = {
            "guest_input_sha256": "0x" + "ab" * 32,
            "public_values": "1234",
        }

        joined = opcode_gas.join_proposal_trace_and_sp1(trace, report)
        self.assertEqual(joined["guest_input_sha256"], "0x" + "ab" * 32)
        self.assertEqual(joined["public_output"], "0x1234")
        self.assertEqual(joined["trace_block_count"], 1)

        with self.assertRaisesRegex(ValueError, "GuestInput hash"):
            opcode_gas.join_proposal_trace_and_sp1(trace, {"public_values": "1234"})
        with self.assertRaisesRegex(ValueError, "GuestInput hash"):
            opcode_gas.join_proposal_trace_and_sp1(
                trace,
                {**report, "guest_input_sha256": "0x" + "cd" * 32},
            )
        with self.assertRaisesRegex(ValueError, "public output"):
            opcode_gas.join_proposal_trace_and_sp1(
                trace,
                {**report, "public_values": "0x5678"},
            )

    def test_run_sp1_proposal_executes_trace_first_and_joins_exact_identities(self):
        calls = []
        guest_hash = "0x" + "ab" * 32

        def fake_run(cmd, check):
            calls.append(cmd)
            json_out = pathlib.Path(cmd[cmd.index("--json-out") + 1])
            if cmd[cmd.index("--stage") + 1] == "proposal-trace":
                self.assertEqual(json_out.suffixes[-2:], [".json", ".gz"])
                json_out.write_bytes(b"\x1f\x8bfull-trace-must-not-be-read")
                summary_out = json_out.with_name(
                    json_out.name.removesuffix(".json.gz") + ".summary.json"
                )
                summary_out.write_text(opcode_gas.json.dumps({
                    "status": "complete",
                    "guest_input_sha256": guest_hash,
                    "public_output": "0x1234",
                    "parity_passed": True,
                    "block_count": 1,
                    "partial_block_count": 0,
                }))
            else:
                json_out.write_text(opcode_gas.json.dumps({
                    "guest_input_sha256": guest_hash,
                    "public_values": "1234",
                    "gas": 99,
                }))

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            output = root / "runs.jsonl"
            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.run_proposal_guest_input(
                    guest_launcher=pathlib.Path("target/release/guest-launcher"),
                    guest_input=root / "guest-input.json",
                    proof_type="sp1",
                    case_name="proposal-1",
                    target_raw_gas=1,
                    target_count=1,
                    out=output,
                )
            row = opcode_gas.json.loads(output.read_text())

        self.assertEqual(calls[0][calls[0].index("--stage") + 1], "proposal-trace")
        self.assertEqual(calls[1][calls[1].index("--stage") + 1], "proposal")
        self.assertEqual(row["guest_input_sha256"], guest_hash)
        self.assertEqual(row["public_output"], "0x1234")
        self.assertTrue(row["trace_ab_passed"])
        self.assertTrue(row["proposal_trace"].endswith(".json.gz"))

    def test_parser_accepts_inventory_command(self):
        args = opcode_gas.build_parser().parse_args(
            [
                "inventory",
                "--manifest",
                "experiments/opcode-gas/manifests/sp1-smoke.toml",
                "--out",
                "/tmp/inventory",
            ]
        )

        self.assertEqual(args.command, "inventory")
        self.assertEqual(
            args.manifest,
            pathlib.Path("experiments/opcode-gas/manifests/sp1-smoke.toml"),
        )
        self.assertEqual(args.out, pathlib.Path("/tmp/inventory"))


if __name__ == "__main__":
    unittest.main()
