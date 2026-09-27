import hashlib
import json
import pathlib
import shutil
import sys
import tarfile
import tempfile
import types
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


FIXTURES = ROOT / "tests/fixtures/risc0-zkgas/2026-09-02-m2-aggregation-direct-v3"

EXPORTED_VERSION_IDENTITY = {
    "taiko_fork": "Unzen",
    "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
    "ethereum_upgrade": "Fusaka",
    "revm_spec_id": "OSAKA",
}

CALIBRATION_VERSION_IDENTITY = {
    **EXPORTED_VERSION_IDENTITY,
    "proving_backend": "sp1",
    "primary_metric": "proverGas",
}


def versioned_schedule():
    return opcode_gas.UnzenSchedule(
        opcode_multipliers={},
        precompile_multipliers={},
        version_identity=EXPORTED_VERSION_IDENTITY,
    )


class RunManifestTests(unittest.TestCase):
    def test_complete_schedule_has_all_opcode_identities_and_hashes_every_field(self):
        raw = {
            "version_identity": EXPORTED_VERSION_IDENTITY,
            "block_limit": 100,
            "tx_intrinsic_zk_gas": 3,
            "failsafe_multiplier": 65535,
            "spawn_estimates": {"call": 1, "callcode": 2, "delegatecall": 3, "staticcall": 4, "create": 5, "create2": 6},
            "opcodes": [
                {"opcode": f"0x{i:02x}", "multiplier": 7 if i == 1 else 65535, "explicit": i == 1}
                for i in range(256)
            ],
            "precompiles": [{"address": "0x0000000000000000000000000000000000000001", "multiplier": 8, "explicit": True}],
            "precompile_fallback_multiplier": 65535,
        }
        schedule = opcode_gas.parse_complete_uzen_schedule(raw)
        self.assertEqual(len(schedule.opcode_multipliers), 256)
        self.assertTrue(schedule.opcode_explicit[1])
        self.assertFalse(schedule.opcode_explicit[2])
        self.assertEqual(schedule.precompile_fallback_multiplier, 65535)
        self.assertEqual(dict(schedule.version_identity), EXPORTED_VERSION_IDENTITY)
        baseline = opcode_gas.schedule_sha256(schedule)
        changed = opcode_gas.parse_complete_uzen_schedule({**raw, "block_limit": 101})
        self.assertNotEqual(baseline, opcode_gas.schedule_sha256(changed))

    def test_select_final_validation_corpus_is_exactly_40_hoodi_and_20_mainnet(self):
        rows = opcode_gas.select_final_validation_corpus(
            [FIXTURES / "hoodi-fit.jsonl", FIXTURES / "validation.jsonl"]
        )
        self.assertEqual(len(rows), 60)
        self.assertEqual(sum(row["network"] == "taiko_hoodi" for row in rows), 40)
        self.assertEqual(sum(row["network"] == "taiko_mainnet" for row in rows), 20)
        self.assertEqual({row["purpose"] for row in rows}, {"final_validation"})
        self.assertEqual(len({(row["network"], row["proposal_id"]) for row in rows}), 60)

    def test_integration_smoke_cannot_enter_or_be_relabelled_into_final_corpus(self):
        rows = opcode_gas.select_final_validation_corpus(
            [FIXTURES / "hoodi-fit.jsonl", FIXTURES / "validation.jsonl"]
        )
        member = rows[0]
        with self.assertRaisesRegex(ValueError, "final_validation corpus"):
            opcode_gas.assert_integration_smoke_is_disjoint(rows, member["network"], member["proposal_id"])
        smoke = {"network": "taiko_hoodi", "proposal_id": 1, "purpose": "integration_smoke"}
        with self.assertRaisesRegex(ValueError, "cannot be relabeled"):
            opcode_gas.freeze_smoke_row(smoke, purpose="final_validation")

    def test_prepare_corpus_uses_fixed_ids_and_valueless_preflight_validate(self):
        rows = [{"network": "taiko_hoodi", "proposal_id": 7, "block_count": 1, "total_zkgas": 1, "purpose": "final_validation"}]
        calls = []

        def fake_run(command, **kwargs):
            calls.append(command)
            if any("stress_shasta_proposal.py" in argument for argument in command):
                out = pathlib.Path(command[command.index("--proposal-out") + 1])
                out.write_text(json.dumps([{"proposal_id": 7, "l1_inclusion_block_number": 10, "last_anchor_block_number": 9, "l2_start": 1, "l2_end": 1, "unzen_activated": True}]))
            else:
                out = pathlib.Path(command[command.index("--output") + 1])
                out.write_text('{"blocks":[{"block_difficulty":1,"timestamp":1}]}\n')
            return subprocess_completed(command)

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas.subprocess, "run", fake_run), mock.patch.object(opcode_gas, "REPO_ROOT", pathlib.Path(tmp)), mock.patch.object(opcode_gas, "git_head", return_value="a" * 40), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
            root = pathlib.Path(tmp)
            spec = root / "spec.json"
            spec.write_text(json.dumps(actual_chain_spec_list("taiko_hoodi", 1)))
            with mock.patch.object(opcode_gas, "select_final_validation_corpus", return_value=rows):
                manifest = opcode_gas.prepare_corpus(
                    corpus_root=root / "corpus",
                    l1_rpc_by_network={"taiko_hoodi": "https://l1.invalid"},
                    l2_rpc_by_network={"taiko_hoodi": "https://l2.invalid"},
                    chain_spec_hash_by_network={"taiko_hoodi": hashlib.sha256(spec.read_bytes()).hexdigest()},
                    chain_spec_path_by_network={"taiko_hoodi": spec},
                    implementation_revision="a" * 40,
                )
        self.assertEqual(manifest["rows"][0]["purpose"], "final_validation")
        preflight = next(call for call in calls if "preflight" in call[0])
        self.assertIn("--validate", preflight)
        self.assertNotIn("true", preflight)
        self.assertIn("--proposal-ids", calls[0])

    def test_publish_corpus_requires_exact_generation_readback_and_hash(self):
        calls = []
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas, "REPO_ROOT", pathlib.Path(tmp)), mock.patch.object(opcode_gas, "git_head", return_value="a" * 40), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
            archive = pathlib.Path(tmp) / "corpus.tar"
            archive.write_bytes(b"frozen corpus")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()

            def fake_run(command, **kwargs):
                calls.append(command)
                if command[:4] == ["gcloud", "storage", "objects", "describe"]:
                    return completed(command, json.dumps({"generation": "9", "size": str(archive.stat().st_size)}))
                if command[:3] == ["gcloud", "storage", "cp"] and command[-1].startswith("gs://"):
                    return completed(command, "Created")
                if command[:3] == ["gcloud", "storage", "cp"]:
                    pathlib.Path(command[-1]).write_bytes(archive.read_bytes())
                    return completed(command, "")
                raise AssertionError(command)

            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                published = opcode_gas.publish_corpus(archive, f"gs://bucket/{digest}.tar")
        self.assertEqual(published["archive_generation"], 9)
        self.assertEqual(published["archive_sha256"], digest)
        self.assertTrue(any("--if-generation-match=0" in call for call in calls))

    def test_prepare_calibration_rejects_dirty_implementation_paths_and_writes_bridge_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            controlled = root / "controlled.toml"
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"test guest launcher")
            controlled.write_text('normalization_reference_key = "opcode:0x01"\nq_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\nbridge_key_ids = ["opcode:0x01", "opcode:0x02", "precompile:0x1", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n')
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=" crates/prover/src/lib.rs\n"):
                with self.assertRaisesRegex(ValueError, "implementation path"):
                    opcode_gas.prepare_calibration(
                        root,
                        controlled,
                        guest_launcher=launcher,
                        implementation_revision="a" * 40,
                        complete_schedule_hash="b" * 64,
                    )
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    controlled,
                    guest_launcher=launcher,
                    implementation_revision="a" * 40,
                    complete_schedule_hash="b" * 64,
                )
            self.assertEqual(experiment["implementation_revision"], "a" * 40)
            self.assertEqual(experiment["complete_schedule_sha256"], "b" * 64)
            self.assertTrue((root / "runs" / experiment["calibration_id"] / "bridge" / "bridge-manifest.json").exists())

    def test_prepare_validation_requires_matching_provenance_and_frozen_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            revision = "a" * 40
            run = root / "runs" / "run"
            (run / "candidate").mkdir(parents=True)
            (run / "bridge").mkdir(parents=True)
            candidate = {"implementation_revision": revision}
            bridge = {"implementation_revision": revision}
            (run / "candidate" / "candidate-manifest.json").write_text(json.dumps(candidate))
            (run / "bridge" / "bridge-root.json").write_text(json.dumps(bridge))
            (run / "candidate" / "candidate.sha256").write_text(
                opcode_gas.sha256_bytes(opcode_gas.canonical_json(candidate)) + "\n"
            )
            (run / "bridge" / "bridge.sha256").write_text(
                opcode_gas.sha256_bytes(opcode_gas.canonical_json(bridge)) + "\n"
            )
            (run / "experiment.json").write_text(json.dumps({"implementation_revision": revision}))
            with mock.patch.object(opcode_gas, "REPO_ROOT", root):
                corpus_path = write_frozen_corpus(root, revision)
                with mock.patch.object(opcode_gas, "git_head", return_value=revision), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                    opcode_gas,
                    "verify_candidate_directory",
                    return_value={
                        "candidate_sha256": opcode_gas.sha256_bytes(
                            opcode_gas.canonical_json(candidate)
                        )
                    },
                ) as verify_candidate, mock.patch.object(
                    opcode_gas,
                    "verify_bridge_directory",
                    return_value={
                        "bridge_sha256": opcode_gas.sha256_bytes(
                            opcode_gas.canonical_json(bridge)
                        )
                    },
                ) as verify_bridge:
                    validation = opcode_gas.prepare_validation(root, run, corpus_path)
                verify_candidate.assert_called_once_with(run)
                verify_bridge.assert_called_once_with(run)
            self.assertEqual(validation["implementation_revision"], revision)
            self.assertTrue((root / "validations" / validation["validation_id"] / "candidate-ref.json").exists())

    def test_prepare_validation_rejects_candidate_or_bridge_provenance_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            revision = "a" * 40
            run = root / "runs" / "run"
            (run / "candidate").mkdir(parents=True)
            (run / "bridge").mkdir(parents=True)
            (run / "experiment.json").write_text(json.dumps({"implementation_revision": revision}))
            (run / "candidate" / "candidate.sha256").write_text("b" * 64 + "\n")
            (run / "bridge" / "bridge.sha256").write_text("c" * 64 + "\n")
            (run / "candidate" / "candidate-manifest.json").write_text(
                json.dumps({"implementation_revision": "d" * 40})
            )
            (run / "bridge" / "bridge-root.json").write_text(
                json.dumps({"implementation_revision": revision})
            )
            corpus_path = root / "corpus.json"
            corpus_path.write_text(json.dumps({"implementation_revision": revision, "archive_uri": "gs://bucket/x.tar#9", "archive_generation": 9, "archive_size_bytes": 1, "archive_sha256": "d" * 64, "rows": complete_rows()}))
            with mock.patch.object(opcode_gas, "git_head", return_value=revision), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                with self.assertRaisesRegex(ValueError, "candidate provenance"):
                    opcode_gas.prepare_validation(root, run, corpus_path)

    def test_prepare_validation_rejects_tampered_bridge_component_before_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            revision = "a" * 40
            run = root / "runs" / "run"
            candidate_dir = run / "candidate"
            bridge_dir = run / "bridge"
            samples_dir = run / "samples"
            candidate_dir.mkdir(parents=True)
            bridge_dir.mkdir()
            samples_dir.mkdir()
            candidate = {"implementation_revision": revision}
            (candidate_dir / "candidate-manifest.json").write_text(
                json.dumps(candidate)
            )
            (run / "experiment.json").write_text(
                json.dumps({"implementation_revision": revision})
            )
            bridge_components = {
                "bridge-manifest.json": {"schema_version": 1},
                "controlled-cycle-cost-samples.json": {
                    "schema_version": 1,
                    "samples": {},
                },
                "controlled-bridge.json": {
                    "schema_version": 1,
                    "status": "insufficient_data",
                },
            }
            component_paths = {
                "bridge-manifest.json": bridge_dir / "bridge-manifest.json",
                "controlled-cycle-cost-samples.json": samples_dir
                / "controlled-cycle-cost-samples.json",
                "controlled-bridge.json": bridge_dir / "controlled-bridge.json",
            }
            for name, payload in bridge_components.items():
                component_paths[name].write_bytes(opcode_gas.canonical_json(payload))
            bridge_root = {
                "schema_version": 1,
                "implementation_revision": revision,
                "components": {
                    name: opcode_gas.sha256_bytes(opcode_gas.canonical_json(payload))
                    for name, payload in bridge_components.items()
                },
                "component_schemas": {name: 1 for name in bridge_components},
                "status": "insufficient_data",
            }
            (bridge_dir / "bridge-root.json").write_bytes(
                opcode_gas.canonical_json(bridge_root)
            )
            (bridge_dir / "bridge.sha256").write_text(
                opcode_gas.sha256_bytes(opcode_gas.canonical_json(bridge_root)) + "\n"
            )
            component_paths["controlled-bridge.json"].write_text(
                '{"schema_version":1,"status":"stable_controlled"}\n'
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root):
                corpus_path = write_frozen_corpus(root, revision)
                with mock.patch.object(
                    opcode_gas, "git_head", return_value=revision
                ), mock.patch.object(
                    opcode_gas, "git_worktree_status", return_value=""
                ), mock.patch.object(
                    opcode_gas,
                    "verify_candidate_directory",
                    return_value={"candidate_sha256": "b" * 64},
                ):
                    with self.assertRaisesRegex(ValueError, "component digest"):
                        opcode_gas.prepare_validation(root, run, corpus_path)
            self.assertFalse((root / "validations").exists())

    def test_parser_exposes_freeze_and_sealing_commands(self):
        parser = opcode_gas.build_parser()
        self.assertEqual(
            parser.parse_args(["prepare-corpus", "--corpus-root", "/tmp/corpus", "--l1-rpc", "taiko_hoodi=https://l1", "--l2-rpc", "taiko_hoodi=https://l2", "--chain-spec-hash", "taiko_hoodi=" + "a" * 64, "--chain-spec-file", "taiko_hoodi=/tmp/spec.json"]).command,
            "prepare-corpus",
        )
        self.assertEqual(parser.parse_args(["publish-corpus", "--archive", "/tmp/a.tar", "--object-uri", "gs://bucket/x.tar", "--manifest", "/tmp/manifest.json"]).command, "publish-corpus")
        self.assertEqual(parser.parse_args(["prepare-calibration", "--out", "/tmp/out", "--controlled-manifest", "/tmp/control.toml", "--guest-launcher", "/tmp/guest-launcher"]).command, "prepare-calibration")
        self.assertEqual(parser.parse_args(["prepare-validation", "--out", "/tmp/out", "--run", "/tmp/run", "--corpus", "/tmp/corpus.json"]).command, "prepare-validation")
        candidate = parser.parse_args([
            "build-candidate", "--run", "/tmp/run",
            "--controlled-manifest", "/tmp/control.toml",
            "--relations", "/tmp/run/opcode-relations.json",
            "--anchor-probe", "/tmp/run/anchor-probe-fit.json",
            "--block-calibration", "/tmp/run/block-calibration.json",
            "--controlled-fit", "/tmp/run/controlled-fit.json",
            "--provenance", "/tmp/run/provenance.json",
        ])
        self.assertEqual(candidate.command, "build-candidate")
        self.assertFalse(hasattr(candidate, "fit"))
        self.assertFalse(hasattr(candidate, "overheads"))
        self.assertEqual(parser.parse_args([
            "prepare-integration-smoke", "--network", "taiko_hoodi",
            "--proposal-id", "1", "--guest-input", "smoke.json",
        ]).command, "prepare-integration-smoke")

    def test_validation_rejects_duplicate_rows_missing_local_fixtures_and_unsealed_archive(self):
        revision = "a" * 40
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas, "REPO_ROOT", pathlib.Path(tmp)), mock.patch.object(opcode_gas, "git_head", return_value="a" * 40), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
            root = pathlib.Path(tmp)
            corpus = {
                "implementation_revision": revision,
                "archive_uri": "gs://bucket/not-content-addressed.tar#9",
                "archive_generation": 9,
                "archive_size_bytes": 1,
                "rows": complete_rows(),
            }
            with self.assertRaisesRegex(ValueError, "archive|exact selected|fixture"):
                opcode_gas._validate_frozen_corpus(corpus, revision)

    def test_publication_rejects_manifest_for_a_different_archive_before_gcloud(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            archive = root / "archive.tar"
            archive.write_bytes(b"archive B")
            manifest = {"archive_sha256": hashlib.sha256(b"archive A").hexdigest(), "rows": []}
            with mock.patch.object(opcode_gas.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "archive_sha256"):
                    opcode_gas.validate_manifest_for_publication(manifest, archive)
            run.assert_not_called()

    def test_actual_smoke_preparation_rejects_final_member_and_relabel(self):
        final = opcode_gas.select_final_validation_corpus(
            [FIXTURES / "hoodi-fit.jsonl", FIXTURES / "validation.jsonl"]
        )[0]
        with self.assertRaisesRegex(ValueError, "final_validation corpus"):
            opcode_gas.prepare_integration_smoke(final["network"], final["proposal_id"])
        with self.assertRaisesRegex(ValueError, "cannot be relabeled"):
            opcode_gas.prepare_integration_smoke("taiko_hoodi", 1, purpose="final_validation")

    def test_integration_smoke_identity_is_exactly_the_frozen_two_proposals(self):
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(
            opcode_gas, "select_final_validation_corpus", return_value=[]
        ):
            root = pathlib.Path(tmp)
            guest_input = root / "guest-input.json"
            for network, proposal_id in (
                ("taiko_hoodi", 79852),
                ("taiko_mainnet", 38261),
            ):
                with self.subTest(network=network):
                    guest_input.write_text(json.dumps({
                        "taiko": {
                            "proposal_id": proposal_id,
                            "chain_spec": {"name": network},
                        }
                    }) + "\n")
                    record = opcode_gas.prepare_integration_smoke(
                        network, proposal_id, guest_input=guest_input
                    )
                    fixture_sha256 = opcode_gas.sha256_file(guest_input)
                    self.assertEqual(record, {
                        "network": network,
                        "proposal_id": proposal_id,
                        "purpose": "integration_smoke",
                        "fixture_sha256": fixture_sha256,
                        "workload_id": opcode_gas.proposal_workload_id(
                            fixture_sha256
                        ),
                    })

            guest_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 79853,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            with self.assertRaisesRegex(ValueError, "frozen integration_smoke"):
                opcode_gas.prepare_integration_smoke(
                    "taiko_hoodi", 79853, guest_input=guest_input
                )

    def test_calibration_records_exact_formula_checkpoint_versions_and_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            controlled = root / "controlled.toml"
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"test guest launcher")
            controlled.write_text(
                'normalization_reference_key = "opcode:0x01"\n'
                'q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
                'bridge_key_ids = ["opcode:0x01", "opcode:0x02", "precompile:0x1", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
            )
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    controlled,
                    guest_launcher=launcher,
                    implementation_revision="a" * 40,
                    complete_schedule_hash="b" * 64,
                )
        self.assertEqual(experiment["q_formula"], ["proposal_startup", "block_base", "tx_base", "native_value_transfer"])
        self.assertEqual(experiment["out_of_fit_checkpoint"]["mapping"], {"4": 8, "16": 32, "64": 128, "256": 512, "1024": 2048})
        self.assertTrue(experiment["rust_version"])
        self.assertTrue(experiment["sp1_sdk_version"])
        self.assertIn("guest_artifacts_sha256", experiment)

    def test_calibration_id_seals_complete_execution_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            controlled = root / "controlled.toml"
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"test guest launcher")
            controlled.write_text(
                'normalization_reference_key = "opcode:0x01"\n'
                'q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
                'bridge_key_ids = ["opcode:0x01", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
            )
            with mock.patch.object(opcode_gas, "current_uzen_schedule", return_value=versioned_schedule()), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(opcode_gas, "_rust_version", return_value="rustc test"), mock.patch.object(
                opcode_gas, "_locked_package_version", return_value="6.3.0"
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    controlled,
                    guest_launcher=launcher,
                    implementation_revision="a" * 40,
                    complete_schedule_hash="b" * 64,
                )
        identity = experiment["calibration_identity"]
        self.assertEqual(identity["version_identity"], CALIBRATION_VERSION_IDENTITY)
        self.assertEqual(identity["rust_version"], "rustc test")
        self.assertEqual(identity["sp1_sdk_version"], "6.3.0")
        self.assertEqual(identity["q_formula"], opcode_gas.Q_FORMULA)
        self.assertEqual(identity["out_of_fit_checkpoint"], {"mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS, "ape_max": 0.10})
        self.assertEqual(identity["guest_artifacts_sha256"], experiment["guest_artifacts_sha256"])
        self.assertEqual(identity["guest_artifacts"], experiment["guest_artifacts"])
        self.assertEqual(
            identity["sp1_execution_parameters"]["engines"],
            {
                "opcode": "gas-estimator",
                "precompile": "standard",
                "overhead": "standard",
                "block": "gas-estimator",
            },
        )
        self.assertEqual(
            identity["sp1_execution_parameters"]["gas_estimator"],
            {"gas_trace_chunk_threshold": 134_217_728, "gas_trace_chunk_slots": 2},
        )

    def test_each_version_axis_changes_calibration_id_and_run_validation_rejects_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            controlled = root / "controlled.toml"
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"test guest launcher")
            controlled.write_text(
                'normalization_reference_key = "opcode:0x01"\n'
                'q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
                'bridge_key_ids = ["opcode:0x01", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
            )
            schedule = versioned_schedule()
            with mock.patch.object(
                opcode_gas, "current_uzen_schedule", return_value=schedule
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "_rust_version", return_value="rustc test"
            ), mock.patch.object(
                opcode_gas, "_locked_package_version", return_value="6.3.0"
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    controlled,
                    guest_launcher=launcher,
                    implementation_revision="a" * 40,
                    complete_schedule_hash="b" * 64,
                )

            source_run = root / "runs" / experiment["calibration_id"]
            for relative in experiment["guest_artifacts"]:
                source = ROOT / relative
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(source.read_bytes())
            for field in CALIBRATION_VERSION_IDENTITY:
                with self.subTest(field=field):
                    changed = json.loads(json.dumps(experiment))
                    changed_identity = changed["calibration_identity"]
                    changed_identity["version_identity"][field] += "-changed"
                    if field == "primary_metric":
                        changed_identity["primary_metric"] += "-changed"
                        changed["primary_metric"] += "-changed"
                    changed_id = opcode_gas.sha256_bytes(
                        opcode_gas.canonical_json(changed_identity)
                    )[:24]
                    self.assertNotEqual(changed_id, experiment["calibration_id"])
                    changed["calibration_id"] = changed_id
                    changed_run = root / "runs" / changed_id
                    shutil.copytree(source_run, changed_run)
                    (changed_run / "experiment.json").write_text(json.dumps(changed))
                    (changed_run / "provenance.json").write_text(
                        json.dumps(opcode_gas.experiment_provenance_declaration(changed))
                    )
                    with mock.patch.object(
                        opcode_gas, "REPO_ROOT", root
                    ), mock.patch.object(
                        opcode_gas, "current_uzen_schedule", return_value=schedule
                    ), mock.patch.object(
                        opcode_gas, "git_head", return_value="a" * 40
                    ), mock.patch.object(
                        opcode_gas, "git_worktree_status", return_value=""
                    ):
                        with self.assertRaisesRegex(ValueError, "version identity"):
                            opcode_gas.validate_calibration_execution_identity(
                                changed_run
                            )

            missing = json.loads(json.dumps(experiment))
            del missing["calibration_identity"]["version_identity"]
            missing_id = opcode_gas.sha256_bytes(
                opcode_gas.canonical_json(missing["calibration_identity"])
            )[:24]
            missing["calibration_id"] = missing_id
            missing_run = root / "runs" / missing_id
            shutil.copytree(source_run, missing_run)
            (missing_run / "experiment.json").write_text(json.dumps(missing))
            (missing_run / "provenance.json").write_text(
                json.dumps(opcode_gas.experiment_provenance_declaration(missing))
            )
            with mock.patch.object(
                opcode_gas, "REPO_ROOT", root
            ), mock.patch.object(
                opcode_gas, "current_uzen_schedule", return_value=schedule
            ), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ):
                with self.assertRaisesRegex(ValueError, "version identity"):
                    opcode_gas.validate_calibration_execution_identity(missing_run)

    def test_publish_corpus_rejects_remote_size_that_does_not_match_local_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = pathlib.Path(tmp) / "corpus.tar"
            archive.write_bytes(b"frozen corpus")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()

            def fake_run(command, **kwargs):
                if command[:4] == ["gcloud", "storage", "objects", "describe"]:
                    return completed(command, json.dumps({"generation": "9", "size": str(archive.stat().st_size + 1)}))
                if command[:3] == ["gcloud", "storage", "cp"] and command[-1].startswith("gs://"):
                    return completed(command, "")
                if command[:3] == ["gcloud", "storage", "cp"]:
                    pathlib.Path(command[-1]).write_bytes(archive.read_bytes())
                    return completed(command, "")
                raise AssertionError(command)

            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                with self.assertRaisesRegex(ValueError, "size"):
                    opcode_gas.publish_corpus(archive, f"gs://bucket/{digest}.tar")

    def test_prepare_corpus_rejects_unverifiable_chain_spec_and_pre_unzen(self):
        rows = [{"network": "taiko_hoodi", "proposal_id": 7, "block_count": 1, "total_zkgas": 1, "purpose": "final_validation"}]
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas, "REPO_ROOT", pathlib.Path(tmp)), mock.patch.object(opcode_gas, "git_head", return_value="a" * 40), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
            root = pathlib.Path(tmp)
            spec = root / "spec.json"
            spec.write_text(json.dumps(actual_chain_spec_list("taiko_hoodi", 100)))
            with self.assertRaisesRegex(ValueError, "chain-spec hash"):
                with mock.patch.object(opcode_gas, "select_final_validation_corpus", return_value=rows):
                    opcode_gas.prepare_corpus(
                        corpus_root=root / "corpus",
                        l1_rpc_by_network={"taiko_hoodi": "https://l1.invalid"},
                        l2_rpc_by_network={"taiko_hoodi": "https://l2.invalid"},
                        chain_spec_hash_by_network={"taiko_hoodi": "not-a-digest"},
                        chain_spec_path_by_network={"taiko_hoodi": spec},
                    )

    def test_prepare_corpus_rejects_discovery_before_unzen(self):
        rows = [{"network": "taiko_hoodi", "proposal_id": 7, "block_count": 1, "total_zkgas": 1, "purpose": "final_validation"}]
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas, "REPO_ROOT", pathlib.Path(tmp)), mock.patch.object(opcode_gas, "git_head", return_value="a" * 40), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
            root = pathlib.Path(tmp)
            spec = root / "spec.json"
            spec.write_text(json.dumps(actual_chain_spec_list("taiko_hoodi", 1)))

            def fake_run(command, **kwargs):
                if any("stress_shasta_proposal.py" in argument for argument in command):
                    pathlib.Path(command[command.index("--proposal-out") + 1]).write_text(
                        json.dumps([{
                            "proposal_id": 7,
                            "l1_inclusion_block_number": 10,
                            "last_anchor_block_number": 9,
                            "l2_start": 1,
                            "l2_end": 1,
                        }])
                    )
                else:
                    pathlib.Path(command[command.index("--output") + 1]).write_text(
                        '{"blocks":[{"block_difficulty":1,"timestamp":0}]}\n'
                    )
                return subprocess_completed(command)

            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                with self.assertRaisesRegex(ValueError, "post-Unzen"):
                    with mock.patch.object(opcode_gas, "select_final_validation_corpus", return_value=rows):
                        opcode_gas.prepare_corpus(
                            corpus_root=root / "corpus",
                            l1_rpc_by_network={"taiko_hoodi": "https://l1.invalid"},
                            l2_rpc_by_network={"taiko_hoodi": "https://l2.invalid"},
                            chain_spec_hash_by_network={"taiko_hoodi": hashlib.sha256(spec.read_bytes()).hexdigest()},
                            chain_spec_path_by_network={"taiko_hoodi": spec},
                            implementation_revision="a" * 40,
                        )

    def test_prepare_validation_rejects_tampered_candidate_seal(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            revision = "a" * 40
            run = root / "runs/run"
            (run / "candidate").mkdir(parents=True)
            (run / "bridge").mkdir(parents=True)
            candidate = {"implementation_revision": revision}
            bridge = {"implementation_revision": revision}
            (run / "experiment.json").write_text(json.dumps({"implementation_revision": revision}))
            (run / "candidate/candidate-manifest.json").write_text(json.dumps(candidate))
            (run / "bridge/bridge-root.json").write_text(json.dumps(bridge))
            (run / "candidate/candidate.sha256").write_text("b" * 64 + "\n")
            (run / "bridge/bridge.sha256").write_text(opcode_gas.sha256_bytes(opcode_gas.canonical_json(bridge)) + "\n")
            with mock.patch.object(opcode_gas, "REPO_ROOT", root):
                corpus = write_frozen_corpus(root, revision)
                with mock.patch.object(opcode_gas, "git_head", return_value=revision), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                    with self.assertRaisesRegex(ValueError, "candidate root digest"):
                        opcode_gas.prepare_validation(root, run, corpus)

    def test_publish_existing_object_with_different_bytes_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = pathlib.Path(tmp) / "corpus.tar"
            archive.write_bytes(b"expected")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()

            def fake_run(command, **kwargs):
                if command[:3] == ["gcloud", "storage", "cp"] and command[-1].startswith("gs://"):
                    raise opcode_gas.subprocess.CalledProcessError(1, command)
                if command[:4] == ["gcloud", "storage", "objects", "describe"]:
                    return completed(command, json.dumps({"generation": "9", "size": "8"}))
                if command[:3] == ["gcloud", "storage", "cp"]:
                    pathlib.Path(command[-1]).write_bytes(b"different")
                    return completed(command, "")
                raise AssertionError(command)

            with mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                with self.assertRaisesRegex(ValueError, "readback SHA256 mismatch"):
                    opcode_gas.publish_corpus(archive, f"gs://bucket/{digest}.tar")

    def test_run_proposal_smoke_rejects_missing_or_relabelled_prepared_record(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = proposal_args(root, purpose="integration_smoke", smoke_record=root / "smoke.json")
            args.guest_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 79852,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "run_proposal_guest_input"
            ) as execute:
                with self.assertRaisesRegex(ValueError, "prepared integration_smoke"):
                    opcode_gas.cmd_run_proposal(args)
                execute.assert_not_called()

                args.smoke_record.write_text(json.dumps({
                    "network": "taiko_hoodi", "proposal_id": 1, "purpose": "final_validation"
                }))
                with self.assertRaisesRegex(ValueError, "cannot be relabeled"):
                    opcode_gas.cmd_run_proposal(args)
                execute.assert_not_called()

    def test_run_proposal_smoke_rejects_non_sp1_backend(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = proposal_args(
                root, purpose="integration_smoke", smoke_record=root / "smoke.json"
            )
            args.proof_type = "risc0"
            with mock.patch.object(opcode_gas, "run_proposal_guest_input") as execute:
                with self.assertRaisesRegex(ValueError, "requires --proof-type sp1"):
                    opcode_gas.cmd_run_proposal(args)
            execute.assert_not_called()

    def test_run_proposal_smoke_persists_verified_identity_in_output(self):
        guest_hash = "0x" + "ab" * 32
        public_output = "0x" + "12" * 32
        executed_guest_inputs = []

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = proposal_args(
                root, purpose="integration_smoke", smoke_record=root / "smoke.json"
            )
            args.guest_launcher.write_bytes(b"reviewed launcher")
            proposal_elf = root / "sp1_shasta_proposal.elf"
            proposal_elf.write_bytes(b"reviewed proposal ELF")
            args.guest_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 79852,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            fixture_sha256 = opcode_gas.sha256_file(args.guest_input)
            workload_id = opcode_gas.proposal_workload_id(fixture_sha256)
            args.smoke_record.write_text(json.dumps({
                "network": "taiko_hoodi",
                "proposal_id": 79852,
                "purpose": "integration_smoke",
                "fixture_sha256": fixture_sha256,
                "workload_id": workload_id,
            }))

            def fake_run(command, check):
                execution_input = pathlib.Path(command[command.index("--input") + 1])
                executed_guest_inputs.append(execution_input.read_bytes())
                json_out = pathlib.Path(command[command.index("--json-out") + 1])
                if command[command.index("--stage") + 1] == "proposal-trace":
                    args.guest_input.write_text(json.dumps({
                        "taiko": {
                            "proposal_id": 99999,
                            "chain_spec": {"name": "taiko_hoodi"},
                        }
                    }) + "\n")
                    json_out.write_bytes(b"\x1f\x8btrace")
                    json_out.with_name(
                        json_out.name.removesuffix(".json.gz") + ".summary.json"
                    ).write_text(json.dumps({
                        "status": "complete",
                        "guest_input_sha256": guest_hash,
                        "public_output": public_output,
                        "parity_passed": True,
                        "block_count": 1,
                        "partial_block_count": 0,
                    }))
                else:
                    json_out.write_text(json.dumps({
                        "input": str(execution_input),
                        "stage": "proposal",
                        "mode": "execute",
                        "sp1_execution_engine": "gas-estimator",
                        "sp1_gas_trace_chunk_threshold": 134_217_728,
                        "sp1_gas_trace_chunk_slots": 2,
                        "guest_input_sha256": guest_hash,
                        "guest_input_bincode_length": 1234,
                        "public_values": public_output,
                        "exit_code": 0,
                        "gas": 99,
                        "primary_workload_metric": {
                            "label": "prover_gas", "count": 99,
                        },
                        "sp1_proposal_elf_sha256": opcode_gas.sha256_file(proposal_elf),
                        "guest_launcher_sha256": opcode_gas.sha256_file(args.guest_launcher),
                    }))

            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "select_final_validation_corpus", return_value=[]
            ), mock.patch.object(
                opcode_gas, "production_sp1_proposal_elf_path", return_value=proposal_elf
            ), mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.cmd_run_proposal(args)

            row = json.loads(args.out.read_text())
            self.assertEqual(row["purpose"], "integration_smoke")
            self.assertEqual(row["network"], "taiko_hoodi")
            self.assertEqual(row["proposal_id"], 79852)
            self.assertEqual(row["input"], str(args.guest_input))
            self.assertEqual(row["fixture_sha256"], fixture_sha256)
            self.assertEqual(row["workload_id"], workload_id)
            self.assertEqual(len(set(executed_guest_inputs)), 1)
            self.assertEqual(
                opcode_gas.sha256_bytes(executed_guest_inputs[0]), fixture_sha256
            )

    def test_run_proposal_smoke_record_must_bind_the_executed_guest_input_bytes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            prepared_input = root / "prepared.json"
            executed_input = root / "executed.json"
            prepared_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 79852,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            executed_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 2,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            smoke_record = root / "smoke.json"
            smoke_record.write_text(json.dumps({
                "network": "taiko_hoodi",
                "proposal_id": 79852,
                "purpose": "integration_smoke",
                "fixture_sha256": opcode_gas.sha256_file(prepared_input),
            }))
            args = proposal_args(
                root, purpose="integration_smoke", smoke_record=smoke_record
            )
            args.guest_input = executed_input
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "select_final_validation_corpus", return_value=[]
            ), mock.patch.object(opcode_gas, "run_proposal_guest_input") as execute:
                with self.assertRaisesRegex(ValueError, "GuestInput"):
                    opcode_gas.cmd_run_proposal(args)
            execute.assert_not_called()

    def test_run_proposal_smoke_record_rejects_wrong_workload_identity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            args = proposal_args(
                root, purpose="integration_smoke", smoke_record=root / "smoke.json"
            )
            args.guest_input.write_text(json.dumps({
                "taiko": {
                    "proposal_id": 79852,
                    "chain_spec": {"name": "taiko_hoodi"},
                }
            }) + "\n")
            args.smoke_record.write_text(json.dumps({
                "network": "taiko_hoodi",
                "proposal_id": 79852,
                "purpose": "integration_smoke",
                "fixture_sha256": opcode_gas.sha256_file(args.guest_input),
                "workload_id": "00" * 32,
            }))
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "select_final_validation_corpus", return_value=[]
            ), mock.patch.object(opcode_gas, "run_proposal_guest_input") as execute:
                with self.assertRaisesRegex(ValueError, "workload identity"):
                    opcode_gas.cmd_run_proposal(args)
            execute.assert_not_called()

    def test_prepare_corpus_resolves_relative_actual_chain_spec_and_passes_it_to_preflight(self):
        rows = [{"network": "taiko_hoodi", "proposal_id": 7, "block_count": 1, "total_zkgas": 1, "purpose": "final_validation"}]
        calls = []

        def fake_run(command, **kwargs):
            calls.append(command)
            if any("stress_shasta_proposal.py" in part for part in command):
                pathlib.Path(command[command.index("--proposal-out") + 1]).write_text(json.dumps([{
                    "proposal_id": 7, "l1_inclusion_block_number": 10,
                    "last_anchor_block_number": 9, "l2_start": 1, "l2_end": 1,
                }]))
            else:
                pathlib.Path(command[command.index("--output") + 1]).write_text(
                    '{"blocks":[{"block_difficulty":1,"timestamp":1}]}\n'
                )
            return subprocess_completed(command)

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            spec = root / "chain-spec.json"
            spec.write_text(json.dumps(actual_chain_spec_list("taiko_hoodi", 1)))
            args = opcode_gas.build_parser().parse_args([
                "prepare-corpus", "--corpus-root", "corpus", "--manifest", "manifest.json",
                "--l1-rpc", "taiko_hoodi=https://l1.invalid",
                "--l2-rpc", "taiko_hoodi=https://l2.invalid",
                "--chain-spec-hash", f"taiko_hoodi={hashlib.sha256(spec.read_bytes()).hexdigest()}",
                "--chain-spec-file", "taiko_hoodi=chain-spec.json",
            ])
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "select_final_validation_corpus", return_value=rows
            ), mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.cmd_prepare_corpus(args)
                discovery = next(
                    command
                    for command in calls
                    if any("stress_shasta_proposal.py" in part for part in command)
                )
                self.assertEqual(
                    discovery[discovery.index("--l1-network") + 1], "hoodi"
                )
                preflight = next(command for command in calls if command[0] == "target/release/preflight")
                self.assertEqual(preflight[preflight.index("--chain-spec-file") + 1], str(spec))
                self.assertTrue((root / "corpus" / "taiko_hoodi" / "proposal_7.json").exists())
                self.assertTrue((root / "manifest.json").exists())

    def test_prepare_corpus_maps_taiko_mainnet_discovery_to_ethereum_l1(self):
        rows = [{
            "network": "taiko_mainnet",
            "proposal_id": 9,
            "block_count": 1,
            "total_zkgas": 1,
            "purpose": "final_validation",
        }]
        calls = []

        def fake_run(command, **kwargs):
            calls.append(command)
            if any("stress_shasta_proposal.py" in part for part in command):
                pathlib.Path(command[command.index("--proposal-out") + 1]).write_text(
                    json.dumps([{
                        "proposal_id": 9,
                        "l1_inclusion_block_number": 10,
                        "last_anchor_block_number": 9,
                        "l2_start": 1,
                        "l2_end": 1,
                    }])
                )
            else:
                pathlib.Path(command[command.index("--output") + 1]).write_text(
                    '{"blocks":[{"block_difficulty":1,"timestamp":1}]}\n'
                )
            return subprocess_completed(command)

        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            spec = root / "chain-spec.json"
            spec.write_text(json.dumps(actual_chain_spec_list("taiko_mainnet", 1)))
            args = opcode_gas.build_parser().parse_args([
                "prepare-corpus", "--corpus-root", "corpus",
                "--l1-rpc", "taiko_mainnet=https://l1.invalid",
                "--l2-rpc", "taiko_mainnet=https://l2.invalid",
                "--chain-spec-hash",
                f"taiko_mainnet={hashlib.sha256(spec.read_bytes()).hexdigest()}",
                "--chain-spec-file", "taiko_mainnet=chain-spec.json",
            ])
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "select_final_validation_corpus", return_value=rows
            ), mock.patch.object(opcode_gas.subprocess, "run", fake_run):
                opcode_gas.cmd_prepare_corpus(args)

        discovery = next(
            command
            for command in calls
            if any("stress_shasta_proposal.py" in part for part in command)
        )
        self.assertEqual(discovery[discovery.index("--l1-network") + 1], "ethereum")

    def test_controlled_manifest_is_materialized_and_swap_is_rejected_before_generation(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            manifest_a = root / "manifest-a.toml"
            manifest_b = root / "manifest-b.toml"
            launcher = root / "guest-launcher"
            launcher.write_bytes(b"test guest launcher")
            manifest_a.write_text(controlled_manifest_text("controlled-a"))
            manifest_b.write_text(controlled_manifest_text("controlled-b"))
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    manifest_a,
                    guest_launcher=launcher,
                    implementation_revision="a" * 40,
                    complete_schedule_hash="b" * 64,
                )
            run = root / "runs" / experiment["calibration_id"]
            self.assertEqual((run / "controlled-manifest.toml").read_bytes(), manifest_a.read_bytes())
            for relative in experiment["guest_artifacts"]:
                source = ROOT / relative
                destination = root / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(source.read_bytes())
            args = types.SimpleNamespace(
                calibration_run=run, manifest=manifest_b, out=root / "fixtures"
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "generate_cases"
            ) as generate:
                with self.assertRaisesRegex(ValueError, "controlled manifest"):
                    opcode_gas.cmd_generate(args)
            generate.assert_not_called()

    def test_controlled_execution_commands_reject_stale_head_before_side_effects(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = write_controlled_run(root, revision="a" * 40)
            common = dict(
                fixtures=root / "fixtures",
                guest_launcher=pathlib.Path("target/release/guest-launcher"),
                elf=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
                precompile_elf=pathlib.Path(
                    "crates/guests/elf/sp1_precompile_lab.elf"
                ),
                out=root / "runs.jsonl",
                calibration_run=run,
                controlled_manifest=run / "controlled-manifest.toml",
            )
            commands = (
                (
                    opcode_gas.cmd_generate,
                    types.SimpleNamespace(
                        calibration_run=run,
                        manifest=run / "controlled-manifest.toml",
                        out=root / "fixtures",
                        generator_max_count=8,
                    ),
                ),
                (opcode_gas.cmd_run, types.SimpleNamespace(**common)),
                (opcode_gas.cmd_run_controlled, types.SimpleNamespace(**common)),
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="c" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "generate_cases"
            ) as generate, mock.patch.object(
                opcode_gas, "run_guest_inputs"
            ) as launcher:
                for command, args in commands:
                    with self.subTest(command=command.__name__), self.assertRaisesRegex(
                        ValueError, "current HEAD"
                    ):
                        command(args)
            generate.assert_not_called()
            launcher.assert_not_called()
            self.assertFalse((run / "controlled-decisions.json").exists())
            self.assertFalse((root / "fixtures").exists())
            self.assertFalse((root / "runs.jsonl").exists())

    def test_controlled_execution_rejects_dirty_source_and_changed_guest_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            revision = "a" * 40
            run = write_controlled_run(root, revision=revision)
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value=revision
            ), mock.patch.object(
                opcode_gas,
                "git_worktree_status",
                return_value=" M experiments/opcode-gas/opcode_gas.py\n",
            ):
                with self.assertRaisesRegex(ValueError, "dirty implementation path"):
                    opcode_gas.validate_calibration_execution_identity(run)

            experiment = json.loads((run / "experiment.json").read_text())
            relative = next(iter(experiment["guest_artifacts"]))
            (root / relative).write_bytes(b"changed guest artifact")
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value=revision
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ):
                with self.assertRaisesRegex(ValueError, "frozen guest artifact changed"):
                    opcode_gas.validate_calibration_execution_identity(run)

    def test_controlled_run_rejects_resume_provenance_mismatch_before_guest_launcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = write_controlled_run(root)
            fixture_dir = root / "fixtures" / "case"
            fixture_dir.mkdir(parents=True)
            (fixture_dir / "guest-input.json").write_text("{}\n")
            (fixture_dir / "case.json").write_text(json.dumps({
                "kind": "opcode", "case": "add", "target_count": 1, "target_raw_gas": 3,
                "calibration_id": "wrong", "controlled_manifest_sha256": "b" * 64,
                "controlled_manifest_rows_sha256": "c" * 64,
            }))
            args = types.SimpleNamespace(
                fixtures=root / "fixtures", guest_launcher=pathlib.Path("target/release/guest-launcher"),
                elf=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
                precompile_elf=pathlib.Path("crates/guests/elf/sp1_precompile_lab.elf"),
                opcode_stage="opcode-lab", out=root / "runs.jsonl", calibration_run=run,
                controlled_manifest=run / "controlled-manifest.toml",
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "run_guest_inputs"
            ) as execute:
                with self.assertRaisesRegex(ValueError, "controlled manifest provenance"):
                    opcode_gas.cmd_run(args)
            execute.assert_not_called()

    def test_controlled_run_rejects_changed_guest_input_before_guest_launcher(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            run = write_controlled_run(root)
            fixture_dir = root / "fixtures" / "case"
            fixture_dir.mkdir(parents=True)
            guest_input = fixture_dir / "guest-input.json"
            guest_input.write_text('{"target_count":1}\n')
            identity = json.loads((run / "experiment.json").read_text())[
                "calibration_identity"
            ]
            (fixture_dir / "case.json").write_text(json.dumps({
                "kind": "opcode",
                "case": "add",
                "target_count": 1,
                "target_raw_gas": 3,
                "calibration_id": run.name,
                "controlled_manifest_sha256": identity["controlled_manifest_sha256"],
                "controlled_manifest_rows_sha256": identity[
                    "controlled_manifest_rows_sha256"
                ],
                "fixture_sha256": opcode_gas.sha256_file(guest_input),
            }))
            guest_input.write_text('{"target_count":2}\n')
            args = types.SimpleNamespace(
                fixtures=root / "fixtures",
                guest_launcher=pathlib.Path("target/release/guest-launcher"),
                elf=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
                precompile_elf=pathlib.Path(
                    "crates/guests/elf/sp1_precompile_lab.elf"
                ),
                opcode_stage="opcode-lab",
                out=root / "runs.jsonl",
                calibration_run=run,
                controlled_manifest=run / "controlled-manifest.toml",
            )
            with mock.patch.object(opcode_gas, "REPO_ROOT", root), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas, "run_guest_inputs"
            ) as execute:
                with self.assertRaisesRegex(ValueError, "GuestInput"):
                    opcode_gas.cmd_run(args)
            execute.assert_not_called()


