import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class ManifestTests(unittest.TestCase):
    def test_load_manifest_parses_smoke_cases(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml"
        )

        self.assertEqual(manifest.name, "sp1-smoke")
        self.assertEqual(manifest.backend, "sp1")
        self.assertEqual(manifest.variants, [0, 1, 2, 4])
        self.assertGreaterEqual(
            {case.name for case in manifest.cases},
            {
                "add",
                "mul",
                "sub",
                "div",
                "mod",
                "lt",
                "gt",
                "eq",
                "and",
                "or",
                "xor",
                "keccak256",
                "identity",
                "sha256",
            },
        )
        self.assertEqual(
            {case.name: case.kind for case in manifest.cases}["identity"],
            "precompile",
        )


if __name__ == "__main__":
    unittest.main()
