import copy
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
from fractions import Fraction
from unittest import mock

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

STATEFUL_MANIFEST = (
    ROOT / "experiments" / "opcode-gas" / "manifests" / "sp1-stateful-opcode-v1.json"
)
STATEFUL_REFERENCE_REGISTRY = ROOT / stateful.REFERENCE_REGISTRY["path"]


class StatefulOpcodeManifestTests(unittest.TestCase):
    def test_tracked_manifest_freezes_execution_and_source_registry(self):
        manifest = stateful.load_stateful_campaign_manifest(STATEFUL_MANIFEST)

        self.assertEqual(
            dict(manifest.execution),
            {
                "elf_path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
                "evm_spec": "osaka",
                "mode": "execute",
                "proof_type": "sp1",
                "sp1_execution_engine": "gas-estimator",
                "stage": "revm-opcode-lab",
            },
        )
        self.assertEqual(
            dict(manifest.reference_registry), stateful.REFERENCE_REGISTRY
        )
        self.assertEqual(
            dict(manifest.controlled_run_noise_floor),
            {
                "prover_gas": 143,
                "provenance_contract_id": (
                    "sp1-controlled-block-fixed-envelope-cross-input-v1"
                ),
                "scope": "signal_gate_only",
            },
        )
        with self.assertRaises(TypeError):
            stateful.CONTROLLED_RUN_NOISE_FLOOR["prover_gas"] = 0
        rows = stateful.stateful_campaign_row_specs(manifest)
        self.assertEqual(len(rows), 1_128)
        self.assertEqual(len({row.logical_identity for row in rows}), len(rows))
        self.assertEqual(
            {row.repeat_index for row in rows},
            {0, 1, 2},
        )

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

    def test_manifest_rejects_execution_or_registry_drift(self):
        mutations = (
            ("stage", "opcode-lab"),
            ("mode", "prove"),
            ("sp1_execution_engine", "standard"),
            ("evm_spec", "prague"),
            ("elf_path", "crates/guests/elf/sp1_opcode_lab.elf"),
        )
        for field, value in mutations:
            payload = stateful.canonical_stateful_manifest_payload()
            payload["execution"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, "frozen contract"
            ):
                stateful.StatefulCampaignManifest.from_mapping(payload)

        payload = stateful.canonical_stateful_manifest_payload()
        payload["reference_registry"]["artifact_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "frozen contract"):
            stateful.StatefulCampaignManifest.from_mapping(payload)

        for field, value in (
            ("prover_gas", 0),
            ("provenance_contract_id", "caller-selected"),
            ("scope", "model_fit"),
        ):
            payload = stateful.canonical_stateful_manifest_payload()
            payload["controlled_run_noise_floor"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, "frozen contract"
            ):
                stateful.StatefulCampaignManifest.from_mapping(payload)


class StatefulCampaignRowIdentityTests(unittest.TestCase):
    def test_row_identity_binds_every_execution_and_trace_input(self):
        base = {
            "scenario": "sload_warm_zero",
            "lane": "target",
            "relation_count": 4,
            "repeat_index": 2,
            "backend_input_sha256": "1" * 64,
            "elf_sha256": "2" * 64,
            "launcher_sha256": "3" * 64,
            "trace_sha256": "4" * 64,
        }
        identity = stateful.stateful_execution_row_identity(**base)

        mutations = {
            "scenario": "sload_warm_nonzero",
            "lane": "control",
            "relation_count": 8,
            "repeat_index": 1,
            "backend_input_sha256": "5" * 64,
            "elf_sha256": "6" * 64,
            "launcher_sha256": "7" * 64,
            "trace_sha256": "8" * 64,
        }
        for field, value in mutations.items():
            changed = dict(base)
            changed[field] = value
            with self.subTest(field=field):
                self.assertNotEqual(
                    stateful.stateful_execution_row_identity(**changed), identity
                )

        for invalid in (-1, 3):
            changed = dict(base)
            changed["repeat_index"] = invalid
            with self.assertRaisesRegex(ValueError, "repeat"):
                stateful.stateful_execution_row_identity(**changed)

    def _pair_specs(self, manifest):
        return tuple(
            row
            for row in stateful.stateful_campaign_row_specs(manifest)
            if row.scenario == "sload_warm_zero" and row.relation_count == 1
        )

    def _write_pair_fixtures(self, root, manifest):
        paths = {}
        fixtures = {}
        for lane in ("control", "target"):
            fixture = stateful.generate_stateful_fixture(
                manifest, "sload_warm_zero", lane=lane, count=1
            )
            directory = root / "sload_warm_zero" / "1" / lane
            directory.mkdir(parents=True)
            (directory / "case.json").write_bytes(
                opcode_gas.canonical_json(fixture["case_record"]) + b"\n"
            )
            input_path = directory / "guest-input.json"
            input_path.write_bytes(
                opcode_gas.canonical_json(fixture["guest_input"]) + b"\n"
            )
            paths[lane] = input_path
            fixtures[lane] = fixture
        return paths, fixtures

    @staticmethod
    def _formal_report(fixture, input_path, repeat_index, *, failed=False, semantic=True):
        bundle = _synthetic_bundle(fixture)
        report = copy.deepcopy(bundle["report"])
        report.update(
            {
                "stage": "revm-opcode-lab",
                "mode": "execute",
                "proof_mode": "compressed",
                "input": str(input_path),
                "exit_code": 1 if failed else 0,
                "gas": 10_000 + repeat_index,
                "primary_workload_metric": {
                    "label": "prover_gas",
                    "count": 10_000 + repeat_index,
                },
                "public_values": _synthetic_expected_public_values(fixture),
                "sp1_execution_engine": "gas-estimator",
                "sp1_gas_trace_chunk_threshold": opcode_gas.SP1_GAS_TRACE_CHUNK_THRESHOLD,
                "sp1_gas_trace_chunk_slots": opcode_gas.SP1_GAS_TRACE_CHUNK_SLOTS,
            }
        )
        if not semantic:
            report["controlled_trace"].pop("semantic_check")
        return report

    def test_row_runner_resumes_exact_rows_and_replays_identity_before_reading_reports(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        self.assertEqual(len(specs), 6)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fixture_paths, fixtures = self._write_pair_fixtures(
                root / "fixtures", manifest
            )
            run = root / "run"
            calls = []

            def replay(_launcher, input_path, _cache):
                calls.append(("replay", input_path))
                lane = json.loads(input_path.read_text())["storage"]["lane"]
                return _synthetic_bundle(fixtures[lane])

            def execute(**kwargs):
                calls.append(("execute", tuple(kwargs["input_paths"])))
                counters = {}
                with kwargs["reports_jsonl"].open("w") as output:
                    for input_path in kwargs["input_paths"]:
                        lane = json.loads(input_path.read_text())["storage"]["lane"]
                        repeat = counters.get(lane, 0)
                        counters[lane] = repeat + 1
                        output.write(
                            json.dumps(
                                self._formal_report(
                                    fixtures[lane], input_path, repeat
                                ),
                                sort_keys=True,
                            )
                            + "\n"
                        )

            first = stateful.run_stateful_campaign_rows(
                manifest,
                specs,
                fixtures_root=root / "fixtures",
                run=run,
                guest_launcher=root / "guest-launcher",
                elf=root / "guest.elf",
                launcher_sha256="a" * 64,
                elf_sha256="b" * 64,
                batch_executor=execute,
                identity_replayer=replay,
            )
            self.assertEqual(len(first), 6)
            for row in first:
                self.assertEqual(
                    row["normalized_report"]["repeat_index"], row["repeat_index"]
                )
                self.assertEqual(len(row["normalized_report"]["execution_row_id"]), 64)
            self.assertEqual([kind for kind, _ in calls[:2]], ["replay", "replay"])
            self.assertEqual(calls[2][0], "execute")

            calls.clear()
            resumed = stateful.run_stateful_campaign_rows(
                manifest,
                specs,
                fixtures_root=root / "fixtures",
                run=run,
                guest_launcher=root / "guest-launcher",
                elf=root / "guest.elf",
                launcher_sha256="a" * 64,
                elf_sha256="b" * 64,
                batch_executor=lambda **_kwargs: self.fail("exact resume re-executed a row"),
                identity_replayer=replay,
            )
            self.assertEqual(resumed, first)
            self.assertEqual([kind for kind, _ in calls], ["replay", "replay"])

            removed = []
            for candidate in (run / "rows").glob("*.json"):
                payload = json.loads(candidate.read_text())
                if payload["repeat_index"] == 2:
                    candidate.unlink()
                    removed.append(candidate)
            self.assertEqual(len(removed), 2)
            calls.clear()
            partial = stateful.run_stateful_campaign_rows(
                manifest,
                specs,
                fixtures_root=root / "fixtures",
                run=run,
                guest_launcher=root / "guest-launcher",
                elf=root / "guest.elf",
                launcher_sha256="a" * 64,
                elf_sha256="b" * 64,
                batch_executor=execute,
                identity_replayer=replay,
            )
            self.assertEqual(len(partial), 6)
            execution_calls = [value for kind, value in calls if kind == "execute"]
            self.assertEqual(len(execution_calls), 1)
            self.assertEqual(len(execution_calls[0]), 2)

            extra = run / "rows" / "extra.json"
            extra.write_text("{}\n")
            with self.assertRaisesRegex(ValueError, "unexpected row"):
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=root / "fixtures",
                    run=run,
                    guest_launcher=root / "guest-launcher",
                    elf=root / "guest.elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=lambda **_kwargs: None,
                    identity_replayer=replay,
                )
            extra.unlink()

            row_file = next((run / "rows").rglob("*.json"))
            row_file.write_text("not-json\n")
            calls.clear()

            def blocked_replay(_launcher, _input_path, _cache):
                raise ValueError("identity replay blocked before report read")

            with self.assertRaisesRegex(ValueError, "identity replay blocked"):
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=root / "fixtures",
                    run=run,
                    guest_launcher=root / "guest-launcher",
                    elf=root / "guest.elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=lambda **_kwargs: None,
                    identity_replayer=blocked_replay,
                )

    def test_row_runner_fails_closed_on_repeat_duplicate_execution_and_semantic_errors(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with self.assertRaisesRegex(ValueError, "three repeats"):
            stateful.validate_stateful_row_specs(specs[:-1], repeats=3)
        with self.assertRaisesRegex(ValueError, "duplicate"):
            stateful.validate_stateful_row_specs((*specs, specs[0]), repeats=3)

        for label, failed, semantic in (
            ("execution", True, True),
            ("semantic", False, False),
        ):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                _paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)

                def replay(_launcher, input_path, _cache):
                    lane = json.loads(input_path.read_text())["storage"]["lane"]
                    return _synthetic_bundle(fixtures[lane])

                def execute(**kwargs):
                    counters = {}
                    with kwargs["reports_jsonl"].open("w") as output:
                        for input_path in kwargs["input_paths"]:
                            lane = json.loads(input_path.read_text())["storage"]["lane"]
                            repeat = counters.get(lane, 0)
                            counters[lane] = repeat + 1
                            output.write(
                                json.dumps(
                                    self._formal_report(
                                        fixtures[lane],
                                        input_path,
                                        repeat,
                                        failed=failed,
                                        semantic=semantic,
                                    )
                                )
                                + "\n"
                            )

                with self.assertRaises(ValueError):
                    stateful.run_stateful_campaign_rows(
                        manifest,
                        specs,
                        fixtures_root=root / "fixtures",
                        run=root / "run",
                        guest_launcher=root / "guest-launcher",
                        elf=root / "guest.elf",
                        launcher_sha256="a" * 64,
                        elf_sha256="b" * 64,
                        batch_executor=execute,
                        identity_replayer=replay,
                    )

    def test_formal_report_rejects_wrong_execution_context_and_public_shape(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)
            report = self._formal_report(fixtures["target"], paths["target"], 0)
            stateful._portable_formal_report(
                report, input_path=paths["target"], fixtures_root=root / "fixtures"
            )
            for field, value in (
                ("stage", "opcode-lab"),
                ("mode", "prove"),
                ("proof_mode", "groth16"),
                ("public_values", "0xdeadbeef"),
            ):
                changed = copy.deepcopy(report)
                changed[field] = value
                with self.subTest(field=field), self.assertRaises(ValueError):
                    stateful._portable_formal_report(
                        changed,
                        input_path=paths["target"],
                        fixtures_root=root / "fixtures",
                    )
            for field in ("stage", "mode", "proof_mode", "public_values"):
                changed = copy.deepcopy(report)
                changed.pop(field)
                with self.subTest(missing=field), self.assertRaises(ValueError):
                    stateful._portable_formal_report(
                        changed,
                        input_path=paths["target"],
                        fixtures_root=root / "fixtures",
                    )

    def test_row_runner_rejects_changed_public_output_across_repeats(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            _paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)

            def replay(_launcher, input_path, _cache):
                lane = json.loads(input_path.read_text())["storage"]["lane"]
                return _synthetic_bundle(fixtures[lane])

            def execute(**kwargs):
                counters = {}
                with kwargs["reports_jsonl"].open("w") as output:
                    for input_path in kwargs["input_paths"]:
                        lane = json.loads(input_path.read_text())["storage"]["lane"]
                        repeat = counters.get(lane, 0)
                        counters[lane] = repeat + 1
                        report = self._formal_report(fixtures[lane], input_path, repeat)
                        if lane == "target" and repeat == 1:
                            report["public_values"] = "0x" + "00" * 32
                        output.write(json.dumps(report) + "\n")

            with self.assertRaisesRegex(ValueError, "canonical guest output"):
                stateful.run_stateful_campaign_rows(
                    manifest, specs, fixtures_root=root / "fixtures", run=root / "run",
                    guest_launcher=root / "launcher", elf=root / "elf",
                    launcher_sha256="a" * 64, elf_sha256="b" * 64,
                    batch_executor=execute, identity_replayer=replay,
                )
            self.assertEqual(list((root / "run" / "rows").glob("*.json")), [])

    def test_row_runner_rejects_consistently_wrong_canonical_public_output(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            _paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)

            def replay(_launcher, input_path, _cache):
                lane = json.loads(input_path.read_text())["storage"]["lane"]
                return _synthetic_bundle(fixtures[lane])

            def execute(**kwargs):
                counters = {}
                with kwargs["reports_jsonl"].open("w") as output:
                    for input_path in kwargs["input_paths"]:
                        lane = json.loads(input_path.read_text())["storage"]["lane"]
                        repeat = counters.get(lane, 0)
                        counters[lane] = repeat + 1
                        report = self._formal_report(
                            fixtures[lane], input_path, repeat
                        )
                        report["public_values"] = "0x" + "00" * 32
                        output.write(json.dumps(report) + "\n")

            with self.assertRaisesRegex(ValueError, "canonical guest output"):
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=root / "fixtures",
                    run=root / "run",
                    guest_launcher=root / "launcher",
                    elf=root / "elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=execute,
                    identity_replayer=replay,
                )
            self.assertEqual(list((root / "run" / "rows").glob("*.json")), [])

    def test_row_runner_rejects_symlinked_fixture_ancestor_before_replay(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            fixtures_root = root / "fixtures"
            self._write_pair_fixtures(fixtures_root, manifest)
            moved = root / "outside-scenario"
            (fixtures_root / "sload_warm_zero").rename(moved)
            (fixtures_root / "sload_warm_zero").symlink_to(moved, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink"):
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=fixtures_root,
                    run=root / "run",
                    guest_launcher=root / "launcher",
                    elf=root / "elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=lambda **_kwargs: self.fail("executed guest"),
                    identity_replayer=lambda *_args: self.fail("replayed identity"),
                )

    def test_row_runner_recovers_crash_between_pair_writes_without_rerunning_lane(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            _paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)
            calls = []

            def replay(_launcher, input_path, _cache):
                lane = json.loads(input_path.read_text())["storage"]["lane"]
                return _synthetic_bundle(fixtures[lane])

            def execute(**kwargs):
                calls.append(tuple(kwargs["input_paths"]))
                with kwargs["reports_jsonl"].open("w") as output:
                    for input_path in kwargs["input_paths"]:
                        lane = json.loads(input_path.read_text())["storage"]["lane"]
                        output.write(json.dumps(self._formal_report(
                            fixtures[lane], input_path, 0
                        )) + "\n")

            original_persist = opcode_gas.persist_immutable_bytes
            persisted = 0

            def crash_second(path, payload):
                nonlocal persisted
                persisted += 1
                if persisted == 2:
                    raise RuntimeError("injected second persist failure")
                return original_persist(path, payload)

            with mock.patch.object(opcode_gas, "persist_immutable_bytes", crash_second), self.assertRaisesRegex(
                RuntimeError, "injected"
            ):
                stateful.run_stateful_campaign_rows(
                    manifest, specs, fixtures_root=root / "fixtures", run=root / "run",
                    guest_launcher=root / "launcher", elf=root / "elf",
                    launcher_sha256="a" * 64, elf_sha256="b" * 64,
                    batch_executor=execute, identity_replayer=replay,
                    calibration_run_id="same-run",
                )
            self.assertEqual(len(list((root / "run" / "rows").glob("*.json"))), 1)
            calls.clear()
            rows = stateful.run_stateful_campaign_rows(
                manifest, specs, fixtures_root=root / "fixtures", run=root / "run",
                guest_launcher=root / "launcher", elf=root / "elf",
                launcher_sha256="a" * 64, elf_sha256="b" * 64,
                batch_executor=execute, identity_replayer=replay,
                calibration_run_id="same-run",
            )
            self.assertEqual(len(rows), 6)
            self.assertEqual([len(call) for call in calls], [5])
            fresh = stateful.run_stateful_campaign_rows(
                manifest, specs, fixtures_root=root / "fixtures", run=root / "fresh-run",
                guest_launcher=root / "launcher", elf=root / "elf",
                launcher_sha256="a" * 64, elf_sha256="b" * 64,
                batch_executor=execute, identity_replayer=replay,
                calibration_run_id="same-run",
            )
            self.assertEqual(rows, fresh)

    def test_row_runner_rejects_tampered_orphan_before_guest_execution(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        for label, mutate in (
            (
                "ordered pair identity",
                lambda payload: payload.__setitem__("ordered_pair_identity", "f" * 64),
            ),
            (
                "unexpected field",
                lambda payload: payload.__setitem__("unexpected", "field"),
            ),
        ):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                _paths, fixtures = self._write_pair_fixtures(
                    root / "fixtures", manifest
                )

                def replay(_launcher, input_path, _cache):
                    lane = json.loads(input_path.read_text())["storage"]["lane"]
                    return _synthetic_bundle(fixtures[lane])

                def execute(**kwargs):
                    counters = {}
                    with kwargs["reports_jsonl"].open("w") as output:
                        for input_path in kwargs["input_paths"]:
                            lane = json.loads(input_path.read_text())["storage"]["lane"]
                            repeat = counters.get(lane, 0)
                            counters[lane] = repeat + 1
                            output.write(
                                json.dumps(
                                    self._formal_report(
                                        fixtures[lane], input_path, repeat
                                    )
                                )
                                + "\n"
                            )

                run = root / "run"
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=root / "fixtures",
                    run=run,
                    guest_launcher=root / "launcher",
                    elf=root / "elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=execute,
                    identity_replayer=replay,
                )
                survivor = next(
                    spec
                    for spec in specs
                    if spec.repeat_index == 0 and spec.lane == "control"
                )
                missing = next(
                    spec
                    for spec in specs
                    if spec.repeat_index == 0 and spec.lane == "target"
                )
                survivor_path = stateful._row_path(run, survivor)
                payload = json.loads(survivor_path.read_text())
                mutate(payload)
                survivor_path.write_bytes(opcode_gas.canonical_json(payload) + b"\n")
                stateful._row_path(run, missing).unlink()
                executor_calls = 0

                def fail_executor(**_kwargs):
                    nonlocal executor_calls
                    executor_calls += 1
                    self.fail("tampered orphan reached guest execution")

                with self.assertRaisesRegex(ValueError, "persisted stateful lane"):
                    stateful.run_stateful_campaign_rows(
                        manifest,
                        specs,
                        fixtures_root=root / "fixtures",
                        run=run,
                        guest_launcher=root / "launcher",
                        elf=root / "elf",
                        launcher_sha256="a" * 64,
                        elf_sha256="b" * 64,
                        batch_executor=fail_executor,
                        identity_replayer=replay,
                    )
                self.assertEqual(executor_calls, 0)

    def test_row_runner_rejects_symlinked_rows_before_replay_or_write(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self._write_pair_fixtures(root / "fixtures", manifest)
            (root / "run").mkdir()
            outside = root / "outside-rows"
            outside.mkdir()
            (root / "run" / "rows").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "symlink"):
                stateful.run_stateful_campaign_rows(
                    manifest, specs, fixtures_root=root / "fixtures", run=root / "run",
                    guest_launcher=root / "launcher", elf=root / "elf",
                    launcher_sha256="a" * 64, elf_sha256="b" * 64,
                    batch_executor=lambda **_kwargs: self.fail("executed guest"),
                    identity_replayer=lambda *_args: self.fail("replayed identity"),
                )
            self.assertEqual(list(outside.iterdir()), [])

    def test_campaign_identity_recomputes_pinned_registry_content_hash(self):
        manifest = stateful.load_stateful_campaign_manifest(STATEFUL_MANIFEST)
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            manifest_path = root / "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json"
            manifest_path.parent.mkdir(parents=True)
            manifest_path.write_bytes(STATEFUL_MANIFEST.read_bytes())
            registry_path = root / stateful.REFERENCE_REGISTRY["path"]
            registry_path.parent.mkdir(parents=True)
            registry = json.loads((ROOT / stateful.REFERENCE_REGISTRY["path"]).read_text())
            registry["body_scale"] = "forged"
            registry_path.write_bytes(opcode_gas._canonical_json_file_bytes(registry))
            launcher = root / "launcher"
            launcher.write_bytes(b"launcher")
            elf = root / stateful.EXECUTION_CONTRACT["elf_path"]
            elf.parent.mkdir(parents=True)
            elf.write_bytes(b"elf")
            self._write_pair_fixtures(root / "fixtures", manifest)
            execution_identity = {
                "implementation_revision": "9" * 40,
                "guest_artifacts": {
                    stateful.EXECUTION_CONTRACT["elf_path"]: opcode_gas.sha256_file(elf)
                },
            }
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), self.assertRaisesRegex(
                ValueError, "artifact content hash"
            ):
                stateful._stateful_campaign_identity(
                    manifest_path=manifest_path,
                    manifest=manifest,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    specs=specs,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, path: opcode_gas.sha256_file(path),
                )

    def test_campaign_wrapper_create_only_terminal_and_portable_verification(self):
        manifest = stateful.load_stateful_campaign_manifest(STATEFUL_MANIFEST)
        specs = self._pair_specs(manifest)
        launcher = ROOT / "target" / "debug" / "guest-launcher"
        elf = ROOT / stateful.EXECUTION_CONTRACT["elf_path"]
        with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
            root = pathlib.Path(directory)
            _paths, fixtures = self._write_pair_fixtures(root / "fixtures", manifest)
            run = root / "run"

            def replay(_launcher, input_path, _cache):
                lane = json.loads(input_path.read_text())["storage"]["lane"]
                return _synthetic_bundle(fixtures[lane])

            def execute(**kwargs):
                counters = {}
                with kwargs["reports_jsonl"].open("w") as output:
                    for input_path in kwargs["input_paths"]:
                        lane = json.loads(input_path.read_text())["storage"]["lane"]
                        repeat = counters.get(lane, 0)
                        counters[lane] = repeat + 1
                        output.write(
                            json.dumps(
                                self._formal_report(fixtures[lane], input_path, repeat)
                            )
                            + "\n"
                        )

            execution_identity = {
                "implementation_revision": "9" * 40,
                "guest_launcher_sha256": opcode_gas.sha256_file(launcher),
                "guest_artifacts": {
                    stateful.EXECUTION_CONTRACT["elf_path"]: opcode_gas.sha256_file(elf)
                },
            }
            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ):
                result = stateful.run_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    batch_executor=execute,
                    identity_replayer=replay,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, _launcher: execution_identity[
                        "guest_launcher_sha256"
                    ],
                )
                verified = stateful.verify_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    identity_replayer=replay,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, _launcher: execution_identity[
                        "guest_launcher_sha256"
                    ],
                )

            self.assertEqual(result, verified)
            self.assertEqual(result["status"], "complete")
            self.assertEqual(result["row_count"], 6)
            self.assertTrue((run / "rows.jsonl").is_file())
            self.assertTrue((run / "decisions.json").is_file())
            self.assertTrue((run / "decisions.sha256").is_file())

            fit_kwargs = {
                "manifest_path": STATEFUL_MANIFEST,
                "calibration_run": root / "calibration",
                "fixtures_root": root / "fixtures",
                "guest_launcher": launcher,
                "elf": elf,
                "run": run,
                "identity_replayer": replay,
                "execution_identity_loader": lambda _run: execution_identity,
                "launcher_validator": lambda _identity, _launcher: execution_identity[
                    "guest_launcher_sha256"
                ],
            }
            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ), mock.patch.object(
                stateful,
                "_fit_stateful_task4_rows",
                return_value={"candidate_eligible": False},
            ):
                fitted = stateful.fit_stateful_task4_run(**fit_kwargs)
            provenance = fitted["task4_provenance"]
            self.assertEqual(
                provenance["campaign_identity_sha256"],
                result["campaign_identity_sha256"],
            )
            self.assertEqual(
                provenance["row_ledger_sha256"], result["row_ledger_sha256"]
            )
            self.assertEqual(
                provenance["terminal_artifact_file_sha256"]["rows.jsonl"],
                result["row_ledger_sha256"],
            )
            self.assertEqual(
                set(provenance["terminal_artifact_file_sha256"]),
                {"rows.jsonl", "decisions.json", "decisions.sha256"},
            )

            terminal_rows_path = run / "rows.jsonl"
            terminal_rows_bytes = terminal_rows_path.read_bytes()
            for coherent in (False, True):
                terminal_rows = list(opcode_gas.iter_jsonl(terminal_rows_path))
                changed = next(row for row in terminal_rows if row["lane"] == "target")
                changed["normalized_report"]["prover_gas"] += 100
                if coherent:
                    changed["formal_report"]["gas"] += 100
                    changed["formal_report"]["primary_workload_metric"]["count"] += 100
                    changed["normalized_report"]["gas"] += 100
                    changed["normalized_report"]["primary_workload_metric"][
                        "count"
                    ] += 100
                    changed["normalized_report"]["workload_value"] += 100
                    changed["formal_report_sha256"] = opcode_gas.sha256_bytes(
                        opcode_gas.canonical_json(changed["formal_report"])
                    )
                terminal_rows_path.write_bytes(
                    b"".join(
                        opcode_gas.canonical_json(row) + b"\n"
                        for row in terminal_rows
                    )
                )
                with self.subTest(coherent=coherent), mock.patch.object(
                    stateful, "stateful_campaign_row_specs", return_value=specs
                ), mock.patch.object(
                    stateful,
                    "_fit_stateful_task4_rows",
                    side_effect=AssertionError("unverified rows reached fitting"),
                ), self.assertRaisesRegex(ValueError, "terminal stateful row ledger"):
                    stateful.fit_stateful_task4_run(**fit_kwargs)
                terminal_rows_path.write_bytes(terminal_rows_bytes)

            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ):
                rerun = stateful.run_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    batch_executor=lambda **_kwargs: self.fail("terminal rerun executed"),
                    identity_replayer=replay,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, _launcher: execution_identity[
                        "guest_launcher_sha256"
                    ],
                )
            self.assertEqual(rerun, result)

            removed = {}
            for row_path in (run / "rows").glob("*.json"):
                payload = json.loads(row_path.read_text())
                if payload["repeat_index"] == 0:
                    removed[row_path] = row_path.read_bytes()
                    row_path.unlink()
            self.assertEqual(len(removed), 2)
            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ), self.assertRaisesRegex(ValueError, "missing row"):
                stateful.run_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    batch_executor=lambda **_kwargs: self.fail("sealed run executed"),
                    identity_replayer=replay,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, _launcher: execution_identity[
                        "guest_launcher_sha256"
                    ],
                )
            for row_path, contents in removed.items():
                row_path.write_bytes(contents)

            seal_path = run / "decisions.sha256"
            seal_bytes = seal_path.read_bytes()
            seal_path.unlink()
            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ), self.assertRaisesRegex(ValueError, "artifact set is incomplete"):
                stateful.run_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    batch_executor=lambda **_kwargs: self.fail("partial terminal executed"),
                    identity_replayer=lambda *_args: self.fail("partial terminal replayed"),
                    execution_identity_loader=lambda _run: self.fail("partial terminal read identity"),
                    launcher_validator=lambda *_args: self.fail("partial terminal validated launcher"),
                )
            seal_path.write_bytes(seal_bytes)

            decisions = json.loads((run / "decisions.json").read_text())
            decisions["row_count"] = 5
            (run / "decisions.json").write_text(json.dumps(decisions) + "\n")
            with mock.patch.object(
                stateful, "stateful_campaign_row_specs", return_value=specs
            ), self.assertRaises(ValueError):
                stateful.verify_stateful_opcode_campaign(
                    manifest_path=STATEFUL_MANIFEST,
                    calibration_run=root / "calibration",
                    fixtures_root=root / "fixtures",
                    guest_launcher=launcher,
                    elf=elf,
                    run=run,
                    identity_replayer=replay,
                    execution_identity_loader=lambda _run: execution_identity,
                    launcher_validator=lambda _identity, _launcher: execution_identity[
                        "guest_launcher_sha256"
                    ],
                )

    def test_campaign_rejects_symlinked_canonical_input(self):
        manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        specs = self._pair_specs(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            paths, _fixtures = self._write_pair_fixtures(root / "fixtures", manifest)
            real_input = paths["target"]
            moved = real_input.with_name("real-input.json")
            real_input.rename(moved)
            real_input.symlink_to(moved)
            with self.assertRaisesRegex(ValueError, "symlink"):
                stateful.run_stateful_campaign_rows(
                    manifest,
                    specs,
                    fixtures_root=root / "fixtures",
                    run=root / "run",
                    guest_launcher=root / "launcher",
                    elf=root / "elf",
                    launcher_sha256="a" * 64,
                    elf_sha256="b" * 64,
                    batch_executor=lambda **_kwargs: None,
                    identity_replayer=lambda *_args: {},
                )


class StatefulExactReferenceTests(unittest.TestCase):
    def setUp(self):
        self.registry_artifact = json.loads(STATEFUL_REFERENCE_REGISTRY.read_text())
        self.registry = stateful.load_stateful_reference_registry(
            self.registry_artifact,
            expected_artifact_sha256=stateful.REFERENCE_REGISTRY["artifact_sha256"],
        )

    def test_fraction_payload_is_exact_and_replays_80_digit_projection(self):
        payload = stateful.exact_fraction_payload(Fraction(1, 3))

        self.assertEqual(payload["numerator"], "1")
        self.assertEqual(payload["denominator"], "3")
        self.assertEqual(
            payload["decimal"],
            "0.33333333333333333333333333333333333333333333333333333333333333333333333333333333",
        )
        self.assertEqual(stateful.replay_exact_fraction(payload), Fraction(1, 3))

        rounded = dict(payload)
        rounded["decimal"] = "0.333333"
        with self.assertRaisesRegex(ValueError, "replay residual"):
            stateful.replay_exact_fraction(rounded)

        noncanonical = dict(payload)
        noncanonical["decimal"] = payload["decimal"][:-1] + "34"
        with self.assertRaisesRegex(ValueError, "80-digit projection"):
            stateful.replay_exact_fraction(noncanonical)

    def test_fraction_payload_projects_high_magnitude_recurring_values(self):
        for value in (Fraction(1_528_426, 7), Fraction(-1_528_426, 7)):
            with self.subTest(value=value):
                payload = stateful.exact_fraction_payload(value)
                fractional = payload["decimal"].lstrip("-").split(".", 1)[1]
                self.assertEqual(len(fractional), 80)
                self.assertEqual(stateful.replay_exact_fraction(payload), value)
                self.assertLessEqual(
                    abs(Fraction(payload["decimal"]) - value),
                    stateful.STATEFUL_REPLAY_RESIDUAL_MAX,
                )

    def test_typed_reference_lookup_reconstructs_signed_cost_and_absolute_slope(self):
        common = Fraction(self.registry_artifact["registry"]["common_dispatch"])
        pop_body = Fraction(
            self.registry_artifact["registry"]["models"]["opcode:0x50"]
            ["parameters"]["body_per_raw_gas"]
        )
        ledger = {"opcode:0x50": -2, "opcode:0x55": 1, "opcode:0x5b": 1}
        typed_inputs = {
            "opcode:0x50": {
                "artifact_sha256": stateful.REFERENCE_REGISTRY["artifact_sha256"],
                "model_id": "opcode:0x50",
                "model_kind": "static_raw_gas",
                "input": {"raw_gas": 2},
            },
            "opcode:0x5b": {
                "artifact_sha256": stateful.REFERENCE_REGISTRY["artifact_sha256"],
                "model_id": "opcode:0x5b",
                "model_kind": "static_raw_gas",
                "input": {"raw_gas": 1},
            },
        }

        resolved = stateful.resolve_signed_reference_cost(
            self.registry,
            ledger,
            state_opcode_key="opcode:0x55",
            typed_inputs=typed_inputs,
        )

        expected = -2 * (common + 2 * pop_body) + common
        self.assertEqual(
            resolved["signed_reference_cost_exact"],
            stateful.exact_fraction_payload(expected),
        )
        self.assertEqual(
            [entry["opcode_key"] for entry in resolved["entries"]],
            ["opcode:0x50", "opcode:0x5b"],
        )
        self.assertEqual(
            resolved["entries"][0]["model_parameters_exact"],
            {"body_per_raw_gas": stateful.exact_fraction_payload(pop_body)},
        )
        self.assertEqual(
            resolved["entries"][0]["common_dispatch_exact"],
            stateful.exact_fraction_payload(common),
        )
        self.assertIn("predictor_decimal", resolved["entries"][0])
        self.assertLessEqual(
            stateful.replay_exact_fraction(
                resolved["entries"][0]["predictor_residual_exact"]
            ),
            stateful.STATEFUL_REPLAY_RESIDUAL_MAX,
        )
        self.assertEqual(sum(ledger.values()), 0, "common dispatch must cancel")

        fit = stateful.fit_stateful_scenario(
            scenario="synthetic_store",
            fit_deltas={
                count: [str(7 + count * 17)] * 3
                for count in stateful.FIT_COUNTS
            },
            reference_cost=Fraction(-3),
        )
        self.assertEqual(
            fit["relative_slope_exact"],
            stateful.exact_fraction_payload(Fraction(17)),
        )
        self.assertEqual(
            fit["nuisance_intercept_exact"],
            stateful.exact_fraction_payload(Fraction(7)),
        )
        self.assertEqual(
            fit["absolute_stateful_cost_exact"],
            stateful.exact_fraction_payload(Fraction(20)),
        )

    def test_reference_lookup_rejects_missing_unsupported_and_non_exact_inputs(self):
        valid = {
            "artifact_sha256": stateful.REFERENCE_REGISTRY["artifact_sha256"],
            "model_id": "opcode:0x19",
            "model_kind": "static_raw_gas",
            "input": {"raw_gas": 3},
        }
        for label, opcode_key, typed, message in (
            ("missing", "opcode:0x0c", valid, "missing"),
            ("unsupported", "opcode:0x00", valid, "unsupported"),
            (
                "wrong model",
                "opcode:0x19",
                {**valid, "model_id": "opcode:0x50"},
                "model",
            ),
            (
                "non-exact input",
                "opcode:0x19",
                {**valid, "input": {"raw_gas": "3"}},
                "typed input",
            ),
            (
                "structured without trace features",
                "opcode:0x0a",
                {
                    **valid,
                    "model_id": "opcode:0x0a",
                    "model_kind": "exp",
                    "input": {"exponent_byte_length": 1},
                },
                "structured",
            ),
        ):
            with self.subTest(label=label), self.assertRaisesRegex(
                ValueError, message
            ):
                stateful.evaluate_typed_reference(self.registry, opcode_key, typed)

        with self.assertRaisesRegex(ValueError, "zero sum"):
            stateful.resolve_signed_reference_cost(
                self.registry,
                {"opcode:0x19": -1, "opcode:0x54": 1, "opcode:0x5b": 1},
                state_opcode_key="opcode:0x54",
                typed_inputs={"opcode:0x19": valid, "opcode:0x5b": valid},
            )

    def test_loaded_registry_is_immutable_after_caller_artifact_mutation(self):
        typed = {
            "artifact_sha256": stateful.REFERENCE_REGISTRY["artifact_sha256"],
            "model_id": "opcode:0x50",
            "model_kind": "static_raw_gas",
            "input": {"raw_gas": 2},
        }
        before = stateful.evaluate_typed_reference(
            self.registry, "opcode:0x50", typed
        )

        self.registry_artifact["registry"]["common_dispatch"] = "999999"
        self.registry_artifact["registry"]["models"]["opcode:0x50"]["parameters"][
            "body_per_raw_gas"
        ] = "999999"

        self.assertEqual(
            stateful.evaluate_typed_reference(
                self.registry, "opcode:0x50", typed
            ),
            before,
        )


class StatefulScenarioGateTests(unittest.TestCase):
    @staticmethod
    def deltas(*, slope=20, intercept=10):
        return {
            count: [str(intercept + count * slope)] * stateful.REPEATS
            for count in stateful.PRIMARY_COUNTS
        }

    def gate(self, deltas=None, *, reference_cost=Fraction(-2)):
        return stateful.fit_and_gate_stateful_scenario(
            scenario="synthetic",
            deltas=self.deltas() if deltas is None else deltas,
            reference_cost=reference_cost,
        )

    def test_exact_linear_scenario_passes_every_predeclared_gate(self):
        report = self.gate()

        self.assertTrue(report["passed"])
        self.assertEqual(report["failures"], [])
        self.assertEqual(
            report["fit"]["r2_exact"],
            stateful.exact_fraction_payload(Fraction(1)),
        )
        self.assertEqual(
            report["fit"]["absolute_stateful_cost_exact"],
            stateful.exact_fraction_payload(Fraction(22)),
        )
        for gate in (
            "positive_signal",
            "r2",
            "relative_slope_standard_error",
            "residual_signal",
            "count_zero_intercept",
            "repeat_spread",
            "holdout_ape",
            "checkpoint_ape",
        ):
            self.assertTrue(report["gates"][gate]["passed"], gate)
        self.assertEqual(
            report["gates"]["positive_signal"]["threshold_exact"],
            stateful.exact_fraction_payload(Fraction(143)),
        )
        self.assertEqual(
            report["gates"]["r2"]["threshold_exact"],
            stateful.exact_fraction_payload(Fraction(995, 1000)),
        )
        self.assertEqual(
            report["controlled_run_noise_floor"],
            stateful.CONTROLLED_RUN_NOISE_FLOOR,
        )

    def test_each_scenario_quality_gate_rejects_its_synthetic_failure(self):
        cases = []

        low_signal = self.deltas(slope=5)
        cases.append(("positive_signal", low_signal, Fraction(-2)))

        noisy_fit = self.deltas()
        noisy_fit[8] = ["270", "170", "170"]
        cases.extend(
            (gate, noisy_fit, Fraction(-2))
            for gate in (
                "r2",
                "relative_slope_standard_error",
                "residual_signal",
            )
        )

        bad_zero = self.deltas()
        bad_zero[0] = ["100", "100", "100"]
        cases.append(("count_zero_intercept", bad_zero, Fraction(-2)))

        one_bad_zero_repeat = self.deltas()
        one_bad_zero_repeat[0] = ["10", "10", "100"]
        cases.append(
            ("count_zero_intercept", one_bad_zero_repeat, Fraction(-2))
        )

        spread = self.deltas()
        spread[32] = ["650", "700", "750"]
        cases.append(("repeat_spread", spread, Fraction(-2)))

        holdout = self.deltas()
        holdout[32] = ["810", "810", "810"]
        cases.append(("holdout_ape", holdout, Fraction(-2)))

        checkpoint = self.deltas()
        checkpoint[64] = ["1610", "1610", "1610"]
        cases.append(("checkpoint_ape", checkpoint, Fraction(-2)))

        for gate, deltas, reference_cost in cases:
            with self.subTest(gate=gate):
                report = self.gate(deltas, reference_cost=reference_cost)
                self.assertFalse(report["gates"][gate]["passed"])
                self.assertIn(gate, report["failures"])

    def test_zero_denominators_fail_closed_without_division_errors(self):
        zero_signal = self.deltas(slope=0)
        report = self.gate(zero_signal, reference_cost=Fraction(0))
        self.assertIn("zero_signal", report["failures"])
        self.assertFalse(report["gates"]["residual_signal"]["passed"])
        self.assertFalse(report["gates"]["count_zero_intercept"]["passed"])

        zero_absolute = self.gate(reference_cost=Fraction(20))
        self.assertIn("zero_absolute_stateful_cost", zero_absolute["failures"])
        self.assertFalse(zero_absolute["gates"]["repeat_spread"]["passed"])

        zero_holdout = self.deltas()
        zero_holdout[32] = ["10", "10", "10"]
        report = self.gate(zero_holdout)
        self.assertIn("zero_holdout_marginal", report["failures"])
        self.assertFalse(report["gates"]["holdout_ape"]["passed"])

        zero_checkpoint = self.deltas()
        zero_checkpoint[64] = ["10", "10", "10"]
        report = self.gate(zero_checkpoint)
        self.assertIn("zero_checkpoint_marginal", report["failures"])
        self.assertFalse(report["gates"]["checkpoint_ape"]["passed"])


class StatefulModelComparisonTests(unittest.TestCase):
    def setUp(self):
        self.manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )

    @staticmethod
    def branch(scenario):
        if scenario.operation_kind == "load":
            return "load"
        if scenario.current_value != scenario.original_value:
            return (
                "restore_original"
                if scenario.new_value == scenario.original_value
                else "dirty_rewrite"
            )
        if scenario.new_value == scenario.current_value:
            return "noop"
        if scenario.current_value == 0:
            return "set"
        if scenario.new_value == 0:
            return "clear"
        return "reset"

    def scenario_data(self, family):
        branch_costs = {
            "noop": 200,
            "set": 300,
            "clear": 400,
            "reset": 500,
            "dirty_rewrite": 600,
            "restore_original": 700,
        }
        data = {}
        for index, scenario in enumerate(self.manifest.scenarios):
            if family == "fixed":
                absolute = 100 if scenario.operation_kind == "load" else 200
            elif family == "access":
                absolute = (
                    100 + (30 if scenario.access == "cold" else 0)
                    if scenario.operation_kind == "load"
                    else 200 + (40 if scenario.access == "cold" else 0)
                )
            else:
                absolute = (
                    100 + (30 if scenario.access == "cold" else 0)
                    if scenario.operation_kind == "load"
                    else branch_costs[self.branch(scenario)]
                    + (40 if scenario.access == "cold" else 0)
                )
            intercept = 10 + index
            data[scenario.name] = {
                "deltas": {
                    count: [str(intercept + absolute * count)] * stateful.REPEATS
                    for count in scenario.counts
                },
                "signed_execution_ledger": stateful._reference_ledger(scenario),
                "signed_reference_cost_exact": stateful.exact_fraction_payload(
                    Fraction(0)
                ),
            }
        return data

    def fit(self, family):
        return stateful.fit_stateful_model_report(
            self.manifest,
            self.scenario_data(family),
        )

    def test_joint_models_share_parameters_and_use_fixed_selection_order(self):
        fixed = self.fit("fixed")
        access = self.fit("access")
        typed = self.fit("typed")

        self.assertEqual(fixed["selection"]["selected_model"], "M_fixed")
        self.assertEqual(access["selection"]["selected_model"], "M_access")
        self.assertEqual(typed["selection"]["selected_model"], "M_typed")
        self.assertEqual(
            typed["selection"]["order"], ["M_fixed", "M_access", "M_typed"]
        )
        self.assertFalse(typed["candidate_eligible"])
        self.assertEqual(
            typed["controlled_run_noise_floor"],
            stateful.CONTROLLED_RUN_NOISE_FLOOR,
        )

        typed_parameters = typed["model_reports"]["M_typed"]["parameters_exact"]
        self.assertEqual(
            typed_parameters["sload_warm_body"],
            stateful.exact_fraction_payload(Fraction(100)),
        )
        self.assertEqual(
            typed_parameters["sstore_cold_extra"],
            stateful.exact_fraction_payload(Fraction(40)),
        )
        self.assertEqual(
            typed_parameters["sstore_branch:set"],
            stateful.exact_fraction_payload(Fraction(300)),
        )
        self.assertEqual(
            typed["model_reports"]["M_typed"]["cold_increment_diagnostic"]
            ["difference_exact"],
            stateful.exact_fraction_payload(Fraction(-10)),
        )
        self.assertEqual(
            typed["model_reports"]["M_typed"]["maximum_ape_threshold_exact"],
            stateful.exact_fraction_payload(Fraction(1, 10)),
        )
        nuisance = typed["model_reports"]["M_typed"]["nuisance_intercepts_exact"]
        self.assertEqual(
            nuisance["sload_cold_zero"],
            stateful.exact_fraction_payload(Fraction(10)),
        )
        self.assertEqual(
            nuisance["sstore_restore_nonzero"],
            stateful.exact_fraction_payload(Fraction(27)),
        )

        raw = typed["model_reports"]["M_raw_gas_diagnostic"]
        self.assertTrue(raw["diagnostic_only"])
        self.assertFalse(raw["eligible"])
        self.assertEqual(set(raw["parameters_exact"]), {"alpha", "beta"})

    def test_all_families_are_frozen_before_holdout_and_never_refit(self):
        data = self.scenario_data("typed")
        original = stateful.fit_stateful_model_report(
            self.manifest, data
        )
        changed = copy.deepcopy(data)
        changed["sstore_set_warm"]["deltas"][stateful.HOLDOUT_COUNT] = [
            "999999",
            "999999",
            "999999",
        ]
        rejected = stateful.fit_stateful_model_report(
            self.manifest, changed
        )

        self.assertEqual(original["frozen_fit_sha256"], rejected["frozen_fit_sha256"])
        self.assertEqual(
            original["frozen_fit_models"], rejected["frozen_fit_models"]
        )
        self.assertTrue(original["model_reports"]["M_typed"]["eligible"])
        self.assertFalse(rejected["model_reports"]["M_typed"]["eligible"])
        self.assertIn(
            "holdout_ape",
            rejected["model_reports"]["M_typed"]["rejection_reasons"],
        )

    def test_model_report_rejects_direct_manifest_noise_floor_forgery(self):
        forged = replace(
            self.manifest,
            controlled_run_noise_floor={
                **stateful.CONTROLLED_RUN_NOISE_FLOOR,
                "prover_gas": 0,
            },
        )
        with self.assertRaisesRegex(ValueError, "noise floor contract"):
            stateful.fit_stateful_model_report(
                forged,
                self.scenario_data("fixed"),
            )

    def test_high_limb_required_scenarios_and_no_eligible_model_fail_closed(self):
        data = self.scenario_data("typed")
        high = copy.deepcopy(data)
        high["sstore_set_high_value_warm"]["deltas"][
            stateful.CHECKPOINT_COUNT
        ] = ["999999", "999999", "999999"]
        report = stateful.fit_stateful_model_report(
            self.manifest, high
        )
        self.assertFalse(report["model_reports"]["M_typed"]["eligible"])
        self.assertIn(
            "high_limb_ape",
            report["model_reports"]["M_typed"]["rejection_reasons"],
        )

        none = copy.deepcopy(data)
        for scenario in self.manifest.scenarios:
            if scenario.diagnostic:
                continue
            none[scenario.name]["deltas"][stateful.HOLDOUT_COUNT] = [
                "999999",
                "999999",
                "999999",
            ]
        report = stateful.fit_stateful_model_report(
            self.manifest, none
        )
        self.assertEqual(report["selection"]["status"], "no_eligible_model")
        self.assertIsNone(report["selection"]["selected_model"])

        incomplete = self.scenario_data("fixed")
        incomplete.pop("sload_warm_zero")
        with self.assertRaisesRegex(ValueError, "required scenario"):
            stateful.fit_stateful_model_report(
                self.manifest, incomplete
            )

    def test_low_high_consistency_is_separate_from_marginal_signal_ape(self):
        data = self.scenario_data("typed")
        high_name = "sstore_set_high_value_warm"
        zero = Fraction(data[high_name]["deltas"][0][0])
        observed_absolute = Fraction(663, 2)  # 10.5% above the typed set cost.
        data[high_name]["deltas"][stateful.CHECKPOINT_COUNT] = [
            str(zero + stateful.CHECKPOINT_COUNT * observed_absolute)
        ] * stateful.REPEATS

        report = stateful.fit_stateful_model_report(
            self.manifest, data
        )
        typed = report["model_reports"]["M_typed"]

        self.assertLessEqual(
            stateful.replay_exact_fraction(typed["maximum_high_limb_ape_exact"]),
            Fraction(1, 10),
        )
        self.assertGreater(
            stateful.replay_exact_fraction(
                typed["maximum_high_limb_consistency_error_exact"]
            ),
            Fraction(1, 10),
        )
        self.assertIn("high_limb_consistency", typed["rejection_reasons"])
        row = next(
            item
            for item in typed["high_limb_comparisons"]
            if item["scenario"] == high_name
        )
        self.assertEqual(
            stateful.replay_exact_fraction(row["observed_absolute_cost_exact"]),
            observed_absolute,
        )

    def test_high_limb_zero_model_cost_denominator_fails_closed(self):
        scenario = self.manifest.scenario("sload_warm_high_value")
        comparison = stateful._high_limb_model_comparison(
            scenario=scenario,
            count=stateful.CHECKPOINT_COUNT,
            zero_deltas=(Fraction(7), Fraction(7), Fraction(7)),
            checkpoint_deltas=(Fraction(71), Fraction(71), Fraction(71)),
            predicted_absolute=Fraction(0),
            reference_cost=Fraction(1),
        )

        self.assertIsNone(comparison["consistency_error_exact"])
        self.assertTrue(comparison["zero_model_cost_denominator"])


