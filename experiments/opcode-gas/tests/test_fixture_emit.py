import hashlib
import pathlib
import sys
import tempfile
import unittest
from collections import Counter
from dataclasses import replace

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

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


def emit_matched_control_case(
    case: opcode_gas.CaseSpec,
    *,
    operand_profile: str = "zero",
    diagnostic_count: int = 2,
    generator_max_count: int = 8,
) -> tuple[
    opcode_gas.CaseSpec,
    dict,
    dict,
    dict,
    dict,
]:
    data = controlled_manifest_data()
    data["variants"] = [diagnostic_count]
    manifest = opcode_gas.parse_controlled_manifest(
        data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
    )
    manifest = replace(manifest, cases=[case])
    with tempfile.TemporaryDirectory() as tmp:
        out_dir = pathlib.Path(tmp)
        opcode_gas.generate_cases(
            manifest,
            out_dir,
            provenance=diagnostic_provenance(),
            generator_max_count=generator_max_count,
            matched_control_diagnostic=True,
            operand_profile=operand_profile,
        )
        pair_root = out_dir / "controlled" / case.name / f"count-{diagnostic_count}"
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
    return case, target_case, control_case, target_input, control_input


def emit_matched_control_pair(
    opcode: int,
    *,
    operand_profile: str = "zero",
    diagnostic_count: int = 2,
    generator_max_count: int = 8,
) -> tuple[
    opcode_gas.CaseSpec,
    dict,
    dict,
    dict,
    dict,
]:
    return emit_matched_control_case(
        opcode_gas.default_opcode_case(opcode),
        operand_profile=operand_profile,
        diagnostic_count=diagnostic_count,
        generator_max_count=generator_max_count,
    )


