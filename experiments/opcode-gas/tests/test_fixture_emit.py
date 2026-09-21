import hashlib
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


def diagnostic_provenance():
    return {
        "calibration_id": "a" * 24,
        "calibration_identity_sha256": "b" * 64,
        "implementation_revision": "c" * 40,
        "controlled_manifest_sha256": "d" * 64,
        "controlled_manifest_rows_sha256": "e" * 64,
    }


class FixtureEmitTests(unittest.TestCase):
    def test_matched_control_binary_emits_equal_footprint_op_minus_pop_pair(self):
        data = controlled_manifest_data()
        data["variants"] = [2]
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(manifest, cases=[opcode_gas.default_opcode_case(0x01)])

        with tempfile.TemporaryDirectory() as tmp:
            out_dir = pathlib.Path(tmp)
            written = opcode_gas.generate_cases(
                manifest,
                out_dir,
                provenance=diagnostic_provenance(),
                generator_max_count=8,
                matched_control_diagnostic=True,
            )
            pair_root = out_dir / "controlled" / "add" / "count-2"
            target_case = opcode_gas.json.loads(
                (pair_root / "target" / "case.json").read_text()
            )
            control_case = opcode_gas.json.loads(
                (pair_root / "control" / "case.json").read_text()
            )
            target_input = opcode_gas.json.loads(
                (pair_root / "target" / "guest-input.json").read_text()
            )
            control_input = opcode_gas.json.loads(
                (pair_root / "control" / "guest-input.json").read_text()
            )

        self.assertEqual(len(written), 2)
        self.assertEqual(
            {target_case["purpose"], control_case["purpose"]},
            {"matched_control_diagnostic"},
        )
        self.assertEqual(target_case["pair_id"], control_case["pair_id"])
        self.assertEqual(target_case["relation"], "OP-POP")
        self.assertEqual(control_case["relation"], "OP-POP")
        self.assertEqual(target_case["signal_kind"], "contextual_relative")
        self.assertEqual(control_case["signal_kind"], "contextual_relative")
        self.assertEqual(target_case["diagnostic_count"], 2)
        self.assertEqual(control_case["diagnostic_count"], 2)
        self.assertEqual(target_case["original_opcode"], "0x01")
        self.assertEqual(control_case["original_opcode"], "0x01")
        self.assertEqual(target_case["operand_profile"], "zero")
        self.assertEqual(control_case["operand_profile"], "zero")
        self.assertEqual(target_case["operands"], [0, 0])
        self.assertEqual(control_case["operands"], [0, 0])
        self.assertEqual(target_case["final_stack_height"], 1)
        self.assertEqual(control_case["final_stack_height"], 1)
        self.assertEqual(target_input["opcode"], 0x01)
        self.assertEqual(target_input["target_count"], 2)
        self.assertEqual(control_input["opcode"], 0x50)
        self.assertEqual(control_input["target_count"], 8)
        self.assertEqual(control_input["target_raw_gas"], 2)
        self.assertEqual(target_input["tx_gas_limit"], control_input["tx_gas_limit"])
        self.assertEqual(
            target_input["fixed_bytecode_len"], control_input["fixed_bytecode_len"]
        )
        self.assertEqual(len(target_input["case"]), len(control_input["case"]))

        target_programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(target_input["bytecode"][2:])
        )
        control_programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(control_input["bytecode"][2:])
        )
        self.assertEqual(
            hashlib.sha256(bytes.fromhex(target_input["bytecode"][2:])).hexdigest(),
            "90ad79a7d0be24c23cd9f15e9db1c31fc95dd501d1f3cd178699ccaabc71003d",
        )
        self.assertEqual(
            hashlib.sha256(bytes.fromhex(control_input["bytecode"][2:])).hexdigest(),
            "b6875ca3228cc58836023a1ac28f2f0ef71926f93e048fe33bd942769aa3fda6",
        )
        self.assertEqual(len(target_programs), 8)
        self.assertEqual([program[:-2] for program in target_programs], [
            program[:-2] for program in control_programs
        ])
        self.assertTrue(all(program.endswith(b"\x50\x00") for program in control_programs))
        self.assertTrue(all(program.endswith(b"\x01\x00") for program in target_programs[:2]))
        self.assertTrue(all(program.endswith(b"\x50\x00") for program in target_programs[2:]))
        pairs = opcode_gas.validate_matched_control_fixture_pairs(
            [target_case, control_case],
            calibration_execution_identity=diagnostic_provenance(),
        )
        self.assertEqual(list(pairs), [target_case["pair_id"]])

        for invalid, message in [
            ([target_case], "exactly one target and one control"),
            ([target_case, target_case], "duplicate lane"),
            ([target_case, {**control_case, "pair_id": "f" * 64}], "pair_id"),
            (
                [target_case, {**control_case, "fixed_bytecode_len": 1}],
                "fixed_bytecode_len",
            ),
        ]:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.validate_matched_control_fixture_pairs(invalid)

        altered_target = {**target_case, "calibration_identity_sha256": "f" * 64}
        altered_control = {**control_case, "calibration_identity_sha256": "f" * 64}
        altered_pair_id = opcode_gas.matched_control_pair_id(
            altered_target, altered_control
        )
        altered_target["pair_id"] = altered_pair_id
        altered_control["pair_id"] = altered_pair_id
        with self.assertRaisesRegex(ValueError, "calibration execution identity"):
            opcode_gas.validate_matched_control_fixture_pairs(
                [altered_target, altered_control],
                calibration_execution_identity=diagnostic_provenance(),
            )

        altered_pair_spec = opcode_gas._matched_control_pair_spec(
            target_case, control_case
        )
        altered_pair_spec["lanes"]["target"]["fixture_sha256"] = "f" * 64
        altered_pair_id = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(
                {"kind": "matched_control_pair", "pair_spec": altered_pair_spec}
            )
        )
        altered_target = {
            **target_case,
            "bytecode": control_case["bytecode"],
            "fixture_sha256": "f" * 64,
            "pair_id": altered_pair_id,
        }
        altered_control = {**control_case, "pair_id": altered_pair_id}
        with self.assertRaisesRegex(ValueError, "matched layout"):
            opcode_gas.validate_matched_control_fixture_pairs(
                [altered_target, altered_control]
            )

        tampered_input = dict(target_input)
        tampered_bytecode = bytearray.fromhex(target_input["bytecode"][2:])
        tampered_bytecode[13] ^= 1
        tampered_input["bytecode"] = "0x" + tampered_bytecode.hex()
        tampered_input_bytes = (
            opcode_gas.json.dumps(tampered_input, indent=2, sort_keys=True) + "\n"
        ).encode()
        altered_target = {
            **target_case,
            "fixture_sha256": opcode_gas.sha256_bytes(tampered_input_bytes),
        }
        altered_control = dict(control_case)
        altered_pair_id = opcode_gas.matched_control_pair_id(
            altered_target, altered_control
        )
        altered_target["pair_id"] = altered_pair_id
        altered_control["pair_id"] = altered_pair_id
        with self.assertRaisesRegex(ValueError, "guest input bytecode"):
            opcode_gas.validate_matched_control_fixture_pairs(
                [altered_target, altered_control],
                guest_inputs={
                    altered_target["fixture_sha256"]: tampered_input,
                    altered_control["fixture_sha256"]: control_input,
                },
            )

    def test_matched_control_small_nonzero_profile_is_bound_to_fixture_identity(self):
        expected_operands = {
            0x01: [7, 3],
            0x0A: [3, 5],
            0x15: [7],
        }
        for opcode, operands in expected_operands.items():
            with self.subTest(
                opcode=f"0x{opcode:02x}"
            ), tempfile.TemporaryDirectory() as tmp:
                data = controlled_manifest_data()
                data["variants"] = [2]
                manifest = opcode_gas.parse_controlled_manifest(
                    data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )
                case = opcode_gas.default_opcode_case(opcode)
                manifest = replace(manifest, cases=[case])
                out_dir = pathlib.Path(tmp)

                opcode_gas.generate_cases(
                    manifest,
                    out_dir,
                    provenance=diagnostic_provenance(),
                    generator_max_count=8,
                    matched_control_diagnostic=True,
                    operand_profile="small_nonzero",
                )
                pair_root = out_dir / "controlled" / case.name / "count-2"
                target_case = opcode_gas.json.loads(
                    (pair_root / "target" / "case.json").read_text()
                )
                control_case = opcode_gas.json.loads(
                    (pair_root / "control" / "case.json").read_text()
                )
                target_input = opcode_gas.json.loads(
                    (pair_root / "target" / "guest-input.json").read_text()
                )
                control_input = opcode_gas.json.loads(
                    (pair_root / "control" / "guest-input.json").read_text()
                )

                self.assertEqual(target_case["operand_profile"], "small_nonzero")
                self.assertEqual(control_case["operand_profile"], "small_nonzero")
                self.assertEqual(target_case["operands"], operands)
                self.assertEqual(control_case["operands"], operands)
                target_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(target_input["bytecode"][2:])
                )
                control_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(control_input["bytecode"][2:])
                )
                self.assertEqual(
                    [program[:-2] for program in target_programs],
                    [program[:-2] for program in control_programs],
                )
                expected_setup = b"".join(
                    opcode_gas._fixed_push(value, target_opcode=opcode)
                    for value in operands
                )
                self.assertTrue(
                    all(program.startswith(expected_setup) for program in target_programs)
                )
                pairs = opcode_gas.validate_matched_control_fixture_pairs(
                    [target_case, control_case],
                    guest_inputs={
                        target_case["fixture_sha256"]: target_input,
                        control_case["fixture_sha256"]: control_input,
                    },
                )
                self.assertEqual(list(pairs), [target_case["pair_id"]])
                pair_spec = opcode_gas._matched_control_pair_spec(
                    target_case, control_case
                )
                self.assertEqual(
                    pair_spec["workload"]["operand_profile"], "small_nonzero"
                )
                self.assertEqual(pair_spec["workload"]["operands"], operands)

                for updates, message in [
                    ({"operand_profile": "unknown"}, "operand profile"),
                    ({"operands": [0] * len(operands)}, "operands"),
                ]:
                    altered_target = {**target_case, **updates}
                    altered_control = {**control_case, **updates}
                    with self.assertRaisesRegex(ValueError, message):
                        opcode_gas.matched_control_pair_id(
                            altered_target, altered_control
                        )

                zero_target = opcode_gas.build_matched_control_bytecode(
                    case, 2, 8, lane="target", operand_profile="zero"
                )
                altered_target = {
                    **target_case,
                    "bytecode": "0x" + zero_target.bytes_hex,
                }
                with self.assertRaisesRegex(ValueError, "matched layout"):
                    opcode_gas.matched_control_pair_id(altered_target, control_case)

                if opcode == 0x01:
                    altered_input = {
                        **target_input,
                        "bytecode": "0x" + zero_target.bytes_hex,
                    }
                    altered_input_bytes = (
                        opcode_gas.json.dumps(
                            altered_input, indent=2, sort_keys=True
                        )
                        + "\n"
                    ).encode()
                    altered_target = {
                        **target_case,
                        "fixture_sha256": opcode_gas.sha256_bytes(
                            altered_input_bytes
                        ),
                    }
                    altered_control = dict(control_case)
                    altered_pair_id = opcode_gas.matched_control_pair_id(
                        altered_target, altered_control
                    )
                    altered_target["pair_id"] = altered_pair_id
                    altered_control["pair_id"] = altered_pair_id
                    with self.assertRaisesRegex(
                        ValueError, "guest input bytecode"
                    ):
                        opcode_gas.validate_matched_control_fixture_pairs(
                            [altered_target, altered_control],
                            guest_inputs={
                                altered_target["fixture_sha256"]: altered_input,
                                altered_control["fixture_sha256"]: control_input,
                            },
                        )

        with self.assertRaisesRegex(ValueError, "operand profile"):
            opcode_gas.matched_control_operands(
                opcode_gas.default_opcode_case(0x01), "unknown"
            )

    def test_matched_control_exp_uses_pop_and_unary_uses_not_reference(self):
        for opcode, relation, control_opcode, target_count, control_count in [
            (0x0A, "OP-POP", 0x50, 2, 8),
            (0x15, "OP-NOT", 0x19, 2, 8),
            (0x19, "OP-NOT", 0x19, 8, 8),
        ]:
            with self.subTest(opcode=f"0x{opcode:02x}"), tempfile.TemporaryDirectory() as tmp:
                data = controlled_manifest_data()
                data["variants"] = [2]
                manifest = opcode_gas.parse_controlled_manifest(
                    data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )
                case = opcode_gas.default_opcode_case(opcode)
                manifest = replace(manifest, cases=[case])
                out_dir = pathlib.Path(tmp)

                opcode_gas.generate_cases(
                    manifest,
                    out_dir,
                    provenance=diagnostic_provenance(),
                    generator_max_count=8,
                    matched_control_diagnostic=True,
                )
                pair_root = out_dir / "controlled" / case.name / "count-2"
                target_case = opcode_gas.json.loads(
                    (pair_root / "target" / "case.json").read_text()
                )
                target_input = opcode_gas.json.loads(
                    (pair_root / "target" / "guest-input.json").read_text()
                )
                control_input = opcode_gas.json.loads(
                    (pair_root / "control" / "guest-input.json").read_text()
                )

                self.assertEqual(target_case["relation"], relation)
                self.assertEqual(
                    target_case["operands"], [2, 2] if opcode == 0x0A else [1]
                )
                self.assertEqual(target_input["target_count"], target_count)
                self.assertEqual(control_input["opcode"], control_opcode)
                self.assertEqual(control_input["target_count"], control_count)
                self.assertEqual(
                    control_input["target_raw_gas"], 2 if opcode == 0x0A else 3
                )
                self.assertEqual(
                    target_input["fixed_bytecode_len"],
                    control_input["fixed_bytecode_len"],
                )
                target_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(target_input["bytecode"][2:])
                )
                control_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(control_input["bytecode"][2:])
                )
                self.assertEqual(
                    [program[:-2] for program in target_programs],
                    [program[:-2] for program in control_programs],
                )
                if opcode == 0x0A:
                    self.assertTrue(
                        all(program.endswith(b"\x50\x00") for program in control_programs)
                    )
                else:
                    self.assertTrue(
                        all(program.endswith(b"\x19\x00") for program in control_programs)
                    )
                    self.assertTrue(
                        all(
                            program.endswith(bytes([opcode, 0x00]))
                            for program in target_programs[:2]
                        )
                    )
                    self.assertTrue(
                        all(program.endswith(b"\x19\x00") for program in target_programs[2:])
                    )
                    if opcode == 0x19:
                        self.assertEqual(target_programs, control_programs)

    def test_matched_control_rejects_ternary_until_two_pop_contract_exists(self):
        data = controlled_manifest_data()
        data["variants"] = [2]
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(manifest, cases=[opcode_gas.default_opcode_case(0x08)])

        with tempfile.TemporaryDirectory() as tmp, self.assertRaisesRegex(
            ValueError, "stack_ternary.*unsupported"
        ):
            opcode_gas.generate_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
                matched_control_diagnostic=True,
            )

    def test_matched_control_case_selection_is_explicit_and_manifest_ordered(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml",
            schedule=fixture_schedule(),
        )

        selected = opcode_gas.select_matched_control_cases(
            manifest, ["iszero", "add", "exp"]
        )

        self.assertEqual([case.name for case in selected.cases], ["add", "exp", "iszero"])
        with self.assertRaisesRegex(ValueError, "unknown matched-control case"):
            opcode_gas.select_matched_control_cases(manifest, ["not-a-case"])

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
        self.assertEqual({row["tx_gas_limit"] for row in inputs}, {1_000_024})
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
        self.assertEqual({row["tx_gas_limit"] for row in first_rows}, {1_000_024})
        self.assertEqual({row["tx_gas_limit"] for row in expanded_rows}, {1_000_096})
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
