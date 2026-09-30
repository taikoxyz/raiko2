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
from types import SimpleNamespace
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas" / "tests"))

import opcode_gas
import context_approximation as approximation
import context_opcode_campaign as context_opcode
from test_context_opcode import (
    HISTORICAL_CONTROL_ELF_SHA256,
    HISTORICAL_LEGACY_ELF_SHA256,
    HISTORICAL_LAUNCHER_SHA256,
    TEST_CONTEXT_ELF_SHA256,
    passing_production_adaptive_evidence,
    passing_production_canary,
    production_campaign_source,
)


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
    / "operation-coverage-v4.json"
)
STATEFUL_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "64065fa462311bdc1848e9d0"
)
CORRECTED_OSAKA_CORE_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "3fc67063a921182e971e7882"
    / "core-opcode-submodel.json"
)
COVERAGE_V5_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "manifests"
    / "operation-coverage-v5.json"
)
CONTEXT_APPROXIMATION_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "1d2758bcb7aa3ae7f09d14eb"
)
GUEST_LAUNCHER = ROOT / "target" / "release" / "guest-launcher"
SP1_PROPOSAL_ELF = ROOT / "crates" / "guests" / "elf" / "sp1_shasta_proposal.elf"
SP1_PROPOSAL_VK = ROOT / "crates" / "guests" / "elf" / "sp1_shasta_proposal.vk.bin"
SEALED_SCHEMA_3_ESTIMATOR = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "estimators"
    / "43cdd0adc466743bbb4a2e3b"
    / "estimator.json"
)