class FixtureEmitTests(unittest.TestCase):
    def test_ordinary_opcode_raw_run_preserves_fixture_and_report_opcode_counts(self):
        data = controlled_manifest_data()
        data["variants"] = [1]
        manifest = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        manifest = replace(
            manifest, cases=[opcode_gas.default_opcode_case(0x01)]
        )
        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            case = opcode_gas.json.loads(written[0].read_text())

        self.assertNotIn("opcode_counts", case)
        static_counts = case["evm_opcode_counts"]
        risc_v_profile = [{"label": "add", "count": 123}]
        raw = opcode_gas.raw_run_from_report(
            case,
            {
                "gas": 456,
                "sp1_execution_engine": "gas-estimator",
                "sp1_gas_trace_chunk_threshold": 134_217_728,
                "sp1_gas_trace_chunk_slots": 2,
                "opcode_counts": risc_v_profile,
            },
        )

        self.assertEqual(raw["evm_opcode_counts"], static_counts)
        self.assertEqual(raw["opcode_counts"], risc_v_profile)

    def test_matched_opcode_raw_run_preserves_fixture_and_report_opcode_counts(self):
        _case, target, _control, _target_input, _control_input = (
            emit_matched_control_pair(0x01)
        )
        self.assertNotIn("opcode_counts", target)
        static_counts = target["evm_opcode_counts"]
        risc_v_profile = [{"label": "add", "count": 123}]
        raw = opcode_gas.raw_run_from_report(
            target,
            {
                "gas": 456,
                "sp1_execution_engine": "gas-estimator",
                "sp1_gas_trace_chunk_threshold": 134_217_728,
                "sp1_gas_trace_chunk_slots": 2,
                "opcode_counts": risc_v_profile,
            },
        )

        self.assertEqual(raw["evm_opcode_counts"], static_counts)
        self.assertEqual(raw["opcode_counts"], risc_v_profile)

    def test_formal_relation_fixture_keys_do_not_overlap_bench_report(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        cases = {case.name: case for case in manifest.cases}
        selected = {}
        for relation in manifest.opcode_relations:
            spec = opcode_gas.relation_matched_control_spec(
                cases[relation.case_id], relation.scenario
            )
            selected.setdefault(spec.compound, relation)
        self.assertEqual(set(selected), {False, True})
        manifest = replace(
            manifest,
            variants=[1],
            opcode_relations=tuple(selected[compound] for compound in (False, True)),
        )

        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            fixtures = [
                opcode_gas.json.loads(path.read_text()) for path in written
            ]

        bench_report_fields = {
            "stage",
            "mode",
            "proof_mode",
            "sp1_execution_engine",
            "sp1_gas_trace_chunk_threshold",
            "sp1_gas_trace_chunk_slots",
            "input",
            "guest_input_sha256",
            "guest_input_bincode_length",
            "public_values",
            "wall_time_ms",
            "primary_workload_metric",
            "workload_metrics",
            "exit_code",
            "gas",
            "total_instruction_count",
            "total_syscall_count",
            "touched_memory_addresses",
            "risc0_image_id",
            "risc0_input_bytes",
            "risc0_user_cycles",
            "risc0_padded_cycles",
            "risc0_segment_count",
            "risc0_po2_counts",
            "cycle_tracker",
            "invocation_tracker",
            "opcode_counts",
            "syscall_counts",
            "memory_snapshots",
            "controlled_trace",
            "controlled_overhead",
            "controlled_block",
        }
        for fixture in fixtures:
            with self.subTest(relation_id=fixture["relation_id"]):
                self.assertFalse(set(fixture).intersection(bench_report_fields))
                self.assertTrue(fixture["evm_opcode_counts"])

    def test_formal_relation_generation_adds_distinct_tail_position_holdout(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        relation = next(
            item for item in manifest.opcode_relations if item.signed_raw_gas_by_key
        )
        manifest = replace(manifest, variants=[0, 1], opcode_relations=(relation,))

        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            rows = [opcode_gas.json.loads(path.read_text()) for path in written]

        target_rows = [row for row in rows if row["lane"] == "target"]
        self.assertEqual(
            [
                (row["relation_placement"], row["diagnostic_count"])
                for row in target_rows
            ],
            [("active_prefix", 0), ("active_prefix", 1), ("active_tail", 1)],
        )
        self.assertEqual(len({row["relation_sample_id"] for row in target_rows}), 3)
        prefix = next(
            row for row in target_rows if row["relation_placement"] == "active_prefix" and row["diagnostic_count"] == 1
        )
        tail = next(
            row for row in target_rows if row["relation_placement"] == "active_tail"
        )
        prefix_programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(prefix["bytecode"].removeprefix("0x"))
        )
        tail_programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(tail["bytecode"].removeprefix("0x"))
        )
        self.assertEqual(prefix_programs[0], tail_programs[-1])
        self.assertEqual(prefix_programs[1:], tail_programs[:-1])
        self.assertEqual(prefix["evm_opcode_counts"], tail["evm_opcode_counts"])

    def test_formal_relation_generation_binds_dynamic_programs_and_exact_raw_gas_rows(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        exp_relations = tuple(
            relation
            for relation in manifest.opcode_relations
            if relation.dynamic_key == "opcode:0x0a"
        )
        manifest = replace(
            manifest,
            variants=[1],
            opcode_relations=exp_relations,
        )

        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            rows = [opcode_gas.json.loads(path.read_text()) for path in written]
            inputs = {
                row["fixture_sha256"]: opcode_gas.json.loads(
                    path.with_name("guest-input.json").read_text()
                )
                for row, path in zip(rows, written)
            }

        self.assertEqual(len(rows), 32)
        self.assertEqual({row["purpose"] for row in rows}, {"formal_opcode_relation"})
        self.assertEqual({row["diagnostic_only"] for row in rows}, {False})
        self.assertEqual({row["signal_kind"] for row in rows}, {"signed_raw_gas_relation"})
        targets = sorted(
            (
                row
                for row in rows
                if row["lane"] == "target"
                and row["relation_placement"] == "active_prefix"
            ),
            key=lambda row: row["target_raw_gas"],
        )
        self.assertEqual(
            [row["target_raw_gas"] for row in targets],
            [10, 60, 110, 210, 410, 810, 1210, 1610],
        )
        self.assertEqual(
            [row["target_raw_gas_by_key"] for row in targets],
            [
                {"opcode:0x0a": "10"},
                {"opcode:0x0a": "60"},
                {"opcode:0x0a": "110"},
                {"opcode:0x0a": "210"},
                {"opcode:0x0a": "410"},
                {"opcode:0x0a": "810"},
                {"opcode:0x0a": "1210"},
                {"opcode:0x0a": "1610"},
            ],
        )
        zero_byte_relations = [
            relation
            for relation in exp_relations
            if relation.scenario["exponent_byte_length"] == 0
        ]
        self.assertEqual(len(zero_byte_relations), 1)
        self.assertEqual(zero_byte_relations[0].model_split, "fit")
        zero_target_rows = [
            row for row in rows if row["relation_id"] == "opcode:0x0a:exp-bytes-0"
        ]
        self.assertEqual(len(zero_target_rows), 4)
        for target, control in zip(zero_target_rows[::2], zero_target_rows[1::2]):
            self.assertEqual(target["lane"], "target")
            self.assertEqual(control["lane"], "control")
            self.assertEqual(target["fixed_bytecode_len"], control["fixed_bytecode_len"])
            self.assertEqual(len(target["bytecode"]), len(control["bytecode"]))
        zero_target = next(
            row
            for row in zero_target_rows
            if row["lane"] == "target"
            and row["relation_placement"] == "active_prefix"
            and row["diagnostic_count"] == 1
        )
        zero_program = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(zero_target["bytecode"].removeprefix("0x"))
        )[0]
        self.assertEqual(
            zero_program,
            b"\x7f"
            + bytes(32)
            + b"\x7f"
            + (2).to_bytes(32, "big")
            + b"\x0a\x00",
        )
        self.assertEqual(
            {row["control_raw_gas_by_key"]["opcode:0x50"] for row in rows},
            {"2"},
        )
        self.assertEqual(
            {row["signed_raw_gas_by_key"]["opcode:0x50"] for row in rows},
            {"-2"},
        )
        exp_32_target = next(
            row for row in targets if row["relation_id"] == "opcode:0x0a:exp-bytes-32"
        )
        exp_32_program = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(exp_32_target["bytecode"].removeprefix("0x"))
        )[0]
        self.assertEqual(
            exp_32_program,
            b"\x7f"
            + (1 << 248).to_bytes(32, "big")
            + b"\x7f"
            + (2).to_bytes(32, "big")
            + b"\x0a\x00",
        )
        pairs = opcode_gas.validate_matched_control_fixture_pairs(
            rows,
            expected_purpose=opcode_gas.FORMAL_RELATION_PURPOSE,
            guest_inputs=inputs,
        )
        self.assertEqual(len(pairs), 16)
        altered = [dict(row) for row in rows]
        first_relation_id = altered[0]["relation_id"]
        for row in altered:
            if row["relation_id"] == first_relation_id:
                row["target_raw_gas_by_key"] = {"opcode:0x0a": "61"}
        with self.assertRaisesRegex(ValueError, "target raw-gas map"):
            opcode_gas.validate_matched_control_fixture_pairs(
                altered,
                expected_purpose=opcode_gas.FORMAL_RELATION_PURPOSE,
                guest_inputs=inputs,
            )

    def test_dynamic_memory_warmup_changes_executed_bytecode_and_target_raw_gas(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        relations = tuple(
            relation
            for relation in manifest.opcode_relations
            if relation.dynamic_key == "opcode:0x20"
            and relation.scenario["input_length"] == 256
        )
        self.assertEqual(
            [dict(relation.scenario) for relation in relations],
            [
                {"input_length": 256, "initial_memory_words": 8},
                {"input_length": 256, "initial_memory_words": 1},
            ],
        )
        manifest = replace(manifest, variants=[1], opcode_relations=relations)

        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            rows = [opcode_gas.json.loads(path.read_text()) for path in written]

        prefix_rows = [
            row
            for row in rows
            if row["relation_placement"] == "active_prefix"
            and row["diagnostic_count"] == 1
        ]
        target_rows = [row for row in prefix_rows if row["lane"] == "target"]
        self.assertEqual([row["target_raw_gas"] for row in target_rows], [78, 99])
        self.assertEqual(
            [row["target_raw_gas_by_key"] for row in target_rows],
            [{"opcode:0x20": "78"}, {"opcode:0x20": "99"}],
        )

        zero_word = bytes(32)

        def expected_warmup(initial_words):
            return (
                b"\x7f"
                + (initial_words * 32).to_bytes(32, "big")
                + b"\x7f"
                + zero_word
                + b"\x7f"
                + zero_word
                + b"\x37"
            )

        first_programs = {}
        for row in prefix_rows:
            first_program = opcode_gas.decode_fixed_microprograms(
                bytes.fromhex(row["bytecode"].removeprefix("0x"))
            )[0]
            initial_words = row["relation_scenario"]["initial_memory_words"]
            self.assertTrue(first_program.startswith(expected_warmup(initial_words)))
            first_programs[(initial_words, row["lane"])] = first_program
        self.assertNotEqual(
            first_programs[(8, "target")], first_programs[(1, "target")]
        )
        for initial_words in (8, 1):
            warmup_len = len(expected_warmup(initial_words))
            self.assertEqual(
                first_programs[(initial_words, "target")][:warmup_len],
                first_programs[(initial_words, "control")][:warmup_len],
            )

    def test_formal_exp_zero_fixture_validator_rejects_noncanonical_byte_lengths(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        zero_relation = next(
            relation
            for relation in manifest.opcode_relations
            if relation.id == "opcode:0x0a:exp-bytes-0"
        )
        manifest = replace(manifest, variants=[1], opcode_relations=(zero_relation,))
        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            rows = [opcode_gas.json.loads(path.read_text()) for path in written]

        target = next(
            row
            for row in rows
            if row["lane"] == "target"
            and row["relation_placement"] == "active_prefix"
            and row["diagnostic_count"] == 1
        )
        control = next(
            row
            for row in rows
            if row["lane"] == "control"
            and row["relation_placement"] == "active_prefix"
            and row["diagnostic_count"] == 1
        )
        pair_spec = opcode_gas._matched_control_pair_spec(target, control)

        for invalid_scenario in (
            {"exponent_byte_length": 0.5, "initial_memory_words": 0},
            {"exponent_byte_length": -0.5, "initial_memory_words": 0},
            {"exponent_byte_length": False, "initial_memory_words": 0},
            {"exponent_byte_length": "0", "initial_memory_words": 0},
            {},
        ):
            with self.subTest(invalid_scenario=invalid_scenario):
                invalid_pair_spec = dict(pair_spec)
                invalid_pair_spec["workload"] = {
                    **pair_spec["workload"],
                    "relation_scenario": invalid_scenario,
                }
                invalid_pair_id = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(
                        {
                            "kind": "matched_control_pair",
                            "pair_spec": invalid_pair_spec,
                        }
                    )
                )
                invalid_target = {
                    **target,
                    "relation_scenario": invalid_scenario,
                    "pair_id": invalid_pair_id,
                }
                invalid_control = {
                    **control,
                    "relation_scenario": invalid_scenario,
                    "pair_id": invalid_pair_id,
                }

                with self.assertRaisesRegex(
                    ValueError, "EXP|dynamic (opcode|relation) scenario"
                ):
                    opcode_gas.validate_matched_control_fixture_pairs(
                        [invalid_target, invalid_control],
                        expected_purpose=opcode_gas.FORMAL_RELATION_PURPOSE,
                    )

    def test_formal_relation_model_split_is_required_and_fail_closed(self):
        manifest = opcode_gas.load_manifest(
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml",
            schedule=fixture_schedule(),
        )
        relation = next(
            relation
            for relation in manifest.opcode_relations
            if relation.dynamic_key is not None and relation.split == "canonical"
        )
        manifest = replace(manifest, variants=[1], opcode_relations=(relation,))
        with tempfile.TemporaryDirectory() as tmp:
            written = opcode_gas.generate_relation_cases(
                manifest,
                pathlib.Path(tmp),
                provenance=diagnostic_provenance(),
                generator_max_count=8,
            )
            rows = [opcode_gas.json.loads(path.read_text()) for path in written]
            inputs = {
                row["fixture_sha256"]: opcode_gas.json.loads(
                    path.with_name("guest-input.json").read_text()
                )
                for row, path in zip(rows, written)
            }

        self.assertEqual({row["model_split"] for row in rows}, {"fit"})
        for mutation in ("missing", "tampered"):
            malformed = [dict(row) for row in rows]
            for row in malformed:
                if mutation == "missing":
                    row.pop("model_split")
                else:
                    row["model_split"] = "holdout"
            with self.subTest(mutation=mutation), self.assertRaisesRegex(
                ValueError, "model split"
            ):
                opcode_gas.validate_matched_control_fixture_pairs(
                    malformed,
                    expected_purpose=opcode_gas.FORMAL_RELATION_PURPOSE,
                    guest_inputs=inputs,
                )

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

    def test_matched_control_additional_families_reuse_exact_fixed_setup(self):
        cases = [
            (0x51, [0], "OP-NOT", 0x19, 3, 1),
            (0x20, [32, 0], "OP-POP", 0x50, 2, 1),
            (0x80, [1], "OP-DUP1", 0x80, 3, 2),
            (0x8F, list(range(1, 17)), "OP-DUP1", 0x80, 3, 17),
            (0x90, [1, 2], "OP-SWAP1", 0x90, 3, 2),
            (0x9F, list(range(1, 18)), "OP-SWAP1", 0x90, 3, 17),
            (0x58, [], "OP-PUSH0", 0x5F, 2, 1),
            (0x59, [], "OP-PUSH0", 0x5F, 2, 1),
            (0x5A, [], "OP-PUSH0", 0x5F, 2, 1),
            (0x5F, [], "OP-PUSH0", 0x5F, 2, 1),
        ]
        for opcode, operands, relation, reference, raw_gas, final_height in cases:
            with self.subTest(opcode=f"0x{opcode:02x}"):
                (
                    case,
                    target_case,
                    control_case,
                    target_input,
                    control_input,
                ) = emit_matched_control_pair(opcode)
                target_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(target_input["bytecode"][2:])
                )
                control_programs = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(control_input["bytecode"][2:])
                )
                instruction = opcode_gas._fixed_target_instruction(case)
                active_slot = opcode_gas._fixed_slot(case, active=True)
                self.assertTrue(active_slot.endswith(instruction + b"\x00"))
                expected_setup = active_slot[: -len(instruction) - 1]
                expected_reference_slot = expected_setup + bytes([reference, 0x00])
                expected_target_slot = expected_setup + bytes([opcode, 0x00])

                self.assertEqual(target_case["operand_profile"], "zero")
                self.assertEqual(target_case["operands"], operands)
                self.assertEqual(target_case["relation"], relation)
                self.assertEqual(target_case["final_stack_height"], final_height)
                self.assertEqual(control_case["opcode"], f"0x{reference:02x}")
                self.assertEqual(control_case["target_raw_gas"], raw_gas)
                self.assertEqual(control_input["target_count"], 8)
                self.assertEqual(
                    target_input["target_count"], 8 if opcode == reference else 2
                )
                self.assertTrue(
                    all(program == expected_reference_slot for program in control_programs)
                )
                self.assertEqual(
                    target_programs,
                    [
                        expected_target_slot
                        if index < 2
                        else expected_reference_slot
                        for index in range(8)
                    ],
                )
                self.assertEqual(
                    target_input["fixed_bytecode_len"],
                    control_input["fixed_bytecode_len"],
                )
                self.assertEqual(
                    target_input["tx_gas_limit"], control_input["tx_gas_limit"]
                )
                pairs = opcode_gas.validate_matched_control_fixture_pairs(
                    [target_case, control_case],
                    guest_inputs={
                        target_case["fixture_sha256"]: target_input,
                        control_case["fixture_sha256"]: control_input,
                    },
                )
                self.assertEqual(list(pairs), [target_case["pair_id"]])
                if opcode == reference:
                    self.assertEqual(target_input["bytecode"], control_input["bytecode"])

    def test_matched_control_additional_families_reject_invalid_contracts(self):
        for opcode in (0x51, 0x20, 0x80, 0x90, 0x58, 0x5F):
            with self.subTest(opcode=f"0x{opcode:02x}"), self.assertRaisesRegex(
                ValueError, "operand profile.*zero"
            ):
                emit_matched_control_pair(
                    opcode, operand_profile="small_nonzero"
                )

        _, target, control, target_input, control_input = emit_matched_control_pair(
            0x51
        )
        for updates, message in [
            ({"relation": "OP-POP"}, "relation"),
            ({"final_stack_height": 2}, "final stack"),
        ]:
            with self.subTest(message=message), self.assertRaisesRegex(
                ValueError, message
            ):
                opcode_gas.matched_control_pair_id(
                    {**target, **updates}, {**control, **updates}
                )
        with self.assertRaisesRegex(ValueError, "control declaration"):
            opcode_gas.matched_control_pair_id(
                target,
                {
                    **control,
                    "opcode": "0x50",
                    "target_raw_gas": 2,
                },
            )

        tampered_input = dict(target_input)
        tampered_bytecode = bytearray.fromhex(target_input["bytecode"][2:])
        tampered_bytecode[13] ^= 1
        tampered_input["bytecode"] = "0x" + tampered_bytecode.hex()
        tampered_bytes = (
            opcode_gas.json.dumps(tampered_input, indent=2, sort_keys=True) + "\n"
        ).encode()
        altered_target = {
            **target,
            "fixture_sha256": opcode_gas.sha256_bytes(tampered_bytes),
        }
        altered_control = dict(control)
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

    def test_matched_control_compound_families_bind_exact_executed_programs(self):
        cases = [
            (
                0x08,
                "OP-(POP+ADD)",
                "08",
                "5001",
                {"0x01": 1, "0x50": 1},
                5,
                0x50,
                2,
                1,
                2,
            ),
            (
                0x09,
                "OP-(POP+ADD)",
                "09",
                "5001",
                {"0x01": 1, "0x50": 1},
                5,
                0x50,
                2,
                1,
                2,
            ),
            (
                0x50,
                "POP-(NOT+POP)",
                "50",
                "1950",
                {"0x19": 1, "0x50": 1},
                5,
                0x19,
                3,
                0,
                8,
            ),
            (
                0x52,
                "OP-(ADD+POP)",
                "52",
                "0150",
                {"0x01": 1, "0x50": 1},
                5,
                0x01,
                3,
                0,
                2,
            ),
            (
                0x53,
                "OP-(ADD+POP)",
                "53",
                "0150",
                {"0x01": 1, "0x50": 1},
                5,
                0x01,
                3,
                0,
                2,
            ),
            (
                0x5E,
                "MCOPY-(ADD+MUL+POP)",
                "5e",
                "010250",
                {"0x01": 1, "0x02": 1, "0x50": 1},
                10,
                0x01,
                3,
                0,
                2,
            ),
            (
                0x60,
                "OP-PUSH0",
                "6000",
                "5f",
                {"0x5f": 1},
                2,
                0x5F,
                2,
                1,
                2,
            ),
            (
                0x7F,
                "OP-PUSH0",
                "7f" + "00" * 32,
                "5f",
                {"0x5f": 1},
                2,
                0x5F,
                2,
                1,
                2,
            ),
            (
                0x56,
                "JUMP-POP",
                "56",
                "50",
                {"0x50": 1},
                2,
                0x50,
                2,
                0,
                2,
            ),
            (
                0x57,
                "JUMPI-(ADD+POP)",
                "57",
                "0150",
                {"0x01": 1, "0x50": 1},
                5,
                0x01,
                3,
                0,
                2,
            ),
            (
                0x5B,
                "JUMPDEST-(PUSH0+POP)",
                "5b",
                "5f50",
                {"0x50": 1, "0x5f": 1},
                4,
                0x5F,
                2,
                0,
                2,
            ),
        ]
        compound_fields = {
            "target_program",
            "reference_program",
            "reference_opcode_counts",
            "reference_raw_gas_total",
            "target_pre_suffix_padding",
            "control_pre_suffix_padding",
            "common_suffix",
            "target_post_suffix_padding",
            "control_post_suffix_padding",
        }
        for (
            opcode,
            relation,
            target_program,
            reference_program,
            reference_counts,
            reference_raw_gas_total,
            primary_reference,
            primary_raw_gas,
            final_height,
            target_count,
        ) in cases:
            with self.subTest(opcode=f"0x{opcode:02x}"):
                _, target, control, target_input, control_input = (
                    emit_matched_control_pair(opcode)
                )
                self.assertTrue(compound_fields <= target.keys())
                self.assertTrue(compound_fields <= control.keys())
                self.assertEqual(target["target_program"], "0x" + target_program)
                self.assertEqual(target["reference_program"], "0x" + reference_program)
                self.assertEqual(target["reference_opcode_counts"], reference_counts)
                self.assertEqual(
                    target["reference_raw_gas_total"], reference_raw_gas_total
                )
                self.assertEqual(target["relation"], relation)
                self.assertEqual(target["final_stack_height"], final_height)
                self.assertEqual(control["opcode"], f"0x{primary_reference:02x}")
                self.assertEqual(control["target_raw_gas"], primary_raw_gas)
                self.assertEqual(control_input["target_count"], 8)
                self.assertEqual(target_input["target_count"], target_count)

                target_slots = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(target_input["bytecode"][2:])
                )
                control_slots = opcode_gas.decode_fixed_microprograms(
                    bytes.fromhex(control_input["bytecode"][2:])
                )
                self.assertEqual(target_slots[2:], control_slots[2:])
                for target_slot, control_slot in zip(target_slots[:2], control_slots[:2]):
                    self.assertEqual(len(target_slot), len(control_slot))
                    self.assertEqual(Counter(target_slot), Counter(control_slot))
                pairs = opcode_gas.validate_matched_control_fixture_pairs(
                    [target, control],
                    guest_inputs={
                        target["fixture_sha256"]: target_input,
                        control["fixture_sha256"]: control_input,
                    },
                )
                self.assertEqual(list(pairs), [target["pair_id"]])

    def test_matched_control_compound_jump_layouts_bind_executed_suffixes(self):
        _, jump_target, jump_control, jump_target_input, jump_control_input = (
            emit_matched_control_pair(0x56)
        )
        jump_target_slot = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(jump_target_input["bytecode"][2:])
        )[0]
        jump_control_slot = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(jump_control_input["bytecode"][2:])
        )[0]
        self.assertEqual(jump_target["common_suffix"], "0x5b00")
        self.assertEqual(jump_target_slot[-4:], bytes.fromhex("565b0050"))
        self.assertEqual(jump_control_slot[-4:], bytes.fromhex("505b0056"))
        self.assertEqual(int.from_bytes(jump_target_slot[1:33], "big"), 34)

        _, jumpi_target, jumpi_control, jumpi_target_input, jumpi_control_input = (
            emit_matched_control_pair(0x57)
        )
        jumpi_target_slot = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(jumpi_target_input["bytecode"][2:])
        )[0]
        jumpi_control_slot = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(jumpi_control_input["bytecode"][2:])
        )[0]
        self.assertEqual(jumpi_target["target_pre_suffix_padding"], "0x00")
        self.assertEqual(jumpi_target["control_pre_suffix_padding"], "0x")
        self.assertEqual(jumpi_target["common_suffix"], "0x5b00")
        self.assertEqual(jumpi_target_slot[-6:], bytes.fromhex("57005b000150"))
        self.assertEqual(jumpi_control_slot[-6:], bytes.fromhex("01505b005700"))
        self.assertEqual(int.from_bytes(jumpi_target_slot[34:66], "big"), 68)

    def test_matched_control_compound_count_zero_lanes_are_identical(self):
        for opcode in (
            0x08,
            0x50,
            0x52,
            0x53,
            0x5E,
            0x60,
            0x7F,
            0x56,
            0x57,
            0x5B,
        ):
            with self.subTest(opcode=f"0x{opcode:02x}"):
                _, _, _, target_input, control_input = emit_matched_control_pair(
                    opcode, diagnostic_count=0
                )
                self.assertEqual(target_input["bytecode"], control_input["bytecode"])

    def test_matched_control_compound_metadata_is_complete_and_canonical(self):
        _, target, control, target_input, control_input = emit_matched_control_pair(
            0x52
        )
        for field, bad_value in (
            ("reference_program", "0x5001"),
            ("reference_opcode_counts", {"0x01": 2}),
            ("reference_raw_gas_total", 6),
            ("common_suffix", "0x0000"),
            ("target_post_suffix_padding", "0x50"),
        ):
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, field.replace("_", " ")
            ):
                opcode_gas.matched_control_pair_id(
                    {**target, field: bad_value}, {**control, field: bad_value}
                )
        partial_target = dict(target)
        partial_control = dict(control)
        del partial_target["common_suffix"]
        del partial_control["common_suffix"]
        with self.assertRaisesRegex(ValueError, "compound metadata"):
            opcode_gas.matched_control_pair_id(partial_target, partial_control)

        tampered_input = dict(target_input)
        tampered = bytearray.fromhex(tampered_input["bytecode"][2:])
        tampered[-1] ^= 1
        tampered_input["bytecode"] = "0x" + tampered.hex()
        tampered_bytes = (
            opcode_gas.json.dumps(tampered_input, indent=2, sort_keys=True) + "\n"
        ).encode()
        altered_target = {
            **target,
            "fixture_sha256": opcode_gas.sha256_bytes(tampered_bytes),
        }
        altered_pair = opcode_gas.matched_control_pair_id(altered_target, control)
        altered_target["pair_id"] = altered_pair
        altered_control = {**control, "pair_id": altered_pair}
        with self.assertRaisesRegex(ValueError, "bytecode differs|guest input bytecode"):
            opcode_gas.validate_matched_control_fixture_pairs(
                [altered_target, altered_control],
                guest_inputs={
                    altered_target["fixture_sha256"]: tampered_input,
                    altered_control["fixture_sha256"]: control_input,
                },
            )

    def test_legacy_one_opcode_matched_control_schema_and_identity_are_unchanged(self):
        _, target, control, _, _ = emit_matched_control_pair(0x01)
        self.assertEqual(
            target["pair_id"],
            "636fb8ef0ca79c416ee253fc568af1713594ec69c7fef8831bae07b5288c4371",
        )
        self.assertEqual(
            target["fixture_sha256"],
            "acd876bc5375b954a7af42b8018d2a5b56cda542381c35142bb7f0ad10d49c92",
        )
        self.assertEqual(
            control["fixture_sha256"],
            "2e117b306bb9593ad6bbe28fa92fbea3924db22dd215b9dff0e3f8ec9bff16ce",
        )
        self.assertNotIn("reference_program", target)

    def test_new_compound_families_reject_nonzero_profile_and_unsupported_opcode(self):
        for opcode in (
            0x08,
            0x50,
            0x52,
            0x53,
            0x5E,
            0x60,
            0x7F,
            0x56,
            0x57,
            0x5B,
        ):
            with self.subTest(opcode=f"0x{opcode:02x}"), self.assertRaisesRegex(
                ValueError, "only supports operand profile zero"
            ):
                emit_matched_control_pair(opcode, operand_profile="small_nonzero")
        with self.assertRaisesRegex(ValueError, "canonical opcode/template"):
            emit_matched_control_case(
                opcode_gas.CaseSpec(
                    name="unsupported_tload",
                    scenario="state",
                    template="memory_load_32",
                    target_raw_gas=100,
                    opcode=0x5C,
                )
            )

    def test_matched_control_rejects_relabelled_opcode_templates(self):
        relabelled = [
            replace(opcode_gas.default_opcode_case(0x52), template="keccak_32"),
            opcode_gas.CaseSpec(
                name="sload_relabelled_mload",
                scenario="memory",
                template="memory_load_32",
                target_raw_gas=100,
                opcode=0x54,
            ),
        ]
        for case in relabelled:
            with self.subTest(opcode=f"0x{case.opcode:02x}"), self.assertRaisesRegex(
                ValueError, "canonical opcode/template"
            ):
                emit_matched_control_case(case)

    def test_pair_and_guest_input_reject_relabelled_mstore_as_keccak(self):
        _, target, control, target_input, control_input = emit_matched_control_pair(
            0x20
        )
        programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(target_input["bytecode"][2:])
        )
        relabelled_programs = [
            program[:-2] + b"\x52\x00" if index < 2 else program
            for index, program in enumerate(programs)
        ]
        relabelled_bytecode = "0x" + opcode_gas.encode_fixed_microprograms(
            relabelled_programs
        ).hex()
        relabelled_input = {
            **target_input,
            "opcode": 0x52,
            "target_raw_gas": 3,
            "bytecode": relabelled_bytecode,
        }
        relabelled_input_bytes = (
            opcode_gas.json.dumps(relabelled_input, indent=2, sort_keys=True) + "\n"
        ).encode()
        relabelled_target = {
            **target,
            "original_opcode": "0x52",
            "opcode": "0x52",
            "target_raw_gas": 3,
            "bytecode": relabelled_bytecode,
            "fixture_sha256": opcode_gas.sha256_bytes(relabelled_input_bytes),
        }
        relabelled_control = {**control, "original_opcode": "0x52"}

        pair_spec = opcode_gas._matched_control_pair_spec(target, control)
        pair_spec["workload"]["original_opcode"] = "0x52"
        pair_spec["lanes"]["target"].update(
            {
                "opcode": "0x52",
                "target_raw_gas": 3,
                "fixture_sha256": relabelled_target["fixture_sha256"],
            }
        )
        pair_id = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(
                {"kind": "matched_control_pair", "pair_spec": pair_spec}
            )
        )
        relabelled_target["pair_id"] = pair_id
        relabelled_control["pair_id"] = pair_id

        with self.assertRaisesRegex(ValueError, "canonical opcode/template"):
            opcode_gas.validate_matched_control_fixture_pairs(
                [relabelled_target, relabelled_control],
                guest_inputs={
                    relabelled_target["fixture_sha256"]: relabelled_input,
                    relabelled_control["fixture_sha256"]: control_input,
                },
            )

    def test_matched_control_does_not_change_formal_fixed_footprint_bytecode(self):
        expected_sha256 = {
            0x51: "f09436418a1cd0a79722cbab751cb09cdfddb67b2fd17b5dbb0e9dbc767c6929",
            0x20: "09acb61e7255ab42b1bc1218611ac4d5594e38b9694f8e821e253800a188eb13",
            0x80: "c31a59546743b46eb565a2bf257756090ee0523584079ea8d8fa18e48e1f1767",
            0x8F: "98312c7e2bb1430339bdf8bb8812e6f9adb4098723524bccf9c02b9a80e3d432",
            0x90: "e9ed881375d1aa8ffc11e7fcef1fe59aa6c39d9bc038c8caf8c001168338c8d9",
            0x9F: "be3ef38c230341700b74ca639abf87092b8b2a610925f4038ad5fe663227798b",
            0x58: "92785baa800dff13cc61fae00ca83078fa580554a1bfb5455cf547f416c4e426",
            0x59: "da0149d73c1eba16281882b30a350f88e74ef6fabb64de1e6ff886784506c0db",
            0x5A: "ac8031745ae6e36d77bb3afcec372204f0668954eea1e66c2535a7e05681ae1c",
            0x5F: "6ca1fd88e7225c9823d813bb114c6879133f7b02ab7685a8b6dcbf9cf305c1b1",
        }
        for opcode, expected in expected_sha256.items():
            with self.subTest(opcode=f"0x{opcode:02x}"):
                generated = opcode_gas.build_fixed_footprint_bytecode(
                    opcode_gas.default_opcode_case(opcode), 2, 8
                )
                self.assertEqual(
                    hashlib.sha256(bytes.fromhex(generated.bytes_hex)).hexdigest(),
                    expected,
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
