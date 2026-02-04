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