def completed(args, stdout):
    return subprocess_completed(args, stdout)


def subprocess_completed(args, stdout=""):
    return opcode_gas.subprocess.CompletedProcess(args=args, returncode=0, stdout=stdout, stderr="")


def complete_rows():
    rows = []
    for i in range(60):
        network = "taiko_hoodi" if i < 40 else "taiko_mainnet"
        digest = f"{i:064x}"
        rows.append({"network": network, "proposal_id": i + 1, "purpose": "final_validation", "block_count": 1, "total_zkgas": 1, "guest_input_sha256": digest, "workload_id": opcode_gas.proposal_workload_id(digest)})
    return rows


def actual_chain_spec_list(network, unzen_timestamp):
    return [{
        "name": network,
        "chain_id": 167013,
        "is_taiko": True,
        "hard_forks": {"UNZEN": {"Timestamp": unzen_timestamp}},
        "rpc": "https://l2.invalid",
        "beacon_rpc": None,
        "verifier_address_forks": {"UNZEN": {"SP1": "0x1"}},
        "genesis_time": 0,
        "seconds_per_slot": 1,
    }]


def proposal_args(root, *, purpose, smoke_record):
    return types.SimpleNamespace(
        guest_launcher=root / "guest-launcher",
        guest_input=root / "guest-input.json",
        proof_type="sp1",
        case="proposal-1",
        target_raw_gas=1,
        target_count=1,
        risc0_execution_po2=20,
        out=root / "run.jsonl",
        purpose=purpose,
        smoke_record=smoke_record,
        network="taiko_hoodi",
        proposal_id=79852,
    )