@contextlib.contextmanager
def _legacy_schema_3_trace_source():
    read_regular_file_bytes_once = opcode_gas._read_regular_file_bytes_once
    reconstruct = (ROOT / "crates" / "zkgas-trace" / "src" / "reconstruct.rs").resolve()

    def read_schema_3(path, *, label):
        raw = read_regular_file_bytes_once(path, label=label)
        if pathlib.Path(path).resolve() == reconstruct:
            raw = raw.replace(
                b"pub const OPERATION_TRACE_SCHEMA_VERSION: u32 = 4;",
                b"pub const OPERATION_TRACE_SCHEMA_VERSION: u32 = 3;",
            )
        return raw

    with mock.patch.object(
        opcode_gas,
        "_read_regular_file_bytes_once",
        side_effect=read_schema_3,
    ):
        yield


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

    def test_historical_projection_does_not_read_live_proposal_guest(self):
        live_guest_paths = {
            (ROOT / "crates/guests/elf/sp1_shasta_proposal.elf").resolve(),
            (ROOT / "crates/guests/elf/sp1_shasta_proposal.vk.bin").resolve(),
        }
        read_regular_file_bytes_once = opcode_gas._read_regular_file_bytes_once

        def reject_live_guest_read(path, *, label):
            if pathlib.Path(path).resolve() in live_guest_paths:
                self.fail(f"historical projection read live proposal guest: {path}")
            return read_regular_file_bytes_once(path, label=label)

        with mock.patch.object(
            opcode_gas,
            "_read_regular_file_bytes_once",
            side_effect=reject_live_guest_read,
        ):
            artifact = self.build()

        self.assertEqual(artifact["selected_round"], 128)

    def test_historical_proposal_guest_identity_is_strict_and_canonical(self):
        valid = {
            "elf_path": "crates/guests/elf/sp1_shasta_proposal.elf",
            "elf_sha256": "1" * 64,
            "vk_path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
            "vk_sha256": "a" * 64,
        }
        opcode_gas._validate_composite_sp1_proposal_guest_identity(valid)

        invalid = []
        for field in valid:
            missing = dict(valid)
            missing.pop(field)
            invalid.append(missing)
        invalid.extend(
            [
                {**valid, "unexpected": True},
                {**valid, "elf_path": "/tmp/sp1_shasta_proposal.elf"},
                {
                    **valid,
                    "vk_path": "crates/guests/elf/./sp1_shasta_proposal.vk.bin",
                },
                {**valid, "elf_sha256": "A" * 64},
                {**valid, "vk_sha256": "a" * 63},
                {**valid, "vk_sha256": "g" * 64},
            ]
        )
        for guest in invalid:
            with self.subTest(guest=guest), self.assertRaisesRegex(
                ValueError, "proposal guest identity"
            ):
                opcode_gas._validate_composite_sp1_proposal_guest_identity(
                    guest
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
        "schema_version": 3,
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
        "sp1_gas_trace_chunk_threshold": 134_217_728,
        "sp1_gas_trace_chunk_slots": 2,
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
        cls.estimator = json.loads(SEALED_SCHEMA_3_ESTIMATOR.read_text())
        opcode_gas._validate_composite_estimator_artifact(cls.estimator)

    @staticmethod
    def build_declared_context_candidate():
        with mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_has_local_commit", return_value=True
        ):
            return opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORRECTED_OSAKA_CORE_SOURCE,
                operation_coverage_path=COVERAGE_V5_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
                stateful_result_path=STATEFUL_SOURCE,
                context_approximation_path=CONTEXT_APPROXIMATION_SOURCE,
                guest_launcher_path=GUEST_LAUNCHER,
                proposal_elf_path=SP1_PROPOSAL_ELF,
                proposal_vk_path=SP1_PROPOSAL_VK,
            )

    def test_declared_context_candidate_keeps_registry_and_binds_exact_sources(self):
        candidate = self.build_declared_context_candidate()
        core = json.loads(CORRECTED_OSAKA_CORE_SOURCE.read_text())
        approximation_artifact = json.loads(
            (CONTEXT_APPROXIMATION_SOURCE / "context-approximation.json").read_text()
        )

        self.assertEqual(candidate["schema_version"], 4)
        self.assertEqual(candidate["trace_schema"]["schema_version"], 4)
        self.assertEqual(candidate["implementation_revision"], "f" * 40)
        self.assertEqual(
            candidate["trace_schema"]["source_sha256"],
            opcode_gas.sha256_file(
                ROOT / candidate["trace_schema"]["source_path"]
            ),
        )
        self.assertEqual(candidate["registry"], core["registry"])
        sources = candidate["source_artifacts"]
        self.assertEqual(
            sources["augmented_core"],
            {
                "path": (
                    "experiments/opcode-gas/derivations/"
                    "3fc67063a921182e971e7882/core-opcode-submodel.json"
                ),
                "file_sha256": opcode_gas.sha256_file(
                    CORRECTED_OSAKA_CORE_SOURCE
                ),
                "artifact_sha256": core["artifact_sha256"],
            },
        )
        coverage_v5 = json.loads(COVERAGE_V5_SOURCE.read_text())
        self.assertEqual(
            sources["operation_coverage"],
            {
                "path": "experiments/opcode-gas/manifests/operation-coverage-v5.json",
                "file_sha256": opcode_gas.sha256_file(COVERAGE_V5_SOURCE),
                "artifact_sha256": coverage_v5["artifact_sha256"],
            },
        )
        self.assertEqual(
            sources["corrected_higher_layer"]["identity_sha256"],
            opcode_gas._COMPOSITE_HIGHER_LAYER_SOURCE["identity_sha256"],
        )
        self.assertEqual(
            sources["stateful_storage"]["result_identity_sha256"],
            opcode_gas._COMPOSITE_STATEFUL_STORAGE_REF[
                "result_identity_sha256"
            ],
        )
        self.assertEqual(
            candidate["source_artifacts"]["declared_context_approximation"],
            {
                "path": (
                    "experiments/opcode-gas/derivations/"
                    "1d2758bcb7aa3ae7f09d14eb"
                ),
                "file_sha256": (
                    "346c2ac112357872f4a981737469c39639d7cc845ae97b0cc8bb77ddfc8a5b58"
                ),
                "approximation_id": "1d2758bcb7aa3ae7f09d14eb",
                "approximation_identity_sha256": (
                    "1d2758bcb7aa3ae7f09d14eb8d539250747992c42d6ae95d204f4cc5efec06cb"
                ),
                "artifact_sha256": (
                    "357a6c47bad8e6def3280be77a300e113e350fa673e08c39af13034861aee3d8"
                ),
            },
        )
        self.assertEqual(
            candidate["declared_context_approximation"]["classes"],
            approximation_artifact["classes"],
        )
        execution = candidate["source_artifacts"]["execution_artifacts"]
        self.assertEqual(execution["guest_launcher"]["file_sha256"], opcode_gas.sha256_file(GUEST_LAUNCHER))
        self.assertEqual(execution["sp1_proposal_guest"]["elf_sha256"], opcode_gas.sha256_file(SP1_PROPOSAL_ELF))
        self.assertEqual(execution["sp1_proposal_guest"]["vk_sha256"], opcode_gas.sha256_file(SP1_PROPOSAL_VK))
        self.assertEqual(
            set(candidate["source_artifacts"]["source_code_sha256s"]),
            set(opcode_gas._COMPOSITE_DECLARED_CONTEXT_SOURCE_PATHS),
        )
        for relative, digest in candidate["source_artifacts"][
            "source_code_sha256s"
        ].items():
            self.assertEqual(digest, opcode_gas.sha256_file(ROOT / relative))

        by_key = {row["key"]: row for row in candidate["execution_coverage"]}
        current_inspector_sha256 = opcode_gas.sha256_file(
            ROOT / "crates/zkgas-trace/src/inspector.rs"
        )
        for opcode in (0x30, 0x33, 0x34, 0x35, 0x36, 0x42):
            row = by_key[f"opcode:0x{opcode:02x}"]
            self.assertEqual(row["classification"], "declared_context_approximation")
            self.assertEqual(row["model_status"], "declared_approximation")
            machine_evidence = [
                evidence
                for evidence in row["source_evidence"]
                if evidence.get("kind") == "machine_trace_selector"
            ]
            self.assertEqual(len(machine_evidence), 1)
            self.assertEqual(
                machine_evidence[0],
                {
                    "kind": "machine_trace_selector",
                    "path": "crates/zkgas-trace/src/inspector.rs",
                    "selector_ref": "opcode_raw_gas_execution",
                    "sha256": current_inspector_sha256,
                },
            )
            self.assertNotEqual(
                machine_evidence[0]["sha256"],
                "f75dff0f7d0e2b45a0ab3e1e3ff1604dc6d1c6a13615298b6a834c32a876f987",
            )

    def test_declared_context_candidate_prices_all_classes_once_from_typed_input(self):
        candidate = self.build_declared_context_candidate()
        cases = (
            (0x30, {"kind": "context_fixed"}, "address"),
            (0x33, {"kind": "context_fixed"}, "caller"),
            (0x34, {"kind": "context_value", "value_class": "zero"}, "callvalue:zero"),
            (0x34, {"kind": "context_value", "value_class": "nonzero"}, "callvalue:nonzero"),
            (0x35, {"kind": "calldata_load", "access_class": "zero"}, "calldataload:zero"),
            (0x35, {"kind": "calldata_load", "access_class": "partial"}, "calldataload:partial"),
            (0x35, {"kind": "calldata_load", "access_class": "full"}, "calldataload:full"),
            (0x36, {"kind": "calldata_size", "input_length": 0}, "calldatasize"),
            (0x42, {"kind": "context_value", "value_class": "nonzero"}, "timestamp"),
        )
        operations = []
        with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
            expected = Decimal(0)
            for index, (opcode, model_input, class_id) in enumerate(cases):
                operation = _opcode(index, opcode, model_input)
                operation["component"]["interpreter_raw_gas"] = 1_000_000 + index
                operations.append(operation)
                expected += Decimal(
                    candidate["declared_context_approximation"]["classes"][class_id][
                        "cost_exact"
                    ]["decimal"]
                )
        trace = _complete_trace(*operations)
        trace["schema_version"] = 4
        report = opcode_gas.estimate_composite_trace(candidate, trace)

        self.assertEqual(report["gaps"], [])
        self.assertEqual(
            report["layer_contributions"]["operations"],
            {"count": 9, "prover_gas": opcode_gas._decimal_text(expected)},
        )
        self.assertEqual(report["coverage"]["typed_feature"]["numerator"], 9)
        self.assertEqual(report["coverage"]["raw_gas"]["numerator"], "9000036")

    def test_declared_context_call_data_size_is_constant_and_not_raw_gas_scaled(self):
        candidate = self.build_declared_context_candidate()
        reports = []
        for raw_gas, input_length in ((3, 0), (999_999, 127)):
            operation = _opcode(
                0,
                0x36,
                {"kind": "calldata_size", "input_length": input_length},
            )
            operation["component"]["interpreter_raw_gas"] = raw_gas
            trace = _complete_trace(operation)
            trace["schema_version"] = 4
            reports.append(opcode_gas.estimate_composite_trace(candidate, trace))
        self.assertEqual(
            reports[0]["layer_contributions"]["operations"],
            reports[1]["layer_contributions"]["operations"],
        )

    def test_declared_context_rejects_missing_extra_or_incompatible_typed_input(self):
        candidate = self.build_declared_context_candidate()
        malformed = (
            (0x30, {}),
            (0x33, {"kind": "context_fixed", "value_class": "zero"}),
            (0x34, {"kind": "context_fixed"}),
            (0x35, {"kind": "calldata_load", "access_class": "other"}),
            (0x36, {"kind": "calldata_size", "input_length": -1}),
            (0x42, {"kind": "context_value", "value_class": "zero"}),
        )
        for opcode, model_input in malformed:
            with self.subTest(opcode=opcode, model_input=model_input):
                trace = _complete_trace(_opcode(0, opcode, model_input))
                trace["schema_version"] = 4
                report = opcode_gas.estimate_composite_trace(candidate, trace)
                self.assertEqual(
                    report["gaps"][0]["reason"],
                    "opcode_model_input_incompatible",
                )

    def test_declared_context_candidate_is_deterministic_and_source_tamper_fails(self):
        first = self.build_declared_context_candidate()
        second = self.build_declared_context_candidate()
        self.assertEqual(first, second)

        read_once = opcode_gas._read_regular_file_bytes_once

        def tamper(path, *, label):
            raw = read_once(path, label=label)
            if pathlib.Path(path).name == "context-approximation.json":
                return raw + b"\n"
            return raw

        with mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_has_local_commit", return_value=True
        ), mock.patch.object(
            opcode_gas, "_read_regular_file_bytes_once", side_effect=tamper
        ), self.assertRaisesRegex(ValueError, "context approximation.*hash"):
            opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORRECTED_OSAKA_CORE_SOURCE,
                operation_coverage_path=COVERAGE_V5_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
                stateful_result_path=STATEFUL_SOURCE,
                context_approximation_path=CONTEXT_APPROXIMATION_SOURCE,
                guest_launcher_path=GUEST_LAUNCHER,
                proposal_elf_path=SP1_PROPOSAL_ELF,
                proposal_vk_path=SP1_PROPOSAL_VK,
            )

    def test_declared_context_candidate_seals_create_only_and_replays_exact_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            out_root = root / "estimators"
            pointer = root / "estimator-path"
            kwargs = {
                "augmented_core_path": CORRECTED_OSAKA_CORE_SOURCE,
                "operation_coverage_path": COVERAGE_V5_SOURCE,
                "higher_layer_package": HIGHER_LAYER_SOURCE,
                "stateful_result_path": STATEFUL_SOURCE,
                "context_approximation_path": CONTEXT_APPROXIMATION_SOURCE,
                "guest_launcher_path": GUEST_LAUNCHER,
                "proposal_elf_path": SP1_PROPOSAL_ELF,
                "proposal_vk_path": SP1_PROPOSAL_VK,
                "out_root": out_root,
                "estimator_path_file": pointer,
            }
            with mock.patch.object(
                opcode_gas, "git_head", return_value="f" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_has_local_commit", return_value=True
            ):
                sealed = opcode_gas.seal_composite_estimator(**kwargs)
                verified = opcode_gas.verify_composite_estimator(sealed)
                self.assertEqual(verified["schema_version"], 4)
                self.assertEqual(pointer.read_text(), f"{sealed}\n")
                with self.assertRaisesRegex(ValueError, "already exists"):
                    opcode_gas.seal_composite_estimator(**kwargs)

    def test_declared_context_coverage_rejects_mismatched_machine_trace_provenance(self):
        mismatches = (
            (
                "sha256",
                "f75dff0f7d0e2b45a0ab3e1e3ff1604dc6d1c6a13615298b6a834c32a876f987",
            ),
            ("path", "crates/zkgas-trace/src/reconstruct.rs"),
            ("selector_ref", "precompile_raw_gas_execution"),
        )
        for field, value in mismatches:
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.build_declared_context_candidate())
                row = next(
                    row
                    for row in candidate["execution_coverage"]
                    if row["key"] == "opcode:0x30"
                )
                evidence = next(
                    evidence
                    for evidence in row["source_evidence"]
                    if evidence.get("kind") == "machine_trace_selector"
                )
                evidence[field] = value
                candidate.pop("artifact_sha256")
                candidate["artifact_sha256"] = opcode_gas.sha256_bytes(
                    opcode_gas.canonical_json(candidate)
                )

                with self.assertRaisesRegex(ValueError, "context coverage"):
                    opcode_gas._validate_composite_estimator_artifact(candidate)

    def test_incomplete_estimator_output_compares_without_fabricated_prediction(self):
        candidate = self.build_declared_context_candidate()
        trace = _complete_trace(
            _opcode(0, 0x31, {"kind": "static_raw_gas", "raw_gas": 2})
        )
        trace["schema_version"] = 4
        estimate = opcode_gas.estimate_composite_trace(candidate, trace)
        self.assertFalse(estimate["coverage_complete"])
        self.assertNotIn("predicted_prover_gas", estimate)

        report = approximation.compare_block_models(
            [
                {
                    "row_id": "validation-incomplete-estimator",
                    "partition": "validation",
                    "candidate_identity_sha256": candidate["artifact_sha256"],
                    "normalization_identity_sha256": "b" * 64,
                    "candidate_coverage_complete": estimate["coverage_complete"],
                    "candidate_predicted_prover_gas": estimate.get(
                        "predicted_prover_gas"
                    ),
                    "observed_prover_gas": "160000000",
                    "finalized_block_zkgas": "100000000",
                }
            ],
            Decimal("1"),
        )

        self.assertEqual(report["validation_status"], "insufficient_coverage")
        self.assertFalse(report["accepted"])
        self.assertIsNone(report["metrics"]["candidate"]["mape"])

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

    def test_sealed_schema_3_artifact_remains_valid_and_binds_every_exact_source(self):
        opcode_gas._validate_composite_estimator_artifact(self.estimator)
        self.assertEqual(
            self.estimator["implementation_revision"],
            "e59ed1f62926ccf399178fce0d2c396b5d130d0d",
        )
        self.assertEqual(
            self.estimator["source_artifacts"]["augmented_core"]["file_sha256"],
            opcode_gas.sha256_file(CORE_SOURCE),
        )
        self.assertEqual(
            self.estimator["source_artifacts"]["operation_coverage"]["file_sha256"],
            opcode_gas.sha256_file(COVERAGE_SOURCE),
        )
        self.assertEqual(self.estimator["trace_schema"]["schema_version"], 3)
        self.assertEqual(
            self.estimator["source_artifacts"]["stateful_storage"]["result_id"],
            "64065fa462311bdc1848e9d0",
        )
        self.assertEqual(
            set(
                self.estimator["source_artifacts"]["stateful_storage"][
                    "file_sha256s"
                ]
            ),
            opcode_gas._COMPOSITE_STATEFUL_RESULT_INVENTORY,
        )
        self.assertEqual(
            self.estimator["storage_model"]["family"], "M_typed"
        )
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

    def test_sealed_schema_3_estimator_rejects_schema_4_trace_join(self):
        trace = _complete_trace()
        trace["schema_version"] = 4

        with self.assertRaisesRegex(
            ValueError, "composite proposal trace schema version differs"
        ):
            opcode_gas.estimate_composite_trace(self.estimator, trace)

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

    def test_storage_inputs_use_exact_typed_model_without_common_dispatch(self):
        operations = [
            _opcode(
                0,
                0x54,
                {"kind": "storage_load", "access": "warm"},
            ),
            _opcode(
                1,
                0x54,
                {"kind": "storage_load", "access": "cold"},
            ),
        ]
        for index, branch in enumerate(
            ("noop", "set", "clear", "reset", "dirty_rewrite", "restore_original"),
            start=2,
        ):
            operations.append(
                _opcode(
                    index,
                    0x55,
                    {
                        "kind": "storage_store",
                        "access": "warm",
                        "branch": branch,
                    },
                )
            )
        for index, branch in enumerate(
            ("noop", "set", "clear", "reset"),
            start=8,
        ):
            operations.append(
                _opcode(
                    index,
                    0x55,
                    {
                        "kind": "storage_store",
                        "access": "cold",
                        "branch": branch,
                    },
                )
            )
        report = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(*operations)
        )
        model = self.estimator["storage_model"]["parameters"]
        with localcontext(opcode_gas._OPCODE_DECIMAL_CONTEXT):
            expected = Decimal(model["sload_warm_body"])
            expected += Decimal(model["sload_warm_body"]) + Decimal(
                model["sload_cold_extra"]
            )
            for branch in (
                "noop",
                "set",
                "clear",
                "reset",
                "dirty_rewrite",
                "restore_original",
            ):
                expected += Decimal(model[f"sstore_branch:{branch}"])
            for branch in ("noop", "set", "clear", "reset"):
                expected += Decimal(model[f"sstore_branch:{branch}"]) + Decimal(
                    model["sstore_cold_extra"]
                )

        self.assertEqual(report["gaps"], [])
        self.assertEqual(
            report["layer_contributions"]["operations"],
            {"count": 12, "prover_gas": opcode_gas._decimal_text(expected)},
        )
        self.assertEqual(report["coverage"]["typed_feature"]["numerator"], 12)

    def test_schema3_context_v1_cannot_promote_without_independent_transport(self):
        manifest = context_opcode.load_context_manifest(
            ROOT / "experiments/opcode-gas/manifests/sp1-context-opcode-v1.json"
        )
        corrected_path = ROOT / "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json"
        corrected = json.loads(corrected_path.read_text())
        package = json.loads(
            (corrected_path.parent / "augmentation.json").read_text()
        )
        v4 = json.loads(COVERAGE_SOURCE.read_text())
        v5 = context_opcode.build_operation_coverage_v5(v4, corrected, package)
        source_identity = {
            "legacy_revm_elf_sha256": HISTORICAL_LEGACY_ELF_SHA256,
            "legacy_revm_elf_path": context_opcode.FROZEN_LEGACY_REVM_ELF_PATH,
            "context_elf_sha256": TEST_CONTEXT_ELF_SHA256,
            "context_elf_path": "crates/guests/elf/sp1_context_opcode_lab.elf",
            "control_opcode_lab_elf_sha256": HISTORICAL_CONTROL_ELF_SHA256,
            "control_opcode_lab_elf_path": "crates/guests/elf/sp1_opcode_lab.elf",
            "launcher_sha256": HISTORICAL_LAUNCHER_SHA256,
            "launcher_path": "target/release/guest-launcher",
            "corrected_osaka_core_artifact_sha256": corrected["artifact_sha256"],
            "corrected_osaka_core_file_sha256": opcode_gas.sha256_file(corrected_path),
            "corrected_osaka_core_path": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
            "operation_coverage_v5_artifact_sha256": v5["artifact_sha256"],
            "operation_coverage_v5_file_sha256": opcode_gas.sha256_bytes(
                (json.dumps(v5, indent=2, sort_keys=True) + "\n").encode()
            ),
            "operation_coverage_v5_path": "experiments/opcode-gas/manifests/operation-coverage-v5.json",
        }
        campaign_source = production_campaign_source(source_identity)
        source_identity.update(
            {
                "calibration_id": campaign_source["calibration_id"],
                "calibration_identity_sha256": campaign_source[
                    "calibration_identity_sha256"
                ],
                "execution_revision": campaign_source[
                    "implementation_revision"
                ],
            }
        )
        context_rows = context_opcode.synthetic_passing_production_rows(manifest)
        result = context_opcode._build_context_result(
            manifest=manifest,
            rows=context_rows,
            source_registry=corrected,
            compatibility_canary=passing_production_canary(
                source_identity=source_identity
            ),
            source_identity=source_identity,
            adaptive_evidence=passing_production_adaptive_evidence(
                manifest, context_rows, source_identity
            ),
        )
        self.assertFalse(result["candidate_eligible"])
        self.assertEqual(result["context_transport"]["status"], "not_evaluated")
        with self.assertRaisesRegex(ValueError, "transport"):
            context_opcode.promote_operation_coverage_v6(v5, result)
        with self.assertRaisesRegex(ValueError, "transport"):
            context_opcode.overlay_context_models(corrected["registry"], result)
        with self.assertRaisesRegex(ValueError, "transport"):
            context_opcode.build_context_composite_estimator(
                base_estimator=self.estimator,
                corrected_core=corrected,
                coverage_v6={"schema_version": 3},
                result=result,
                context_source_path=f"experiments/opcode-gas/derivations/{result['result_id']}",
            )

    def test_stateful_source_binds_every_terminal_file(self):
        read_regular_file_bytes_once = opcode_gas._read_regular_file_bytes_once

        def corrupt_manifest(path, *, label):
            raw = read_regular_file_bytes_once(path, label=label)
            if pathlib.Path(path).name == "campaign-manifest.json":
                return raw + b"\n"
            return raw

        with mock.patch.object(
            opcode_gas,
            "_read_regular_file_bytes_once",
            side_effect=corrupt_manifest,
        ), mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_has_local_commit", return_value=True
        ), self.assertRaisesRegex(ValueError, "campaign-manifest.json source file hash"):
            opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORE_SOURCE,
                operation_coverage_path=COVERAGE_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
                stateful_result_path=STATEFUL_SOURCE,
            )

    def test_storage_prediction_ignores_raw_gas_but_rejects_unmeasured_combinations(self):
        warm = _opcode(
            0,
            0x54,
            {"kind": "storage_load", "access": "warm"},
        )
        warm["component"]["interpreter_raw_gas"] = 100
        changed = copy.deepcopy(warm)
        changed["component"]["interpreter_raw_gas"] = 9_999_999
        first = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(warm)
        )
        second = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(changed)
        )
        self.assertEqual(
            first["layer_contributions"]["operations"],
            second["layer_contributions"]["operations"],
        )

        cold_dirty = _opcode(
            0,
            0x55,
            {
                "kind": "storage_store",
                "access": "cold",
                "branch": "dirty_rewrite",
            },
        )
        malformed = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(cold_dirty)
        )
        self.assertEqual(
            malformed["gaps"][0]["reason"],
            "opcode_model_input_incompatible",
        )

        dirty_noop = _opcode(
            0,
            0x55,
            {
                "kind": "storage_store",
                "access": "warm",
                "branch": "dirty_noop",
            },
        )
        uncalibrated = opcode_gas.estimate_composite_trace(
            self.estimator, _complete_trace(dirty_noop)
        )
        self.assertEqual(
            uncalibrated["gaps"][0]["reason"],
            "opcode_model_input_incompatible",
        )

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
            (
                lambda artifact: artifact["storage_model"]["parameters"].__setitem__(
                    "sstore_branch:noop", "4475"
                ),
                "storage model source digest",
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

    def test_trace_header_rejects_contradictory_parity(self):
        trace = _complete_trace()
        trace["parity"] = {"passed": True, "mismatch_fields": ["state_root"]}

        with self.assertRaisesRegex(ValueError, "parity"):
            opcode_gas.estimate_composite_trace(self.estimator, trace)

    def test_spawn_wrapper_variants_require_exact_schema_and_spawn_opcode(self):
        selected = {
            "kind": "opcode",
            "opcode": 0xF1,
            "spawned": True,
            "dispatch_status": "selected_not_dispatched",
        }
        confirmed = {
            "kind": "opcode",
            "opcode": 0xF1,
            "pricing_basis": "fixed_per_event",
            "spawned": True,
            "dispatch_status": "confirmed",
        }
        for label, component in (
            ("selected non-spawn", {**selected, "opcode": 0x01}),
            ("selected extra field", {**selected, "pricing_basis": None}),
            ("confirmed non-spawn", {**confirmed, "opcode": 0x01}),
            ("confirmed extra field", {**confirmed, "unexpected": 1}),
        ):
            with self.subTest(label=label):
                operation = {
                    "operation_id": 0,
                    "phase": "transaction",
                    "tx_index": 2,
                    "frame_depth": 0,
                    "component": component,
                }
                with self.assertRaisesRegex(ValueError, "spawn wrapper"):
                    opcode_gas.estimate_composite_trace(
                        self.estimator, _complete_trace(operation)
                    )

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
            0, 0x31, {"kind": "static_raw_gas", "raw_gas": 100}
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

    def test_report_join_requires_exact_gas_estimator_chunk_configuration(self):
        trace = _complete_trace()
        for field, value in (
            ("sp1_gas_trace_chunk_threshold", None),
            ("sp1_gas_trace_chunk_threshold", 1),
            ("sp1_gas_trace_chunk_slots", None),
            ("sp1_gas_trace_chunk_slots", 999),
        ):
            with self.subTest(field=field, value=value):
                sp1_report = _sp1_report(trace)
                if value is None:
                    sp1_report.pop(field)
                else:
                    sp1_report[field] = value
                report = opcode_gas.estimate_composite_trace(
                    self.estimator, trace, sp1_report=sp1_report
                )

                self.assertEqual(report["actual_report_join"], "mismatch")
                self.assertIn(field, report["actual_report_join_mismatches"])
                self.assertEqual(
                    report["validation_status"], "report_join_mismatch"
                )
                self.assertNotIn("ape", report)
                self.assertEqual(
                    report["predicted_prover_gas"], report["modeled_subtotal"]
                )

    def test_generated_estimator_status_is_allowed_without_allowing_dirty_sources(self):
        estimator_status = (
            "?? experiments/opcode-gas/estimators/"
            "a59ffea7eb9d8d50ea4563dd/estimator.json\n"
        )

        opcode_gas.assert_generated_paths_only(estimator_status)

        with self.assertRaisesRegex(ValueError, "dirty implementation path"):
            opcode_gas.assert_generated_paths_only(
                estimator_status + " M experiments/opcode-gas/opcode_gas.py\n"
            )

    def test_schema_3_document_validates_but_new_sealing_rejects_schema_4_source(self):
        opcode_gas._validate_composite_estimator_artifact(self.estimator)
        tampered = copy.deepcopy(self.estimator)
        tampered["fixed_costs"]["tx_base"] = "1"
        with self.assertRaisesRegex(ValueError, "artifact SHA256"):
            opcode_gas._validate_composite_estimator_artifact(tampered)

        with tempfile.TemporaryDirectory() as temporary:
            out_root = pathlib.Path(temporary) / "estimators"
            pointer = pathlib.Path(temporary) / "estimator-path"
            with mock.patch.object(
                opcode_gas, "git_head", return_value="f" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_has_local_commit", return_value=True
            ), self.assertRaisesRegex(
                ValueError, "composite trace schema version differs from source"
            ):
                opcode_gas.seal_composite_estimator(
                    augmented_core_path=CORE_SOURCE,
                    operation_coverage_path=COVERAGE_SOURCE,
                    higher_layer_package=HIGHER_LAYER_SOURCE,
                    stateful_result_path=STATEFUL_SOURCE,
                    out_root=out_root,
                    estimator_path_file=pointer,
                )
            self.assertFalse(out_root.exists())
            self.assertFalse(pointer.exists())

    def test_seal_rejects_source_and_handoff_overlap_before_writing(self):
        stateful_before = {
            path.name: opcode_gas.sha256_file(path)
            for path in STATEFUL_SOURCE.iterdir()
        }
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            cases = (
                (
                    "estimator root inside stateful source",
                    STATEFUL_SOURCE,
                    root / "pointer",
                ),
                (
                    "handoff overwrites stateful source",
                    root / "estimators-a",
                    STATEFUL_SOURCE / "campaign-decisions.sha256",
                ),
                (
                    "handoff inside estimator root",
                    root / "estimators-b",
                    root / "estimators-b" / "pointer",
                ),
            )
            for label, out_root, pointer in cases:
                with self.subTest(label=label), mock.patch.object(
                    opcode_gas, "git_head", return_value="f" * 40
                ), mock.patch.object(
                    opcode_gas, "git_worktree_status", return_value=""
                ), mock.patch.object(
                    opcode_gas, "git_has_local_commit", return_value=True
                ), _legacy_schema_3_trace_source(
                ), self.assertRaisesRegex(ValueError, "overlaps"):
                    opcode_gas.seal_composite_estimator(
                        augmented_core_path=CORE_SOURCE,
                        operation_coverage_path=COVERAGE_SOURCE,
                        higher_layer_package=HIGHER_LAYER_SOURCE,
                        stateful_result_path=STATEFUL_SOURCE,
                        out_root=out_root,
                        estimator_path_file=pointer,
                    )

            self.assertFalse((root / "estimators-a").exists())
            self.assertFalse((root / "estimators-b").exists())
        self.assertEqual(
            stateful_before,
            {
                path.name: opcode_gas.sha256_file(path)
                for path in STATEFUL_SOURCE.iterdir()
            },
        )

    def test_failed_path_handoff_rolls_back_new_estimator_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            out_root = pathlib.Path(temporary) / "estimators"
            with mock.patch.object(
                opcode_gas, "git_head", return_value="f" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_has_local_commit", return_value=True
            ), _legacy_schema_3_trace_source(
            ), mock.patch.object(
                opcode_gas,
                "write_run_path_file",
                side_effect=ValueError("path handoff failed"),
            ), self.assertRaisesRegex(ValueError, "path handoff failed"):
                opcode_gas.seal_composite_estimator(
                    augmented_core_path=CORE_SOURCE,
                    operation_coverage_path=COVERAGE_SOURCE,
                    higher_layer_package=HIGHER_LAYER_SOURCE,
                    stateful_result_path=STATEFUL_SOURCE,
                    out_root=out_root,
                    estimator_path_file=pathlib.Path(temporary) / "estimator-path",
                )

            self.assertEqual(list(out_root.iterdir()), [])

    def test_post_publication_handoff_failure_restores_pointer_and_artifact(self):
        def publish_then_fail(path, run, *, durable_identity_name):
            self.assertEqual(durable_identity_name, "estimator.json")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(str(run) + "\n")
            raise OSError("post-publication directory fsync failed")

        for initial in (None, b"", b"preserve\n"):
            with self.subTest(initial=initial), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                out_root = root / "estimators"
                pointer = root / "estimator-path"
                if initial is not None:
                    pointer.write_bytes(initial)
                with mock.patch.object(
                    opcode_gas, "git_head", return_value="f" * 40
                ), mock.patch.object(
                    opcode_gas, "git_worktree_status", return_value=""
                ), mock.patch.object(
                    opcode_gas, "git_has_local_commit", return_value=True
                ), _legacy_schema_3_trace_source(
                ), mock.patch.object(
                    opcode_gas,
                    "write_run_path_file",
                    side_effect=publish_then_fail,
                ), self.assertRaisesRegex(OSError, "directory fsync failed"):
                    opcode_gas.seal_composite_estimator(
                        augmented_core_path=CORE_SOURCE,
                        operation_coverage_path=COVERAGE_SOURCE,
                        higher_layer_package=HIGHER_LAYER_SOURCE,
                        stateful_result_path=STATEFUL_SOURCE,
                        out_root=out_root,
                        estimator_path_file=pointer,
                    )

                self.assertEqual(list(out_root.iterdir()), [])
                if initial is None:
                    self.assertFalse(pointer.exists())
                else:
                    self.assertEqual(pointer.read_bytes(), initial)

    def test_trace_estimate_output_is_create_only_and_cannot_alias_inputs(self):
        for case in ("existing", "symlink", "estimator_child", "symlink_parent"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as temporary:
                root = pathlib.Path(temporary)
                estimator_dir = root / "estimator"
                estimator_dir.mkdir()
                trace_path = root / "trace.json"
                trace_bytes = (
                    json.dumps(_complete_trace(), sort_keys=True) + "\n"
                ).encode()
                trace_path.write_bytes(trace_bytes)
                if case == "existing":
                    output = root / "estimate.json"
                    output.write_bytes(b"preserve\n")
                elif case == "symlink":
                    output = root / "estimate.json"
                    output.symlink_to(trace_path)
                elif case == "estimator_child":
                    output = estimator_dir / "estimate.json"
                else:
                    alias = root / "estimator-alias"
                    alias.symlink_to(estimator_dir, target_is_directory=True)
                    output = alias / "estimate.json"
                args = SimpleNamespace(
                    estimator=estimator_dir,
                    trace=trace_path,
                    sp1_report=None,
                    out=output,
                )

                with mock.patch.object(
                    opcode_gas,
                    "verify_composite_estimator",
                    return_value=self.estimator,
                ), self.assertRaisesRegex(ValueError, "output"):
                    opcode_gas.cmd_estimate_composite_trace(args)

                self.assertEqual(trace_path.read_bytes(), trace_bytes)
                if case == "existing":
                    self.assertEqual(output.read_bytes(), b"preserve\n")
                elif case == "estimator_child":
                    self.assertFalse(output.exists())

    def test_trace_estimate_output_cannot_mutate_stateful_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            estimator_dir = pathlib.Path(temporary) / "estimator"
            estimator_dir.mkdir()
            output = STATEFUL_SOURCE / "unexpected-estimate.json"
            self.assertFalse(output.exists())
            with self.assertRaisesRegex(ValueError, "overlaps a sealed input"):
                opcode_gas._composite_output_path(
                    output,
                    estimator_directory=estimator_dir,
                    estimator=self.estimator,
                )
            self.assertFalse(output.exists())

    def test_trace_estimate_publishes_one_new_canonical_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            estimator_dir = root / "estimator"
            estimator_dir.mkdir()
            trace_path = root / "trace.json"
            trace_path.write_text(json.dumps(_complete_trace(), sort_keys=True) + "\n")
            output = root / "estimate.json"
            args = SimpleNamespace(
                estimator=estimator_dir,
                trace=trace_path,
                sp1_report=None,
                out=output,
            )
            with mock.patch.object(
                opcode_gas,
                "verify_composite_estimator",
                return_value=self.estimator,
            ):
                opcode_gas.cmd_estimate_composite_trace(args)
                published = output.read_bytes()
                self.assertEqual(
                    published,
                    opcode_gas._canonical_json_file_bytes(json.loads(published)),
                )
                with self.assertRaisesRegex(ValueError, "output already exists"):
                    opcode_gas.cmd_estimate_composite_trace(args)
                self.assertEqual(output.read_bytes(), published)

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
        args = parser.parse_args(
            [
                "seal-composite-estimator",
                "--augmented-core",
                str(CORRECTED_OSAKA_CORE_SOURCE),
                "--operation-coverage",
                str(COVERAGE_V5_SOURCE),
                "--higher-layer",
                str(HIGHER_LAYER_SOURCE),
                "--stateful-result",
                str(STATEFUL_SOURCE),
                "--context-approximation",
                str(CONTEXT_APPROXIMATION_SOURCE),
                "--guest-launcher",
                str(GUEST_LAUNCHER),
                "--proposal-elf",
                str(SP1_PROPOSAL_ELF),
                "--proposal-vk",
                str(SP1_PROPOSAL_VK),
                "--out-root",
                "estimators",
                "--estimator-path-file",
                "estimator-path",
            ]
        )
        self.assertEqual(args.context_approximation, CONTEXT_APPROXIMATION_SOURCE)
        self.assertEqual(args.guest_launcher, GUEST_LAUNCHER)
        self.assertEqual(args.proposal_elf, SP1_PROPOSAL_ELF)
        self.assertEqual(args.proposal_vk, SP1_PROPOSAL_VK)


if __name__ == "__main__":
    unittest.main()
