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

    def test_stack_unary_variant_counts_target_opcode(self):
        case = opcode_gas.CaseSpec(
            name="iszero",
            opcode=0x15,
            scenario="bitwise",
            template="stack_unary",
            target_raw_gas=3,
        )

        generated = opcode_gas.build_bytecode(case, 3)

        self.assertEqual(generated.opcode_counts[0x15], 3)
        self.assertTrue(generated.bytes_hex.endswith("00"))

    def test_stack_ternary_variant_counts_target_opcode(self):
        case = opcode_gas.CaseSpec(
            name="addmod",
            opcode=0x08,
            scenario="arithmetic",
            template="stack_ternary",
            target_raw_gas=8,
        )

        zero = opcode_gas.build_bytecode(case, 0)
        three = opcode_gas.build_bytecode(case, 3)

        self.assertEqual(zero.opcode_counts.get(0x08, 0), 0)
        self.assertEqual(three.opcode_counts[0x08], 3)
        self.assertTrue(zero.bytes_hex.endswith("00"))
        self.assertTrue(three.bytes_hex.endswith("00"))

    def test_stack_exp_variant_counts_exp_opcode(self):
        case = opcode_gas.CaseSpec(
            name="exp",
            opcode=0x0A,
            scenario="arithmetic",
            template="stack_exp",
            target_raw_gas=60,
        )

        generated = opcode_gas.build_bytecode(case, 2)

        self.assertEqual(generated.opcode_counts[0x0A], 2)
        self.assertTrue(generated.bytes_hex.endswith("00"))

    def test_memory_templates_count_target_opcode(self):
        cases = [
            ("mload", 0x51, "memory_load_32"),
            ("mstore", 0x52, "memory_store_32"),
            ("mstore8", 0x53, "memory_store8"),
        ]

        for name, opcode, template in cases:
            with self.subTest(name=name):
                case = opcode_gas.CaseSpec(
                    name=name,
                    opcode=opcode,
                    scenario="memory",
                    template=template,
                    target_raw_gas=3,
                )
                generated = opcode_gas.build_bytecode(case, 2)

                self.assertEqual(generated.opcode_counts[opcode], 2)
                self.assertTrue(generated.bytes_hex.endswith("00"))

    def test_stack_pop_and_swap_templates_count_target_opcode(self):
        cases = [
            ("pop", 0x50, "stack_pop"),
            ("swap1", 0x90, "stack_swap1"),
        ]

        for name, opcode, template in cases:
            with self.subTest(name=name):
                case = opcode_gas.CaseSpec(
                    name=name,
                    opcode=opcode,
                    scenario="stack",
                    template=template,
                    target_raw_gas=3,
                )
                generated = opcode_gas.build_bytecode(case, 2)

                self.assertEqual(generated.opcode_counts[opcode], 2)
                self.assertTrue(generated.bytes_hex.endswith("00"))


if __name__ == "__main__":
    unittest.main()
