import hashlib
import json
import pathlib
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


FIXTURES = ROOT / "tests/fixtures/risc0-zkgas/2026-09-02-m2-aggregation-direct-v3"


class RunManifestTests(unittest.TestCase):
    def test_complete_schedule_has_all_opcode_identities_and_hashes_every_field(self):
        raw = {
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
            spec.write_text(json.dumps({"taiko_hoodi": {"unzen_time": 1}}))
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
            controlled.write_text('normalization_reference_key = "opcode:0x01"\nq_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\nbridge_key_ids = ["opcode:0x01", "opcode:0x02", "precompile:0x1", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n')
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=" crates/prover/src/lib.rs\n"):
                with self.assertRaisesRegex(ValueError, "implementation path"):
                    opcode_gas.prepare_calibration(
                        root,
                        controlled,
                        implementation_revision="a" * 40,
                        complete_schedule_hash="b" * 64,
                    )
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ):
                experiment = opcode_gas.prepare_calibration(
                    root,
                    controlled,
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
                with mock.patch.object(opcode_gas, "git_head", return_value=revision), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                    validation = opcode_gas.prepare_validation(root, run, corpus_path)
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

    def test_parser_exposes_freeze_and_sealing_commands(self):
        parser = opcode_gas.build_parser()
        self.assertEqual(
            parser.parse_args(["prepare-corpus", "--corpus-root", "/tmp/corpus", "--l1-rpc", "taiko_hoodi=https://l1", "--l2-rpc", "taiko_hoodi=https://l2", "--chain-spec-hash", "taiko_hoodi=" + "a" * 64, "--chain-spec-file", "taiko_hoodi=/tmp/spec.json"]).command,
            "prepare-corpus",
        )
        self.assertEqual(parser.parse_args(["publish-corpus", "--archive", "/tmp/a.tar", "--object-uri", "gs://bucket/x.tar", "--manifest", "/tmp/manifest.json"]).command, "publish-corpus")
        self.assertEqual(parser.parse_args(["prepare-calibration", "--out", "/tmp/out", "--controlled-manifest", "/tmp/control.toml"]).command, "prepare-calibration")
        self.assertEqual(parser.parse_args(["prepare-validation", "--out", "/tmp/out", "--run", "/tmp/run", "--corpus", "/tmp/corpus.json"]).command, "prepare-validation")
        self.assertEqual(parser.parse_args(["prepare-integration-smoke", "--network", "taiko_hoodi", "--proposal-id", "1"]).command, "prepare-integration-smoke")

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

    def test_calibration_records_exact_formula_checkpoint_versions_and_artifacts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            controlled = root / "controlled.toml"
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
            controlled.write_text(
                'normalization_reference_key = "opcode:0x01"\n'
                'q_formula = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
                'bridge_key_ids = ["opcode:0x01", "proposal_startup", "block_base", "tx_base", "native_value_transfer"]\n'
            )
            with mock.patch.object(opcode_gas, "git_worktree_status", return_value=""), mock.patch.object(
                opcode_gas, "git_head", return_value="a" * 40
            ), mock.patch.object(opcode_gas, "_rust_version", return_value="rustc test"), mock.patch.object(
                opcode_gas, "_locked_package_version", return_value="6.3.0"
            ):
                experiment = opcode_gas.prepare_calibration(
                    root, controlled, implementation_revision="a" * 40, complete_schedule_hash="b" * 64
                )
        identity = experiment["calibration_identity"]
        self.assertEqual(identity["rust_version"], "rustc test")
        self.assertEqual(identity["sp1_sdk_version"], "6.3.0")
        self.assertEqual(identity["q_formula"], opcode_gas.Q_FORMULA)
        self.assertEqual(identity["out_of_fit_checkpoint"], {"mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS, "ape_max": 0.10})
        self.assertEqual(identity["guest_artifacts_sha256"], experiment["guest_artifacts_sha256"])
        self.assertEqual(identity["guest_artifacts"], experiment["guest_artifacts"])

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
            spec.write_text(json.dumps({"taiko_hoodi": {"unzen_time": 100}}))
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
            spec.write_text(json.dumps({"taiko_hoodi": {"unzen_time": 1}}))

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
                    with self.assertRaisesRegex(ValueError, "candidate seal"):
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
