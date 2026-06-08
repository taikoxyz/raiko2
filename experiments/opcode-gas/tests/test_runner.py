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


if __name__ == "__main__":
    unittest.main()