def controlled_manifest_text(name):
    return (
        f'name = "{name}"\n'
        'backend = "sp1"\n'
        'variants = [1]\n'
        'normalization_reference_key = "opcode:0x01"\n'
        'q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
        'bridge_key_ids = ["opcode:0x01", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
        '[[cases]]\n'
        'name = "add"\n'
        'scenario = "controlled"\n'
        'template = "repeat"\n'
        'target_raw_gas = 3\n'
        'opcode = "0x01"\n'
    )


def write_controlled_run(root, revision="a" * 40):
    manifest = root / "controlled.toml"
    manifest.write_text(controlled_manifest_text("controlled"))
    digest = hashlib.sha256(manifest.read_bytes()).hexdigest()
    rows = opcode_gas.controlled_manifest_rows_sha256(manifest)
    guest_artifact = root / "crates/guests/elf/sp1-test.elf"
    guest_artifact.parent.mkdir(parents=True)
    guest_artifact.write_bytes(b"test SP1 guest artifact")
    guest_artifacts = {
        str(guest_artifact.relative_to(root)): opcode_gas.sha256_file(guest_artifact)
    }
    guest_launcher = root / "target/release/guest-launcher"
    guest_launcher.parent.mkdir(parents=True)
    guest_launcher.write_bytes(b"test guest launcher")
    identity = {
        "implementation_revision": revision,
        "alethia_reth_revision": "d" * 40,
        "rust_version": "rustc test",
        "sp1_sdk_version": "test-sdk",
        "controlled_manifest_sha256": digest,
        "controlled_manifest_rows_sha256": rows,
        "complete_schedule_sha256": "e" * 64,
        "guest_artifacts": guest_artifacts,
        "guest_artifacts_sha256": opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(guest_artifacts)
        ),
        "guest_launcher_sha256": opcode_gas.sha256_file(guest_launcher),
        "normalization_reference_key": "opcode:0x01",
        "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
        "primary_metric": "proverGas",
        "sp1_instruction_count": "secondary_non_gating",
        "workload_identity_schema_version": 1,
        "workload_canonicalization": "sha256(canonical_json(workload_spec))",
        "primary_formulas": {"candidate_cost": "g_p(k) / r(k)"},
        "q_formula": list(opcode_gas.Q_FORMULA),
        "out_of_fit_checkpoint": {"mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS},
        "quality_gates": {"checkpoint_ape_max": 0.10},
        "bridge": {"model": "through_origin_equal_key_median"},
        "version_identity": dict(CALIBRATION_VERSION_IDENTITY),
    }
    calibration_id = opcode_gas.sha256_bytes(opcode_gas.canonical_json(identity))[:24]
    run = root / "runs" / calibration_id
    run.mkdir(parents=True)
    (run / "controlled-manifest.toml").write_bytes(manifest.read_bytes())
    (run / "controlled-manifest.sha256").write_text(digest + "\n")
    experiment = {
        "schema_version": 1,
        "calibration_id": calibration_id,
        "dirty_state": False,
        "calibration_identity": identity,
        **{
            field: identity[field]
            for field in opcode_gas.EXPERIMENT_IDENTITY_DUPLICATE_FIELDS
        },
    }
    (run / "experiment.json").write_text(json.dumps(experiment))
    (run / "provenance.json").write_text(
        json.dumps(opcode_gas.experiment_provenance_declaration(experiment))
    )
    return run


