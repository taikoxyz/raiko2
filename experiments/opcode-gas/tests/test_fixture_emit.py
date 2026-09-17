import pathlib
import sys
import tempfile
import unittest
from dataclasses import replace

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from test_manifest import CONTROLLED_SCHEDULE_KEYS, controlled_manifest_data


def fixture_schedule():
    return opcode_gas.UnzenSchedule(
        opcode_multipliers={opcode: 1 for opcode in opcode_gas.UZEN_OPCODE_NAMES},
        precompile_multipliers={address: 1 for address in opcode_gas.UZEN_PRECOMPILE_NAMES},
    )


class FixtureEmitTests(unittest.TestCase):
    def test_generate_writes_case_metadata(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml",
            schedule=fixture_schedule(),
        )

        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            opcode_gas.generate_cases(manifest, out_dir)
            cases = sorted(out_dir.glob("**/case.json"))
            guest_inputs = sorted(out_dir.glob("**/guest-input.json"))
            identity = next(path for path in cases if "/identity/" in str(path))
            identity_payload = opcode_gas.json.loads(identity.read_text())
            identity_input = opcode_gas.json.loads(identity.with_name("guest-input.json").read_text())

        self.assertEqual(len(cases), len(manifest.cases) * len(manifest.variants))
        self.assertEqual(len(guest_inputs), len(cases))
        self.assertEqual(identity_payload["kind"], "precompile")
        self.assertEqual(identity_input["address"], 4)
        self.assertEqual(identity_input["input_size"], 32)

    def test_controlled_precompile_emits_typed_target_and_control_pair(self):
        data = controlled_manifest_data()
        data["variants"] = [2]
        data["cases"][2].update(
            template="precompile_fixed_32",
            paired=True,
            expected_output_size=32,
        )
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(manifest, cases=[manifest.cases[2]])

        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            opcode_gas.generate_cases(manifest, out_dir)
            pair_root = out_dir / "controlled" / "identity" / "count-2"
            target = opcode_gas.json.loads(
                (pair_root / "target" / "guest-input.json").read_text()
            )
            control = opcode_gas.json.loads(
                (pair_root / "control" / "guest-input.json").read_text()
            )
            target_case = opcode_gas.json.loads(
                (pair_root / "target" / "case.json").read_text()
            )
            control_case = opcode_gas.json.loads(
                (pair_root / "control" / "case.json").read_text()
            )

        self.assertEqual(target["lane"], "target")
        self.assertEqual(control["lane"], "control")
        self.assertEqual(target["input"], control["input"])
        self.assertEqual(target["target_count"], control["target_count"])
        self.assertEqual(target["expected_output_size"], 32)
        self.assertEqual(control["expected_output_size"], 32)
        self.assertNotEqual(target_case["fixture_sha256"], control_case["fixture_sha256"])
        self.assertEqual(target_case["pair_id"], control_case["pair_id"])

    def test_controlled_opcode_fixture_freezes_initial_adaptive_round(self):
        data = controlled_manifest_data()
        data["variants"] = [0, 1, 2, 4, 8, 16]
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(
            manifest,
            cases=[opcode_gas.default_opcode_case(0x01)],
        )

        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            written = opcode_gas.generate_cases(manifest, out_dir)
            inputs = [
                opcode_gas.json.loads(path.with_name("guest-input.json").read_text())
                for path in written
            ]

        self.assertEqual([row["target_count"] for row in inputs], [0, 1, 2, 4, 8])
        self.assertEqual({row["generator_max_count"] for row in inputs}, {8})
        self.assertEqual(len({row["fixed_bytecode_len"] for row in inputs}), 1)
        self.assertEqual(
            {
                len(opcode_gas.decode_fixed_microprograms(bytes.fromhex(row["bytecode"][2:])))
                for row in inputs
            },
            {8},
        )

    def test_adaptive_expansion_regenerates_every_prefix_point_with_one_new_footprint(self):
        data = controlled_manifest_data()
        data["variants"] = [0, 1, 2, 4, 8, 16, 32]
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(manifest, cases=[opcode_gas.default_opcode_case(0x01)])

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            first = opcode_gas.generate_cases(
                manifest, root / "round-8", generator_max_count=8
            )
            expanded = opcode_gas.generate_cases(
                manifest, root / "round-32", generator_max_count=32
            )
            first_rows = [opcode_gas.json.loads(path.read_text()) for path in first]
            expanded_rows = [opcode_gas.json.loads(path.read_text()) for path in expanded]

        self.assertEqual([row["target_count"] for row in first_rows], [0, 1, 2, 4, 8])
        self.assertEqual(
            [row["target_count"] for row in expanded_rows],
            [0, 1, 2, 4, 8, 16, 32],
        )
        self.assertEqual({row["generator_max_count"] for row in first_rows}, {8})
        self.assertEqual({row["generator_max_count"] for row in expanded_rows}, {32})
        self.assertEqual(len({row["fixed_bytecode_len"] for row in expanded_rows}), 1)
        first_by_count = {row["target_count"]: row for row in first_rows}
        expanded_by_count = {row["target_count"]: row for row in expanded_rows}
        for count in first_by_count:
            self.assertNotEqual(
                first_by_count[count]["fixture_sha256"],
                expanded_by_count[count]["fixture_sha256"],
            )


if __name__ == "__main__":
    unittest.main()
