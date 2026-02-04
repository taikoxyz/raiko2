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


class TestAggregationGrouping(unittest.TestCase):
    def test_grouping(self):
        from shasta_regression import group_for_aggregation

        proofs = ["a", "b", "c", "d", "e"]
        groups = group_for_aggregation(proofs, size=2)
        self.assertEqual(groups, [["a", "b"], ["c", "d"], ["e"]])


class TestSummary(unittest.TestCase):
    def test_write_summary(self):
        from shasta_regression import write_summary

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "run_summary.json"
            write_summary(path, {"successes": [1], "failures": []})
            data = json.loads(path.read_text())
            self.assertIn("successes", data)


class TestExtradataParsing(unittest.TestCase):
    def test_extract_proposal_id_from_extradata(self):
        from shasta_regression import extract_proposal_id_from_extradata

        # 0x + 1 byte config + 6 bytes proposal id (uint48, big-endian)
        extradata = "0x4b000000000005"
        self.assertEqual(extract_proposal_id_from_extradata(extradata), 5)


class TestDiscovery(unittest.TestCase):
    def test_discover_proposals_from_blocks(self):
        from shasta_regression import discover_proposals_from_blocks

        blocks = [
            {"number": 1, "extraData": "0x4b000000000001"},
            {"number": 2, "extraData": "0x4b000000000001"},
            {"number": 3, "extraData": "0x4b000000000002"},
        ]
        self.assertEqual(discover_proposals_from_blocks(blocks), [1, 2])


class TestLatestDiscovery(unittest.TestCase):
    def test_discover_latest_proposals_from_blocks(self):
        from shasta_regression import discover_latest_proposals_from_blocks

        blocks = [
            {"number": 1, "extraData": "0x4b000000000001"},
            {"number": 2, "extraData": "0x4b000000000001"},
            {"number": 3, "extraData": "0x4b000000000002"},
            {"number": 4, "extraData": "0x4b000000000002"},
            {"number": 5, "extraData": "0x4b000000000003"},
        ]
        latest = discover_latest_proposals_from_blocks(list(reversed(blocks)), count=2)
        self.assertEqual(latest, [2, 3])
