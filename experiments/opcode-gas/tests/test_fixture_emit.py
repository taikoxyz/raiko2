import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


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


if __name__ == "__main__":
    unittest.main()
