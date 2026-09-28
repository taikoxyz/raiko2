import copy
import json
import pathlib
import sys
import tempfile
import unittest
from dataclasses import replace

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
import stateful_opcode_campaign as stateful


EXPECTED_SCENARIOS = (
    "sload_cold_zero",
    "sload_cold_nonzero",
    "sload_warm_zero",
    "sload_warm_nonzero",
    "sstore_noop_zero_cold",
    "sstore_noop_zero_warm",
    "sstore_noop_nonzero_cold",
    "sstore_noop_nonzero_warm",
    "sstore_set_cold",
    "sstore_set_warm",
    "sstore_clear_cold",
    "sstore_clear_warm",
    "sstore_reset_nonzero_cold",
    "sstore_reset_nonzero_warm",
    "sstore_dirty_rewrite",
    "sstore_restore_zero",
    "sstore_dirty_rewrite_nonzero",
    "sstore_restore_nonzero",
    "sload_cold_high_value",
    "sload_warm_high_value",
    "sstore_noop_high_value_cold",
    "sstore_noop_high_value_warm",
    "sstore_set_high_value_cold",
    "sstore_set_high_value_warm",
    "sstore_clear_high_value_cold",
    "sstore_clear_high_value_warm",
    "sstore_reset_high_value_cold",
    "sstore_reset_high_value_warm",
    "sstore_dirty_rewrite_high_value",
    "sstore_restore_high_value",
    "sload_cold_nonzero_high_slot",
    "sload_warm_nonzero_high_slot",
    "sstore_noop_nonzero_cold_high_slot",
    "sstore_noop_nonzero_warm_high_slot",
    "sstore_set_cold_high_slot",
    "sstore_set_warm_high_slot",
    "sstore_clear_cold_high_slot",
    "sstore_clear_warm_high_slot",
    "sstore_reset_nonzero_cold_high_slot",
    "sstore_reset_nonzero_warm_high_slot",
)


class StatefulOpcodeManifestTests(unittest.TestCase):
    def test_manifest_freezes_inventory_counts_and_repeats(self):
        payload = stateful.canonical_stateful_manifest_payload()
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "stateful.json"
            path.write_bytes(opcode_gas.canonical_json(payload) + b"\n")
            manifest = stateful.load_stateful_campaign_manifest(path)

        self.assertEqual(tuple(case.name for case in manifest.scenarios), EXPECTED_SCENARIOS)
        self.assertEqual(manifest.fit_counts, (1, 2, 4, 8, 16))
        self.assertEqual(manifest.holdout_count, 32)
        self.assertEqual(manifest.checkpoint_count, 64)
        self.assertEqual(manifest.structural_zero_count, 0)
        self.assertEqual(manifest.repeats, 3)
        self.assertEqual(manifest.primary_counts, (0, 1, 2, 4, 8, 16, 32, 64))
        self.assertEqual(manifest.diagnostic_counts, (0, 64))
        self.assertEqual(
            dict(manifest.reference_registry),
            {
                "path": "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json",
                "artifact_sha256": "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b",
            },
        )

    def test_manifest_rejects_undeclared_scenario_and_malformed_u256(self):
        payload = stateful.canonical_stateful_manifest_payload()
        payload["scenarios"].append(copy.deepcopy(payload["scenarios"][0]))
        payload["scenarios"][-1]["name"] = "undeclared"
        with self.assertRaisesRegex(ValueError, "scenario inventory"):
            stateful.StatefulCampaignManifest.from_mapping(payload)

        for malformed in (
            "0x00",
            "0x" + "00" * 33,
            "-1",
            "0x" + "gg" + "00" * 31,
            "0x" + "A0" + "00" * 31,
        ):
            payload = stateful.canonical_stateful_manifest_payload()
            payload["scenarios"][0]["slot"] = malformed
            with self.subTest(value=malformed), self.assertRaisesRegex(
                ValueError, "canonical U256"
            ):
                stateful.StatefulCampaignManifest.from_mapping(payload)


