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
                out.write_text(json.dumps([{"proposal_id": 7, "l1_inclusion_block_number": 10, "last_anchor_block_number": 9, "l2_start": 1, "l2_end": 1}]))
            else:
                out = pathlib.Path(command[command.index("--output") + 1])
                out.write_text('{"blocks":[{"block_difficulty":1}]}\n')
            return subprocess_completed(command)

        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(opcode_gas.subprocess, "run", fake_run):
            manifest = opcode_gas.prepare_corpus(
                rows=rows,
                corpus_root=pathlib.Path(tmp) / "corpus",
                l1_rpc_by_network={"taiko_hoodi": "https://l1.invalid"},
                l2_rpc_by_network={"taiko_hoodi": "https://l2.invalid"},
                chain_spec_hash_by_network={"taiko_hoodi": "a" * 64},
            )
        self.assertEqual(manifest["rows"][0]["purpose"], "final_validation")
        preflight = next(call for call in calls if "preflight" in call[0])
        self.assertIn("--validate", preflight)
        self.assertNotIn("true", preflight)
        self.assertIn("--proposal-ids", calls[0])

    def test_publish_corpus_requires_exact_generation_readback_and_hash(self):
        calls = []
        with tempfile.TemporaryDirectory() as tmp:
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
            controlled.write_text('normalization_reference_key = "opcode:0x01"\nbridge_key_ids = ["opcode:0x01", "opcode:0x02", "precompile:0x1"]\n')
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
            (run / "candidate" / "candidate.sha256").write_text("b" * 64 + "\n")
            (run / "bridge" / "bridge.sha256").write_text("c" * 64 + "\n")
            (run / "candidate" / "candidate-manifest.json").write_text(
                json.dumps({"implementation_revision": revision})
            )
            (run / "bridge" / "bridge-root.json").write_text(
                json.dumps({"implementation_revision": revision})
            )
            (run / "experiment.json").write_text(json.dumps({"implementation_revision": revision}))
            corpus = {"implementation_revision": revision, "archive_uri": "gs://bucket/x.tar#9", "archive_generation": 9, "archive_size_bytes": 1, "archive_sha256": "d" * 64, "rows": complete_rows()}
            corpus_path = root / "corpus.json"
            corpus_path.write_text(json.dumps(corpus))
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
            parser.parse_args(["prepare-corpus", "--corpus-root", "/tmp/corpus", "--l1-rpc", "taiko_hoodi=https://l1", "--l2-rpc", "taiko_hoodi=https://l2", "--chain-spec-hash", "taiko_hoodi=" + "a" * 64]).command,
            "prepare-corpus",
        )
        self.assertEqual(parser.parse_args(["publish-corpus", "--archive", "/tmp/a.tar", "--object-uri", "gs://bucket/x.tar", "--manifest", "/tmp/manifest.json"]).command, "publish-corpus")
        self.assertEqual(parser.parse_args(["prepare-calibration", "--out", "/tmp/out", "--controlled-manifest", "/tmp/control.toml"]).command, "prepare-calibration")
        self.assertEqual(parser.parse_args(["prepare-validation", "--out", "/tmp/out", "--run", "/tmp/run", "--corpus", "/tmp/corpus.json"]).command, "prepare-validation")


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


if __name__ == "__main__":
    unittest.main()
