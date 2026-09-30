import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
OPCODE_GAS = ROOT / "experiments" / "opcode-gas"
sys.path.insert(0, str(OPCODE_GAS))

import block_comparison
import opcode_gas
from test_sp1_composite_estimator import (
    CONTEXT_APPROXIMATION_SOURCE,
    CORRECTED_OSAKA_CORE_SOURCE,
    COVERAGE_V5_SOURCE,
    HIGHER_LAYER_SOURCE,
    SP1_PROPOSAL_ELF,
    SP1_PROPOSAL_VK,
    STATEFUL_SOURCE,
)


class BlockComparisonManifestTests(unittest.TestCase):
    @staticmethod
    def _rehash_manifest(manifest):
        manifest.pop("artifact_sha256", None)
        manifest["artifact_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(manifest)
        )

    @classmethod
    def _build_manifest(cls, **overrides):
        arguments = {
            "candidate": cls.candidate,
            "candidate_file_bytes": cls.candidate_bytes,
            "source_bytes": cls.sources,
            "frozen_bundles": cls.bundles,
            "launcher_path": cls.launcher,
        }
        arguments.update(overrides)
        with mock.patch.object(
            block_comparison, "_run_native_freezes", return_value=cls.bundles
        ):
            return block_comparison.build_manifest(**arguments)

    @classmethod
    def _seal_manifest(cls, manifest, output_root):
        with mock.patch.object(
            block_comparison, "_read_bound_sources", return_value=cls.sources
        ), mock.patch.object(
            block_comparison, "_run_native_freezes", return_value=cls.bundles
        ):
            return block_comparison.seal_manifest(
                manifest,
                output_root,
                source_paths=cls.source_paths,
            )

    @classmethod
    def setUpClass(cls):
        subprocess.run(
            ["cargo", "build", "--release", "-p", "guest-launcher"],
            cwd=ROOT,
            check=True,
            timeout=300,
        )
        cls.launcher = ROOT / "target/release/guest-launcher"
        with mock.patch.object(
            opcode_gas, "git_head", return_value="f" * 40
        ), mock.patch.object(
            opcode_gas, "git_worktree_status", return_value=""
        ), mock.patch.object(
            opcode_gas, "git_has_local_commit", return_value=True
        ):
            cls.candidate = opcode_gas.build_composite_estimator_artifact(
                augmented_core_path=CORRECTED_OSAKA_CORE_SOURCE,
                operation_coverage_path=COVERAGE_V5_SOURCE,
                higher_layer_package=HIGHER_LAYER_SOURCE,
                stateful_result_path=STATEFUL_SOURCE,
                context_approximation_path=CONTEXT_APPROXIMATION_SOURCE,
                guest_launcher_path=cls.launcher,
                proposal_elf_path=SP1_PROPOSAL_ELF,
                proposal_vk_path=SP1_PROPOSAL_VK,
                terminal_approximation=True,
            )
        cls.candidate_bytes = (
            json.dumps(cls.candidate, indent=2, sort_keys=True) + "\n"
        ).encode()
        cls.temporary = tempfile.TemporaryDirectory()
        cls.prepared_path = pathlib.Path(cls.temporary.name) / "prepared.json"
        cls.bundles = block_comparison.prepare_bundles(
            launcher_path=cls.launcher, output_path=cls.prepared_path
        )
        cls.sources = {
            "candidate": cls.candidate_bytes,
            "launcher": cls.launcher.read_bytes(),
            "elf": SP1_PROPOSAL_ELF.read_bytes(),
            "vk": SP1_PROPOSAL_VK.read_bytes(),
            "trace": (ROOT / "crates/zkgas-trace/src/reconstruct.rs").read_bytes(),
            "fixture": (ROOT / "bin/guest-launcher/src/controlled_workload.rs").read_bytes(),
            "builder": (OPCODE_GAS / "block_comparison.py").read_bytes(),
        }
        cls.source_paths = {
            "candidate": ROOT / "unused-test-candidate.json",
            "launcher": cls.launcher,
            "elf": SP1_PROPOSAL_ELF,
            "vk": SP1_PROPOSAL_VK,
            "trace": ROOT / "crates/zkgas-trace/src/reconstruct.rs",
            "fixture": ROOT / "bin/guest-launcher/src/controlled_workload.rs",
            "builder": OPCODE_GAS / "block_comparison.py",
        }
        cls.manifest = cls._build_manifest()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_fixed_matrix_and_real_prepare_cover_every_required_dimension(self):
        requests = block_comparison.row_requests()
        self.assertEqual(len(requests), 32)
        self.assertEqual(sum(row["partition"] == "calibration" for row in requests), 12)
        self.assertEqual(sum(row["partition"] == "validation" for row in requests), 20)
        self.assertEqual(
            {row["split"] for row in requests if row["partition"] == "calibration"},
            {"fit"},
        )
        self.assertEqual(
            {row["split"] for row in requests if row["partition"] == "validation"},
            {"holdout"},
        )
        self.assertGreaterEqual(
            {row["category"] for row in requests}, block_comparison.REQUIRED_CATEGORIES
        )
        self.assertEqual(
            block_comparison.covered_context_classes(requests),
            block_comparison.REQUIRED_CONTEXT_CLASSES,
        )
        self.assertEqual(len({block_comparison.workload_id(row) for row in requests}), 32)
        self.assertEqual(json.loads(self.prepared_path.read_bytes()), self.bundles)
        self.assertTrue(
            all(
                set(bundle)
                == {
                    "schema_version",
                    "fixture_spec_sha256",
                    "spec",
                    "observation",
                    "candidate_trace",
                }
                for bundle in self.bundles
            )
        )

    def test_manifest_is_compact_and_does_not_embed_native_traces(self):
        encoded = opcode_gas.canonical_json(self.manifest)
        self.assertEqual(self.manifest["schema_version"], 2)
        self.assertLess(len(encoded), 1024 * 1024)
        self.assertNotIn(b'"candidate_trace"', encoded)
        self.assertNotIn(b'"frozen_bundle"', encoded)
        for row in self.manifest["rows"]:
            self.assertEqual(
                set(row),
                {
                    "partition",
                    "category",
                    "candidate_identity",
                    "workload_id",
                    "row_id",
                    "backend_input_sha256",
                    "candidate_coverage_complete",
                    "candidate_predicted_prover_gas",
                    "bundle_sha256",
                    "trace_sha256",
                    "frozen_identity",
                },
            )
            self.assertEqual(
                set(row["frozen_identity"]),
                {"schema_version", "fixture_spec_sha256", "spec", "observation"},
            )

    def test_real_native_traces_have_zero_gaps_and_exact_storage_branches(self):
        self.assertEqual(len(self.manifest["rows"]), 32)
        self.assertTrue(all(row["candidate_coverage_complete"] for row in self.manifest["rows"]))
        self.assertTrue(
            all(
                row["frozen_identity"]["observation"]["finalized_block_zkgas"]
                > 0
                for row in self.manifest["rows"]
            )
        )
        identities = [
            value
            for row in self.manifest["rows"]
            for value in (row["workload_id"], row["row_id"], row["backend_input_sha256"])
        ]
        self.assertEqual(len(identities), len(set(identities)))
        for partition in ("calibration", "validation"):
            storage_keys = {
                key
                for row in self.manifest["rows"]
                if row["partition"] == partition
                for key in row["frozen_identity"]["spec"].get(
                    "expected_storage_features", {}
                )
            }
            for branch in ("set", "clear", "reset", "restore_original"):
                self.assertTrue(
                    any(key.endswith(f"branch:{branch}") for key in storage_keys),
                    f"{partition} is missing {branch}",
                )
            self.assertTrue(any(key.startswith("storage_load:") for key in storage_keys))
        for request, bundle in zip(
            block_comparison.row_requests(), self.bundles, strict=True
        ):
            program = request["program"]
            if program.get("kind") != "block_comparison" or program["scenario"][
                "kind"
            ] not in {"storage_round_trip", "mixed"}:
                continue
            scenario = program["scenario"]
            repeats = scenario["repeat_count"]
            transaction_groups = request["block_count"] * request["transaction_count"]
            total = repeats * transaction_groups
            original = scenario["original_value"]
            written = scenario["written_value"]
            first_branch = (
                "set" if original == 0 else "clear" if written == 0 else "reset"
            )
            expected = {
                f"storage_store:opcode:0x55:access:cold:branch:{first_branch}": transaction_groups,
                "storage_load:opcode:0x54:access:warm": total,
                "storage_store:opcode:0x55:access:warm:branch:restore_original": total,
            }
            if total > transaction_groups:
                expected[
                    f"storage_store:opcode:0x55:access:warm:branch:{first_branch}"
                ] = total - transaction_groups
            self.assertEqual(
                bundle["spec"].get("expected_storage_features", {}), expected
            )

    def test_old_candidate_and_native_identity_tamper_fail_closed(self):
        old_path = next((OPCODE_GAS / "estimators").glob("0bae*/estimator.json"))
        old = json.loads(old_path.read_bytes())
        with self.assertRaisesRegex(ValueError, "schema-5 successor"):
            self._build_manifest(
                candidate=old,
                candidate_file_bytes=old_path.read_bytes(),
                source_bytes={**self.sources, "candidate": old_path.read_bytes()},
            )
        mutations = (
            lambda bundle: bundle.__setitem__("fixture_spec_sha256", "0" * 64),
            lambda bundle: bundle["spec"].__setitem__(
                "expected_final_state_root", "0x" + "11" * 32
            ),
            lambda bundle: bundle["spec"].__setitem__("expected_raw_gas_by_key", {}),
            lambda bundle: bundle["candidate_trace"].__setitem__("status", "failed"),
        )
        for mutate in mutations:
            bundles = copy.deepcopy(self.bundles)
            mutate(bundles[0])
            with self.assertRaises(ValueError):
                self._build_manifest(frozen_bundles=bundles)

    def test_forbidden_fields_atomic_create_only_and_exact_replay(self):
        for field in ("actual_prover_gas", "scalar", "mape", "proposal_id", "verdict"):
            manifest = copy.deepcopy(self.manifest)
            manifest["rows"][0][field] = 1
            with self.assertRaisesRegex(ValueError, "forbidden"):
                block_comparison.validate_manifest(manifest)
        for source in self.manifest["sources"].values():
            self.assertFalse(pathlib.PurePosixPath(source["path"]).is_absolute())
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            sealed = self._seal_manifest(self.manifest, root)
            self.assertEqual(sealed.stat().st_mode & 0o777, 0o444)
            with mock.patch.object(
                block_comparison, "_read_bound_sources", return_value=self.sources
            ), mock.patch.object(
                block_comparison, "_run_native_freezes", return_value=self.bundles
            ):
                self.assertEqual(
                    block_comparison.verify_sealed_manifest(
                        sealed, source_paths=self.source_paths
                    ),
                    self.manifest,
                )
            sealed.chmod(0o644)
            with mock.patch.object(
                block_comparison, "_read_bound_sources", return_value=self.sources
            ), mock.patch.object(
                block_comparison, "_run_native_freezes", return_value=self.bundles
            ):
                self.assertEqual(
                    block_comparison.verify_sealed_manifest(
                        sealed, source_paths=self.source_paths
                    ),
                    self.manifest,
                )
            for mode in (0o600, 0o666, 0o755):
                sealed.chmod(mode)
                with self.subTest(mode=oct(mode)), self.assertRaisesRegex(
                    ValueError, "inventory"
                ):
                    block_comparison.verify_sealed_manifest(
                        sealed, source_paths=self.source_paths
                    )
            sealed.chmod(0o444)
            linked = root / "linked"
            linked.symlink_to(sealed.parent, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "inventory"):
                block_comparison.verify_sealed_manifest(
                    linked / "manifest.json", source_paths=self.source_paths
                )
            with self.assertRaises(FileExistsError):
                self._seal_manifest(self.manifest, root)
            (sealed.parent / "extra").write_text("tamper")
            with self.assertRaisesRegex(ValueError, "inventory"):
                block_comparison.verify_sealed_manifest(
                    sealed, source_paths=self.source_paths
                )

    def test_every_source_byte_and_duplicate_identity_tamper_is_rejected(self):
        for name in self.sources:
            sources = dict(self.sources)
            sources[name] += b"tamper"
            with self.subTest(source=name), self.assertRaises(ValueError):
                self._build_manifest(
                    candidate_file_bytes=sources["candidate"],
                    source_bytes=sources,
                )
        with mock.patch.object(
            block_comparison, "_read", return_value=b"different launcher bytes"
        ), self.assertRaisesRegex(ValueError, "executed launcher bytes"):
            self._build_manifest()
        with mock.patch.object(
            block_comparison,
            "_read",
            side_effect=[self.sources["launcher"], b"changed during replay"],
        ), mock.patch.object(
            block_comparison,
            "_run_native_freezes",
            return_value=self.bundles,
        ), self.assertRaisesRegex(ValueError, "changed during native replay"):
            block_comparison._run_bound_native_freezes(
                self.launcher, self.sources["launcher"]
            )
        for field in ("workload_id", "row_id", "backend_input_sha256"):
            manifest = copy.deepcopy(self.manifest)
            manifest["rows"][1][field] = manifest["rows"][0][field]
            self._rehash_manifest(manifest)
            with self.subTest(identity=field), self.assertRaises(ValueError):
                block_comparison.validate_manifest(manifest)

    def test_compact_hash_evidence_trace_and_prediction_tamper_are_rejected(self):
        mutations = {
            "bundle_hash": lambda row: row.__setitem__("bundle_sha256", "0" * 64),
            "compact_evidence": lambda row: row["frozen_identity"]["spec"].__setitem__(
                "expected_final_state_root", "0x" + "11" * 32
            ),
            "trace_hash": lambda row: row.__setitem__("trace_sha256", "0" * 64),
            "prediction": lambda row: row.__setitem__(
                "candidate_predicted_prover_gas", "1"
            ),
        }
        for name, mutate in mutations.items():
            manifest = copy.deepcopy(self.manifest)
            mutate(manifest["rows"][0])
            self._rehash_manifest(manifest)
            with tempfile.TemporaryDirectory() as temporary:
                output_root = pathlib.Path(temporary)
                with self.subTest(tamper=name), self.assertRaises(ValueError):
                    self._seal_manifest(manifest, output_root)
                self.assertEqual(list(output_root.iterdir()), [])

    def test_native_replay_rejects_self_consistent_swapped_trace(self):
        forged = copy.deepcopy(self.bundles)
        forged_trace = copy.deepcopy(forged[1]["candidate_trace"])
        backend_id = forged[0]["observation"]["backend_input_sha256"]
        forged_trace["guest_input_sha256"] = f"0x{backend_id}"
        host_trace_id = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(forged_trace)
        )
        forged[0]["candidate_trace"] = forged_trace
        forged[0]["spec"]["expected_host_trace_sha256"] = host_trace_id
        forged[0]["observation"]["host_trace_sha256"] = host_trace_id
        forged[0]["fixture_spec_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(forged[0]["spec"])
        )
        block_comparison._validated_bundle(
            forged[0], block_comparison.row_requests()[0]
        )

        with self.assertRaisesRegex(ValueError, "exact native launcher replay"):
            self._build_manifest(frozen_bundles=forged)

        forged_manifest = copy.deepcopy(self.manifest)
        forged_manifest["rows"][0]["frozen_identity"] = (
            block_comparison._compact_identity(forged[0])
        )
        forged_manifest["rows"][0]["bundle_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(forged[0])
        )
        forged_manifest["rows"][0]["trace_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(forged_trace)
        )
        forged_estimate = block_comparison.estimate_trace(
            self.candidate, forged_trace
        )
        self.assertTrue(forged_estimate["coverage_complete"])
        self.assertEqual(forged_estimate["gaps"], [])
        forged_manifest["rows"][0]["candidate_predicted_prover_gas"] = (
            forged_estimate["predicted_prover_gas"]
        )
        self._rehash_manifest(forged_manifest)
        block_comparison.validate_manifest(forged_manifest)
        with tempfile.TemporaryDirectory() as temporary:
            output_root = pathlib.Path(temporary)
            with self.assertRaisesRegex(ValueError, "exact native launcher replay"):
                self._seal_manifest(forged_manifest, output_root)
            self.assertEqual(list(output_root.iterdir()), [])
            sealed = block_comparison._publish_manifest(
                forged_manifest, pathlib.Path(temporary)
            )
            with mock.patch.object(
                block_comparison, "_read_bound_sources", return_value=self.sources
            ), mock.patch.object(
                block_comparison, "_run_native_freezes", return_value=self.bundles
            ), self.assertRaisesRegex(ValueError, "exact native launcher replay"):
                block_comparison.verify_sealed_manifest(
                    sealed, source_paths=self.source_paths
                )

    def test_prepare_rejects_symlink_launcher_and_existing_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            link = root / "guest-launcher"
            link.symlink_to(self.launcher)
            with self.assertRaisesRegex(ValueError, "not canonical"):
                block_comparison.prepare_bundles(
                    launcher_path=link, output_path=root / "prepared.json"
                )
            with mock.patch.object(block_comparison.subprocess, "run") as run:
                with self.assertRaises(FileExistsError):
                    block_comparison.prepare_bundles(
                        launcher_path=self.launcher, output_path=self.prepared_path
                    )
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
