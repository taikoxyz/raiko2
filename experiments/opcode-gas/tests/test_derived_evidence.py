import json
import pathlib
import sys
import tempfile
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas


class DerivedCoreEvidenceTests(unittest.TestCase):
    """Exercise derived publication without requiring guest execution fixtures."""

    source_revision = opcode_gas.git_head()
    derivation_revision = "b" * 40

    def _source_run(
        self, root: pathlib.Path, *, implementation_revision: str | None = None
    ) -> tuple[pathlib.Path, dict]:
        identity = {
            "implementation_revision": implementation_revision or self.source_revision,
            "alethia_reth_revision": "c" * 40,
            "complete_schedule_sha256": "d" * 64,
            "controlled_manifest_sha256": "e" * 64,
            "controlled_manifest_rows_sha256": "f" * 64,
            "guest_artifacts": {
                "crates/guests/elf/sp1_opcode_lab.elf": "1" * 64
            },
            "guest_artifacts_sha256": "",
            "guest_launcher_sha256": "2" * 64,
            "rust_version": "rustc test",
            "sp1_sdk_version": "test-sdk",
            "normalization_reference_key": "opcode:0x01",
            "primary_metric": "proverGas",
            "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
            "sp1_instruction_count": "secondary_non_gating",
            "workload_identity_schema_version": 1,
            "workload_canonicalization": "sha256(canonical_json(workload_spec))",
            "primary_formulas": {"candidate_cost": "test"},
            "q_formula": list(opcode_gas.Q_FORMULA),
            "out_of_fit_checkpoint": {"mapping": {}, "ape_max": 0.1},
            "quality_gates": {"checkpoint_ape_max": 0.1},
        }
        identity["guest_artifacts_sha256"] = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(identity["guest_artifacts"])
        )
        calibration_id = opcode_gas.sha256_bytes(
            opcode_gas.canonical_json(identity)
        )[:24]
        source = root / "runs" / calibration_id
        (source / "raw").mkdir(parents=True)
        (source / "generated" / "anchor-probe").mkdir(parents=True)
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
        declaration = opcode_gas.experiment_provenance_declaration(experiment)
        (source / "experiment.json").write_text(json.dumps(experiment))
        (source / "provenance.json").write_text(json.dumps(declaration))
        (source / "controlled-manifest.toml").write_text("frozen = true\n")
        (source / "controlled-manifest.sha256").write_text("e" * 64 + "\n")
        (source / "formal-relation-decisions.json").write_text('{"rounds": []}\n')
        (source / "formal-relation-decisions.sha256").write_text("3" * 64 + "\n")
        (source / "raw" / "formal-relations.jsonl").write_text('{"formal": true}\n')
        (source / "opcode-relations.json").write_text(
            json.dumps(
                {
                    "artifact_sha256": "4" * 64,
                    "raw_rows_sha256": "5" * 64,
                    "provenance": declaration,
                }
            )
        )
        (source / "anchor-probe-fit.json").write_text(
            json.dumps(
                {
                    "artifact_sha256": "6" * 64,
                    "primary_artifact_sha256": "7" * 64,
                }
            )
        )
        (source / "raw" / "anchor-probe.jsonl").write_text('{"anchor": true}\n')
        (source / "generated" / "anchor-probe" / "anchor-probe-manifest.json").write_text(
            '{"fixtures": true}\n'
        )
        (source / "block-calibration-rows.jsonl").write_text('{"block": true}\n')
        return source, declaration

    def _declared_anchor_fixture(
        self, source: pathlib.Path
    ) -> tuple[dict, pathlib.Path, str]:
        relative = pathlib.Path("EXP/1/target/guest-input.json")
        fixture_path = source / "generated" / "anchor-probe" / relative
        fixture_path.parent.mkdir(parents=True)
        fixture_path.write_text('{"fixture": true}\n')
        source_relative = str(pathlib.Path("generated/anchor-probe") / relative)
        return {"fixtures": [{"guest_input_path": str(relative)}]}, fixture_path, source_relative

    def _replay_values(self, declaration: dict) -> tuple[dict, dict, dict, dict]:
        relation = {
            "artifact_sha256": "4" * 64,
            "raw_rows_sha256": "5" * 64,
            "provenance": declaration,
        }
        anchor = {
            "artifact_sha256": "6" * 64,
            "primary_artifact_sha256": "7" * 64,
        }
        dynamic = {
            "artifact_sha256": "8" * 64,
            "status": "supported",
            "candidate_eligible": False,
        }
        core = {
            "artifact_sha256": "9" * 64,
            "status": "supported_core_submodel",
            "candidate_eligible": False,
        }
        return relation, anchor, dynamic, core

    def _patch_replay(self, declaration: dict, *, dynamic=None, core=None):
        relation, anchor, default_dynamic, default_core = self._replay_values(declaration)
        return mock.patch.multiple(
            opcode_gas,
            verify_frozen_controlled_manifest=mock.DEFAULT,
            load_terminal_formal_relation_artifacts=mock.DEFAULT,
            validate_opcode_relations_artifact=mock.DEFAULT,
            load_validated_anchor_probe_run=mock.DEFAULT,
            _affine_model_from_validated_artifact=mock.DEFAULT,
            fit_block_calibration_artifact=mock.DEFAULT,
            fit_dynamic_opcode_models_artifact=mock.DEFAULT,
            build_core_opcode_submodel_artifact=mock.DEFAULT,
            validate_core_opcode_submodel_artifact=mock.DEFAULT,
            _validated_anchor_probe_fixture_manifest=mock.DEFAULT,
        ), relation, anchor, dynamic or default_dynamic, core or default_core

    def _configure_replay(
        self,
        patched,
        *,
        source: pathlib.Path,
        declaration: dict,
        relation: dict,
        anchor: dict,
        dynamic: dict,
        core: dict,
    ) -> None:
        source_identity = json.loads((source / "experiment.json").read_text())["calibration_identity"]
        patched["verify_frozen_controlled_manifest"].return_value = (object(), source_identity)
        patched["load_terminal_formal_relation_artifacts"].return_value = {
            "decisions": {"rounds": []},
            "rows": [{"raw": True}],
            "formal_relation_decisions_sha256": "3" * 64,
        }
        patched["load_validated_anchor_probe_run"].return_value = (
            {"opcode:0x50": 1},
            [{"anchor": True}],
        )
        patched["fit_block_calibration_artifact"].return_value = {
            "artifact_sha256": "a" * 64
        }
        patched["_affine_model_from_validated_artifact"].return_value = object()
        patched["fit_dynamic_opcode_models_artifact"].return_value = dynamic
        patched["build_core_opcode_submodel_artifact"].return_value = core
        patched["validate_core_opcode_submodel_artifact"].return_value = core
        patched["_validated_anchor_probe_fixture_manifest"].return_value = (
            {"fixtures": []},
            {},
        )

    def test_derives_historical_source_at_current_analysis_revision_without_execution(self):
        """A current analysis may derive only from a sealed historical identity."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            source_before = {
                path.relative_to(source): opcode_gas.sha256_file(path)
                for path in source.rglob("*")
                if path.is_file()
            }
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(
                opcode_gas, "git_worktree_status", return_value=""
            ), mock.patch.object(
                opcode_gas,
                "validate_calibration_execution_identity",
                side_effect=AssertionError("derived evidence must not execute source run"),
            ), mock.patch.object(
                opcode_gas,
                "run_guest_inputs",
                side_effect=AssertionError("derived evidence must not launch guests"),
            ):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                result = opcode_gas.derive_core_opcode_submodel(
                    source, root / "derivations"
                )

            derived = pathlib.Path(result["directory"])
            envelope = json.loads((derived / "derivation.json").read_text())
            self.assertNotEqual(
                envelope["derivation_identity"]["derivation_revision"],
                self.source_revision,
            )
            self.assertEqual(
                envelope["source"]["calibration_id"], source.name
            )
            self.assertEqual(
                envelope["output_hashes"]["dynamic_artifact_sha256"], "8" * 64
            )
            self.assertEqual(
                envelope["output_hashes"]["core_artifact_sha256"], "9" * 64
            )
            self.assertEqual(
                opcode_gas.sha256_file(derived / "dynamic-opcode-models.json"),
                envelope["output_hashes"]["dynamic_file_sha256"],
            )
            self.assertEqual(
                opcode_gas.sha256_file(derived / "core-opcode-submodel.json"),
                envelope["output_hashes"]["core_file_sha256"],
            )
            self.assertEqual(
                {
                    path.relative_to(source): opcode_gas.sha256_file(path)
                    for path in source.rglob("*")
                    if path.is_file()
                },
                source_before,
            )

    def test_envelope_hashes_each_anchor_fixture_consumed_by_validation(self):
        """Every validated anchor guest input is bound in the source-file envelope."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            anchor_manifest, fixture_path, source_relative = self._declared_anchor_fixture(
                source
            )
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                patched["_validated_anchor_probe_fixture_manifest"].return_value = (
                    anchor_manifest,
                    {},
                )
                result = opcode_gas.derive_core_opcode_submodel(
                    source, root / "derivations"
                )
            envelope = json.loads(
                (pathlib.Path(result["directory"]) / "derivation.json").read_text()
            )
            self.assertEqual(
                envelope["source"]["source_hashes"]["source_files_sha256"][
                    source_relative
                ],
                opcode_gas.sha256_file(fixture_path),
            )

    def test_anchor_fixture_mutation_after_validation_rejects_publication(self):
        """A changed validated guest input is detected by the final source rehash."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            anchor_manifest, fixture_path, _source_relative = self._declared_anchor_fixture(
                source
            )
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                patched["_validated_anchor_probe_fixture_manifest"].return_value = (
                    anchor_manifest,
                    {},
                )

                def mutate_anchor_fixture(*_args, **_kwargs):
                    fixture_path.write_text('{"fixture": "mutated"}\n')
                    return {"artifact_sha256": "a" * 64}

                patched["fit_block_calibration_artifact"].side_effect = mutate_anchor_fixture
                with self.assertRaisesRegex(ValueError, "source evidence changed"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )
            self.assertFalse((root / "derivations").exists())

    def test_rejects_syntactically_valid_historical_revision_missing_locally(self):
        """A historical identity must name a locally available commit object."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(
                root, implementation_revision="0" * 40
            )
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                with self.assertRaisesRegex(ValueError, "local commit"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )
            self.assertFalse((root / "derivations").exists())

    def test_rejects_every_material_source_validation_failure_without_publication(self):
        """A failed sealed-source check cannot leave a derived directory behind."""
        categories = (
            "controlled manifest",
            "formal decision ledger",
            "relation artifact",
            "anchor evidence",
            "block rows",
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            for category in categories:
                with self.subTest(category=category):
                    replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                        declaration
                    )
                    with replay_patch as patched, mock.patch.object(
                        opcode_gas, "git_head", return_value=self.derivation_revision
                    ), mock.patch.object(
                        opcode_gas, "git_worktree_status", return_value=""
                    ):
                        self._configure_replay(
                            patched,
                            source=source,
                            declaration=declaration,
                            relation=relation,
                            anchor=anchor,
                            dynamic=dynamic,
                            core=core,
                        )
                        failure = ValueError(f"{category} hash mismatch")
                        target = {
                            "controlled manifest": "verify_frozen_controlled_manifest",
                            "formal decision ledger": "load_terminal_formal_relation_artifacts",
                            "relation artifact": "validate_opcode_relations_artifact",
                            "anchor evidence": "load_validated_anchor_probe_run",
                            "block rows": "fit_block_calibration_artifact",
                        }[category]
                        patched[target].side_effect = failure
                        with self.assertRaisesRegex(ValueError, category):
                            opcode_gas.derive_core_opcode_submodel(
                                source, root / "derivations"
                            )
                    self.assertFalse((root / "derivations").exists())

    def test_rejects_identity_mismatch_and_second_publication(self):
        """Source provenance and content-addressed publication are both create-only."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            (source / "provenance.json").write_text('{"wrong": true}\n')
            with mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                with self.assertRaisesRegex(ValueError, "persisted calibration provenance"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )

            (source / "provenance.json").write_text(json.dumps(declaration))
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                result = opcode_gas.derive_core_opcode_submodel(
                    source, root / "derivations"
                )
                with self.assertRaisesRegex(ValueError, "derivation directory already exists"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )
            self.assertTrue(pathlib.Path(result["directory"]).is_dir())

    def test_dynamic_failure_does_not_publish_partial_derivation(self):
        """Unsupported dynamic evidence fails closed before any derivation is published."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            dynamic = {
                "artifact_sha256": "8" * 64,
                "status": "not_supported",
                "candidate_eligible": False,
            }
            replay_patch, relation, anchor, _default_dynamic, core = self._patch_replay(
                declaration, dynamic=dynamic
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                with self.assertRaisesRegex(ValueError, "not supported"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )
                self.assertFalse(patched["build_core_opcode_submodel_artifact"].called)
            self.assertFalse((root / "derivations").exists())

    def test_static_rank_failure_does_not_publish_partial_derivation(self):
        """A core rank failure leaves no directory that could be mistaken for evidence."""
        with tempfile.TemporaryDirectory() as temp_dir:
            root = pathlib.Path(temp_dir)
            source, declaration = self._source_run(root)
            replay_patch, relation, anchor, dynamic, core = self._patch_replay(
                declaration
            )
            with replay_patch as patched, mock.patch.object(
                opcode_gas, "git_head", return_value=self.derivation_revision
            ), mock.patch.object(opcode_gas, "git_worktree_status", return_value=""):
                self._configure_replay(
                    patched,
                    source=source,
                    declaration=declaration,
                    relation=relation,
                    anchor=anchor,
                    dynamic=dynamic,
                    core=core,
                )
                patched["build_core_opcode_submodel_artifact"].side_effect = ValueError(
                    "core static relation basis is rank-deficient"
                )
                with self.assertRaisesRegex(ValueError, "rank-deficient"):
                    opcode_gas.derive_core_opcode_submodel(
                        source, root / "derivations"
                    )
            self.assertFalse((root / "derivations").exists())
