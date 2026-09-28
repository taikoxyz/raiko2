import copy
import json
import pathlib
import shutil
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


CORE_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "f945e67bb2c38c9c8ef50530"
    / "core-opcode-submodel.json"
)
CORE_REF = CORE_PATH.relative_to(ROOT).as_posix()
MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v4.json"
)
V3_MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v3.json"
)
V2_MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v2.json"
)
V1_MANIFEST_PATH = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v1.json"
)
V1_ARTIFACT_SHA256 = (
    "fbb4817d50b04147d0d9c86a25c82324d5e36ce9d6acbf49c51dd165cf1905a3"
)
V1_FILE_SHA256 = (
    "c5e5a7c28bb2640249f70df2298f6e8a4ee5d46b3bfe2d204c4cb6a86bf1d9be"
)
V2_ARTIFACT_SHA256 = (
    "35fd25a7878dd407522ab676c8a9965c663c6885e3190df0fd51f6c497b84e77"
)
V2_FILE_SHA256 = (
    "75fec3c4307c59539cc6180e6511dd7fc1111197746c6b0abd9a872902c665a3"
)
V3_ARTIFACT_SHA256 = (
    "fbe97920148965d065f5d297194b220b70b866f25136cf217e081632d3003520"
)
V3_FILE_SHA256 = (
    "be5cd1621be3502a21536ea6925629c0a7ab417a1c3f4301ab5c82f5edf7c256"
)
V4_ARTIFACT_SHA256 = (
    "2383d9788302f8447522abdcdc99a1f6490cc2b910c376ef9b4e265cc1c978be"
)
V4_FILE_SHA256 = (
    "941170c66baa285c1019592e9e5a215c3461695e8b548f0c5d810ebce06cf7cf"
)


def reseal(manifest):
    manifest.pop("artifact_sha256", None)
    manifest["artifact_sha256"] = opcode_gas.sha256_bytes(
        opcode_gas.canonical_json(manifest)
    )


def transaction_trace(*, started_tx_index, is_anchor=False, disposition="attempted"):
    row = {
        "recovered_index": 1,
        "manifest_index": 0,
        "tx_hash": "0x" + "11" * 32,
        "is_anchor": is_anchor,
        "disposition": disposition,
        "native_value_transfer": False,
    }
    if started_tx_index is not None:
        row["started_tx_index"] = started_tx_index
    return row


def operation_trace(component, *, phase="transaction", tx_index=0, frame_depth=0):
    row = {
        "operation_id": 0,
        "phase": phase,
        "frame_depth": frame_depth,
        "component": component,
    }
    if tx_index is not None:
        row["tx_index"] = tx_index
    return row


class OperationCoverageTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schedule = opcode_gas.load_current_uzen_schedule()
        cls.core = json.loads(CORE_PATH.read_text())

    def build(self):
        return opcode_gas.build_operation_coverage_manifest(
            schedule=self.schedule,
            augmented_core=self.core,
            augmented_core_ref=CORE_REF,
        )

    def assert_rejected(self, mutate, expected):
        manifest = self.build()
        mutate(manifest)
        reseal(manifest)
        with self.assertRaisesRegex(ValueError, expected):
            opcode_gas.validate_operation_coverage_manifest(
                manifest,
                schedule=self.schedule,
                augmented_core=self.core,
                augmented_core_ref=CORE_REF,
            )

    def test_builder_classifies_every_active_named_execution_exactly_once(self):
        manifest = self.build()

        opcode_gas.validate_operation_coverage_manifest(
            manifest,
            schedule=self.schedule,
            augmented_core=self.core,
            augmented_core_ref=CORE_REF,
        )
        rows = manifest["execution_coverage"]
        self.assertEqual(len(rows), 168)
        self.assertEqual(len({row["key"] for row in rows}), 168)
        self.assertEqual(
            {row["classification"] for row in rows},
            {
                "static_raw_gas",
                "structured_opcode",
                "direct_precompile",
                "explicitly_unsupported",
            },
        )
        counts = {
            classification: sum(
                row["classification"] == classification for row in rows
            )
            for classification in {
                "static_raw_gas",
                "structured_opcode",
                "direct_precompile",
                "explicitly_unsupported",
            }
        }
        self.assertEqual(
            counts,
            {
                "static_raw_gas": 97,
                "structured_opcode": 6,
                "direct_precompile": 18,
                "explicitly_unsupported": 47,
            },
        )

    def test_sstore_and_call_keep_execution_separate_from_owned_side_effects(self):
        manifest = self.build()
        executions = {row["key"]: row for row in manifest["execution_coverage"]}
        events = {
            row["event_id"]: row for row in manifest["side_effect_ownership"]
        }

        for key in ("opcode:0x55", "opcode:0xf1"):
            self.assertEqual(executions[key]["classification"], "explicitly_unsupported")
            self.assertEqual(executions[key]["model_status"], "unsupported")
            self.assertNotIn("owner", executions[key])
        self.assertEqual(
            executions["opcode:0xf1"]["spawn_trace_semantics"],
            {
                "confirmed": "wrapper_substitutes_opcode_raw_gas",
                "selected_not_dispatched": "diagnostic_no_charge",
            },
        )
        self.assertEqual(events["storage_access"]["owner"], "state_trie")
        self.assertEqual(events["dirty_state_update"]["owner"], "state_trie")
        self.assertEqual(events["final_trie_update"]["owner"], "state_trie")
        self.assertEqual(
            events["confirmed_spawn_wrapper"]["owner"], "operation_wrapper"
        )
        self.assertEqual(events["child_execution"]["owner"], "operation_wrapper")
        self.assertEqual(
            events["child_execution"]["observation_status"], "derived_grouping"
        )
        self.assertTrue(events["child_execution"]["covered_by_execution_coverage"])
        self.assertEqual(events["child_execution"]["extra_charge"], 0)
        self.assertEqual(
            events["confirmed_spawn_wrapper"]["observation_status"], "emitted_trace"
        )
        self.assertEqual(
            events["confirmed_spawn_wrapper"]["cost_treatment"],
            "substitutes_spawn_opcode_raw_gas",
        )
        self.assertEqual(
            events["dirty_state_update"]["observation_status"],
            "declared_not_emitted",
        )

    def test_rejects_unknown_duplicate_and_missing_execution_keys(self):
        def unknown(manifest):
            row = copy.deepcopy(manifest["execution_coverage"][0])
            row["key"] = "opcode:0xaa"
            manifest["execution_coverage"].append(row)

        self.assert_rejected(unknown, "unknown execution key")
        self.assert_rejected(
            lambda manifest: manifest["execution_coverage"].append(
                copy.deepcopy(manifest["execution_coverage"][0])
            ),
            "duplicate execution key",
        )
        self.assert_rejected(
            lambda manifest: manifest["execution_coverage"].pop(),
            "missing execution keys",
        )

    def test_rejects_unknown_duplicate_and_missing_owned_events(self):
        def unknown(manifest):
            row = copy.deepcopy(manifest["side_effect_ownership"][0])
            row["event_id"] = "unknown_event"
            manifest["side_effect_ownership"].append(row)

        self.assert_rejected(unknown, "unknown side-effect event")
        self.assert_rejected(
            lambda manifest: manifest["side_effect_ownership"].append(
                copy.deepcopy(manifest["side_effect_ownership"][0])
            ),
            "duplicate side-effect event",
        )
        self.assert_rejected(
            lambda manifest: manifest["side_effect_ownership"].pop(),
            "missing side-effect events",
        )

    def test_rejects_missing_or_wrong_source_evidence(self):
        self.assert_rejected(
            lambda manifest: manifest["execution_coverage"][0].update(
                source_evidence=[]
            ),
            "missing source evidence",
        )
        self.assert_rejected(
            lambda manifest: manifest["side_effect_ownership"][0].update(
                source_evidence=[]
            ),
            "missing source evidence",
        )

    def test_rejects_measured_execution_without_artifact_reference(self):
        def mutate(manifest):
            row = next(
                row
                for row in manifest["execution_coverage"]
                if row["model_status"] == "measured"
            )
            row.pop("artifact_ref")

        self.assert_rejected(mutate, "measured execution.*artifact")

    def test_rejects_unsupported_execution_without_machine_reason(self):
        def mutate(manifest):
            row = next(
                row
                for row in manifest["execution_coverage"]
                if row["classification"] == "explicitly_unsupported"
            )
            row.pop("reason")

        self.assert_rejected(mutate, "unsupported execution.*reason")

    def test_rejects_whole_sstore_or_call_assigned_to_higher_layer(self):
        for key, owner in (("opcode:0x55", "state_trie"), ("opcode:0xf1", "block")):
            with self.subTest(key=key):
                def mutate(manifest, key=key, owner=owner):
                    row = next(
                        row for row in manifest["execution_coverage"] if row["key"] == key
                    )
                    row["owner"] = owner

                self.assert_rejected(mutate, "opcode execution cannot be assigned")

    def test_rejects_unknown_owner_and_false_emitter_claim(self):
        self.assert_rejected(
            lambda manifest: manifest["side_effect_ownership"][0].update(
                owner="proposal"
            ),
            "unknown side-effect owner",
        )

        def false_emitter(manifest):
            row = next(
                row
                for row in manifest["side_effect_ownership"]
                if row["event_id"] == "dirty_state_update"
            )
            row["observation_status"] = "emitted_trace"

        self.assert_rejected(false_emitter, "side-effect declaration differs")

    def test_rejects_core_model_kind_or_schedule_identity_drift(self):
        def model_kind(manifest):
            row = next(
                row
                for row in manifest["execution_coverage"]
                if row["key"] == "opcode:0x0a"
            )
            row["artifact_ref"]["model_kind"] = "static_raw_gas"

        self.assert_rejected(model_kind, "execution coverage differs")
        self.assert_rejected(
            lambda manifest: manifest["sources"]["exported_unzen_schedule"].update(
                schedule_sha256="0" * 64
            ),
            "schedule source differs",
        )

    def test_builder_rejects_unindexed_extra_core_model(self):
        forged_core = copy.deepcopy(self.core)
        forged_core["registry"]["models"]["opcode:0xaa"] = {
            "kind": "static_raw_gas",
            "parameters": {"body_per_raw_gas": "1"},
        }
        reseal(forged_core)

        with self.assertRaisesRegex(ValueError, "differs from pinned core bytes"):
            opcode_gas.build_operation_coverage_manifest(
                schedule=self.schedule,
                augmented_core=forged_core,
                augmented_core_ref=CORE_REF,
            )

    def test_builder_rejects_resealed_add_coefficient_forgery(self):
        forged_core = copy.deepcopy(self.core)
        forged_core["registry"]["models"]["opcode:0x01"]["parameters"][
            "body_per_raw_gas"
        ] = "999"
        reseal(forged_core)

        with self.assertRaisesRegex(ValueError, "differs from pinned core bytes"):
            opcode_gas.build_operation_coverage_manifest(
                schedule=self.schedule,
                augmented_core=forged_core,
                augmented_core_ref=CORE_REF,
            )

    def test_builder_rejects_resealed_same_cardinality_model_support_swap(self):
        forged_core = copy.deepcopy(self.core)
        add = "opcode:0x01"
        address = "opcode:0x30"
        forged_core["registry"]["models"][address] = forged_core["registry"][
            "models"
        ].pop(add)
        forged_core["registry"]["opcode_model_ids"][0x01] = None
        forged_core["registry"]["opcode_model_ids"][0x30] = address
        forged_core["unsupported_named_opcode_keys"].remove(address)
        forged_core["unsupported_named_opcode_keys"].append(add)
        forged_core["unsupported_named_opcode_keys"].sort()
        forged_core["unsupported_opcode_reasons"].pop(address)
        forged_core["unsupported_opcode_reasons"][add] = "outside_core_opcode_scope"
        reseal(forged_core)

        with self.assertRaisesRegex(ValueError, "differs from pinned core bytes"):
            opcode_gas.build_operation_coverage_manifest(
                schedule=self.schedule,
                augmented_core=forged_core,
                augmented_core_ref=CORE_REF,
            )

    def test_builder_rejects_noncanonical_or_missing_core_reference(self):
        for core_ref in (
            "experiments/opcode-gas/derivations/missing/core-opcode-submodel.json",
            "../core-opcode-submodel.json",
        ):
            with self.subTest(core_ref=core_ref):
                with self.assertRaisesRegex(ValueError, "pinned augmented core reference"):
                    opcode_gas.build_operation_coverage_manifest(
                        schedule=self.schedule,
                        augmented_core=self.core,
                        augmented_core_ref=core_ref,
                    )

    def _copy_pinned_sources(self, temporary_root):
        for relative in (
            *opcode_gas._OPERATION_TRACE_SOURCE_PATHS,
            "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/augmentation.json",
            "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/compatibility-canary.json",
            "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json",
            "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/opcode-supplement.json",
        ):
            source = ROOT / relative
            destination = temporary_root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)

    def test_builder_rejects_pinned_core_disk_byte_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = pathlib.Path(directory)
            self._copy_pinned_sources(temporary_root)
            pinned = temporary_root / CORE_REF
            pinned.write_bytes(pinned.read_bytes() + b"\n")

            with mock.patch.object(opcode_gas, "REPO_ROOT", temporary_root):
                with self.assertRaisesRegex(ValueError, "pinned core file SHA256"):
                    self.build()

    def test_builder_rejects_symlinked_pinned_core(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = pathlib.Path(directory)
            self._copy_pinned_sources(temporary_root)
            pinned = temporary_root / CORE_REF
            real = temporary_root / "real-core.json"
            pinned.rename(real)
            pinned.symlink_to(real)

            with mock.patch.object(opcode_gas, "REPO_ROOT", temporary_root):
                with self.assertRaisesRegex(ValueError, "regular non-symlink"):
                    self.build()

    def test_builder_rejects_resealed_task3_envelope_identity_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            temporary_root = pathlib.Path(directory)
            self._copy_pinned_sources(temporary_root)
            envelope_path = (
                temporary_root
                / "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/augmentation.json"
            )
            envelope = json.loads(envelope_path.read_text())
            envelope["augmentation_identity"]["analysis_implementation_revision"] = (
                "0" * 40
            )
            reseal(envelope)
            envelope_path.write_text(json.dumps(envelope, indent=2, sort_keys=True) + "\n")

            with mock.patch.object(opcode_gas, "REPO_ROOT", temporary_root):
                with self.assertRaisesRegex(ValueError, "envelope identity differs"):
                    self.build()

    def test_builder_rejects_declared_selector_missing_from_source(self):
        declarations = copy.deepcopy(opcode_gas._OPERATION_SIDE_EFFECT_DECLARATIONS)
        declarations[0]["source_selectors"] = ["not_a_real_trace_selector"]

        with mock.patch.object(
            opcode_gas,
            "_OPERATION_SIDE_EFFECT_DECLARATIONS",
            declarations,
        ):
            with self.assertRaisesRegex(ValueError, "source selector is missing"):
                self.build()

    def test_confirmed_call_substitutes_wrapper_for_opcode_execution_charge(self):
        transaction = transaction_trace(started_tx_index=0)
        confirmed_call = operation_trace(
            {
                "kind": "opcode",
                "opcode": 0xF1,
                "pricing_basis": "fixed_per_event",
                "spawned": True,
                "dispatch_status": "confirmed",
            }
        )

        result = opcode_gas.classify_operation_trace_charge(
            confirmed_call, [transaction]
        )

        self.assertFalse(result["matches_execution_coverage"])
        self.assertEqual(result["side_effect_event"], "confirmed_spawn_wrapper")
        self.assertEqual(result["charge_source"], "operation_wrapper")
        self.assertEqual(result["charge_count"], 1)
        self.assertEqual(result["wrapper_semantics"], "substitutes_opcode_raw_gas")

    def test_child_opcode_is_one_execution_charge_and_zero_extra_grouping_charge(self):
        transaction = transaction_trace(started_tx_index=0)
        child_add = operation_trace(
            {
                "kind": "opcode",
                "opcode": 0x01,
                "pricing_basis": "raw_gas_slope",
                "interpreter_raw_gas": 3,
                "spawned": False,
                "dispatch_status": "not_applicable",
            },
            frame_depth=1,
        )

        result = opcode_gas.classify_operation_trace_charge(child_add, [transaction])

        self.assertTrue(result["matches_execution_coverage"])
        self.assertEqual(result["execution_key"], "opcode:0x01")
        self.assertEqual(result["charge_source"], "execution_coverage")
        self.assertEqual(result["charge_count"], 1)
        self.assertEqual(result["child_grouping_event"], "child_execution")
        self.assertTrue(result["child_grouping_covered_by_execution_coverage"])
        self.assertEqual(result["child_grouping_extra_charge"], 0)

    def test_precompile_native_gas_row_matches_execution_coverage(self):
        transaction = transaction_trace(started_tx_index=0)
        precompile = operation_trace(
            {
                "kind": "precompile",
                "address": "0x0000000000000000000000000000000000000004",
                "pricing_basis": "raw_gas_slope",
                "native_gas": 18,
            },
            frame_depth=1,
        )

        result = opcode_gas.classify_operation_trace_charge(precompile, [transaction])

        self.assertTrue(result["matches_execution_coverage"])
        self.assertEqual(result["execution_key"], "precompile:0x04")
        self.assertEqual(result["charge_count"], 1)
        self.assertEqual(result["child_grouping_extra_charge"], 0)

    def test_unknown_opcode_or_precompile_cannot_match_frozen_execution_coverage(self):
        transaction = transaction_trace(started_tx_index=0)
        unknown_opcode = operation_trace(
            {
                "kind": "opcode",
                "opcode": 0xAA,
                "pricing_basis": "raw_gas_slope",
                "interpreter_raw_gas": 3,
                "spawned": False,
                "dispatch_status": "not_applicable",
            }
        )
        unknown_precompile = operation_trace(
            {
                "kind": "precompile",
                "address": "0x00000000000000000000000000000000000000ff",
                "pricing_basis": "raw_gas_slope",
                "native_gas": 18,
            }
        )

        for operation in (unknown_opcode, unknown_precompile):
            with self.subTest(operation=operation):
                with self.assertRaisesRegex(ValueError, "frozen execution coverage"):
                    opcode_gas.classify_operation_trace_charge(
                        operation, [transaction]
                    )

    def test_confirmed_wrapper_with_raw_gas_cannot_double_match(self):
        transaction = transaction_trace(started_tx_index=0)
        malformed = operation_trace(
            {
                "kind": "opcode",
                "opcode": 0xF1,
                "pricing_basis": "fixed_per_event",
                "interpreter_raw_gas": 700,
                "spawned": True,
                "dispatch_status": "confirmed",
            }
        )

        with self.assertRaisesRegex(ValueError, "declared selector"):
            opcode_gas.classify_operation_trace_charge(malformed, [transaction])

    def test_selected_not_dispatched_spawn_is_exact_no_charge_diagnostic(self):
        transaction = transaction_trace(started_tx_index=0)
        selected = operation_trace(
            {
                "kind": "opcode",
                "opcode": 0xF1,
                "spawned": True,
                "dispatch_status": "selected_not_dispatched",
            }
        )

        result = opcode_gas.classify_operation_trace_charge(selected, [transaction])

        self.assertFalse(result["matches_execution_coverage"])
        self.assertEqual(
            result["side_effect_event"], "selected_not_dispatched_spawn"
        )
        self.assertEqual(result["charge_count"], 0)

    def test_transaction_envelope_requires_started_non_anchor_transaction(self):
        started = transaction_trace(started_tx_index=0)
        unattempted = transaction_trace(
            started_tx_index=None, disposition="unattempted"
        )
        anchor = transaction_trace(started_tx_index=0, is_anchor=True)

        self.assertEqual(
            opcode_gas.classify_transaction_trace_ownership(started),
            {
                "transaction_envelope": True,
                "anchor_transaction": False,
                "native_value_transfer": False,
                "owner": "transaction",
            },
        )
        self.assertEqual(
            opcode_gas.classify_transaction_trace_ownership(unattempted)[
                "transaction_envelope"
            ],
            False,
        )
        self.assertEqual(
            opcode_gas.classify_transaction_trace_ownership(anchor),
            {
                "transaction_envelope": False,
                "anchor_transaction": True,
                "native_value_transfer": False,
                "owner": "block",
            },
        )

    def test_native_value_transfer_requires_committed_successful_non_anchor_tx(self):
        native = transaction_trace(
            started_tx_index=0, disposition="committed_success"
        )
        native["native_value_transfer"] = True
        result = opcode_gas.classify_transaction_trace_ownership(native)
        self.assertTrue(result["transaction_envelope"])
        self.assertTrue(result["native_value_transfer"])

        invalid_rows = (
            transaction_trace(started_tx_index=0, disposition="attempted"),
            transaction_trace(started_tx_index=0, disposition="committed_failure"),
            transaction_trace(started_tx_index=None, disposition="unattempted"),
            transaction_trace(
                started_tx_index=0,
                is_anchor=True,
                disposition="committed_success",
            ),
        )
        for row in invalid_rows:
            row["native_value_transfer"] = True
            with self.subTest(row=row):
                with self.assertRaisesRegex(ValueError, "native value transfer"):
                    opcode_gas.classify_transaction_trace_ownership(row)

    def test_native_transfer_ownership_has_machine_selector(self):
        manifest = self.build()
        events = {
            row["event_id"]: row for row in manifest["side_effect_ownership"]
        }
        selector = manifest["trace_selectors"]["native_value_transfer"]

        self.assertEqual(
            events["native_value_transfer"]["selector_ref"],
            "native_value_transfer",
        )
        self.assertEqual(selector["scope"], "transaction_envelope")
        self.assertEqual(selector["transaction_disposition"], "committed_success")
        self.assertTrue(selector["native_value_transfer"])
        self.assertEqual(
            events["native_value_transfer"]["source_evidence"][0]["selectors"],
            [
                "pub native_value_transfer: bool",
                "TransactionDisposition::CommittedSuccess",
                "!value.is_zero()",
                "is_call",
                "!has_operation_trace",
            ],
        )

    def test_authoritative_design_matches_confirmed_spawn_substitution(self):
        design = (
            ROOT / "docs/plans/2026-09-26-zkgas-calibration-design.md"
        ).read_text()

        self.assertIn(
            "confirmed spawn replaces the pending opcode raw-gas row with one fixed wrapper row",
            design,
        )
        self.assertIn(
            "`child_execution` is a derived grouping with zero additional charge",
            design,
        )
        self.assertNotIn(
            "`CALL` keeps its opcode body while a confirmed spawn wrapper",
            design,
        )

    def test_anchor_and_system_operations_never_match_execution_coverage(self):
        raw_add = {
            "kind": "opcode",
            "opcode": 0x01,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": 3,
            "spawned": False,
            "dispatch_status": "not_applicable",
        }
        anchor = transaction_trace(started_tx_index=0, is_anchor=True)
        anchor_result = opcode_gas.classify_operation_trace_charge(
            operation_trace(raw_add), [anchor]
        )
        system_result = opcode_gas.classify_operation_trace_charge(
            operation_trace(raw_add, phase="system", tx_index=None), []
        )

        self.assertFalse(anchor_result["matches_execution_coverage"])
        self.assertEqual(anchor_result["charge_source"], "block")
        self.assertFalse(system_result["matches_execution_coverage"])
        self.assertEqual(system_result["charge_source"], "block")

    def test_committed_manifest_is_exact_source_replay(self):
        self.assertTrue(MANIFEST_PATH.is_file(), "canonical v4 manifest is missing")
        committed = json.loads(MANIFEST_PATH.read_text())
        schedule = opcode_gas.load_current_uzen_schedule()
        expected = opcode_gas.build_operation_coverage_manifest(
            schedule=schedule,
            augmented_core=self.core,
            augmented_core_ref=CORE_REF,
        )

        self.assertEqual(committed["artifact_sha256"], V4_ARTIFACT_SHA256)
        self.assertEqual(opcode_gas.sha256_file(MANIFEST_PATH), V4_FILE_SHA256)
        self.assertEqual(
            MANIFEST_PATH.read_bytes(),
            opcode_gas._canonical_json_file_bytes(committed),
        )
        self.assertEqual(committed, expected)
        opcode_gas.validate_operation_coverage_manifest(
            committed,
            schedule=schedule,
            augmented_core=self.core,
            augmented_core_ref=CORE_REF,
        )

    def test_historical_v1_path_and_hashes_are_preserved(self):
        historical = json.loads(V1_MANIFEST_PATH.read_text())

        self.assertEqual(
            V1_MANIFEST_PATH.relative_to(ROOT).as_posix(),
            "experiments/opcode-gas/manifests/operation-coverage-v1.json",
        )
        self.assertEqual(historical["artifact_sha256"], V1_ARTIFACT_SHA256)
        self.assertEqual(opcode_gas.sha256_file(V1_MANIFEST_PATH), V1_FILE_SHA256)

    def test_historical_v2_path_and_hashes_are_preserved(self):
        historical = json.loads(V2_MANIFEST_PATH.read_text())

        self.assertEqual(
            V2_MANIFEST_PATH.relative_to(ROOT).as_posix(),
            "experiments/opcode-gas/manifests/operation-coverage-v2.json",
        )
        self.assertEqual(historical["artifact_sha256"], V2_ARTIFACT_SHA256)
        self.assertEqual(opcode_gas.sha256_file(V2_MANIFEST_PATH), V2_FILE_SHA256)

    def test_historical_v3_path_and_hashes_are_preserved(self):
        historical = json.loads(V3_MANIFEST_PATH.read_text())

        self.assertEqual(
            V3_MANIFEST_PATH.relative_to(ROOT).as_posix(),
            "experiments/opcode-gas/manifests/operation-coverage-v3.json",
        )
        self.assertEqual(historical["artifact_sha256"], V3_ARTIFACT_SHA256)
        self.assertEqual(opcode_gas.sha256_file(V3_MANIFEST_PATH), V3_FILE_SHA256)


if __name__ == "__main__":
    unittest.main()
