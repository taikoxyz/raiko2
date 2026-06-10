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
                    "--out",
                    str(out_path),
                ]
            )

            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
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
