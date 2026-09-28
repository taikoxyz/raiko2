import copy
import hashlib
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


def _opcode_counts(case_record):
    counts = {}
    active = case_record["active_program"]["opcode_counts"]
    inactive = case_record["inactive_program"]["opcode_counts"]
    active_slots = case_record["relation_count"] if case_record["lane"] == "target" else 0
    for slot in range(case_record["generator_max_count"]):
        program = active if slot < active_slots else inactive
        for opcode, count in program.items():
            key = f"opcode:{opcode}"
            counts[key] = counts.get(key, 0) + count
    return counts


def _synthetic_report(fixture, marker):
    record = fixture["case_record"]
    guest = fixture["guest_input"]
    backend_hash = hashlib.sha256(f"{marker}:{record['case_id']}".encode()).hexdigest()
    counts = _opcode_counts(record)
    target_key = f"opcode:0x{record['opcode']:02x}"
    raw_gas = {key: count * 3 for key, count in counts.items()}
    if target_key in raw_gas:
        raw_gas[target_key] = record["target_count"] * record["target_raw_gas"]
    storage = record["storage"]
    operation = storage["operation"]
    dirty = (
        operation["kind"] == "store"
        and operation["current_value"] != storage["original_value"]
    )
    programs = []
    for index in range(record["generator_max_count"]):
        measured = record["lane"] == "target" and index < record["relation_count"]
        loads = []
        expected_final = observed_final = None
        if operation["kind"] == "load" and measured:
            loads = [operation["expected_value"]]
        elif operation["kind"] == "store":
            if measured:
                expected_final = operation["new_value"]
            elif dirty:
                expected_final = operation["current_value"]
            else:
                expected_final = storage["original_value"]
            observed_final = expected_final
        programs.append(
            {
                "program_index": index,
                "result_status": "success",
                "expected_load_values": loads,
                "observed_load_values": loads,
                "expected_final_storage": expected_final,
                "observed_final_storage": observed_final,
            }
        )
    bytecode = bytes.fromhex(guest["bytecode"][2:])
    decoded = opcode_gas.decode_fixed_microprograms(bytecode)
    common_identity = hashlib.sha256(
        opcode_gas.canonical_json(
            {
                "gas_limit": record["tx_gas_limit"],
                "slot": storage["slot"],
                "original_value": storage["original_value"],
                "access": storage["access"],
            }
        )
    ).hexdigest()
    trace = {
        "kind": "revm_opcode",
        "schema_version": 2,
        "workload_id": hashlib.sha256(record["case_id"].encode()).hexdigest(),
        "backend_input_sha256": backend_hash,
        "backend_input_len": 999,
        "target_opcode": record["opcode"],
        "declared_target_count": record["target_count"],
        "declared_target_raw_gas": record["target_raw_gas"],
        "tx_gas_limit": record["tx_gas_limit"],
        "executed_target_count": record["target_count"],
        "executed_target_raw_gas": record["target_count"] * record["target_raw_gas"],
        "non_target_counts": {},
        "non_target_raw_gas": 0,
        "total_raw_gas": sum(raw_gas.values()),
        "bytecode_len": len(bytecode),
        "evm_spec": "osaka",
        "revm_version": "41.0.0",
        "shared_constructor": "raiko2-opcode-lab",
        "transaction_envelope_sha256": common_identity,
        "access_list_sha256": hashlib.sha256(storage["access"].encode()).hexdigest(),
        "prestate_sha256": hashlib.sha256(
            (storage["slot"] + storage["original_value"]).encode()
        ).hexdigest(),
        "bytecode_sha256": hashlib.sha256(bytecode).hexdigest(),
        "program_sha256": [hashlib.sha256(program).hexdigest() for program in decoded],
        "executed_opcode_counts": counts,
        "executed_opcode_raw_gas": raw_gas,
        "executed_measurement_count": (
            record["relation_count"] if record["lane"] == "target" else 0
        ),
        "executed_measurement_raw_gas": (
            record["relation_count"] * record["target_raw_gas"]
            if record["lane"] == "target"
            else 0
        ),
        "executed_prefix_count": record["generator_max_count"] if dirty else 0,
        "result_statuses": {"success": record["generator_max_count"]},
        "storage": storage,
        "semantic_check": {
            "schema_version": 1,
            "backend_input_sha256": backend_hash,
            "checked_programs": record["generator_max_count"],
            "passed": True,
            "programs": programs,
        },
    }
    return {
        "guest_input_sha256": f"0x{backend_hash}",
        "guest_input_bincode_length": 999,
        "controlled_trace": trace,
    }


class StatefulTraceAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )

    def fixture(self, scenario="sstore_dirty_rewrite", lane="target", count=4):
        return stateful.generate_stateful_fixture(
            self.manifest, scenario, lane=lane, count=count
        )

    def test_exact_trace_and_semantic_check_make_one_lane_runnable(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")

        admission = stateful.admit_stateful_fixture_trace(
            self.manifest, fixture, [report], repeat_index=2
        )

        self.assertEqual(admission["scenario"], "sstore_dirty_rewrite")
        self.assertEqual(admission["lane"], "target")
        self.assertEqual(admission["relation_count"], 4)
        self.assertEqual(admission["repeat_index"], 2)
        self.assertEqual(admission["backend_input_sha256"], report["controlled_trace"]["backend_input_sha256"])
        self.assertEqual(len(admission["trace_sha256"]), 64)
        self.assertEqual(len(admission["semantic_check_sha256"]), 64)
        self.assertEqual(len(admission["row_identity"]), 64)

    def test_lane_admission_rejects_zero_duplicate_and_wrong_report_identity(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        with self.assertRaisesRegex(ValueError, "exactly one host trace report"):
            stateful.admit_stateful_fixture_trace(
                self.manifest, fixture, [], repeat_index=0
            )
        with self.assertRaisesRegex(ValueError, "exactly one host trace report"):
            stateful.admit_stateful_fixture_trace(
                self.manifest, fixture, [report, copy.deepcopy(report)], repeat_index=0
            )

        wrong = copy.deepcopy(report)
        wrong["guest_input_sha256"] = "0x" + "00" * 32
        with self.assertRaisesRegex(ValueError, "backend-input identity"):
            stateful.admit_stateful_fixture_trace(
                self.manifest, fixture, [wrong], repeat_index=0
            )

    def test_lane_admission_rejects_fixture_trace_and_semantic_drift(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        mutations = (
            ("gas limit", lambda f, r: r["controlled_trace"].__setitem__("tx_gas_limit", 1)),
            ("bytecode length", lambda f, r: r["controlled_trace"].__setitem__("bytecode_len", 1)),
            ("REVM identity", lambda f, r: r["controlled_trace"].__setitem__("evm_spec", "prague")),
            ("storage identity", lambda f, r: r["controlled_trace"]["storage"].__setitem__("slot", stateful.u256_hex(8))),
            ("semantic check", lambda f, r: r["controlled_trace"]["semantic_check"].__setitem__("passed", False)),
            ("result status", lambda f, r: r["controlled_trace"].__setitem__("result_statuses", {"halt": 64})),
            ("exact opcode ledger", lambda f, r: r["controlled_trace"]["executed_opcode_counts"].__setitem__("opcode:0x55", 1)),
        )
        for label, mutate in mutations:
            changed_fixture = copy.deepcopy(fixture)
            changed_report = copy.deepcopy(report)
            mutate(changed_fixture, changed_report)
            with self.subTest(label=label), self.assertRaises(ValueError):
                stateful.admit_stateful_fixture_trace(
                    self.manifest, changed_fixture, [changed_report], repeat_index=0
                )

    def test_lane_admission_rejects_matching_but_wrong_semantic_values(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        program = report["controlled_trace"]["semantic_check"]["programs"][0]
        program["expected_final_storage"] = stateful.ZERO
        program["observed_final_storage"] = stateful.ZERO

        with self.assertRaisesRegex(ValueError, "semantic SSTORE"):
            stateful.admit_stateful_fixture_trace(
                self.manifest, fixture, [report], repeat_index=0
            )

    def test_pair_admission_binds_order_and_rejects_all_confounds(self):
        target = self.fixture(lane="target")
        control = self.fixture(lane="control")
        target_report = _synthetic_report(target, "target")
        control_report = _synthetic_report(control, "control")

        pair = stateful.admit_stateful_pair(
            self.manifest,
            target,
            [target_report],
            control,
            [control_report],
            repeat_index=1,
        )
        expected = stateful.stateful_ordered_pair_identity(
            scenario="sstore_dirty_rewrite",
            measurement_opcode=stateful.SSTORE,
            relation_count=4,
            repeat_index=1,
            target_hash=target_report["controlled_trace"]["backend_input_sha256"],
            control_hash=control_report["controlled_trace"]["backend_input_sha256"],
        )
        self.assertEqual(pair["ordered_pair_identity"], expected)
        self.assertEqual(pair["signed_execution_ledger"], target["case_record"]["reference_ledger"])

        for label, field in (
            ("transaction envelope", "transaction_envelope_sha256"),
            ("access list", "access_list_sha256"),
            ("prestate", "prestate_sha256"),
        ):
            changed = copy.deepcopy(control_report)
            changed["controlled_trace"][field] = "00" * 32
            with self.subTest(label=label), self.assertRaisesRegex(ValueError, label):
                stateful.admit_stateful_pair(
                    self.manifest, target, [target_report], control, [changed], repeat_index=1
                )

        with self.assertRaisesRegex(ValueError, "target lane"):
            stateful.admit_stateful_pair(
                self.manifest, control, [control_report], target, [target_report], repeat_index=1
            )

        other_control = self.fixture(scenario="sstore_restore_zero", lane="control")
        with self.assertRaisesRegex(ValueError, "same pair"):
            stateful.admit_stateful_pair(
                self.manifest,
                target,
                [target_report],
                other_control,
                [_synthetic_report(other_control, "other")],
                repeat_index=1,
            )

        with self.assertRaisesRegex(ValueError, "backend-input identity"):
            stateful.admit_stateful_pair(
                self.manifest,
                target,
                [control_report],
                control,
                [target_report],
                repeat_index=1,
            )

        same_hash_control = copy.deepcopy(control_report)
        target_hash = target_report["controlled_trace"]["backend_input_sha256"]
        same_hash_control["guest_input_sha256"] = f"0x{target_hash}"
        same_hash_control["controlled_trace"]["backend_input_sha256"] = target_hash
        same_hash_control["controlled_trace"]["semantic_check"][
            "backend_input_sha256"
        ] = target_hash
        with self.assertRaisesRegex(ValueError, "distinct backend-input hashes"):
            stateful.admit_stateful_pair(
                self.manifest,
                target,
                [target_report],
                control,
                [same_hash_control],
                repeat_index=1,
            )

    def test_pair_admission_rejects_layout_reference_and_signed_ledger_drift(self):
        target = self.fixture(lane="target")
        control = self.fixture(lane="control")
        target_report = _synthetic_report(target, "target")
        control_report = _synthetic_report(control, "control")

        for label, mutate in (
            ("inactive-slot layout", lambda f: f["guest_input"].__setitem__("bytecode", "0x00")),
            ("reference ledger", lambda f: f["case_record"]["reference_ledger"].__setitem__("opcode:0x50", -1)),
        ):
            changed = copy.deepcopy(control)
            mutate(changed)
            with self.subTest(label=label), self.assertRaises(ValueError):
                stateful.admit_stateful_pair(
                    self.manifest,
                    target,
                    [target_report],
                    changed,
                    [control_report],
                    repeat_index=0,
                )

        changed_report = copy.deepcopy(target_report)
        changed_report["controlled_trace"]["executed_opcode_counts"]["opcode:0x55"] += 1
        with self.assertRaisesRegex(ValueError, "opcode ledger"):
            stateful.admit_stateful_pair(
                self.manifest,
                target,
                [changed_report],
                control,
                [control_report],
                repeat_index=0,
            )

    def test_every_frozen_scenario_and_count_has_one_runnable_ordered_pair(self):
        admitted = 0
        for scenario in self.manifest.scenarios:
            for count in scenario.counts:
                target = self.fixture(scenario=scenario.name, lane="target", count=count)
                control = self.fixture(scenario=scenario.name, lane="control", count=count)
                pair = stateful.admit_stateful_pair(
                    self.manifest,
                    target,
                    [_synthetic_report(target, "target")],
                    control,
                    [_synthetic_report(control, "control")],
                    repeat_index=0,
                )
                self.assertEqual(pair["relation_count"], count)
                admitted += 1

        self.assertEqual(admitted, 188)


if __name__ == "__main__":
    unittest.main()
