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

    def test_stack_family_templates_count_target_opcode(self):
        cases = [
            ("push17", 0x70, "stack_push"),
            ("dup16", 0x8F, "stack_dup"),
            ("swap16", 0x9F, "stack_swap"),
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
                generated = opcode_gas.build_bytecode(case, 3)

                self.assertEqual(generated.opcode_counts[opcode], 3)
                self.assertTrue(generated.bytes_hex.endswith("00"))

    def test_control_and_mcopy_templates_count_target_opcode(self):
        cases = [
            ("jump", 0x56, "jump_chain"),
            ("jumpi", 0x57, "jumpi_chain"),
            ("pc", 0x58, "stack_unary_producer"),
            ("msize", 0x59, "stack_unary_producer"),
            ("gas", 0x5A, "stack_unary_producer"),
            ("jumpdest", 0x5B, "jumpdest_chain"),
            ("mcopy", 0x5E, "memory_copy_32"),
        ]

        for name, opcode, template in cases:
            with self.subTest(name=name):
                case = opcode_gas.CaseSpec(
                    name=name,
                    opcode=opcode,
                    scenario="control",
                    template=template,
                    target_raw_gas=3,
                )
                generated = opcode_gas.build_bytecode(case, 2)

                self.assertEqual(generated.opcode_counts[opcode], 2)
                self.assertTrue(generated.bytes_hex.endswith("00"))

    def test_precompile_templates_emit_expected_input_sizes(self):
        cases = [
            ("ecrecover", 0x01, "precompile_ecrecover_valid", 128),
            ("ripemd160", 0x03, "precompile_fixed_32", 32),
            ("modexp", 0x05, "precompile_modexp_small", 99),
            ("bn128_add", 0x06, "precompile_bn254_add", 128),
            ("bn128_mul", 0x07, "precompile_bn254_mul", 96),
            ("bn128_pairing", 0x08, "precompile_bn254_pairing", 192),
            ("blake2f", 0x09, "precompile_blake2f_12_rounds", 213),
            ("point_evaluation", 0x0A, "precompile_kzg_point_evaluation", 192),
            ("bls12_g1add", 0x0B, "precompile_bls12_g1add_zero", 256),
            ("bls12_g1msm", 0x0C, "precompile_bls12_g1msm_zero", 160),
            ("bls12_g2add", 0x0D, "precompile_bls12_g2add_zero", 512),
            ("bls12_g2msm", 0x0E, "precompile_bls12_g2msm_zero", 288),
            ("bls12_pairing", 0x0F, "precompile_bls12_pairing_zero", 384),
            ("bls12_map_fp_to_g1", 0x10, "precompile_bls12_map_fp_to_g1_zero", 64),
            ("bls12_map_fp2_to_g2", 0x11, "precompile_bls12_map_fp2_to_g2_zero", 128),
        ]

        for name, address, template, input_size in cases:
            with self.subTest(name=name):
                case = opcode_gas.default_precompile_case(address)

                encoded = opcode_gas.build_precompile_input(case)

                self.assertEqual(case.name, name)
                self.assertEqual(case.template, template)
                self.assertEqual(case.input_size, input_size)
                self.assertEqual(len(bytes.fromhex(encoded.removeprefix("0x"))), input_size)


if __name__ == "__main__":
    unittest.main()
