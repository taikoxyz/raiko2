import contextlib
import copy
import io
import json
import pathlib
import shutil
import sys
import tempfile
import unittest
from decimal import Decimal, localcontext
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


HIGHER_LAYER_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "3e4de6eecb5e92aa59a6a4b9"
)
CORE_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "f945e67bb2c38c9c8ef50530"
    / "core-opcode-submodel.json"
)
COVERAGE_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v3.json"
)


class CorrectedHigherLayerProjectionTests(unittest.TestCase):
    def build(self):
        with mock.patch.object(opcode_gas, "git_head", return_value="f" * 40), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(opcode_gas, "git_has_local_commit", return_value=True):
            return opcode_gas.build_corrected_higher_layer_projection(
                HIGHER_LAYER_SOURCE
            )

    def test_replays_sealed_rows_in_production_scaled_registry_units(self):
        artifact = self.build()

        self.assertEqual(
            artifact["fixed_costs"],
            {
                "proposal_startup": (
                    "156283811.1873039742401027024744315560468269800644814744765685865289508241987047"
                ),
                "block_base": (
                    "2507390.6829493087557603686635944700460829493087557603686635944700460829493087559"
                ),
                "tx_base": (
                    "172459.2594934340082738577239479478141801412535255303095356380020061857039730915"
                ),
                "native_value_transfer": "5017",
            },
        )
        self.assertEqual(artifact["selected_round"], 128)
        self.assertEqual(
            artifact["coarse_state_trie"]["status"], "coarse_model_accepted"
        )
        self.assertEqual(
            [row["status"] for row in artifact["state_holdouts"]],
            ["accepted"] * 6,
        )
        self.assertEqual(artifact["registry_parameter_basis"], "production_scaled")
        self.assertEqual(artifact["resolver_semantics"], "typed_registry_v1")
        self.assertEqual(
            artifact["artifact_sha256"],
            opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in artifact.items()
                        if key != "artifact_sha256"
                    }
                )
            ),
        )

    def test_rejects_source_tamper_and_dirty_or_missing_revision(self):
        with tempfile.TemporaryDirectory() as temporary:
            copied = pathlib.Path(temporary) / "source"
            shutil.copytree(HIGHER_LAYER_SOURCE, copied)
            calibration = json.loads(
                (copied / "higher-layer-calibration.json").read_text()
            )
            calibration["coarse_state_trie"]["status"] = "needs_state_split"
            (copied / "higher-layer-calibration.json").write_text(
                json.dumps(calibration, sort_keys=True) + "\n"
            )
            with self.assertRaisesRegex(ValueError, "source file hash"):
                with mock.patch.object(
                    opcode_gas, "git_head", return_value="f" * 40
                ), mock.patch.object(
                    opcode_gas, "git_worktree_status", return_value=""
                ), mock.patch.object(
                    opcode_gas, "git_has_local_commit", return_value=True
                ):
                    opcode_gas.build_corrected_higher_layer_projection(copied)

        with mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=" M source.py\n"
        ), self.assertRaisesRegex(ValueError, "dirty implementation path"):
            opcode_gas.build_corrected_higher_layer_projection(HIGHER_LAYER_SOURCE)

        with mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_has_local_commit", return_value=False
        ), self.assertRaisesRegex(ValueError, "local commit"):
            opcode_gas.build_corrected_higher_layer_projection(HIGHER_LAYER_SOURCE)

    def test_replay_is_deterministic_and_does_not_mutate_source(self):
        before = {
            path.name: path.read_bytes()
            for path in HIGHER_LAYER_SOURCE.iterdir()
            if path.is_file()
        }

        first = self.build()
        second = self.build()

        self.assertEqual(first, second)
        self.assertEqual(
            before,
            {
                path.name: path.read_bytes()
                for path in HIGHER_LAYER_SOURCE.iterdir()
                if path.is_file()
            },
        )

    def test_reads_and_hashes_the_same_bytes_across_path_replacement(self):
        original = {"artifact_sha256": "1" * 64, "value": 1}
        replacement = {"artifact_sha256": "2" * 64, "value": 2}
        original_bytes = opcode_gas._canonical_json_file_bytes(original)
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "source.json"
            replacement_path = root / "replacement.json"
            source.write_bytes(original_bytes)
            replacement_path.write_bytes(
                opcode_gas._canonical_json_file_bytes(replacement)
            )
            real_open = opcode_gas.os.open
            replaced = False

            def open_then_replace(path, flags):
                nonlocal replaced
                descriptor = real_open(path, flags)
                if pathlib.Path(path) == source and not replaced:
                    replacement_path.replace(source)
                    replaced = True
                return descriptor

            with mock.patch.object(opcode_gas.os, "open", side_effect=open_then_replace):
                parsed = opcode_gas._read_canonical_json_mapping_once(
                    source,
                    label="test source",
                    expected_sha256=opcode_gas.sha256_bytes(original_bytes),
                )

        self.assertEqual(parsed, original)