class StatefulOpcodeGeneratorTests(unittest.TestCase):
    def setUp(self):
        self.manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )

    def test_fixture_uses_push32_and_emits_storage_in_both_records(self):
        first = stateful.generate_stateful_fixture(
            self.manifest, "sstore_set_warm", lane="target", count=4
        )
        second = stateful.generate_stateful_fixture(
            self.manifest, "sstore_set_warm", lane="target", count=4
        )

        self.assertEqual(first, second)
        self.assertEqual(first["case_record"]["case_id"], second["case_record"]["case_id"])
        self.assertEqual(first["guest_input"]["case"], first["case_record"]["case_id"])
        self.assertEqual(first["guest_input"]["scenario"], "sstore_set_warm")
        self.assertEqual(first["case_record"]["kind"], "opcode")
        self.assertEqual(first["case_record"]["opcode"], first["guest_input"]["opcode"])
        self.assertEqual(
            first["case_record"]["bytecode"], first["guest_input"]["bytecode"]
        )
        self.assertEqual(
            first["case_record"]["storage"], first["guest_input"]["storage"]
        )
        self.assertEqual(first["guest_input"]["opcode"], 0x55)
        programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(first["guest_input"]["bytecode"][2:])
        )
        self.assertEqual(len(programs), 64)
        for program in programs:
            self.assertEqual(program[0], 0x7F)
            self.assertEqual(program[33], 0x7F)
        self.assertEqual(sum(program[-3] == 0x55 for program in programs), 4)

    def test_control_keeps_storage_scenario_but_declares_reference_opcode(self):
        load = stateful.generate_stateful_fixture(
            self.manifest, "sload_cold_nonzero", lane="control", count=8
        )
        store = stateful.generate_stateful_fixture(
            self.manifest, "sstore_set_warm", lane="control", count=8
        )

        self.assertEqual(load["case_record"]["relation_count"], 8)
        self.assertEqual(load["guest_input"]["opcode"], 0x19)
        self.assertEqual(load["guest_input"]["target_count"], 64)
        self.assertEqual(load["guest_input"]["target_raw_gas"], 3)
        self.assertEqual(load["guest_input"]["storage"]["measurement_opcode"], 0x54)
        self.assertEqual(load["guest_input"]["storage"]["lane"], "control")
        self.assertEqual(store["guest_input"]["opcode"], 0x50)
        self.assertEqual(store["guest_input"]["target_count"], 128)
        self.assertEqual(store["guest_input"]["target_raw_gas"], 2)
        programs = opcode_gas.decode_fixed_microprograms(
            bytes.fromhex(load["guest_input"]["bytecode"][2:])
        )
        self.assertFalse(any(0x54 in program or 0x55 in program for program in programs))

    def test_fixture_hashes_name_their_json_representations(self):
        fixture = stateful.generate_stateful_fixture(
            self.manifest, "sload_warm_zero", lane="target", count=4
        )
        record = fixture["case_record"]
        payload = opcode_gas.canonical_json(fixture["guest_input"])

        self.assertNotIn("guest_input_sha256", record)
        self.assertEqual(
            record["guest_input_json_payload_sha256"],
            opcode_gas.sha256_bytes(payload),
        )
        self.assertEqual(
            record["guest_input_json_file_sha256"],
            opcode_gas.sha256_bytes(payload + b"\n"),
        )

    def test_reference_ledgers_are_zero_sum_and_stable_for_high_variants(self):
        expected = {
            "sload_warm_nonzero": {"opcode:0x19": -1, "opcode:0x54": 1},
            "sstore_set_warm": {
                "opcode:0x50": -2,
                "opcode:0x55": 1,
                "opcode:0x5b": 1,
            },
        }
        for scenario_name, ledger in expected.items():
            with self.subTest(scenario=scenario_name):
                fixture = stateful.generate_stateful_fixture(
                    self.manifest, scenario_name, lane="target", count=1
                )
                self.assertEqual(fixture["case_record"]["reference_ledger"], ledger)
                self.assertEqual(sum(ledger.values()), 0)

        low = stateful.generate_stateful_fixture(
            self.manifest, "sstore_reset_nonzero_warm", lane="target", count=1
        )
        high = stateful.generate_stateful_fixture(
            self.manifest, "sstore_reset_high_value_warm", lane="target", count=64
        )
        self.assertEqual(
            low["case_record"]["reference_ledger"],
            high["case_record"]["reference_ledger"],
        )

    def test_manifest_admission_checks_every_high_variant_program_shape(self):
        payload = stateful.canonical_stateful_manifest_payload()
        row = next(
            row
            for row in payload["scenarios"]
            if row["name"] == "sload_warm_high_value"
        )
        row["reference_opcode"] = 0x18

        with self.assertRaisesRegex(ValueError, "program shape"):
            stateful.StatefulCampaignManifest.from_mapping(payload)

    def test_dirty_prefix_is_byte_identical_in_target_and_control(self):
        scenario = self.manifest.scenario("sstore_dirty_rewrite")
        target = stateful.build_stateful_program(scenario, lane="target", measured=True)
        control = stateful.build_stateful_program(scenario, lane="control", measured=True)
        first_sstore = target.bytecode.index(0x55)

        self.assertEqual(target.bytecode[: first_sstore + 1], control.bytecode[: first_sstore + 1])
        self.assertEqual(target.bytecode.count(0x55), 2)
        self.assertEqual(control.bytecode.count(0x55), 1)
        self.assertEqual(target.operand_immediate_spans[:2], control.operand_immediate_spans[:2])

    def test_high_limb_pair_differs_only_in_declared_immediate_spans(self):
        low = self.manifest.scenario("sstore_reset_nonzero_warm")
        high = self.manifest.scenario("sstore_reset_high_value_warm")
        low_program = stateful.build_stateful_program(low, lane="target", measured=True)
        high_program = stateful.build_stateful_program(high, lane="target", measured=True)

        stateful.validate_program_shape_pair(low_program, high_program)
        self.assertEqual(low_program.program_shape_sha256, high_program.program_shape_sha256)
        self.assertNotEqual(low_program.bytecode, high_program.bytecode)

        changed_opcode = bytearray(high_program.bytecode)
        changed_opcode[0] = 0x60
        with self.assertRaisesRegex(ValueError, "PUSH32 opcode"):
            stateful.validate_program_shape_pair(
                low_program, replace(high_program, bytecode=bytes(changed_opcode))
            )

        moved_span = replace(
            high_program.operand_immediate_spans[0],
            start=high_program.operand_immediate_spans[0].start + 1,
        )
        with self.assertRaisesRegex(ValueError, "operand spans"):
            stateful.validate_program_shape_pair(
                low_program,
                replace(
                    high_program,
                    operand_immediate_spans=(
                        moved_span,
                        *high_program.operand_immediate_spans[1:],
                    ),
                ),
            )

        changed_suffix = bytearray(high_program.bytecode)
        changed_suffix[-1] = 0x5B
        with self.assertRaisesRegex(ValueError, "program shape"):
            stateful.validate_program_shape_pair(
                low_program, replace(high_program, bytecode=bytes(changed_suffix))
            )

        reference = stateful.build_stateful_program(low, lane="control", measured=True)
        high_reference = stateful.build_stateful_program(high, lane="control", measured=True)
        changed_reference = bytearray(high_reference.bytecode)
        changed_reference[-3] = 0x18
        with self.assertRaisesRegex(ValueError, "program shape"):
            stateful.validate_program_shape_pair(
                reference,
                replace(high_reference, bytecode=bytes(changed_reference)),
            )


if __name__ == "__main__":
    unittest.main()
