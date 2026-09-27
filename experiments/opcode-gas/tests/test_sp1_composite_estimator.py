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


HIGHER_LAYER_SOURCE = (
    ROOT
    / "experiments"
    / "opcode-gas"
    / "derivations"
    / "3e4de6eecb5e92aa59a6a4b9"
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


if __name__ == "__main__":
    unittest.main()
