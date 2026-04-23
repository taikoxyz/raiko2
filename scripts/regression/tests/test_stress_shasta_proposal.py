import json
import logging
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import web3.middleware as web3_middleware

if not hasattr(web3_middleware, "ExtraDataToPOAMiddleware"):
    web3_middleware.ExtraDataToPOAMiddleware = object()

from stress_shasta_proposal import (
    BatchMonitor,
    DEFAULT_CHAIN_SPEC_LIST,
    DEFAULT_SHASTA_ANCHOR_ABI,
    DEFAULT_SHASTA_IINBOX_ABI,
    ProposalGroup,
    build_discovered_proposal_record,
    resolve_monitor_config,
    write_discovered_proposals,
)


class _FakeProposedEvent:
    def __init__(self, logs):
        self._logs = logs
        self.calls = []

    def get_logs(self, *, from_block, to_block):
        self.calls.append((from_block, to_block))
        return list(self._logs)


class _FakeEvents:
    def __init__(self, logs):
        self.Proposed = _FakeProposedEvent(logs)


class TestBatchMonitorL1SearchWindow(unittest.IsolatedAsyncioTestCase):
    def make_monitor(self, logs=None, latest_l1=0):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.logger = logging.getLogger("test_stress_shasta_proposal")
        monitor.proposal_block_cache = {}
        monitor.evt_contract = SimpleNamespace(events=_FakeEvents(logs or []))
        monitor._extract_proposal_id_from_proposed_log = (
            lambda log: getattr(log, "proposal_id", None)
        )
        monitor.get_latest_l1_block_number = lambda: latest_l1
        return monitor

    async def test_find_l1_inclusion_block_clamps_search_end_to_latest_l1(self):
        log = SimpleNamespace(proposal_id=17341, blockNumber=34820)
        monitor = self.make_monitor(logs=[log], latest_l1=34820)

        l1_block = await monitor.find_l1_inclusion_block(17341, 34735)

        self.assertEqual(l1_block, 34820)
        self.assertEqual(
            monitor.evt_contract.events.Proposed.calls,
            [(34736, 34820)],
        )

    async def test_find_l1_inclusion_block_skips_query_when_l1_head_is_before_search_start(self):
        monitor = self.make_monitor(logs=[], latest_l1=34735)

        l1_block = await monitor.find_l1_inclusion_block(17341, 34735)

        self.assertIsNone(l1_block)
        self.assertEqual(monitor.evt_contract.events.Proposed.calls, [])
        self.assertIn(17341, monitor.proposal_block_cache)
        self.assertIsNone(monitor.proposal_block_cache[17341])

    async def test_batch_find_proposal_blocks_clamps_search_end_to_latest_l1(self):
        logs = [
            SimpleNamespace(proposal_id=17341, blockNumber=34819),
            SimpleNamespace(proposal_id=17342, blockNumber=34820),
        ]
        monitor = self.make_monitor(logs=logs, latest_l1=34820)

        results = await monitor.batch_find_proposal_blocks(
            [(17341, 34735), (17342, 34735)],
            34736,
            34863,
        )

        self.assertEqual(
            monitor.evt_contract.events.Proposed.calls,
            [(34736, 34820)],
        )
        self.assertEqual(results[17341], 34819)
        self.assertEqual(results[17342], 34820)


class TestStressChainSpecResolution(unittest.TestCase):
    def test_resolves_hoodi_defaults_from_chain_specs(self):
        resolved = resolve_monitor_config(
            chain_spec_list=DEFAULT_CHAIN_SPEC_LIST,
            network="taiko_hoodi",
            l1_network="hoodi",
            l1_rpc=None,
            l2_rpc=None,
            event_contract=None,
            abi_file=None,
            anchor_abi_file=None,
        )

        self.assertEqual(
            resolved.l1_rpc, "https://ethereum-hoodi-rpc.publicnode.com"
        )
        self.assertEqual(resolved.l2_rpc, "http://34.71.217.85:8545")
        self.assertEqual(
            resolved.event_contract, "0xeF4bB7A442Bd68150A3aa61A6a097B86b91700BF"
        )
        self.assertEqual(Path(resolved.abi_file), DEFAULT_SHASTA_IINBOX_ABI)
        self.assertEqual(Path(resolved.anchor_abi_file), DEFAULT_SHASTA_ANCHOR_ABI)

    def test_explicit_values_override_chain_specs(self):
        resolved = resolve_monitor_config(
            chain_spec_list=DEFAULT_CHAIN_SPEC_LIST,
            network="taiko_hoodi",
            l1_network="hoodi",
            l1_rpc="http://l1.override",
            l2_rpc="http://l2.override",
            event_contract="0x1111111111111111111111111111111111111111",
            abi_file="/tmp/IInbox.override.json",
            anchor_abi_file="/tmp/Anchor.override.json",
        )

        self.assertEqual(resolved.l1_rpc, "http://l1.override")
        self.assertEqual(resolved.l2_rpc, "http://l2.override")
        self.assertEqual(
            resolved.event_contract, "0x1111111111111111111111111111111111111111"
        )
        self.assertEqual(resolved.abi_file, "/tmp/IInbox.override.json")
        self.assertEqual(resolved.anchor_abi_file, "/tmp/Anchor.override.json")


class TestStressDiscoveryOutput(unittest.TestCase):
    def test_builds_preflight_ready_proposal_record(self):
        record = build_discovered_proposal_record(
            network="taiko_hoodi",
            l1_network="hoodi",
            group=ProposalGroup(
                proposal_id=17771,
                anchor_number=2674327,
                l2_block_numbers=[7225402, 7225403],
            ),
            l1_inclusion_block=2674375,
            last_anchor_block_number=2674326,
        )

        self.assertEqual(
            record,
            {
                "network": "taiko_hoodi",
                "l1_network": "hoodi",
                "proposal_id": 17771,
                "l1_inclusion_block_number": 2674375,
                "last_anchor_block_number": 2674326,
                "l2_start": 7225402,
                "l2_end": 7225403,
                "l2_block_numbers": [7225402, 7225403],
            },
        )

    def test_writes_discovered_proposals_json(self):
        record = build_discovered_proposal_record(
            network="taiko_hoodi",
            l1_network="hoodi",
            group=ProposalGroup(
                proposal_id=17771,
                anchor_number=2674327,
                l2_block_numbers=[7225402],
            ),
            l1_inclusion_block=2674375,
            last_anchor_block_number=2674326,
        )

        with tempfile.TemporaryDirectory() as temp_dir:
            output = Path(temp_dir) / "proposal.json"
            write_discovered_proposals(output, [record])
            payload = json.loads(output.read_text())

        self.assertEqual(payload, {"proposals": [record]})


if __name__ == "__main__":
    unittest.main()
