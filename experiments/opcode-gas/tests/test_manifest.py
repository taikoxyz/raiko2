import pathlib
import sys
import tempfile
import tomllib
import unittest
from copy import deepcopy
from decimal import Decimal

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


def fixture_schedule():
    return opcode_gas.UnzenSchedule(
        opcode_multipliers={opcode: 1 for opcode in opcode_gas.UZEN_OPCODE_NAMES},
        precompile_multipliers={address: 1 for address in opcode_gas.UZEN_PRECOMPILE_NAMES},
    )


def controlled_manifest_data():
    data = {
        "name": "controlled",
        "backend": "sp1",
        "variants": [0, 1, 2, 4],
        "normalization_reference_key": "opcode:0x01",
        "system_operation_ownership": "block_base",
        "anchor_operation_ownership": "block_base",
        "opcode_relation_anchors": list(opcode_gas.OPCODE_RELATION_ANCHORS),
        "dynamic_raw_gas_keys": list(opcode_gas.DYNAMIC_RAW_GAS_KEYS),
        "q_formula": list(opcode_gas.Q_FORMULA),
        "bridge_key_ids": [
            "opcode:0x01",
            "opcode:0x02",
            "precompile:0x04",
            *opcode_gas.Q_FORMULA,
        ],
        "bridge_model": "through_origin_equal_key_median",
        "bridge_controlled_max_ape": "0.10",
        "bridge_proposal_max_ape": "0.10",
        "cases": [
            {
                "name": "add",
                "kind": "opcode",
                "opcode": "0x01",
                "scenario": "arithmetic",
                "execution_basis": "interpreter_raw_gas",
                "template": "fixed_runtime_loop",
                "target_raw_gas": 3,
            },
            {
                "name": "mul",
                "kind": "opcode",
                "opcode": "0x02",
                "scenario": "arithmetic",
                "execution_basis": "interpreter_raw_gas",
                "template": "fixed_runtime_loop",
                "target_raw_gas": 5,
            },
            {
                "name": "identity",
                "kind": "precompile",
                "address": "0x04",
                "scenario": "precompile",
                "execution_basis": "native_gas",
                "template": "precompile_fixed_32",
                "input_size": 32,
                "target_raw_gas": 18,
                "paired": True,
                "expected_output_size": 32,
            },
        ],
        "measurement_keys": [
            {
                "id": "opcode:0x01",
                "production_schedule_key": "opcode:0x01",
                "event_match": {"component": "opcode", "opcode": "0x01"},
                "pricing_basis": "raw_gas_slope",
                "required_case_ids": ["add"],
                "diagnostic_case_ids": [],
            },
            {
                "id": "opcode:0x02",
                "production_schedule_key": "opcode:0x02",
                "event_match": {"component": "opcode", "opcode": "0x02"},
                "pricing_basis": "raw_gas_slope",
                "required_case_ids": ["mul"],
                "diagnostic_case_ids": [],
            },
            {
                "id": "precompile:0x04",
                "production_schedule_key": "precompile:0x04",
                "event_match": {"component": "precompile", "address": "0x04"},
                "pricing_basis": "raw_gas_slope",
                "required_case_ids": ["identity"],
                "diagnostic_case_ids": [],
            },
        ],
        "overhead_keys": [
            {
                "id": "tx_base",
                "unit": "started_non_anchor_transaction",
                "formula_role": "required",
                "subtract_keys": [],
                "bundled_keys": [],
                "bundled_ratios": {},
                "required_case_ids": ["tx_base_no_code_no_value"],
                "diagnostic_case_ids": [],
            },
            {
                "id": "native_value_transfer",
                "unit": "native_value_transfer",
                "formula_role": "required",
                "subtract_keys": ["tx_base"],
                "bundled_keys": [],
                "bundled_ratios": {},
                "required_case_ids": ["native_transfer_positive_vs_zero"],
                "diagnostic_case_ids": [],
            },
            {
                "id": "block_base",
                "unit": "block",
                "formula_role": "required",
                "subtract_keys": ["tx_base", "native_value_transfer"],
                "bundled_keys": [],
                "bundled_ratios": {},
                "required_case_ids": ["block_base_one_vs_two_minimal_blocks"],
                "diagnostic_case_ids": [],
            },
            {
                "id": "proposal_startup",
                "unit": "proposal",
                "formula_role": "required",
                "subtract_keys": ["block_base"],
                "bundled_keys": [],
                "bundled_ratios": {},
                "required_case_ids": [
                    "startup_minimal_no_candidate_tx",
                    "startup_minimal_one_no_code_tx",
                ],
                "diagnostic_case_ids": [],
            },
            {
                "id": "witness_byte",
                "unit": "witness_byte",
                "formula_role": "diagnostic",
                "subtract_keys": [],
                "bundled_keys": [],
                "bundled_ratios": {},
                "required_case_ids": [],
                "diagnostic_case_ids": ["witness_bytes"],
            },
        ],
        "overhead_cases": [
            {
                "name": "tx_base_no_code_no_value",
                "overhead_key_id": "tx_base",
                "target_template": "tx_target",
                "control_template": "tx_control",
                "expected_changed_feature_keys": ["tx_base"],
            },
            {
                "name": "native_transfer_positive_vs_zero",
                "overhead_key_id": "native_value_transfer",
                "target_template": "transfer_target",
                "control_template": "transfer_control",
                "expected_changed_feature_keys": ["native_value_transfer"],
            },
            {
                "name": "block_base_one_vs_two_minimal_blocks",
                "overhead_key_id": "block_base",
                "target_template": "block_target",
                "control_template": "block_control",
                "expected_changed_feature_keys": ["block_base"],
            },
            {
                "name": "startup_minimal_no_candidate_tx",
                "overhead_key_id": "proposal_startup",
                "target_template": "startup_empty",
                "control_template": "startup_control",
                "expected_changed_feature_keys": [
                    "block_base",
                    "proposal_startup",
                ],
            },
            {
                "name": "startup_minimal_one_no_code_tx",
                "overhead_key_id": "proposal_startup",
                "target_template": "startup_one_tx",
                "control_template": "startup_control",
                "expected_changed_feature_keys": [
                    "block_base",
                    "proposal_startup",
                    "tx_base",
                ],
            },
            {
                "name": "witness_bytes",
                "overhead_key_id": "witness_byte",
                "target_template": "witness_target",
                "control_template": "witness_control",
                "expected_changed_feature_keys": ["witness_byte"],
            },
        ],
    }
    frozen = tomllib.loads(
        (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        ).read_text()
    )
    data["block_calibration_rows"] = frozen["block_calibration_rows"]
    data["static_count_control_rows"] = frozen["static_count_control_rows"]
    return data


