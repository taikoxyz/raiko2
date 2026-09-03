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
        measured_opcodes = {
            case.opcode
            for case in manifest.cases
            if case.kind == "opcode" and case.opcode is not None
        }
        measured_precompiles = {
            case.address
            for case in manifest.cases
            if case.kind == "precompile" and case.address is not None
        }
        self.assertFalse(opcode_gas.PLANNED_PURE_OPCODE_OPCODES - measured_opcodes)
        self.assertEqual(measured_precompiles, set(opcode_gas.PRECOMPILE_BODY_DEFAULTS))
        self.assertGreaterEqual(
            {case.name for case in manifest.cases},
            {
                "add",
                "mul",
                "sub",
                "div",
                "sdiv",
                "mod",
                "smod",
                "addmod",
                "mulmod",
                "exp",
                "signextend",
                "lt",
                "gt",
                "slt",
                "sgt",
                "eq",
                "and",
                "or",
                "xor",
                "iszero",
                "not",
                "byte",
                "shl",
                "shr",
                "sar",
                "keccak256",
                "pop",
                "mload",
                "mstore",
                "mstore8",
                "mcopy",
                "jump",
                "jumpi",
                "pc",
                "msize",
                "gas",
                "jumpdest",
                "push0",
                "push32",
                "dup16",
                "swap1",
                "swap16",
                "ecrecover",
                "identity",
                "sha256",
                "ripemd160",
                "modexp",
                "bn128_add",
                "bn128_mul",
                "bn128_pairing",
                "blake2f",
                "point_evaluation",
                "bls12_g1add",
                "bls12_g1msm",
                "bls12_g2add",
                "bls12_g2msm",
                "bls12_pairing",
                "bls12_map_fp_to_g1",
                "bls12_map_fp2_to_g2",
            },
        )
        self.assertEqual(
            {case.name: case.kind for case in manifest.cases}["identity"],
            "precompile",
        )


if __name__ == "__main__":
    unittest.main()
