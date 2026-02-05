import json
import sys
import tempfile
import unittest
from unittest import mock
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


class TestSelectionAll(unittest.TestCase):
    def test_select_all_proposals_returns_all(self):
        from shasta_regression import select_all_proposals

        proposals = [3, 4, 7]
        self.assertEqual(select_all_proposals(proposals), proposals)


class TestAggregationGrouping(unittest.TestCase):
    def test_grouping(self):
        from shasta_regression import group_for_aggregation

        proofs = ["a", "b", "c", "d", "e"]
        groups = group_for_aggregation(proofs, size=2)
        self.assertEqual(groups, [["a", "b"], ["c", "d"], ["e"]])


class TestAggregationGroupingPaths(unittest.TestCase):
    def test_grouping_paths(self):
        from shasta_regression import group_for_aggregation

        proofs = [Path("/tmp/a"), Path("/tmp/b"), Path("/tmp/c")]
        groups = group_for_aggregation(proofs, size=2)
        self.assertEqual(groups, [[Path("/tmp/a"), Path("/tmp/b")], [Path("/tmp/c")]])


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
        from shasta_regression import resolve_event_address_from_config

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
            self.assertEqual(resolve_event_address_from_config(config), "0xdef")


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


class TestGuestLauncherCommand(unittest.TestCase):
    def test_build_guest_launcher_command(self):
        from shasta_regression import build_guest_launcher_cmd

        cmd = build_guest_launcher_cmd(
            guest_bin="/bin/guest-launcher",
            input_path=Path("/tmp/input.json"),
            mode="prove",
            proof_mode="compressed",
            output_path=Path("/tmp/proof.json"),
        )
        self.assertIn("--mode", cmd)
        self.assertIn("prove", cmd)
        self.assertIn("--proof-mode", cmd)
        self.assertIn("compressed", cmd)
        self.assertIn("--output", cmd)
        self.assertIn("/tmp/proof.json", cmd)


class TestProgressLogging(unittest.TestCase):
    def test_format_progress(self):
        from shasta_regression import format_progress

        msg = format_progress(2, 10, "preflight", proposal_id=42)
        self.assertIn("2/10", msg)
        self.assertIn("preflight", msg)
        self.assertIn("42", msg)


class TestChainSpecCache(unittest.TestCase):
    def test_resolve_rpc_from_specs(self):
        from shasta_regression import resolve_rpc_from_specs

        specs = [
            {"name": "l1", "rpc": "http://l1"},
            {"name": "l2", "rpc": "http://l2"},
        ]
        self.assertEqual(resolve_rpc_from_specs(specs, "l2"), "http://l2")


class TestRpcCall(unittest.TestCase):
    def test_rpc_call_handles_request_error(self):
        from shasta_regression import rpc_call

        with mock.patch("shasta_regression.requests.post") as post:
            post.side_effect = Exception("boom")
            self.assertIsNone(rpc_call("http://rpc", "eth_blockNumber", [], 1))


class TestCountValidation(unittest.TestCase):
    def test_count_is_capped(self):
        from shasta_regression import validate_count_option

        ok, message = validate_count_option(count=6)
        self.assertFalse(ok)
        self.assertIn("max", message.lower())


class TestRangeParsing(unittest.TestCase):
    def test_parse_range_invalid_returns_none(self):
        from shasta_regression import parse_range

        self.assertIsNone(parse_range("1"))
        self.assertIsNone(parse_range("1:"))
        self.assertIsNone(parse_range("1:2:3"))
        self.assertIsNone(parse_range("a:b"))


class TestConfigErrors(unittest.TestCase):
    def test_load_config_missing_file_raises(self):
        from shasta_regression import load_config

        with self.assertRaises(ValueError):
            load_config(Path("/nope/config.json"))

    def test_load_config_invalid_json_raises(self):
        from shasta_regression import load_config

        with tempfile.TemporaryDirectory() as tmp:
            cfg_path = Path(tmp) / "config.json"
            cfg_path.write_text("{invalid json}")
            with self.assertRaises(ValueError):
                load_config(cfg_path)


class TestChainSpecErrors(unittest.TestCase):
    def test_load_chain_specs_invalid_json_raises(self):
        from shasta_regression import load_chain_specs

        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text("{invalid json}")
            with self.assertRaises(ValueError):
                load_chain_specs(spec_path)


class TestRangeExpansion(unittest.TestCase):
    def test_expand_range_to_proposal_boundaries(self):
        from shasta_regression import expand_range_to_proposal_boundaries

        def extradata(pid: int) -> str:
            return f"0x4b{pid:012x}"

        block_map = {}
        for num in range(8, 13):
            block_map[num] = {"number": num, "extraData": extradata(100)}
        for num in range(13, 16):
            block_map[num] = {"number": num, "extraData": extradata(101)}
        for num in range(16, 19):
            block_map[num] = {"number": num, "extraData": extradata(102)}
        for num in range(19, 23):
            block_map[num] = {"number": num, "extraData": extradata(103)}

        def fake_get_block(_rpc, num, _timeout):
            return block_map.get(num)

        with mock.patch("shasta_regression.get_l2_block", side_effect=fake_get_block):
            with mock.patch(
                "shasta_regression.get_latest_l2_block_number", return_value=22
            ):
                start, end, start_pid, end_pid = expand_range_to_proposal_boundaries(
                    "http://rpc", 11, 17, timeout=1
                )
        self.assertEqual((start, end), (8, 18))
        self.assertEqual((start_pid, end_pid), (100, 102))


class TestCompletedProposalDiscovery(unittest.TestCase):
    def test_discover_latest_completed_proposals_skips_current(self):
        from shasta_regression import discover_latest_completed_proposals_from_l2

        def extradata(pid: int) -> str:
            return f"0x4b{pid:012x}"

        block_map = {}
        for num in range(0, 4):
            block_map[num] = {"number": num, "extraData": extradata(1)}
        for num in range(4, 7):
            block_map[num] = {"number": num, "extraData": extradata(2)}
        for num in range(7, 10):
            block_map[num] = {"number": num, "extraData": extradata(3)}

        def fake_get_block(_rpc, num, _timeout):
            return block_map.get(num)

        with mock.patch("shasta_regression.get_l2_block", side_effect=fake_get_block):
            with mock.patch(
                "shasta_regression.get_latest_l2_block_number", return_value=9
            ):
                proposals = discover_latest_completed_proposals_from_l2(
                    "http://rpc", count=2, timeout=1, max_scan=20
                )
        self.assertEqual(proposals, [1, 2])


if __name__ == "__main__":
    unittest.main()