class StatefulPairLedgerExtractionTests(unittest.TestCase):
    def setUp(self):
        self.manifest = stateful.StatefulCampaignManifest.from_mapping(
            stateful.canonical_stateful_manifest_payload()
        )
        artifact = json.loads(STATEFUL_REFERENCE_REGISTRY.read_text())
        self.registry = stateful.load_stateful_reference_registry(
            artifact,
            expected_artifact_sha256=stateful.REFERENCE_REGISTRY["artifact_sha256"],
        )

    @staticmethod
    def raw_gas_for(scenario, opcode_key):
        opcode = int(opcode_key.removeprefix("opcode:0x"), 16)
        if opcode == scenario.measurement_opcode:
            return scenario.target_raw_gas
        return {
            stateful.NOT: 3,
            stateful.POP: 2,
            stateful.JUMPDEST: 1,
            stateful.PUSH32: 3,
            stateful.STOP: 0,
        }[opcode]

    def rows(self, scenario_name):
        scenario = self.manifest.scenario(scenario_name)
        rows = []
        for count in scenario.counts:
            fixtures = {
                lane: stateful.generate_stateful_fixture(
                    self.manifest, scenario_name, lane=lane, count=count
                )
                for lane in ("target", "control")
            }
            for repeat_index in range(self.manifest.repeats):
                for lane in ("target", "control"):
                    fixture = fixtures[lane]
                    counts = stateful._expected_executed_opcode_counts(
                        fixture["case_record"]
                    )
                    raw = {
                        key: value * self.raw_gas_for(scenario, key)
                        for key, value in counts.items()
                    }
                    base = 10_000 + repeat_index
                    delta = 10 + 200 * count
                    rows.append(
                        {
                            "scenario": scenario_name,
                            "lane": lane,
                            "relation_count": count,
                            "repeat_index": repeat_index,
                            "ordered_pair_identity": hashlib.sha256(
                                f"{scenario_name}:{count}:{repeat_index}".encode()
                            ).hexdigest(),
                            "normalized_report": {
                                "prover_gas": base + (delta if lane == "target" else 0)
                            },
                            "formal_report": {
                                "controlled_trace": {
                                    "executed_opcode_counts": counts,
                                    "executed_opcode_raw_gas": raw,
                                }
                            },
                        }
                    )
        return rows

    def test_task4_rows_reconstruct_every_pair_ledger_and_exact_typed_reference(self):
        rows = [
            *self.rows("sload_warm_zero"),
            *self.rows("sstore_set_warm"),
        ]

        extracted = stateful.extract_stateful_pair_observations(
            self.manifest, rows, self.registry
        )

        self.assertEqual(len(extracted["pair_ledgers"]), 2 * 8 * 3)
        self.assertEqual(
            extracted["scenario_data"]["sload_warm_zero"][
                "signed_execution_ledger"
            ],
            {"opcode:0x19": -1, "opcode:0x54": 1},
        )
        self.assertEqual(
            extracted["scenario_data"]["sstore_set_warm"][
                "signed_execution_ledger"
            ],
            {"opcode:0x50": -2, "opcode:0x55": 1, "opcode:0x5b": 1},
        )
        typed = extracted["typed_references"]["sstore_set_warm"]
        self.assertEqual(typed["opcode:0x50"]["input"], {"raw_gas": 2})
        self.assertEqual(typed["opcode:0x5b"]["input"], {"raw_gas": 1})
        zero_pairs = [
            row
            for row in extracted["pair_ledgers"]
            if row["relation_count"] == 0
        ]
        self.assertTrue(zero_pairs)
        self.assertTrue(
            all(row["signed_execution_total"] == {} for row in zero_pairs)
        )

    def test_raw_gas_quotients_require_positive_counts_integrality_and_consistency(self):
        rows = self.rows("sload_warm_zero")
        changed = copy.deepcopy(rows)
        target = next(
            row
            for row in changed
            if row["lane"] == "target"
            and row["relation_count"] == 2
            and row["repeat_index"] == 1
        )
        target["formal_report"]["controlled_trace"]["executed_opcode_raw_gas"][
            "opcode:0x19"
        ] += 1
        with self.assertRaisesRegex(ValueError, "raw-gas quotient"):
            stateful.extract_stateful_pair_observations(
                self.manifest, changed, self.registry
            )

        zero_only = [row for row in rows if row["relation_count"] == 0]
        with self.assertRaisesRegex(ValueError, "positive-count"):
            stateful.extract_stateful_pair_observations(
                self.manifest, zero_only, self.registry
            )

        inconsistent = copy.deepcopy(rows)
        target = next(
            row
            for row in inconsistent
            if row["lane"] == "target"
            and row["relation_count"] == 2
            and row["repeat_index"] == 1
        )
        target["formal_report"]["controlled_trace"]["executed_opcode_raw_gas"][
            "opcode:0x19"
        ] += 2
        with self.assertRaisesRegex(ValueError, "differs across"):
            stateful.extract_stateful_pair_observations(
                self.manifest, inconsistent, self.registry
            )

    def test_full_task4_rows_bind_reference_evidence_into_derived_report(self):
        rows = [
            row
            for scenario in self.manifest.scenarios
            for row in self.rows(scenario.name)
        ]
        artifact = json.loads(STATEFUL_REFERENCE_REGISTRY.read_text())

        report = stateful._fit_stateful_task4_rows(
            self.manifest,
            rows,
            artifact,
        )

        self.assertEqual(report["reference_registry"], stateful.REFERENCE_REGISTRY)
        self.assertEqual(len(report["pair_ledgers"]), 564)
        self.assertEqual(set(report["reference_evidence"]), set(EXPECTED_SCENARIOS))
        self.assertEqual(report["selection"]["selected_model"], "M_fixed")
        self.assertFalse(report["candidate_eligible"])
        self.assertNotIn("raw_rows", report)