def _tx(index, *, anchor=False, disposition="committed_success", native=False):
    return {
        "recovered_index": index,
        "started_tx_index": index,
        "manifest_index": None if anchor else index - 1,
        "tx_hash": "0x" + f"{index + 1:02x}" * 32,
        "is_anchor": anchor,
        "disposition": disposition,
        "native_value_transfer": native,
    }


def _opcode(operation_id, opcode, model_input, *, phase="transaction", tx_index=2):
    row = {
        "operation_id": operation_id,
        "phase": phase,
        "frame_depth": 0,
        "component": {
            "kind": "opcode",
            "opcode": opcode,
            "pricing_basis": "raw_gas_slope",
            "interpreter_raw_gas": model_input.get("raw_gas", 0),
            "model_input": model_input,
            "spawned": False,
            "dispatch_status": "not_applicable",
        },
    }
    if tx_index is not None:
        row["tx_index"] = tx_index
    return row


def _complete_trace(*operations):
    guest_hash = "0x" + "11" * 32
    public_output = "0x" + "22" * 32
    transactions = [
        _tx(0, anchor=True),
        _tx(1, native=True),
        _tx(2, disposition="committed_failure"),
        {
            "recovered_index": 3,
            "manifest_index": 2,
            "tx_hash": "0x" + "04" * 32,
            "is_anchor": False,
            "disposition": "unattempted",
            "native_value_transfer": False,
        },
    ]
    return {
        "schema_version": 2,
        "guest_input_sha256": guest_hash,
        "guest_input_bincode_length": 1234,
        "status": "complete",
        "public_output": public_output,
        "blocks": [
            {
                "block_index": 0,
                "block_number": 100,
                "input_transaction_count": 4,
                "started_transaction_count": 3,
                "committed_transaction_hashes": [
                    transactions[0]["tx_hash"],
                    transactions[1]["tx_hash"],
                    transactions[2]["tx_hash"],
                ],
                "attempted_transaction_hashes": [],
                "unattempted_transaction_hashes": [transactions[3]["tx_hash"]],
                "native_value_transfer_count": 1,
                "finalized_block_zkgas": 0,
                "transactions": transactions,
                "operations": list(operations),
            }
        ],
        "partial_blocks": [],
        "recovery_failures": [],
        "parity": {"passed": True, "mismatch_fields": []},
    }


def _sp1_report(trace, gas=160_000_000):
    return {
        "stage": "proposal",
        "mode": "execute",
        "proof_mode": "compressed",
        "sp1_execution_engine": "gas-estimator",
        "guest_input_sha256": trace["guest_input_sha256"],
        "guest_input_bincode_length": trace["guest_input_bincode_length"],
        "public_values": trace["public_output"],
        "exit_code": 0,
        "gas": gas,
        "primary_workload_metric": {"label": "prover_gas", "count": gas},
    }


class CompositeEstimatorTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with mock.patch.object(opcode_gas, "git_head", return_value="f" * 40), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(opcode_gas, "git_has_local_commit", return_value=True):
            cls.estimator = opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORE_SOURCE,
                operation_coverage_path=COVERAGE_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
            )

    def test_strict_registry_loader_rejects_float_and_slot_drift(self):
        core = json.loads(CORE_SOURCE.read_text())
        coverage = json.loads(COVERAGE_SOURCE.read_text())
        registry = opcode_gas.load_composite_registry(core, coverage)
        self.assertEqual(registry.opcode_model_ids[0x01], "opcode:0x01")

        floated = json.loads(json.dumps(core))
        floated["registry"]["common_dispatch"] = 1.5
        floated.pop("artifact_sha256")
        floated["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(floated)
        )
        with self.assertRaisesRegex(ValueError, "canonical Decimal string"):
            opcode_gas.load_composite_registry(floated, coverage)

        drifted = json.loads(json.dumps(core))
        drifted["registry"]["opcode_model_ids"][0x01] = None
        drifted.pop("artifact_sha256")
        drifted["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(drifted)
        )
        with self.assertRaisesRegex(ValueError, "measured opcode index"):
            opcode_gas.load_composite_registry(drifted, coverage)

    def test_artifact_is_deterministic_and_binds_every_exact_source(self):
        with mock.patch.object(opcode_gas, "git_head", return_value="f" * 40), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(opcode_gas, "git_has_local_commit", return_value=True):
            repeated = opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORE_SOURCE,
                operation_coverage_path=COVERAGE_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
            )

        self.assertEqual(repeated, self.estimator)
        self.assertEqual(self.estimator["implementation_revision"], "f" * 40)
        self.assertEqual(
            self.estimator["source_artifacts"]["augmented_core"]["file_sha256"],
            opcode_gas.sha256_file(CORE_SOURCE),
        )
        self.assertEqual(
            self.estimator["source_artifacts"]["operation_coverage"]["file_sha256"],
            opcode_gas.sha256_file(COVERAGE_SOURCE),
        )
        self.assertEqual(self.estimator["trace_schema"]["schema_version"], 2)
        self.assertEqual(
            self.estimator["artifact_sha256"],
            opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    {
                        key: value
                        for key, value in self.estimator.items()
                        if key != "artifact_sha256"
                    }
                )
            ),
        )

    def test_complete_typed_trace_emits_prediction_before_report_and_ape_after_join(self):
        trace = _complete_trace(
            _opcode(0, 0x01, {"kind": "static_raw_gas", "raw_gas": 3}),
            _opcode(1, 0x0A, {"kind": "exp", "exponent_byte_length": 2}),
        )
        without_report = opcode_gas.estimate_composite_trace(self.estimator, trace)
        self.assertEqual(without_report["gaps"], [])
        self.assertEqual(
            without_report["predicted_prover_gas"],
            without_report["modeled_subtotal"],
        )
        self.assertNotIn("ape", without_report)
        self.assertEqual(without_report["actual_report_join"], "not_provided")
        self.assertEqual(without_report["validation_status"], "not_evaluated")

        report = opcode_gas.estimate_composite_trace(
            self.estimator, trace, sp1_report=_sp1_report(trace)
        )
        fixed = self.estimator["fixed_costs"]
        registry = self.estimator["registry"]
        with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
            expected = (
                Decimal(fixed["proposal_startup"])
                + Decimal(fixed["block_base"])
                + Decimal(2) * Decimal(fixed["tx_base"])
                + Decimal(fixed["native_value_transfer"])
                + Decimal(2) * Decimal(registry["common_dispatch"])
                + Decimal(3)
                * Decimal(
                    registry["models"]["opcode:0x01"]["parameters"][
                        "body_per_raw_gas"
                    ]
                )
                + Decimal(
                    registry["models"]["opcode:0x0a"]["parameters"][
                        "small_bucket_body"
                    ]
                )
            )
            expected_ape = (
                abs(expected - Decimal(160_000_000)) / Decimal(160_000_000)
            )
        self.assertEqual(
            report["modeled_subtotal"], opcode_gas._decimal_text(expected)
        )
        self.assertEqual(report["predicted_prover_gas"], report["modeled_subtotal"])
        self.assertEqual(
            report["ape"],
            opcode_gas._decimal_text(expected_ape),
        )
        self.assertEqual(
            report["layer_contributions"]["transaction_base"]["count"], 2
        )
        self.assertEqual(
            report["layer_contributions"]["native_value_transfer"]["count"], 1
        )
        self.assertEqual(report["validation_status"], "evaluated")

    def test_resealed_semantic_policy_or_coverage_tamper_is_rejected(self):
        for mutate, expected in (
            (
                lambda artifact: artifact["ownership_policy"].__setitem__(
                    "system_and_anchor_operations", "operation_model"
                ),
                "ownership policy",
            ),
            (
                lambda artifact: artifact["execution_coverage"][1].__setitem__(
                    "classification", "structured_opcode"
                ),
                "coverage",
            ),
            (
                lambda artifact: artifact["fixed_cost_statuses"].__setitem__(
                    "native_value_transfer", "accepted"
                ),
                "fixed cost statuses",
            ),
        ):
            with self.subTest(expected=expected):
                artifact = copy.deepcopy(self.estimator)
                mutate(artifact)
                artifact.pop("artifact_sha256")
                artifact["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(artifact)
                )
                with self.assertRaisesRegex(ValueError, expected):
                    opcode_gas.estimate_composite_trace(
                        artifact, _complete_trace()
                    )

    def test_trace_header_rejects_noncanonical_status_and_public_output(self):
        trace = _complete_trace()
        trace["status"] = "partial"
        with self.assertRaisesRegex(ValueError, "identity"):
            opcode_gas.estimate_composite_trace(self.estimator, trace)

        trace = _complete_trace()
        trace["public_output"] = "0x12"
        with self.assertRaisesRegex(ValueError, "public output"):
            opcode_gas.estimate_composite_trace(self.estimator, trace)

    def test_system_anchor_and_unattempted_work_are_not_double_charged(self):
        system = _opcode(
            0,
            0x01,
            {"kind": "static_raw_gas", "raw_gas": 3},
            phase="system",
            tx_index=None,
        )
        anchor = _opcode(
            1,
            0x01,
            {"kind": "static_raw_gas", "raw_gas": 3},
            tx_index=0,
        )
        trace = _complete_trace(system, anchor)
        report = opcode_gas.estimate_composite_trace(self.estimator, trace)

        self.assertEqual(report["layer_contributions"]["operations"]["count"], 0)
        self.assertEqual(report["block_owned_operation_count"], 2)
        self.assertEqual(
            report["layer_contributions"]["transaction_base"]["count"], 2
        )
        self.assertEqual(report["gaps"], [])

    def test_transaction_trace_cannot_masquerade_user_work_as_anchor(self):
        trace = _complete_trace()
        trace["blocks"][0]["transactions"][1]["is_anchor"] = True
        with self.assertRaisesRegex(ValueError, "anchor identity"):
            opcode_gas.estimate_composite_trace(self.estimator, trace)

    def test_unmeasured_execution_and_feature_errors_are_exact_gaps(self):
        unsupported = _opcode(
            0, 0x55, {"kind": "static_raw_gas", "raw_gas": 100}
        )
        precompile = {
            "operation_id": 1,
            "phase": "transaction",
            "tx_index": 2,
            "frame_depth": 0,
            "component": {
                "kind": "precompile",
                "address": "0x0000000000000000000000000000000000000004",
                "pricing_basis": "raw_gas_slope",
                "native_gas": 18,
            },
        }
        wrapper = {
            "operation_id": 2,
            "phase": "transaction",
            "tx_index": 2,
            "frame_depth": 0,
            "component": {
                "kind": "opcode",
                "opcode": 0xF1,
                "pricing_basis": "fixed_per_event",
                "spawned": True,
                "dispatch_status": "confirmed",
            },
        }
        feature_error = {
            "operation_id": 3,
            "phase": "transaction",
            "tx_index": 2,
            "frame_depth": 0,
            "component": {
                "kind": "opcode_feature_error",
                "opcode": 0x0A,
                "interpreter_raw_gas": 10,
                "error": {
                    "kind": "stack_underflow",
                    "feature": "exponent_byte_length",
                },
            },
        }
        trace = _complete_trace(unsupported, precompile, wrapper, feature_error)
        report = opcode_gas.estimate_composite_trace(
            self.estimator, trace, sp1_report=_sp1_report(trace)
        )

        self.assertEqual(
            [gap["reason"] for gap in report["gaps"]],
            [
                "explicitly_unsupported_opcode",
                "declared_unmeasured_precompile",
                "confirmed_spawn_wrapper_unmeasured",
                "opcode_feature_error",
            ],
        )
        self.assertNotIn("predicted_prover_gas", report)
        self.assertNotIn("ape", report)
        self.assertEqual(report["validation_status"], "insufficient_coverage")
        self.assertEqual(report["coverage"]["operation_count"]["denominator"], 4)
        self.assertEqual(report["coverage"]["operation_count"]["numerator"], 0)
        self.assertEqual(report["coverage"]["raw_gas"]["denominator"], "128")
        self.assertEqual(report["coverage"]["spawn_wrapper"]["denominator"], 1)

    def test_native_transfer_cost_requires_committed_success(self):
        trace = _complete_trace()
        trace["blocks"][0]["transactions"][1]["disposition"] = "committed_failure"
        report = opcode_gas.estimate_composite_trace(self.estimator, trace)

        self.assertEqual(
            report["layer_contributions"]["native_value_transfer"]["count"], 0
        )
        self.assertEqual(report["gaps"][0]["reason"], "invalid_native_transfer_marker")

    def test_native_transfer_with_operation_is_a_gap_and_is_not_double_charged(self):
        trace = _complete_trace(
            _opcode(
                0,
                0x01,
                {"kind": "static_raw_gas", "raw_gas": 3},
                tx_index=1,
            )
        )
        report = opcode_gas.estimate_composite_trace(self.estimator, trace)

        self.assertEqual(
            report["layer_contributions"]["native_value_transfer"]["count"], 0
        )
        self.assertEqual(report["gaps"][0]["reason"], "invalid_native_transfer_marker")

    def test_executed_opcode_schema_mismatch_is_a_gap(self):
        operation = _opcode(
            0, 0x01, {"kind": "static_raw_gas", "raw_gas": 3}
        )
        operation["component"]["pricing_basis"] = "fixed_per_event"
        report = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(operation)
        )

        self.assertEqual(report["gaps"][0]["reason"], "opcode_trace_schema_mismatch")
        self.assertEqual(report["layer_contributions"]["operations"]["count"], 0)

    def test_multiblock_estimate_charges_startup_once_and_each_block_and_transaction(self):
        trace = _complete_trace()
        second = copy.deepcopy(trace["blocks"][0])
        second["block_index"] = 1
        second["block_number"] = 101
        trace["blocks"].append(second)
        report = opcode_gas.estimate_composite_trace(self.estimator, trace)

        self.assertEqual(
            report["layer_contributions"]["proposal_startup"]["count"], 1
        )
        self.assertEqual(report["layer_contributions"]["block_base"]["count"], 2)
        self.assertEqual(
            report["layer_contributions"]["transaction_base"]["count"], 4
        )
        self.assertEqual(
            report["layer_contributions"]["native_value_transfer"]["count"], 2
        )

    def test_report_join_mismatch_is_separate_from_coverage_and_suppresses_only_ape(self):
        trace = _complete_trace()
        sp1_report = _sp1_report(trace)
        sp1_report["public_values"] = "0x" + "33" * 32
        report = opcode_gas.estimate_composite_trace(
            self.estimator, trace, sp1_report=sp1_report
        )

        self.assertEqual(report["actual_report_join"], "mismatch")
        self.assertEqual(report["actual_report_join_mismatches"], ["public_values"])
        self.assertEqual(report["validation_status"], "report_join_mismatch")
        self.assertEqual(report["gaps"], [])
        self.assertTrue(report["coverage_complete"])
        self.assertEqual(report["predicted_prover_gas"], report["modeled_subtotal"])
        self.assertNotIn("ape", report)

    def test_seal_is_create_only_and_verify_rejects_tamper(self):
        with tempfile.TemporaryDirectory() as temporary:
            out_root = pathlib.Path(temporary) / "estimators"
            pointer = pathlib.Path(temporary) / "estimator-path"
            with mock.patch.object(
                opcode_gas, "git_head", return_value="f" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_has_local_commit", return_value=True
            ):
                sealed = opcode_gas.seal_composite_estimator(
                    augmented_core_path=CORE_SOURCE,
                    operation_coverage_path=COVERAGE_SOURCE,
                    higher_layer_package=HIGHER_LAYER_SOURCE,
                    out_root=out_root,
                    estimator_path_file=pointer,
                )
                verified = opcode_gas.verify_composite_estimator(sealed)
                self.assertEqual(
                    verified["artifact_sha256"], self.estimator["artifact_sha256"]
                )
                with self.assertRaisesRegex(ValueError, "already exists"):
                    opcode_gas.seal_composite_estimator(
                        augmented_core_path=CORE_SOURCE,
                        operation_coverage_path=COVERAGE_SOURCE,
                        higher_layer_package=HIGHER_LAYER_SOURCE,
                        out_root=out_root,
                        estimator_path_file=pointer,
                    )

                artifact_path = sealed / "estimator.json"
                artifact = json.loads(artifact_path.read_text())
                artifact["fixed_costs"]["tx_base"] = "1"
                artifact_path.write_text(json.dumps(artifact, sort_keys=True) + "\n")
                with self.assertRaisesRegex(
                    ValueError, "canonical JSON|artifact SHA256"
                ):
                    opcode_gas.verify_composite_estimator(sealed)

    def test_failed_path_handoff_rolls_back_new_estimator_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            out_root = pathlib.Path(temporary) / "estimators"
            with mock.patch.object(
                opcode_gas, "git_head", return_value="f" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_has_local_commit", return_value=True
            ), mock.patch.object(
                opcode_gas,
                "write_run_path_file",
                side_effect=ValueError("path handoff failed"),
            ), self.assertRaisesRegex(ValueError, "path handoff failed"):
                opcode_gas.seal_composite_estimator(
                    augmented_core_path=CORE_SOURCE,
                    operation_coverage_path=COVERAGE_SOURCE,
                    higher_layer_package=HIGHER_LAYER_SOURCE,
                    out_root=out_root,
                    estimator_path_file=pathlib.Path(temporary) / "estimator-path",
                )

            self.assertEqual(list(out_root.iterdir()), [])

    def test_cli_exposes_seal_verify_and_trace_estimate_commands(self):
        parser = opcode_gas.build_parser()
        for command in (
            "seal-composite-estimator",
            "verify-composite-estimator",
            "estimate-composite-trace",
        ):
            with self.subTest(command=command):
                with contextlib.redirect_stdout(io.StringIO()), self.assertRaises(
                    SystemExit
                ) as raised:
                    parser.parse_args([command, "--help"])
                self.assertEqual(raised.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
