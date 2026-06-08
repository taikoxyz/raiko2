import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class GenerateTests(unittest.TestCase):
    def test_stack_binary_variant_increases_only_target_opcode_count(self):
        case = opcode_gas.CaseSpec(
            name="add",
            opcode=0x01,
            scenario="arithmetic",
            template="stack_binary",
            target_raw_gas=3,
        )

        zero = opcode_gas.build_bytecode(case, 0)
        four = opcode_gas.build_bytecode(case, 4)

        self.assertEqual(zero.opcode_counts.get(0x01, 0), 0)
        self.assertEqual(four.opcode_counts[0x01], 4)
        self.assertTrue(zero.bytes_hex.endswith("00"))
        self.assertTrue(four.bytes_hex.endswith("00"))

    def test_keccak_variant_keeps_target_count_visible(self):
        case = opcode_gas.CaseSpec(
            name="keccak256",
            opcode=0x20,
            scenario="memory",
            template="keccak_32",
            target_raw_gas=36,
        )

        generated = opcode_gas.build_bytecode(case, 2)

        self.assertEqual(generated.opcode_counts[0x20], 2)
        self.assertTrue(generated.bytes_hex.endswith("00"))


if __name__ == "__main__":
    unittest.main()