class StatefulResultSealingTests(unittest.TestCase):
    EXPECTED_INVENTORY = {
        "result.json",
        "campaign-manifest.json",
        "calibration-identity.json",
        "campaign-identity.json",
        "rows.jsonl",
        "campaign-decisions.json",
        "campaign-decisions.sha256",
        "source-registry.json",
        "model-report.json",
    }

    @classmethod
    def setUpClass(cls):
        cls.manifest = stateful.load_stateful_campaign_manifest(STATEFUL_MANIFEST)
        cls.revision = "a" * 40
        cls.launcher_bytes = b"reviewed synthetic host-native identity helper\n"
        cls.launcher_sha256 = opcode_gas.sha256_bytes(cls.launcher_bytes)
        cls.elf_sha256 = opcode_gas.sha256_file(
            ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf"
        )
        schedule = opcode_gas.current_uzen_schedule()
        controlled_manifest = (
            ROOT
            / "experiments/opcode-gas/manifests/sp1-calibration-v1.toml"
        )
        controlled_data = opcode_gas.tomllib.loads(controlled_manifest.read_text())
        bridge_key_ids = controlled_data["bridge_key_ids"]
        guest_artifacts = {
            str(path.relative_to(ROOT)): opcode_gas.sha256_file(path)
            for path in sorted((ROOT / "crates/guests/elf").glob("sp1*"))
            if path.is_file()
            and (path.name.endswith(".elf") or path.name.endswith(".vk.bin"))
        }
        cls.calibration_identity = {
            "implementation_revision": cls.revision,
            "alethia_reth_revision": opcode_gas.tomllib.loads(
                (ROOT / "Cargo.toml").read_text()
            )["workspace"]["dependencies"]["alethia-reth-chainspec"]["rev"],
            "rust_version": opcode_gas._rust_version(),
            "sp1_sdk_version": opcode_gas._locked_package_version("sp1-sdk"),
            "controlled_manifest_sha256": opcode_gas.sha256_file(
                controlled_manifest
            ),
            "controlled_manifest_rows_sha256": (
                opcode_gas.controlled_manifest_rows_sha256(controlled_manifest)
            ),
            "complete_schedule_sha256": opcode_gas.schedule_sha256(schedule),
            "guest_artifacts": guest_artifacts,
            "guest_artifacts_sha256": opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(guest_artifacts)
            ),
            "guest_launcher_sha256": cls.launcher_sha256,
            "normalization_reference_key": "opcode:0x01",
            "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
            "primary_metric": "proverGas",
            "sp1_instruction_count": "secondary_non_gating",
            "workload_identity_schema_version": 1,
            "workload_canonicalization": "sha256(canonical_json(workload_spec))",
            "primary_formulas": {
                "candidate_cost": "g_p(k) / r(k)",
                "candidate_multiplier": "c_p(k) / c_p(opcode:0x01)",
            },
            "q_formula": list(opcode_gas.Q_FORMULA),
            "out_of_fit_checkpoint": {
                "mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS,
                "ape_max": 0.10,
            },
            "quality_gates": {"checkpoint_ape_max": 0.10},
            "bridge": {
                "bridge_key_ids": bridge_key_ids,
                "model": "through_origin_equal_key_median",
                "controlled_ape_max": "0.10",
                "proposal_ape_max": "0.10",
                "missing_data": "insufficient_data_is_sealable_and_non_gating",
            },
            "version_identity": opcode_gas.calibration_version_identity(schedule),
        }
        cls.calibration_id = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(cls.calibration_identity)
        )[:24]
        registry = json.loads(STATEFUL_REFERENCE_REGISTRY.read_text())
        typed_registry = stateful.load_stateful_reference_registry(
            registry,
            expected_artifact_sha256=stateful.REFERENCE_REGISTRY[
                "artifact_sha256"
            ],
        )
        specs = stateful.stateful_campaign_row_specs(cls.manifest)
        specs_by_key = {
            (spec.scenario, spec.relation_count, spec.repeat_index, spec.lane): spec
            for spec in specs
        }
        fixture_cache = {}
        bundle_by_input_bytes = {}
        rows_by_identity = {}
        fixture_inventory = []
        with tempfile.TemporaryDirectory() as directory:
            fixtures_root = pathlib.Path(directory) / "fixtures"
            for scenario in cls.manifest.scenarios:
                for count in scenario.counts:
                    lane_data = {}
                    for lane in ("control", "target"):
                        fixture = stateful.generate_stateful_fixture(
                            cls.manifest, scenario.name, lane=lane, count=count
                        )
                        fixture_cache[scenario.name, count, lane] = fixture
                        fixture_dir = fixtures_root / scenario.name / str(count) / lane
                        fixture_dir.mkdir(parents=True)
                        case_bytes = (
                            opcode_gas.canonical_json(fixture["case_record"]) + b"\n"
                        )
                        input_bytes = (
                            opcode_gas.canonical_json(fixture["guest_input"]) + b"\n"
                        )
                        (fixture_dir / "case.json").write_bytes(case_bytes)
                        input_path = fixture_dir / "guest-input.json"
                        input_path.write_bytes(input_bytes)
                        bundle = _synthetic_bundle(fixture)
                        trace = bundle["report"]["controlled_trace"]
                        raw_gas = {
                            key: value
                            * StatefulPairLedgerExtractionTests.raw_gas_for(
                                scenario, key
                            )
                            for key, value in trace[
                                "executed_opcode_counts"
                            ].items()
                        }
                        trace["executed_opcode_raw_gas"] = raw_gas
                        trace["total_raw_gas"] = sum(raw_gas.values())
                        trace["non_target_raw_gas"] = (
                            trace["total_raw_gas"]
                            - trace["executed_target_raw_gas"]
                        )
                        bundle_by_input_bytes[input_bytes] = bundle
                        lane_data[lane] = (fixture, input_path, bundle)
                        fixture_inventory.append(
                            {
                                "scenario": scenario.name,
                                "relation_count": count,
                                "lane": lane,
                                "case_sha256": opcode_gas.sha256_bytes(case_bytes),
                                "guest_input_sha256": opcode_gas.sha256_bytes(
                                    input_bytes
                                ),
                            }
                        )
                    for repeat_index in range(cls.manifest.repeats):
                        reports = {}
                        for lane in ("target", "control"):
                            fixture, input_path, bundle = lane_data[lane]
                            gas = 10_000 + repeat_index + (
                                10 + 200 * count if lane == "target" else 0
                            )
                            report = copy.deepcopy(bundle["report"])
                            report.update(
                                {
                                    "stage": "revm-opcode-lab",
                                    "mode": "execute",
                                    "proof_mode": "compressed",
                                    "input": str(input_path),
                                    "exit_code": 0,
                                    "gas": gas,
                                    "primary_workload_metric": {
                                        "label": "prover_gas",
                                        "count": gas,
                                    },
                                    "public_values": bundle[
                                        "expected_public_values"
                                    ],
                                    "sp1_execution_engine": "gas-estimator",
                                    "sp1_gas_trace_chunk_threshold": (
                                        opcode_gas.SP1_GAS_TRACE_CHUNK_THRESHOLD
                                    ),
                                    "sp1_gas_trace_chunk_slots": (
                                        opcode_gas.SP1_GAS_TRACE_CHUNK_SLOTS
                                    ),
                                }
                            )
                            reports[lane] = report
                        target, control = stateful._pair_records(
                            cls.manifest,
                            target_spec=specs_by_key[
                                scenario.name, count, repeat_index, "target"
                            ],
                            control_spec=specs_by_key[
                                scenario.name, count, repeat_index, "control"
                            ],
                            target_fixture=lane_data["target"][0],
                            control_fixture=lane_data["control"][0],
                            target_bundle=lane_data["target"][2],
                            control_bundle=lane_data["control"][2],
                            target_report=reports["target"],
                            control_report=reports["control"],
                            target_input=lane_data["target"][1],
                            control_input=lane_data["control"][1],
                            fixtures_root=fixtures_root,
                            launcher_sha256=cls.launcher_sha256,
                            elf_sha256=cls.elf_sha256,
                            calibration_run_id=cls.calibration_id,
                        )
                        rows_by_identity[target["logical_identity"]] = target
                        rows_by_identity[control["logical_identity"]] = control
        rows = [rows_by_identity[spec.logical_identity] for spec in specs]
        cls.bundle_by_input_bytes = bundle_by_input_bytes
        cls.fixture_inventory_sha256 = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(fixture_inventory)
        )
        cls.rows = rows
        cls.rows_bytes = b"".join(
            opcode_gas.canonical_json(row) + b"\n" for row in rows
        )
        cls.base_report = stateful._fit_stateful_task4_rows(
            cls.manifest, rows, registry
        )
        cls.typed_registry = typed_registry

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temporary.name)
        self.run = self.root / "task4-run"
        self.run.mkdir()
        self.out = self.root / "results"
        self.identity_replay_calls = []
        self.revision = type(self).revision
        self.calibration_identity = copy.deepcopy(type(self).calibration_identity)
        self.calibration_id = type(self).calibration_id
        self.launcher = self.root / "guest-launcher"
        self.launcher.write_bytes(self.launcher_bytes)
        specs = stateful.stateful_campaign_row_specs(self.manifest)
        self.campaign_identity = {
            "schema_version": 1,
            "purpose": stateful.PURPOSE,
            "implementation_revision": self.revision,
            "calibration_id": self.calibration_id,
            "calibration_identity_sha256": opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(self.calibration_identity)
            ),
            "manifest": {
                "path": "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json",
                "file_sha256": opcode_gas.sha256_file(STATEFUL_MANIFEST),
            },
            "execution": dict(self.manifest.execution),
            "guest_launcher": {
                "path": "guest-launcher",
                "file_sha256": self.launcher_sha256,
            },
            "guest_elf": {
                "path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
                "file_sha256": self.elf_sha256,
            },
            "source_registry": {
                **stateful.REFERENCE_REGISTRY,
                "file_sha256": opcode_gas.sha256_file(
                    STATEFUL_REFERENCE_REGISTRY
                ),
            },
            "fixtures_root": "fixtures",
            "fixture_inventory_sha256": self.fixture_inventory_sha256,
            "row_inventory_sha256": opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(
                    [spec.logical_identity for spec in specs]
                )
            ),
            "row_count": len(self.rows),
            "repeats": self.manifest.repeats,
        }
        (self.run / "identity.json").write_bytes(
            opcode_gas.canonical_json(self.campaign_identity) + b"\n"
        )
        (self.run / "rows.jsonl").write_bytes(self.rows_bytes)
        rows_bytes, decisions, decisions_bytes = stateful._terminal_campaign_payloads(
            self.campaign_identity, self.rows
        )
        self.assertEqual(rows_bytes, self.rows_bytes)
        (self.run / "decisions.json").write_bytes(decisions_bytes)
        (self.run / "decisions.sha256").write_text(
            opcode_gas.sha256_bytes(decisions_bytes) + "\n"
        )
        self.decisions = decisions
        self.report = copy.deepcopy(self.base_report)
        self.report["task4_provenance"] = {
            "campaign_identity_sha256": decisions["campaign_identity_sha256"],
            "row_ledger_sha256": decisions["row_ledger_sha256"],
            "terminal_artifact_file_sha256": {
                "rows.jsonl": opcode_gas.sha256_bytes(self.rows_bytes),
                "decisions.json": opcode_gas.sha256_bytes(decisions_bytes),
                "decisions.sha256": opcode_gas.sha256_file(
                    self.run / "decisions.sha256"
                ),
            },
        }

    def tearDown(self):
        self.temporary.cleanup()

    def seal(self, out=None):
        out = out or self.out
        with mock.patch.object(
            stateful, "fit_stateful_task4_run", return_value=self.report
        ) as fitter, mock.patch.object(
            opcode_gas,
            "validate_calibration_execution_identity",
            return_value=self.calibration_identity,
        ):
            result = stateful.seal_stateful_opcode_result(
                out_root=out,
                manifest_path=STATEFUL_MANIFEST,
                calibration_run=self.root / self.calibration_id,
                fixtures_root=self.root / "fixtures",
                guest_launcher=self.launcher,
                elf=ROOT / "crates/guests/elf/sp1_revm_opcode_lab.elf",
                run=self.run,
            )
        fitter.assert_called_once()
        return result

    def replay_identity(self, _launcher, input_path, _cache):
        self.identity_replay_calls.append(input_path.read_bytes())
        return copy.deepcopy(
            self.bundle_by_input_bytes[input_path.read_bytes()]
        )

    def verify(self, directory):
        self.identity_replay_calls = []
        with mock.patch.object(opcode_gas, "git_head", return_value=self.revision), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ):
            result = stateful.verify_stateful_opcode_result(
                directory,
                guest_launcher=self.launcher,
                identity_replayer=self.replay_identity,
            )
        self.last_identity_replay_calls = self.identity_replay_calls
        return result

    def test_seal_is_deterministic_exact_idempotent_and_freezes_boundaries(self):
        first = self.seal()
        second = self.seal()
        self.assertEqual(first, second)
        directory = pathlib.Path(first["directory"])
        self.assertEqual(directory.name, first["result_id"])
        self.assertEqual({path.name for path in directory.iterdir()}, self.EXPECTED_INVENTORY)
        verified = self.verify(directory)
        self.assertEqual(verified["result_id"], first["result_id"])
        self.assertEqual(len(self.last_identity_replay_calls), 376)

        result = json.loads((directory / "result.json").read_text())
        self.assertFalse(result["candidate_eligible"])
        self.assertFalse(result["proposal_validated"])
        self.assertFalse(result["production_registry_modified"])
        self.assertEqual(
            result["ownership"],
            {
                "measured": (
                    "stateful REVM execution cost including storage execution, "
                    "journal updates, and result-state construction"
                ),
                "excluded": [
                    "witness materialization",
                    "persistent dirty-state commit",
                    "trie hashing",
                    "final state root",
                ],
            },
        )

        other = self.seal(self.root / "other-results")
        self.assertEqual(other["result_id"], first["result_id"])
        (directory / "model-report.json").write_text("{}\n")
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.seal()

    def test_sealed_rows_bind_campaign_guest_and_trace_evidence(self):
        stateful._validate_sealed_task4_row_evidence(
            self.rows, self.campaign_identity
        )
        for field, value in (
            ("elf_sha256", "0" * 64),
            ("trace_sha256", "1" * 64),
            ("semantic_check_sha256", "2" * 64),
        ):
            changed = copy.deepcopy(self.rows[:1])
            changed[0][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(
                ValueError, "row evidence"
            ):
                stateful._validate_sealed_task4_row_evidence(
                    changed, self.campaign_identity
                )

    def test_sealed_calibration_fixture_and_helper_authorities_are_independent(self):
        stateful._validate_sealed_calibration_identity(
            self.calibration_identity
        )
        abbreviated = {
            "implementation_revision": self.revision,
            "guest_launcher_sha256": self.launcher_sha256,
            "guest_artifacts": {
                "crates/guests/elf/sp1_revm_opcode_lab.elf": self.elf_sha256
            },
        }
        with self.assertRaisesRegex(ValueError, "calibration identity"):
            stateful._validate_sealed_calibration_identity(abbreviated)

        changed_campaign = copy.deepcopy(self.campaign_identity)
        changed_campaign["fixture_inventory_sha256"] = "4" * 64
        with self.assertRaisesRegex(ValueError, "campaign identity"):
            stateful._validate_sealed_campaign_identity(
                changed_campaign,
                self.calibration_identity,
                self.manifest,
                manifest_file_sha256=opcode_gas.sha256_file(STATEFUL_MANIFEST),
                registry_artifact_sha256=stateful.REFERENCE_REGISTRY[
                    "artifact_sha256"
                ],
                registry_file_sha256=opcode_gas.sha256_file(
                    STATEFUL_REFERENCE_REGISTRY
                ),
            )

        sealed = self.seal()
        wrong_helper = self.root / "wrong-helper"
        wrong_helper.write_bytes(b"not the reviewed identity helper\n")
        with mock.patch.object(
            opcode_gas, "git_head", return_value=self.revision
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), self.assertRaisesRegex(ValueError, "guest-launcher differs"):
            stateful.verify_stateful_opcode_result(
                pathlib.Path(sealed["directory"]),
                guest_launcher=wrong_helper,
                identity_replayer=lambda *_args: self.fail(
                    "unbound helper reached identity replay"
                ),
            )

    def test_verifier_rejects_inventory_symlinks_escape_and_noncanonical_bytes(self):
        sealed = self.seal()
        source = pathlib.Path(sealed["directory"])

        cases = ("extra", "missing", "file_symlink", "noncanonical")
        for case in cases:
            destination = self.root / f"copy-{case}" / source.name
            destination.parent.mkdir()
            shutil.copytree(source, destination)
            if case == "extra":
                (destination / "extra.json").write_text("{}\n")
            elif case == "missing":
                (destination / "model-report.json").unlink()
            elif case == "file_symlink":
                path = destination / "model-report.json"
                path.unlink()
                path.symlink_to(source / "model-report.json")
            else:
                payload = json.loads((destination / "model-report.json").read_text())
                (destination / "model-report.json").write_text(json.dumps(payload) + "\n")
            with self.subTest(case=case), self.assertRaises(ValueError):
                self.verify(destination)

        link = self.root / "result-link"
        link.symlink_to(source, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.verify(link)
        with self.assertRaises(ValueError):
            self.verify(source / ".." / source.name)

    def test_verifier_rejects_tamper_and_provenance_drift_without_guest_execution(self):
        sealed = self.seal()
        source = pathlib.Path(sealed["directory"])
        for filename, mutate in (
            (
                "source-registry.json",
                lambda value: value.__setitem__("artifact_sha256", "0" * 64),
            ),
            (
                "rows.jsonl",
                lambda value: value[0]["normalized_report"].__setitem__(
                    "prover_gas", value[0]["normalized_report"]["prover_gas"] + 1
                ),
            ),
            (
                "campaign-decisions.json",
                lambda value: value.__setitem__("status", "failed"),
            ),
            (
                "model-report.json",
                lambda value: value["selection"].__setitem__(
                    "selected_model", "M_typed"
                ),
            ),
            (
                "result.json",
                lambda value: value["ownership"].__setitem__("measured", "drift"),
            ),
        ):
            destination = self.root / f"tamper-{filename}" / source.name
            destination.parent.mkdir()
            shutil.copytree(source, destination)
            path = destination / filename
            if filename == "rows.jsonl":
                payload = list(opcode_gas.iter_jsonl(path))
                mutate(payload)
                path.write_bytes(
                    b"".join(opcode_gas.canonical_json(row) + b"\n" for row in payload)
                )
            else:
                payload = json.loads(path.read_text())
                mutate(payload)
                path.write_bytes(opcode_gas._canonical_json_file_bytes(payload))
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                self.verify(destination)

        with mock.patch.object(
            stateful,
            "verify_stateful_opcode_campaign",
            side_effect=AssertionError("guest-facing Task4 verifier executed"),
        ), mock.patch.object(
            stateful,
            "fit_stateful_task4_run",
            side_effect=AssertionError("guest-facing Task4 fitter executed"),
        ):
            self.assertEqual(
                self.verify(source)["result_id"], sealed["result_id"]
            )

        with mock.patch.object(opcode_gas, "git_head", return_value="b" * 40), self.assertRaisesRegex(
            ValueError, "implementation revision"
        ):
            stateful.verify_stateful_opcode_result(
                source,
                guest_launcher=self.launcher,
                identity_replayer=self.replay_identity,
            )
        with mock.patch.object(opcode_gas, "git_head", return_value=self.revision), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=" M experiments/opcode-gas/opcode_gas.py\n"
        ), self.assertRaisesRegex(ValueError, "dirty implementation"):
            stateful.verify_stateful_opcode_result(
                source,
                guest_launcher=self.launcher,
                identity_replayer=self.replay_identity,
            )

    def test_verifier_rejects_coherently_resealed_semantic_forgery_before_fit(self):
        sealed = self.seal()
        directory = pathlib.Path(sealed["directory"])
        campaign_identity = json.loads(
            (directory / "campaign-identity.json").read_text()
        )
        calibration_identity = json.loads(
            (directory / "calibration-identity.json").read_text()
        )
        registry = json.loads((directory / "source-registry.json").read_text())
        rows = list(opcode_gas.iter_jsonl(directory / "rows.jsonl"))
        forged = rows[0]
        forged["formal_report"]["controlled_trace"]["semantic_check"] = {
            "forged": True
        }
        trace = forged["formal_report"]["controlled_trace"]
        forged["semantic_check_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(trace["semantic_check"])
        )
        forged["trace_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(trace)
        )
        forged["formal_report_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(forged["formal_report"])
        )
        forged["row_identity"] = stateful.stateful_execution_row_identity(
            scenario=forged["scenario"],
            lane=forged["lane"],
            relation_count=forged["relation_count"],
            repeat_index=forged["repeat_index"],
            backend_input_sha256=forged["backend_input_sha256"],
            elf_sha256=forged["elf_sha256"],
            launcher_sha256=forged["launcher_sha256"],
            trace_sha256=forged["trace_sha256"],
        )
        rows_bytes, decisions, decisions_bytes = stateful._terminal_campaign_payloads(
            campaign_identity, rows
        )
        decisions_seal = (opcode_gas.sha256_bytes(decisions_bytes) + "\n").encode()
        (directory / "rows.jsonl").write_bytes(rows_bytes)
        (directory / "campaign-decisions.json").write_bytes(decisions_bytes)
        (directory / "campaign-decisions.sha256").write_bytes(decisions_seal)

        model_report = json.loads((directory / "model-report.json").read_text())
        model_report["task4_provenance"] = {
            "campaign_identity_sha256": decisions["campaign_identity_sha256"],
            "row_ledger_sha256": decisions["row_ledger_sha256"],
            "terminal_artifact_file_sha256": {
                "rows.jsonl": opcode_gas.sha256_bytes(rows_bytes),
                "decisions.json": opcode_gas.sha256_bytes(decisions_bytes),
                "decisions.sha256": opcode_gas.sha256_bytes(decisions_seal),
            },
        }
        model_report_bytes = opcode_gas._canonical_json_file_bytes(model_report)
        (directory / "model-report.json").write_bytes(model_report_bytes)
        identity = stateful._stateful_result_identity(
            implementation_revision=calibration_identity["implementation_revision"],
            campaign_identity_sha256=decisions["campaign_identity_sha256"],
            row_ledger_sha256=decisions["row_ledger_sha256"],
            manifest_file_sha256=opcode_gas.sha256_file(
                directory / "campaign-manifest.json"
            ),
            calibration_identity_file_sha256=opcode_gas.sha256_file(
                directory / "calibration-identity.json"
            ),
            campaign_identity_file_sha256=opcode_gas.sha256_file(
                directory / "campaign-identity.json"
            ),
            decisions_file_sha256=opcode_gas.sha256_bytes(decisions_bytes),
            decisions_seal_file_sha256=opcode_gas.sha256_bytes(decisions_seal),
            registry_artifact_sha256=registry["artifact_sha256"],
            registry_file_sha256=opcode_gas.sha256_file(
                directory / "source-registry.json"
            ),
        )
        envelope = stateful._stateful_result_envelope(identity, model_report_bytes)
        (directory / "result.json").write_bytes(
            opcode_gas._canonical_json_file_bytes(envelope)
        )
        forged_directory = directory.with_name(envelope["result_id"])
        directory.rename(forged_directory)

        with mock.patch.object(
            stateful,
            "_fit_stateful_task4_rows",
            side_effect=AssertionError("forged semantic evidence reached fitting"),
        ), self.assertRaisesRegex(ValueError, "semantic"):
            self.verify(forged_directory)

    def test_verifier_allows_only_byte_identical_evidence_descendant(self):
        sealed = self.seal()
        source = pathlib.Path(sealed["directory"])

        def frozen_bytes(_revision, relative):
            return (ROOT / relative).read_bytes()

        evidence_paths = (
            f"experiments/opcode-gas/derivations/{source.name}/result.json",
            "experiments/opcode-gas/README.md",
            "docs/plans/2026-09-26-zkgas-calibration-progress.md",
        )
        with mock.patch.object(opcode_gas, "git_head", return_value="b" * 40), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_revision_is_ancestor", return_value=True
        ), mock.patch.object(
            opcode_gas, "git_changed_paths_since", return_value=evidence_paths
        ), mock.patch.object(
            opcode_gas, "git_file_bytes_at_revision", side_effect=frozen_bytes
        ):
            self.assertEqual(
                stateful.verify_stateful_opcode_result(
                    source,
                    guest_launcher=self.launcher,
                    identity_replayer=self.replay_identity,
                )["result_id"],
                sealed["result_id"],
            )

        registry_path = stateful.REFERENCE_REGISTRY["path"]

        def drifted_frozen_bytes(_revision, relative):
            if relative == registry_path:
                return b"historical registry bytes differ"
            return (ROOT / relative).read_bytes()

        with mock.patch.object(opcode_gas, "git_head", return_value="b" * 40), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_revision_is_ancestor", return_value=True
        ), mock.patch.object(
            opcode_gas,
            "git_changed_paths_since",
            return_value=(registry_path,),
        ), mock.patch.object(
            opcode_gas,
            "git_file_bytes_at_revision",
            side_effect=drifted_frozen_bytes,
        ), self.assertRaisesRegex(ValueError, "non-evidence source"):
            stateful.verify_stateful_opcode_result(
                source,
                guest_launcher=self.launcher,
                identity_replayer=self.replay_identity,
            )

    def test_cli_exposes_direct_seal_and_verify_result_paths(self):
        parser = opcode_gas.build_parser()
        seal = parser.parse_args(
            [
                "seal-stateful-opcode-result",
                "--manifest", str(STATEFUL_MANIFEST.relative_to(ROOT)),
                "--calibration-run", "calibration",
                "--fixtures", "fixtures",
                "--guest-launcher", "guest-launcher",
                "--elf", "crates/guests/elf/sp1_revm_opcode_lab.elf",
                "--run", "task4-run",
                "--out", "results",
            ]
        )
        verify = parser.parse_args(
            [
                "verify-stateful-opcode-result",
                "--result", "results/abc",
                "--guest-launcher", "guest-launcher",
            ]
        )
        self.assertIs(seal.func, opcode_gas.cmd_seal_stateful_opcode_result)
        self.assertIs(verify.func, opcode_gas.cmd_verify_stateful_opcode_result)
        self.assertFalse(hasattr(seal, "rows"))
        self.assertFalse(hasattr(seal, "model_report"))


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
    non_target_counts = dict(counts)
    remaining_target_count = non_target_counts.get(target_key, 0) - record["target_count"]
    if remaining_target_count:
        non_target_counts[target_key] = remaining_target_count
    else:
        non_target_counts.pop(target_key, None)
    executed_target_raw_gas = record["target_count"] * record["target_raw_gas"]
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
        "executed_target_raw_gas": executed_target_raw_gas,
        "non_target_counts": non_target_counts,
        "non_target_raw_gas": sum(raw_gas.values()) - executed_target_raw_gas,
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


