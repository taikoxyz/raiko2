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


class TestChainSpecLookup(unittest.TestCase):
    def test_resolve_rpc_from_chain_spec(self):
        from shasta_regression import resolve_rpc_from_chain_spec

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps(
                    [
                        {"name": "l1", "rpc": "http://l1"},
                        {"name": "l2", "rpc": "http://l2"},
                    ]
                )
            )
            self.assertEqual(resolve_rpc_from_chain_spec(spec_path, "l2"), "http://l2")


class TestChainSpecContracts(unittest.TestCase):
    def test_resolve_event_address_from_chain_spec(self):
        from shasta_regression import resolve_event_address_from_chain_spec

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps(
                    [
                        {
                            "name": "taiko_dev",
                            "l1_contract": {"SHASTA": "0xabc"},
                        }
                    ]
                )
            )
            self.assertEqual(
                resolve_event_address_from_chain_spec(spec_path, "taiko_dev", "SHASTA"),
                "0xabc",
            )


class TestConfigEventAddress(unittest.TestCase):
    def test_resolve_event_address_from_config(self):
        from shasta_regression import resolve_event_address_from_config

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps([
                    {
                        "name": "taiko_dev_l1",
                        "l1_contract": {"SHASTA": "0xabc"},
                    }
                ])
            )
            config = {
                "chain_spec_list": str(spec_path),
                "l1_chain": "taiko_dev_l1",
                "l1_contract_fork": "SHASTA",
            }
            self.assertEqual(resolve_event_address_from_config(config), "0xabc")


class TestConfigValidation(unittest.TestCase):
    def test_resolve_event_address_missing_chain_spec(self):
        from shasta_regression import resolve_event_address_from_config

        config = {"l1_chain": "taiko_dev_l1"}
        self.assertIsNone(resolve_event_address_from_config(config))


class TestEventAddressFallback(unittest.TestCase):
    def test_event_address_fallback_to_l2_chain(self):
        from shasta_regression import resolve_event_address_from_config

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps(
                    [
                        {"name": "l1", "l1_contract": {}},
                        {"name": "l2", "l1_contract": {"SHASTA": "0xdef"}},
                    ]
                )
            )
            config = {
                "chain_spec_list": str(spec_path),
                "l1_chain": "l1",
                "l2_chain": "l2",
                "l1_contract_fork": "SHASTA",
            }
            self.assertEqual(resolve_event_address_from_config(config), "0xdef")


class TestEventAddressOverride(unittest.TestCase):
    def test_event_address_from_chain_spec_used(self):
        from shasta_regression import event_address_from_config

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps(
                    [
                        {"name": "l2", "l1_contract": {"SHASTA": "0xdef"}},
                    ]
                )
            )
            config = {
                "chain_spec_list": str(spec_path),
                "l2_chain": "l2",
                "l1_contract_fork": "SHASTA",
            }
            self.assertEqual(event_address_from_config(config), "0xdef")


class TestPreflightCommand(unittest.TestCase):
    def test_build_preflight_command(self):
        from shasta_regression import build_preflight_cmd

        cmd = build_preflight_cmd(
            preflight_bin="/bin/preflight",
            proposal_id=7,
            rpc_url="http://l1",
            l2_chain_id=123,
            l1_chain_id=1,
            output_path=Path("/tmp/out.json"),
            proof_type="native",
        )
        self.assertIn("--rpc-url", cmd)
        self.assertIn("--l2-chain-id", cmd)
        self.assertIn("--l1-chain-id", cmd)
        self.assertIn("--proposal-id", cmd)
        self.assertNotIn("--l1-rpc", cmd)


class TestPreflightRpc(unittest.TestCase):
    def test_preflight_rpc_uses_l2(self):
        from shasta_regression import preflight_rpc_from_config

        cfg = {"l1_rpc": "http://l1", "l2_rpc": "http://l2"}
        self.assertEqual(preflight_rpc_from_config(cfg), "http://l2")


class TestProgressLogging(unittest.TestCase):
    def test_format_progress(self):
        from shasta_regression import format_progress

        msg = format_progress(2, 10, "preflight", proposal_id=42)
        self.assertIn("2/10", msg)
        self.assertIn("preflight", msg)
        self.assertIn("42", msg)