def write_frozen_corpus(root, revision):
    corpus_root = root / "experiments/opcode-gas/corpora/sp1-mainnet-hoodi-v1"
    rows = []
    for selected in opcode_gas.select_final_validation_corpus():
        block_count = selected["block_count"]
        total = selected["total_zkgas"]
        base, remainder = divmod(total, block_count)
        blocks = [{"block_difficulty": base + (1 if index < remainder else 0)} for index in range(block_count)]
        fixture = corpus_root / selected["network"] / f"proposal_{selected['proposal_id']}.json"
        fixture.parent.mkdir(parents=True, exist_ok=True)
        fixture.write_text(json.dumps({"blocks": blocks}) + "\n")
        digest = hashlib.sha256(fixture.read_bytes()).hexdigest()
        rows.append({
            **selected,
            "fixture_path": str(fixture.relative_to(root)),
            "guest_input_sha256": digest,
            "workload_id": opcode_gas.proposal_workload_id(digest),
            "block_difficulty_sum": total,
            "acquisition_chain_spec_sha256": "c" * 64,
        })
    archive = corpus_root.parent / "sp1-mainnet-hoodi-v1.tar"
    archive_sha = opcode_gas.write_deterministic_tar(corpus_root, archive)
    manifest = {
        "implementation_revision": revision,
        "corpus_root": str(corpus_root.relative_to(root)),
        "archive_path": str(archive.relative_to(root)),
        "archive_uri": f"gs://bucket/{archive_sha}.tar#9",
        "archive_generation": 9,
        "archive_size_bytes": archive.stat().st_size,
        "archive_sha256": archive_sha,
        "rows": rows,
    }
    path = root / "corpus.json"
    path.write_text(json.dumps(manifest))
    return path


if __name__ == "__main__":
    unittest.main()