def _synthetic_identity(fixture, report):
    trace = report["controlled_trace"]
    return {
        "schema_version": 1,
        "input": copy.deepcopy(fixture["guest_input"]),
        "backend_input_sha256": trace["backend_input_sha256"],
        "backend_input_len": trace["backend_input_len"],
        "workload_id": trace["workload_id"],
        "transaction_envelope_sha256": trace["transaction_envelope_sha256"],
        "access_list_sha256": trace["access_list_sha256"],
        "prestate_sha256": trace["prestate_sha256"],
    }


def _synthetic_expected_public_values(fixture):
    return "0x" + hashlib.sha256(
        fixture["case_record"]["case_id"].encode()
    ).hexdigest()


def _synthetic_bundle(fixture):
    lane = fixture["case_record"]["lane"]
    native_report = _synthetic_report(fixture, lane)
    return {
        "schema_version": 1,
        "expected_public_values": _synthetic_expected_public_values(fixture),
        "identity": _synthetic_identity(fixture, native_report),
        "report": native_report,
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

    def bundle(self, fixture):
        return _synthetic_bundle(fixture)

    def admit_fixture(self, fixture, reports, *, repeat_index):
        return stateful.admit_stateful_fixture_trace(
            self.manifest,
            fixture,
            reports,
            expected_bundle=self.bundle(fixture),
            repeat_index=repeat_index,
        )

    def admit_pair(
        self,
        target_fixture,
        target_reports,
        control_fixture,
        control_reports,
        *,
        repeat_index,
    ):
        return stateful.admit_stateful_pair(
            self.manifest,
            target_fixture,
            target_reports,
            control_fixture,
            control_reports,
            target_bundle=self.bundle(target_fixture),
            control_bundle=self.bundle(control_fixture),
            repeat_index=repeat_index,
        )

    def test_exact_trace_and_semantic_check_make_one_lane_runnable(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")

        admission = self.admit_fixture(fixture, [report], repeat_index=2)

        self.assertEqual(admission["scenario"], "sstore_dirty_rewrite")
        self.assertEqual(admission["lane"], "target")
        self.assertEqual(admission["relation_count"], 4)
        self.assertEqual(admission["repeat_index"], 2)
        self.assertEqual(admission["backend_input_sha256"], report["controlled_trace"]["backend_input_sha256"])
        self.assertEqual(len(admission["trace_sha256"]), 64)
        self.assertEqual(len(admission["semantic_check_sha256"]), 64)
        self.assertEqual(len(admission["row_identity"]), 64)

    def test_real_rust_identity_and_report_round_trip_into_python_admission(self):
        fixture = self.fixture(scenario="sload_warm_nonzero", count=1)
        with tempfile.TemporaryDirectory() as directory:
            input_path = pathlib.Path(directory) / "guest-input.json"
            expected_bundle_path = pathlib.Path(directory) / "expected-bundle.json"
            formal_bundle_path = pathlib.Path(directory) / "formal-bundle.json"
            input_path.write_bytes(
                opcode_gas.canonical_json(fixture["guest_input"]) + b"\n"
            )
            launcher = ROOT / "target" / "debug" / "guest-launcher"
            self.assertTrue(launcher.is_file(), "reviewed guest-launcher binary is missing")
            for output_path in (expected_bundle_path, formal_bundle_path):
                subprocess.run(
                    [
                        str(launcher),
                        "--stage",
                        "revm-opcode-identity",
                        "--proof-type",
                        "native",
                        "--input",
                        str(input_path),
                        "--json-out",
                        str(output_path),
                    ],
                    cwd=ROOT,
                    check=True,
                )
            expected_bundle = json.loads(expected_bundle_path.read_text())
            formal_bundle = json.loads(formal_bundle_path.read_text())

        self.assertEqual(expected_bundle, formal_bundle)

        admission = stateful.admit_stateful_fixture_trace(
            self.manifest,
            fixture,
            [formal_bundle["report"]],
            expected_bundle=expected_bundle,
            repeat_index=0,
        )

        self.assertEqual(
            admission["backend_input_sha256"],
            expected_bundle["identity"]["backend_input_sha256"],
        )
        self.assertEqual(
            admission["identity_evidence_sha256"],
            hashlib.sha256(
                opcode_gas.canonical_json(expected_bundle["identity"])
            ).hexdigest(),
        )

    def test_lane_admission_rejects_zero_duplicate_and_wrong_report_identity(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        with self.assertRaisesRegex(ValueError, "exactly one host trace report"):
            self.admit_fixture(fixture, [], repeat_index=0)
        with self.assertRaisesRegex(ValueError, "exactly one host trace report"):
            self.admit_fixture(
                fixture, [report, copy.deepcopy(report)], repeat_index=0
            )

        wrong = copy.deepcopy(report)
        wrong["guest_input_sha256"] = "0x" + "00" * 32
        with self.assertRaisesRegex(ValueError, "backend-input identity"):
            self.admit_fixture(fixture, [wrong], repeat_index=0)

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
            ("non-target count ledger", lambda f, r: r["controlled_trace"].__setitem__("non_target_counts", {"forged": 1})),
            ("non-target raw gas", lambda f, r: r["controlled_trace"].__setitem__("non_target_raw_gas", 1)),
            (
                "target raw-gas ledger consistency",
                lambda f, r: (
                    r["controlled_trace"]["executed_opcode_raw_gas"].__setitem__(
                        "opcode:0x55",
                        r["controlled_trace"]["executed_opcode_raw_gas"]["opcode:0x55"] + 1,
                    ),
                    r["controlled_trace"].__setitem__(
                        "total_raw_gas", r["controlled_trace"]["total_raw_gas"] + 1
                    ),
                ),
            ),
        )
        for label, mutate in mutations:
            changed_fixture = copy.deepcopy(fixture)
            changed_report = copy.deepcopy(report)
            mutate(changed_fixture, changed_report)
            with self.subTest(label=label), self.assertRaises(ValueError):
                self.admit_fixture(changed_fixture, [changed_report], repeat_index=0)

    def test_lane_admission_rejects_matching_but_wrong_semantic_values(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        program = report["controlled_trace"]["semantic_check"]["programs"][0]
        program["expected_final_storage"] = stateful.ZERO
        program["observed_final_storage"] = stateful.ZERO

        with self.assertRaisesRegex(ValueError, "semantic SSTORE"):
            self.admit_fixture(fixture, [report], repeat_index=0)

    def test_lane_admission_rejects_fully_consistent_raw_gas_mutation(self):
        fixture = self.fixture()
        report = _synthetic_report(fixture, "target")
        trace = report["controlled_trace"]
        trace["executed_opcode_raw_gas"]["opcode:0x55"] += 777
        trace["total_raw_gas"] += 777
        trace["non_target_raw_gas"] += 777

        with self.assertRaisesRegex(ValueError, "native trace"):
            self.admit_fixture(fixture, [report], repeat_index=0)

    def test_pair_admission_binds_order_and_rejects_all_confounds(self):
        target = self.fixture(lane="target")
        control = self.fixture(lane="control")
        target_report = _synthetic_report(target, "target")
        control_report = _synthetic_report(control, "control")

        pair = self.admit_pair(
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

        for label, field, error in (
            ("transaction envelope", "transaction_envelope_sha256", "transaction_envelope"),
            ("access list", "access_list_sha256", "access_list"),
            ("prestate", "prestate_sha256", "prestate"),
        ):
            changed = copy.deepcopy(control_report)
            changed["controlled_trace"][field] = "00" * 32
            with self.subTest(label=label), self.assertRaisesRegex(ValueError, error):
                self.admit_pair(
                    target, [target_report], control, [changed], repeat_index=1
                )

        with self.assertRaisesRegex(ValueError, "target lane"):
            self.admit_pair(
                control, [control_report], target, [target_report], repeat_index=1
            )

        other_control = self.fixture(scenario="sstore_restore_zero", lane="control")
        with self.assertRaisesRegex(ValueError, "same pair"):
            self.admit_pair(
                target,
                [target_report],
                other_control,
                [_synthetic_report(other_control, "other")],
                repeat_index=1,
            )

        with self.assertRaisesRegex(ValueError, "backend-input identity"):
            self.admit_pair(
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
        with self.assertRaisesRegex(ValueError, "backend-input identity"):
            self.admit_pair(
                target,
                [target_report],
                control,
                [same_hash_control],
                repeat_index=1,
            )

    def test_pair_admission_rejects_consistently_forged_duplicate_identities(self):
        target = self.fixture(lane="target")
        control = self.fixture(lane="control")
        target_report = _synthetic_report(target, "target")
        control_report = _synthetic_report(control, "control")
        for report, backend in (
            (target_report, "aa" * 32),
            (control_report, "bb" * 32),
        ):
            trace = report["controlled_trace"]
            report["guest_input_sha256"] = f"0x{backend}"
            trace["backend_input_sha256"] = backend
            trace["semantic_check"]["backend_input_sha256"] = backend
            trace["workload_id"] = "cc" * 32
            trace["transaction_envelope_sha256"] = "dd" * 32
            trace["access_list_sha256"] = "ee" * 32
            trace["prestate_sha256"] = "ff" * 32
            trace["non_target_counts"] = {"forged": 123}
            trace["non_target_raw_gas"] = 456

        with self.assertRaisesRegex(ValueError, "identity"):
            self.admit_pair(
                target,
                [target_report],
                control,
                [control_report],
                repeat_index=0,
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
                self.admit_pair(
                    target,
                    [target_report],
                    changed,
                    [control_report],
                    repeat_index=0,
                )

        changed_report = copy.deepcopy(target_report)
        changed_report["controlled_trace"]["executed_opcode_counts"]["opcode:0x55"] += 1
        with self.assertRaisesRegex(ValueError, "opcode ledger"):
            self.admit_pair(
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
                pair = self.admit_pair(
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
