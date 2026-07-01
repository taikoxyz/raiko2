import argparse
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
    MAX_BLOCKS_PER_PROPOSAL,
    ProposalGroup,
    build_discovered_proposal_record,
    parse_proposal_ids,
    resolve_monitor_config,
    write_discovered_proposals,
)


class _FakeProposedEvent:
    def __init__(self, logs):
        self._logs = logs
        self.calls = []

    def get_logs(
        self,
        *,
        from_block=None,
        to_block=None,
        fromBlock=None,
        toBlock=None,
        argument_filters=None,
    ):
        if fromBlock is not None:
            from_block = fromBlock
        if toBlock is not None:
            to_block = toBlock
        if argument_filters is None:
            self.calls.append((from_block, to_block))
        else:
            self.calls.append((from_block, to_block, argument_filters))
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

    async def test_find_l1_inclusion_block_by_indexed_proposal_id_uses_id_filter(self):
        log = SimpleNamespace(blockNumber=2701561)
        monitor = self.make_monitor(logs=[log], latest_l1=2701561)

        l1_block = monitor.find_l1_inclusion_block_by_indexed_proposal_id(22758)

        self.assertEqual(l1_block, 2701561)
        self.assertEqual(
            monitor.evt_contract.events.Proposed.calls,
            [(0, "latest", {"id": 22758})],
        )
        self.assertEqual(monitor.proposal_block_cache[22758], 2701561)


class TestStressProposalIdParsing(unittest.TestCase):
    def test_max_blocks_per_proposal_matches_unzen_cap(self):
        self.assertEqual(MAX_BLOCKS_PER_PROPOSAL, 768)

    def test_parse_proposal_ids_accepts_comma_separated_values(self):
        self.assertEqual(parse_proposal_ids("22733, 22734"), [22733, 22734])

    def test_parse_proposal_ids_rejects_empty_values(self):
        with self.assertRaises(argparse.ArgumentTypeError):
            parse_proposal_ids(",")


class TestBatchMonitorInitialization(unittest.TestCase):
    def test_log_file_none_keeps_file_logging_disabled(self):
        original_init_contract_event = BatchMonitor._BatchMonitor__init_contract_event
        BatchMonitor._BatchMonitor__init_contract_event = lambda self, *_args: None
        try:
            monitor = BatchMonitor(
                l1_rpc="http://l1.invalid",
                l2_rpc="http://l2.invalid",
                abi_file=str(DEFAULT_SHASTA_IINBOX_ABI),
                evt_address="0x0000000000000000000000000000000000000000",
                raiko_rpc="http://raiko.invalid",
                log_file=None,
            )
            monitor.append_log("no-op")
        finally:
            BatchMonitor._BatchMonitor__init_contract_event = original_init_contract_event

        self.assertIsNone(monitor.log_file)


class TestBatchMonitorPayload(unittest.TestCase):
    def make_monitor(self, prove_type):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.prove_type = prove_type
        monitor.network = "taiko_hoodi"
        monitor.l1_network = "hoodi"
        return monitor

    def test_payload_includes_network_pair(self):
        payload = self.make_monitor("sgx").generate_post_data([], aggregate=False)

        self.assertEqual(payload["network"], "taiko_hoodi")
        self.assertEqual(payload["l1_network"], "hoodi")

    def test_native_payload_omits_legacy_prover_args(self):
        payload = self.make_monitor("native").generate_post_data([], aggregate=False)

        self.assertEqual(payload["proof_type"], "native")
        self.assertNotIn("native", payload)
        self.assertNotIn("risc0", payload)
        self.assertNotIn("sgx", payload)
        self.assertNotIn("sgxgeth", payload)
        self.assertNotIn("sp1", payload)

    def test_sp1_payload_keeps_supported_sp1_overrides(self):
        payload = self.make_monitor("sp1").generate_post_data([], aggregate=False)

        self.assertEqual(payload["proof_type"], "sp1")
        self.assertIn("sp1", payload)


