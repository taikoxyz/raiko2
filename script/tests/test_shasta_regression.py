import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from shasta_regression import load_config, output_paths


class TestConfigAndPaths(unittest.TestCase):
    def test_load_config_and_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            cfg_path = Path(tmp) / "config.json"
            cfg_path.write_text(
                json.dumps(
                    {
                        "l1_rpc": "http://l1",
                        "l2_rpc": "http://l2",
                        "event_address": "0x0000000000000000000000000000000000000001",
                        "event_abi": "abi.json",
                        "anchor_abi": "anchor.json",
                    }
                )
            )

            cfg = load_config(cfg_path)
            self.assertEqual(cfg["l1_rpc"], "http://l1")

            out_dir = Path(tmp) / "out"
            paths = output_paths(out_dir, proposal_id=42)
            self.assertTrue(paths["input"].name.endswith("proposal_42.json"))
            self.assertTrue(paths["proof"].name.endswith("proposal_42.proof.json"))


if __name__ == "__main__":
    unittest.main()


class TestBinaries(unittest.TestCase):
    def test_missing_binaries(self):
        from shasta_regression import check_binaries

        missing = check_binaries("/nope/preflight", "/nope/guest-launcher")
        self.assertTrue(missing)


class TestSelection(unittest.TestCase):
    def test_range_overrides_count(self):
        from shasta_regression import select_proposals

        proposals = list(range(1, 11))
        picked = select_proposals(proposals, range_tuple=(3, 6), count=2)
        self.assertEqual(picked, [3, 4, 5, 6])

    def test_count_selects_latest(self):
        from shasta_regression import select_proposals

        proposals = [1, 2, 3, 4, 5]
        picked = select_proposals(proposals, range_tuple=None, count=2)
        self.assertEqual(picked, [4, 5])