CONTROLLED_SCHEDULE_KEYS = {"opcode:0x01", "opcode:0x02", "precompile:0x04"}


class ManifestTests(unittest.TestCase):
    def test_v1_requires_exact_static_control_collection_and_row_keys(self):
        path = (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        )
        data = tomllib.loads(path.read_text())
        schedule = fixture_schedule()
        schedule_keys = {
            *(f"opcode:0x{opcode:02x}" for opcode in schedule.opcode_multipliers),
            *(f"precompile:0x{address:02x}" for address in schedule.precompile_multipliers),
        }

        missing_controls = deepcopy(data)
        del missing_controls["static_count_control_rows"]
        with self.assertRaisesRegex(ValueError, "exactly six static-count controls"):
            opcode_gas.parse_controlled_manifest(
                missing_controls, schedule_keys=schedule_keys
            )

        for field_name in ("block_calibration_rows", "static_count_control_rows"):
            with self.subTest(field_name=field_name):
                tampered = deepcopy(data)
                tampered[field_name][0]["unexpected"] = 1
                with self.assertRaisesRegex(ValueError, "unexpected fields"):
                    opcode_gas.parse_controlled_manifest(
                        tampered,
                        schedule_keys=schedule_keys,
                    )

    def test_v1_materializes_exact_block_calibration_inventory(self):
        path = (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        )
        manifest = opcode_gas.load_manifest(path, schedule=fixture_schedule())
        rows = manifest.block_calibration_rows
        families = {
            "pop_family",
            "push_family",
            "dup_family",
            "swap_family",
            "proposal_startup",
            "block_base",
            "tx_base",
            "native_value_transfer",
        }
        diagnostics = {
            "guest_input_bincode_length",
            "witness_node_count",
            "witness_byte_count",
            "blob_count",
            "kzg_invocation_count",
            "calldata_length",
            "bytecode_length",
            "touched_state_key_count",
        }

        self.assertEqual(len(rows), 48)
        self.assertEqual({row.workload_family for row in rows}, families)
        self.assertEqual(len({row.row_id for row in rows}), 48)
        controls = manifest.static_count_control_rows
        self.assertEqual(len(controls), 6)
        self.assertEqual(
            [row.program.count for row in controls], [1, 2, 4, 8, 16, 32]
        )
        self.assertTrue(
            all(
                row.workload_family == "static_count_control"
                and row.split == "diagnostic"
                and row.block_count == 1
                and row.transaction_count == 1
                and row.program.kind == "opcode_loop"
                and row.program.family == "static_count_control"
                and row.program.scenario == "push3_pop_fixed_pop"
                for row in controls
            )
        )
        for family in families:
            family_rows = [row for row in rows if row.workload_family == family]
            self.assertEqual(
                [row.split for row in family_rows],
                ["fit", "fit", "fit", "fit", "fit", "holdout"],
            )
            self.assertEqual(
                {key for row in family_rows for key in row.expected_diagnostics},
                diagnostics,
            )
            self.assertTrue(
                all(set(row.expected_diagnostics) == diagnostics for row in family_rows)
            )

        for family in (
            "pop_family",
            "push_family",
            "dup_family",
            "swap_family",
            "block_base",
            "tx_base",
            "native_value_transfer",
        ):
            fit_rows = [
                row
                for row in rows
                if row.workload_family == family and row.split == "fit"
            ]
            counts = [
                row.program.count
                if row.program.kind == "opcode_loop"
                else row.block_count
                if family == "block_base"
                else row.transaction_count
                for row in fit_rows
            ]
            self.assertEqual(counts, [1, 2, 4, 8, 16])

        startup = [row for row in rows if row.workload_family == "proposal_startup"]
        self.assertEqual(len({row.row_id for row in startup}), 6)
        self.assertTrue(all(row.expected_features["proposal_startup"] == 1 for row in rows))

    def test_v1_freezes_formal_relation_anchors_and_dynamic_scenario_matrix(self):
        path = (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        )

        manifest = opcode_gas.load_manifest(path, schedule=fixture_schedule())

        self.assertEqual(manifest.opcode_relation_anchors, opcode_gas.OPCODE_RELATION_ANCHORS)
        self.assertEqual(manifest.dynamic_raw_gas_keys, opcode_gas.DYNAMIC_RAW_GAS_KEYS)
        scenarios = {}
        for relation in manifest.opcode_relations:
            if relation.dynamic_key is not None:
                scenarios.setdefault(relation.dynamic_key, []).append(relation)
        self.assertEqual(set(scenarios), set(opcode_gas.DYNAMIC_RAW_GAS_KEYS))
        expected = {
            "opcode:0x0a": [
                ("canonical", "fit", {"exponent_byte_length": 1, "initial_memory_words": 0}),
                ("dynamic_holdout", "fit", {"exponent_byte_length": 4, "initial_memory_words": 0}),
                ("dynamic_holdout", "fit", {"exponent_byte_length": 8, "initial_memory_words": 0}),
                ("dynamic_holdout", "fit", {"exponent_byte_length": 16, "initial_memory_words": 0}),
                ("dynamic_holdout", "fit", {"exponent_byte_length": 32, "initial_memory_words": 0}),
                ("dynamic_holdout", "holdout", {"exponent_byte_length": 2, "initial_memory_words": 0}),
                ("dynamic_holdout", "holdout", {"exponent_byte_length": 24, "initial_memory_words": 0}),
            ],
            "opcode:0x20": [
                ("canonical", "fit", {"input_length": 32, "initial_memory_words": 1}),
                ("dynamic_holdout", "fit", {"input_length": 256, "initial_memory_words": 8}),
                ("dynamic_holdout", "fit", {"input_length": 1024, "initial_memory_words": 32}),
                ("dynamic_holdout", "fit", {"input_length": 256, "initial_memory_words": 1}),
                ("dynamic_holdout", "fit", {"input_length": 1024, "initial_memory_words": 1}),
                ("dynamic_holdout", "holdout", {"input_length": 512, "initial_memory_words": 16}),
                ("dynamic_holdout", "holdout", {"input_length": 512, "initial_memory_words": 1}),
            ],
            "opcode:0x5e": [
                ("canonical", "fit", {"copy_length": 32, "initial_memory_words": 1}),
                ("dynamic_holdout", "fit", {"copy_length": 256, "initial_memory_words": 8}),
                ("dynamic_holdout", "fit", {"copy_length": 1024, "initial_memory_words": 32}),
                ("dynamic_holdout", "fit", {"copy_length": 256, "initial_memory_words": 1}),
                ("dynamic_holdout", "fit", {"copy_length": 1024, "initial_memory_words": 1}),
                ("dynamic_holdout", "holdout", {"copy_length": 512, "initial_memory_words": 16}),
                ("dynamic_holdout", "holdout", {"copy_length": 512, "initial_memory_words": 1}),
            ],
        }
        memory_scenarios = [
            ("canonical", "fit", {"highest_touched_offset": 0, "initial_memory_words": 1}),
            ("dynamic_holdout", "fit", {"highest_touched_offset": 256, "initial_memory_words": 1}),
            ("dynamic_holdout", "fit", {"highest_touched_offset": 4096, "initial_memory_words": 1}),
            ("dynamic_holdout", "holdout", {"highest_touched_offset": 256, "initial_memory_words": 9}),
            ("dynamic_holdout", "fit", {"highest_touched_offset": 1024, "initial_memory_words": 1}),
            ("dynamic_holdout", "holdout", {"highest_touched_offset": 4096, "initial_memory_words": 129}),
            ("dynamic_holdout", "holdout", {"highest_touched_offset": 2048, "initial_memory_words": 1}),
            ("dynamic_holdout", "holdout", {"highest_touched_offset": 4064, "initial_memory_words": 1}),
            ("dynamic_holdout", "holdout", {"highest_touched_offset": 8192, "initial_memory_words": 1}),
        ]
        for key in ("opcode:0x51", "opcode:0x52", "opcode:0x53"):
            expected[key] = memory_scenarios

        self.assertEqual(
            {
                key: [
                    (relation.split, relation.model_split, dict(relation.scenario))
                    for relation in relations
                ]
                for key, relations in scenarios.items()
            },
            expected,
        )
        self.assertEqual(
            sum(relation.split == "canonical" for relations in scenarios.values() for relation in relations),
            6,
        )
        self.assertEqual(
            sum(relation.split != "canonical" for relations in scenarios.values() for relation in relations),
            42,
        )

    def test_formal_relation_manifest_rejects_anchor_dynamic_and_relation_contract_drift(self):
        path = (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        )
        base = tomllib.loads(path.read_text())
        schedule_keys = {
            *(f"opcode:0x{opcode:02x}" for opcode in opcode_gas.UZEN_OPCODE_NAMES),
            *(f"precompile:0x{address:02x}" for address in opcode_gas.UZEN_PRECOMPILE_NAMES),
        }

        mutations = []
        reordered = deepcopy(base)
        reordered["opcode_relation_anchors"] = list(reversed(reordered["opcode_relation_anchors"]))
        mutations.append((reordered, "anchors"))
        missing_dynamic = deepcopy(base)
        missing_dynamic["dynamic_raw_gas_keys"].remove("opcode:0x0a")
        mutations.append((missing_dynamic, "dynamic raw-gas"))
        duplicate = deepcopy(base)
        duplicate["opcode_relation_scenarios"].append(
            deepcopy(duplicate["opcode_relation_scenarios"][0])
        )
        mutations.append((duplicate, "duplicate relation ID"))
        wrong_units = deepcopy(base)
        wrong_units["opcode_relation_scenarios"][0]["target_raw_gas"] += 1
        mutations.append((wrong_units, "target raw-gas total"))
        wrong_matrix = deepcopy(base)
        wrong_matrix_row = next(
            row
            for row in wrong_matrix["opcode_relation_scenarios"]
            if row["case_id"] == "keccak256"
            and row["scenario"] == {"input_length": 256, "initial_memory_words": 1}
        )
        wrong_matrix_row["scenario"]["initial_memory_words"] = 2
        wrong_matrix_row["target_raw_gas"] = 96
        mutations.append((wrong_matrix, "frozen scenario matrix"))

        for data, message in mutations:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.parse_controlled_manifest(data, schedule_keys=schedule_keys)

        unmarked = controlled_manifest_data()
        unmarked["cases"][0]["template"] = "stack_exp"
        unmarked["cases"][0]["target_raw_gas"] = 60
        with self.assertRaisesRegex(ValueError, "unmarked dynamic raw-gas template"):
            opcode_gas.parse_controlled_manifest(
                unmarked, schedule_keys=CONTROLLED_SCHEDULE_KEYS
            )

    def test_materialized_v1_enumerates_every_case_key_and_keeps_state_diagnostics_out_of_q(self):
        path = (
            ROOT
            / "experiments"
            / "opcode-gas"
            / "manifests"
            / "sp1-calibration-v1.toml"
        )
        text = path.read_text()
        self.assertNotIn("include_uzen_", text)

        manifest = opcode_gas.load_manifest(path, schedule=fixture_schedule())
        expected_keys = {
            *(f"opcode:0x{opcode:02x}" for opcode in opcode_gas.PLANNED_PURE_OPCODE_OPCODES),
            *(f"precompile:0x{address:02x}" for address in opcode_gas.PRECOMPILE_BODY_DEFAULTS),
        }
        self.assertEqual({key.id for key in manifest.measurement_keys}, expected_keys)
        self.assertEqual(len(manifest.cases), len(expected_keys))
        self.assertEqual(set(manifest.bridge_key_ids), {*expected_keys, *opcode_gas.Q_FORMULA})
        self.assertTrue(
            all(
                case.paired and case.expected_output_size is not None
                for case in manifest.cases
                if case.kind == "precompile"
            )
        )
        diagnostic = {
            key.id: key for key in manifest.overhead_keys if key.formula_role == "diagnostic"
        }
        self.assertGreaterEqual(
            set(diagnostic),
            {
                "witness_byte",
                "witness_node",
                "stdin_byte",
                "blob_byte",
                "kzg_invocation",
                "unique_state_access",
                "dirty_state_entry",
            },
        )
        self.assertTrue(set(diagnostic).isdisjoint(manifest.q_formula))
        self.assertTrue(set(diagnostic).isdisjoint(manifest.bridge_key_ids))
        self.assertNotIn("trie", {key.id for key in manifest.overhead_keys})
        self.assertNotIn("merkle", {key.id for key in manifest.overhead_keys})
        self.assertEqual(manifest.system_operation_ownership, "block_base")
        self.assertEqual(manifest.anchor_operation_ownership, "block_base")
        tx_base = next(key for key in manifest.overhead_keys if key.id == "tx_base")
        self.assertEqual(tx_base.unit, "started_non_anchor_transaction")
        self.assertEqual(
            tx_base.required_case_ids,
            ("tx_base_no_code_no_value", "tx_base_minimal_contract_call"),
        )
        changed_features = {
            case.name: case.expected_changed_feature_keys
            for case in manifest.overhead_cases
        }
        self.assertEqual(
            changed_features["native_transfer_positive_vs_zero"],
            ("native_value_transfer",),
        )
        self.assertEqual(
            changed_features["block_base_one_vs_two_minimal_blocks"],
            ("block_base",),
        )
        self.assertEqual(
            changed_features["startup_minimal_no_candidate_tx"],
            ("proposal_startup", "block_base"),
        )
        self.assertEqual(
            changed_features["startup_minimal_one_no_code_tx"],
            ("proposal_startup", "block_base", "tx_base"),
        )

    def test_v1_rejects_any_system_operation_ownership_except_block_base(self):
        data = controlled_manifest_data()
        data["system_operation_ownership"] = "execution_terms"
        with self.assertRaisesRegex(ValueError, "system operation ownership"):
            opcode_gas.parse_controlled_manifest(
                data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
            )
    def test_controlled_manifest_rows_hash_binds_all_candidate_boundary_fields(self):
        base = """
name = "controlled"
backend = "sp1"
variants = [0, 1, 2, 4]
normalization_reference_key = "opcode:0x01"
system_operation_ownership = "block_base"
anchor_operation_ownership = "block_base"
q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]
bridge_key_ids = ["opcode:0x01", "opcode:0x02", "precompile:0x04", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]
bridge_model = "through_origin_equal_key_median"
bridge_controlled_max_ape = "0.10"
bridge_proposal_max_ape = "0.10"

[[measurement_keys]]
id = "opcode:0x01"
production_schedule_key = "opcode:0x01"
pricing_basis = "raw_gas_slope"
required_case_ids = ["add"]
diagnostic_case_ids = []
event_match = { component = "opcode", opcode = "0x01" }

[[cases]]
name = "add"
kind = "opcode"
opcode = "0x01"
scenario = "arithmetic"
execution_basis = "interpreter_raw_gas"
template = "fixed_runtime_loop"
target_raw_gas = 3
"""
        with tempfile.TemporaryDirectory() as tmp:
            first = pathlib.Path(tmp) / "first.toml"
            second = pathlib.Path(tmp) / "second.toml"
            first.write_text(base)
            second.write_text(base.replace('pricing_basis = "raw_gas_slope"', 'pricing_basis = "fixed_per_event"'))
            self.assertNotEqual(
                opcode_gas.controlled_manifest_rows_sha256(first),
                opcode_gas.controlled_manifest_rows_sha256(second),
            )

    def test_load_manifest_parses_smoke_cases(self):
        manifest = opcode_gas.load_manifest(
            ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-smoke.toml",
            schedule=fixture_schedule(),
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

    def test_pure_opcode_defaults_do_not_overlap_excluded_categories(self):
        excluded = (
            opcode_gas.STATE_OR_REVM_OPCODES
            | opcode_gas.SPAWN_WRAPPER_OPCODES
            | opcode_gas.ZERO_OR_HALTING_OPCODES
        )

        self.assertFalse(opcode_gas.PLANNED_PURE_OPCODE_OPCODES & excluded)

    def test_unknown_default_entries_raise_value_error(self):
        with self.assertRaisesRegex(ValueError, "no pure opcode default"):
            opcode_gas.default_opcode_case(0xAA)
        with self.assertRaisesRegex(ValueError, "no precompile body default"):
            opcode_gas.default_precompile_case(0x101)

    def test_controlled_manifest_parses_typed_contract(self):
        manifest = opcode_gas.parse_controlled_manifest(
            controlled_manifest_data(), schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )

        self.assertEqual(manifest.normalization_reference_key, "opcode:0x01")
        self.assertEqual(manifest.measurement_keys[0].event_match.opcode, 0x01)
        self.assertEqual(manifest.bridge_controlled_max_ape, Decimal("0.10"))
        self.assertEqual(
            manifest.subtract_closure["proposal_startup"],
            ("block_base", "native_value_transfer", "tx_base"),
        )

    def test_rejects_duplicate_missing_or_cross_list_case_ids(self):
        fixtures = []
        duplicate_key = controlled_manifest_data()
        duplicate_key["measurement_keys"].append(deepcopy(duplicate_key["measurement_keys"][0]))
        fixtures.append((duplicate_key, "duplicate measurement key"))
        missing_schedule = controlled_manifest_data()
        missing_schedule["measurement_keys"][0]["production_schedule_key"] = "opcode:0x03"
        fixtures.append((missing_schedule, "production schedule key"))
        empty_required = controlled_manifest_data()
        empty_required["measurement_keys"][0]["required_case_ids"] = []
        fixtures.append((empty_required, "required_case_ids"))
        unknown_case = controlled_manifest_data()
        unknown_case["measurement_keys"][0]["required_case_ids"] = ["missing"]
        fixtures.append((unknown_case, "unknown case"))
        duplicate_case = controlled_manifest_data()
        duplicate_case["measurement_keys"][0]["required_case_ids"] = ["add", "add"]
        fixtures.append((duplicate_case, "duplicate case"))
        cross_list = controlled_manifest_data()
        cross_list["measurement_keys"][0]["diagnostic_case_ids"] = ["add"]
        fixtures.append((cross_list, "required and diagnostic"))

        for fixture, message in fixtures:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.parse_controlled_manifest(
                    fixture, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )

    def test_rejects_unsupported_overlapping_or_invalid_spawn_matches(self):
        unsupported = controlled_manifest_data()
        unsupported["measurement_keys"][0]["event_match"]["warm"] = True
        with self.assertRaisesRegex(ValueError, "unsupported event-match field"):
            opcode_gas.parse_controlled_manifest(
                unsupported, schedule_keys=CONTROLLED_SCHEDULE_KEYS
            )

        non_call_spawn = controlled_manifest_data()
        non_call_spawn["measurement_keys"][0]["event_match"]["spawned"] = False
        with self.assertRaisesRegex(ValueError, "spawned.*CALL/CREATE"):
            opcode_gas.parse_controlled_manifest(
                non_call_spawn, schedule_keys=CONTROLLED_SCHEDULE_KEYS
            )

        call = controlled_manifest_data()
        call["cases"].extend(
            [
                {
                    "name": "call_plain",
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "scenario": "ignored",
                    "execution_basis": "interpreter_raw_gas",
                    "template": "fixed_runtime_loop",
                    "target_raw_gas": 100,
                    "spawned": False,
                }
            ]
        )
        call["measurement_keys"].append(
            {
                "id": "opcode:0xf1:generic",
                "production_schedule_key": "opcode:0xf1",
                "event_match": {"component": "opcode", "opcode": "0xf1"},
                "pricing_basis": "raw_gas_slope",
                "required_case_ids": ["call_plain"],
                "diagnostic_case_ids": [],
            }
        )
        with self.assertRaisesRegex(ValueError, "CALL/CREATE.*spawned"):
            opcode_gas.parse_controlled_manifest(
                call, schedule_keys=CONTROLLED_SCHEDULE_KEYS | {"opcode:0xf1"}
            )

        call["measurement_keys"][-1]["event_match"]["spawned"] = False
        parsed = opcode_gas.parse_controlled_manifest(
            call, schedule_keys=CONTROLLED_SCHEDULE_KEYS | {"opcode:0xf1"}
        )
        spawned_false = parsed.measurement_keys[-1].event_match
        spawned_true = opcode_gas.EventMatchSpec(
            component="opcode",
            opcode=0xF1,
            spawned=True,
            dispatch_status="confirmed",
        )
        self.assertTrue(
            opcode_gas.event_matches(
                spawned_false,
                {
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                },
            )
        )
        self.assertFalse(
            opcode_gas.event_matches(
                spawned_true,
                {
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "spawned": False,
                    "dispatch_status": "not_applicable",
                    "pricing_basis": "raw_gas_slope",
                },
            )
        )

    def test_rejects_basis_and_three_way_identity_mismatches(self):
        mutations = [
            (lambda data: data["measurement_keys"][0].update(pricing_basis="missing"), "pricing basis"),
            (lambda data: data["measurement_keys"][0].update(pricing_basis="fixed_per_event"), "fixed_per_event"),
            (lambda data: data["cases"][0].update(opcode="0x02"), "case opcode"),
            (lambda data: data["cases"][2].update(address="0x02"), "case precompile address"),
            (lambda data: data["cases"][0].update(kind="precompile", address="0x01"), "case component"),
            (lambda data: data["measurement_keys"][0].update(production_schedule_key="opcode:0x02"), "production schedule key"),
            (lambda data: data["cases"][0].update(execution_basis="native_gas"), "execution basis"),
        ]
        for mutate, message in mutations:
            fixture = controlled_manifest_data()
            mutate(fixture)
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.parse_controlled_manifest(
                    fixture, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )

        scenario_only = controlled_manifest_data()
        scenario_only["cases"][0]["scenario"] = "precompile CALL spawned anything"
        parsed = opcode_gas.parse_controlled_manifest(
            scenario_only, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )
        self.assertEqual(parsed.cases[0].opcode, 0x01)

    def test_rejects_bad_overhead_graph_and_bridge_contract(self):
        fixtures = []
        cycle = controlled_manifest_data()
        cycle["overhead_keys"][0]["subtract_keys"] = ["proposal_startup"]
        fixtures.append((cycle, "cycle"))
        missing = controlled_manifest_data()
        missing["overhead_keys"][0]["subtract_keys"] = ["missing"]
        fixtures.append((missing, "missing dependency"))
        undeclared = controlled_manifest_data()
        undeclared["overhead_cases"][0]["expected_changed_feature_keys"] = ["tx_base", "blob_byte"]
        fixtures.append((undeclared, "changed feature"))
        bad_required = controlled_manifest_data()
        bad_required["overhead_keys"][0]["formula_role"] = "diagnostic"
        fixtures.append((bad_required, "four frozen"))
        empty_bridge = controlled_manifest_data()
        empty_bridge["bridge_key_ids"] = []
        fixtures.append((empty_bridge, "bridge_key_ids"))
        duplicate_bridge = controlled_manifest_data()
        duplicate_bridge["bridge_key_ids"].append("opcode:0x01")
        fixtures.append((duplicate_bridge, "duplicate bridge"))
        diagnostic_bridge = controlled_manifest_data()
        diagnostic_bridge["bridge_key_ids"].append("witness_byte")
        fixtures.append((diagnostic_bridge, "diagnostic-only"))
        missing_add = controlled_manifest_data()
        missing_add["bridge_key_ids"].remove("opcode:0x01")
        fixtures.append((missing_add, "normalization"))
        no_extra_opcode = controlled_manifest_data()
        no_extra_opcode["bridge_key_ids"].remove("opcode:0x02")
        fixtures.append((no_extra_opcode, "additional opcode"))
        no_precompile = controlled_manifest_data()
        no_precompile["bridge_key_ids"].remove("precompile:0x04")
        fixtures.append((no_precompile, "precompile"))
        bad_model = controlled_manifest_data()
        bad_model["bridge_model"] = "least_squares"
        fixtures.append((bad_model, "bridge model"))
        bad_threshold = controlled_manifest_data()
        bad_threshold["bridge_controlled_max_ape"] = "0.1001"
        fixtures.append((bad_threshold, "exactly 0.10"))

        for fixture, message in fixtures:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                opcode_gas.parse_controlled_manifest(
                    fixture, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )

    def test_spawned_and_nonspawned_call_matches_are_disjoint_and_dispatch_bound(self):
        data = controlled_manifest_data()
        data["cases"].extend(
            [
                {
                    "name": "call_plain",
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "scenario": "descriptive only",
                    "execution_basis": "interpreter_raw_gas",
                    "template": "fixed_runtime_loop",
                    "target_raw_gas": 100,
                    "spawned": False,
                },
                {
                    "name": "call_spawned",
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "scenario": "descriptive only",
                    "execution_basis": "fixed_per_event",
                    "template": "paired_spawn",
                    "spawned": True,
                    "dispatch_status": "confirmed",
                },
            ]
        )
        data["measurement_keys"].extend(
            [
                {
                    "id": "opcode:0xf1:not_spawned",
                    "production_schedule_key": "opcode:0xf1",
                    "event_match": {
                        "component": "opcode",
                        "opcode": "0xf1",
                        "spawned": False,
                    },
                    "pricing_basis": "raw_gas_slope",
                    "required_case_ids": ["call_plain"],
                    "diagnostic_case_ids": [],
                },
                {
                    "id": "opcode:0xf1:spawned",
                    "production_schedule_key": "opcode:0xf1",
                    "event_match": {
                        "component": "opcode",
                        "opcode": "0xf1",
                        "spawned": True,
                        "dispatch_status": "confirmed",
                    },
                    "pricing_basis": "fixed_per_event",
                    "required_case_ids": ["call_spawned"],
                    "diagnostic_case_ids": [],
                },
            ]
        )
        parsed = opcode_gas.parse_controlled_manifest(
            data, schedule_keys=CONTROLLED_SCHEDULE_KEYS | {"opcode:0xf1"}
        )
        self.assertFalse(
            opcode_gas.event_matches(
                parsed.measurement_keys[-2].event_match,
                {
                    "kind": "opcode",
                    "opcode": "0xf1",
                    "spawned": True,
                    "dispatch_status": "confirmed",
                },
            )
        )

        data["cases"][-1]["target_raw_gas"] = 1
        with self.assertRaisesRegex(ValueError, "fixed-event.*raw-gas"):
            opcode_gas.parse_controlled_manifest(
                data, schedule_keys=CONTROLLED_SCHEDULE_KEYS | {"opcode:0xf1"}
            )

        del data["cases"][-1]["target_raw_gas"]
        data["measurement_keys"][-1]["event_match"]["dispatch_status"] = (
            "selected_not_dispatched"
        )
        with self.assertRaisesRegex(ValueError, "dispatch_status=confirmed"):
            opcode_gas.parse_controlled_manifest(
                data, schedule_keys=CONTROLLED_SCHEDULE_KEYS | {"opcode:0xf1"}
            )

    def test_bundled_ratios_require_exact_positive_canonical_ownership(self):
        accepted = controlled_manifest_data()
        accepted["overhead_keys"][2]["bundled_keys"] = ["witness_byte"]
        accepted["overhead_keys"][2]["bundled_ratios"] = {
            "witness_byte": {"numerator": 1, "denominator": 32}
        }
        accepted["overhead_cases"][2]["expected_changed_feature_keys"].append(
            "witness_byte"
        )
        opcode_gas.parse_controlled_manifest(
            accepted, schedule_keys=CONTROLLED_SCHEDULE_KEYS
        )

        for ratio, message in [
            ({}, "bundled ratios"),
            ({"witness_byte": {"numerator": 0, "denominator": 32}}, "positive"),
            ({"witness_byte": {"numerator": "01", "denominator": 32}}, "canonical"),
        ]:
            data = controlled_manifest_data()
            data["overhead_keys"][2]["bundled_keys"] = ["witness_byte"]
            data["overhead_keys"][2]["bundled_ratios"] = ratio
            data["overhead_cases"][2]["expected_changed_feature_keys"].append(
                "witness_byte"
            )
            with self.subTest(ratio=ratio), self.assertRaisesRegex(ValueError, message):
                opcode_gas.parse_controlled_manifest(
                    data, schedule_keys=CONTROLLED_SCHEDULE_KEYS
                )


if __name__ == "__main__":
    unittest.main()
