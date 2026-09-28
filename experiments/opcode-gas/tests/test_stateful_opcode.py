import copy
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from dataclasses import replace
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
                "public_values": "0x"
                + hashlib.sha256(fixture["case_record"]["case_id"].encode()).hexdigest(),
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

            with self.assertRaisesRegex(ValueError, "public output differs"):
                stateful.run_stateful_campaign_rows(
                    manifest, specs, fixtures_root=root / "fixtures", run=root / "run",
                    guest_launcher=root / "launcher", elf=root / "elf",
                    launcher_sha256="a" * 64, elf_sha256="b" * 64,
                    batch_executor=execute, identity_replayer=replay,
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


def _synthetic_bundle(fixture):
    lane = fixture["case_record"]["lane"]
    native_report = _synthetic_report(fixture, lane)
    return {
        "schema_version": 1,
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