class TestBatchMonitorProposalIdSearch(unittest.IsolatedAsyncioTestCase):
    async def test_find_l2_block_for_proposal_id_skips_pre_shasta_blocks(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.logger = logging.getLogger("test_stress_shasta_proposal")
        monitor.get_latest_l2_block_number = lambda: 8
        proposal_ids_by_block = {
            0: None,
            1: None,
            2: None,
            3: 5,
            4: 5,
            5: 6,
            6: 6,
            7: 7,
            8: 7,
        }

        async def get_l2_block_proposal_id(block_number):
            return proposal_ids_by_block[block_number]

        monitor.get_l2_block_proposal_id = get_l2_block_proposal_id

        self.assertEqual(await monitor.find_l2_block_for_proposal_id(6), 5)


class TestStressChainSpecResolution(unittest.TestCase):
    def test_resolves_hoodi_defaults_from_chain_specs(self):
        specs = json.loads(DEFAULT_CHAIN_SPEC_LIST.read_text())
        l1_spec = next(spec for spec in specs if spec["name"] == "hoodi")
        l2_spec = next(spec for spec in specs if spec["name"] == "taiko_hoodi")
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

        self.assertEqual(resolved.l1_rpc, l1_spec["rpc"])
        self.assertEqual(resolved.l2_rpc, l2_spec["rpc"])
        self.assertEqual(resolved.event_contract, l2_spec["l1_contract"]["SHASTA"])
        self.assertEqual(Path(resolved.abi_file), DEFAULT_SHASTA_IINBOX_ABI)
        self.assertEqual(Path(resolved.anchor_abi_file), DEFAULT_SHASTA_ANCHOR_ABI)

    def test_resolves_single_configured_contract_when_shasta_key_is_absent(self):
        with tempfile.TemporaryDirectory() as tmp:
            spec_path = Path(tmp) / "chain_spec.json"
            spec_path.write_text(
                json.dumps(
                    [
                        {"name": "l1", "rpc": "http://l1"},
                        {
                            "name": "taiko_dev",
                            "rpc": "http://l2",
                            "l1_contract": {
                                "CANCUN": "0xb432bbe475e569b2adef4830ae43d587932f139c"
                            },
                        },
                    ]
                )
            )

            resolved = resolve_monitor_config(
                chain_spec_list=spec_path,
                network="taiko_dev",
                l1_network="l1",
                l1_rpc=None,
                l2_rpc=None,
                event_contract=None,
                abi_file=None,
                anchor_abi_file=None,
            )

        self.assertEqual(
            resolved.event_contract, "0xb432bbe475e569b2adef4830ae43d587932f139c"
        )

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


class TestBatchMonitorAnchorInfoCache(unittest.IsolatedAsyncioTestCase):
    async def test_get_anchor_info_caches_negative_parse_results_after_fetch(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.logger = logging.getLogger("test_stress_shasta_proposal")
        monitor.anchor_info_cache = {}

        calls = 0

        async def get_l2_block(_block_number):
            nonlocal calls
            calls += 1
            return {"extraData": "0x"}

        monitor.get_l2_block = get_l2_block

        self.assertIsNone(await monitor.get_anchor_info(11))
        self.assertIsNone(await monitor.get_anchor_info(11))
        self.assertEqual(calls, 1)
        self.assertIn(11, monitor.anchor_info_cache)
        self.assertIsNone(monitor.anchor_info_cache[11])

    async def test_get_anchor_info_does_not_cache_missing_blocks(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.logger = logging.getLogger("test_stress_shasta_proposal")
        monitor.anchor_info_cache = {}

        calls = 0

        async def get_l2_block(_block_number):
            nonlocal calls
            calls += 1
            return None

        monitor.get_l2_block = get_l2_block

        self.assertIsNone(await monitor.get_anchor_info(12))
        self.assertIsNone(await monitor.get_anchor_info(12))
        self.assertEqual(calls, 2)
        self.assertNotIn(12, monitor.anchor_info_cache)


class TestBatchMonitorRangeIteration(unittest.IsolatedAsyncioTestCase):
    async def test_get_next_batches_starts_from_range_start_when_state_is_uninitialized(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.logger = logging.getLogger("test_stress_shasta_proposal")
        monitor.l2_block_range = (10, 12)
        monitor.ts_offset = None
        monitor.running_count = 1
        monitor.time_speed = 1.0
        monitor.last_block_ts_in_real_world = 0

        now = 1_700_000_000

        async def align_ts_offset(first_block):
            self.assertEqual(first_block, 10)
            monitor.ts_offset = 0
            monitor.last_block_ts_in_real_world = now
            return True

        async def get_block(block_number):
            self.assertEqual(block_number, 10)
            return {"timestamp": hex(now)}

        monitor.align_ts_offset = align_ts_offset
        monitor.get_block = get_block
        monitor.get_batch_events_in_block = lambda block_number: [42] if block_number == 10 else []

        result = await monitor.get_next_batches()

        self.assertEqual(result, (10, [42]))


class TestBatchMonitorRequestHeaders(unittest.TestCase):
    def test_request_headers_omits_api_key_when_unset(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.api_key = None

        self.assertEqual(
            monitor._request_headers(),
            {"Content-Type": "application/json"},
        )

    def test_request_headers_includes_api_key_when_set(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.api_key = "secret"

        self.assertEqual(
            monitor._request_headers(),
            {"Content-Type": "application/json", "x-api-key": "secret"},
        )


class TestBatchMonitorRequestPayload(unittest.TestCase):
    def test_generate_post_data_omits_legacy_public_prover_args_for_sgx(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.prove_type = "sgx"
        monitor.network = "taiko_hoodi"
        monitor.l1_network = "hoodi"

        payload = monitor.generate_post_data(
            [
                {
                    "proposal_id": 25127,
                    "l1_inclusion_block_number": 2766898,
                    "l2_block_numbers": [4140789, 4140790],
                    "checkpoint": None,
                    "last_anchor_block_number": 2766861,
                }
            ],
            aggregate=False,
        )

        self.assertEqual(payload["proof_type"], "sgx")
        self.assertNotIn("native", payload)
        self.assertNotIn("sgx", payload)
        self.assertNotIn("sgxgeth", payload)
        self.assertNotIn("risc0", payload)
        self.assertNotIn("sp1", payload)

    def test_generate_post_data_keeps_sp1_public_prover_args_for_sp1(self):
        monitor = BatchMonitor.__new__(BatchMonitor)
        monitor.prove_type = "sp1"
        monitor.network = "taiko_hoodi"
        monitor.l1_network = "hoodi"

        payload = monitor.generate_post_data(
            [
                {
                    "proposal_id": 25127,
                    "l1_inclusion_block_number": 2766898,
                    "l2_block_numbers": [4140789, 4140790],
                    "checkpoint": None,
                    "last_anchor_block_number": 2766861,
                }
            ],
            aggregate=False,
        )

        self.assertEqual(payload["proof_type"], "sp1")
        self.assertIn("sp1", payload)


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
