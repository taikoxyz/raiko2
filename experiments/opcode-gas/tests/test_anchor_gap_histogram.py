import pathlib
import sys
import tempfile
import unittest
import copy


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments/opcode-gas"))

import anchor_gap_histogram as histogram


HOODI = {
    "network": "taiko_hoodi",
    "proposal_id": 80907,
    "guest_input_sha256": "64083abb0a43a73654b75ce4d8856083fdbc98657856f0bc9886a812ed07d7a2",
    "compressed_trace_sha256": "bb25e90d2bc0bed1b0dec20f24fae41df38bbe63583eabfc8837f7ccd5f425a9",
    "record_sha256": "27d7f7c49e8edfcfb4efd03cdab640284ab2809b7c6d13d2e00a7334d0acef87",
    "summary_sha256": "cacdfa1fe6489bb1ffe25bdcd23428464abb06b6d247ed68d5c1f9d3634b7034",
}
MAINNET = {
    "network": "taiko_mainnet",
    "proposal_id": 39339,
    "guest_input_sha256": "7e79d122ff315a200b9e5362b2614d961e52cd09ead8935ff875169d3fcb8c0a",
    "compressed_trace_sha256": "aff03938bc4bc12c172edb7465e141f5059a801348c51681bcd423668d7b7f04",
    "record_sha256": "5d6d9f67ef38bb6661a568949f7a5e625b0a70748b2a24b3caa589265164a750",
    "summary_sha256": "34665671d06a1f9df737d70f58d3d574522f6b0482e6d35a5002e7b1b9bbdbd4",
}


def _opcode(opcode, raw_gas):
    return {
        "kind": "opcode",
        "opcode": opcode,
        "pricing_basis": "raw_gas_slope",
        "interpreter_raw_gas": raw_gas,
        "model_input": {"kind": "static_raw_gas", "raw_gas": raw_gas},
        "spawned": False,
        "dispatch_status": "not_applicable",
    }


def _trace():
    return {
        "schema_version": 4,
        "guest_input_sha256": "0x" + HOODI["guest_input_sha256"],
        "status": "complete",
        "parity": {"passed": True, "mismatch_fields": []},
        "partial_blocks": [],
        "recovery_failures": [],
        "blocks": [
            {
                "block_index": 0,
                "transactions": [
                    {
                        "started_tx_index": 0,
                        "is_anchor": True,
                        "disposition": "committed_success",
                        "native_value_transfer": False,
                    },
                    {
                        "started_tx_index": 1,
                        "is_anchor": False,
                        "disposition": "committed_success",
                        "native_value_transfer": False,
                    },
                ],
                "operations": [
                    {"operation_id": 0, "phase": "system", "frame_depth": 0, "component": _opcode(0x01, 3)},
                    {"operation_id": 1, "phase": "transaction", "tx_index": 0, "frame_depth": 0, "component": _opcode(0x40, 20)},
                    {
                        "operation_id": 2,
                        "phase": "transaction",
                        "tx_index": 0,
                        "frame_depth": 0,
                        "component": {
                            "kind": "precompile",
                            "address": "0x01",
                            "pricing_basis": "raw_gas_slope",
                            "native_gas": 3000,
                        },
                    },
                    {
                        "operation_id": 3,
                        "phase": "transaction",
                        "tx_index": 0,
                        "frame_depth": 1,
                        "component": {
                            "kind": "opcode",
                            "opcode": 0xF1,
                            "pricing_basis": "fixed_per_event",
                            "interpreter_raw_gas": None,
                            "model_input": {"kind": "invalid"},
                            "spawned": True,
                            "dispatch_status": "confirmed",
                        },
                    },
                    {
                        "operation_id": 4,
                        "phase": "transaction",
                        "tx_index": 0,
                        "frame_depth": 1,
                        "component": {
                            "kind": "opcode",
                            "opcode": 0xFA,
                            "pricing_basis": None,
                            "interpreter_raw_gas": None,
                            "model_input": {"kind": "invalid"},
                            "spawned": True,
                            "dispatch_status": "selected_not_dispatched",
                        },
                    },
                    {
                        "operation_id": 5,
                        "phase": "transaction",
                        "tx_index": 0,
                        "frame_depth": 0,
                        "component": {
                            **_opcode(0x30, 2),
                            "model_input": {"kind": "context_fixed"},
                        },
                    },
                    {
                        "operation_id": 6,
                        "phase": "transaction",
                        "tx_index": 0,
                        "frame_depth": 0,
                        "component": {
                            **_opcode(0x54, 100),
                            "model_input": {"kind": "storage_load", "access": "warm"},
                        },
                    },
                    {"operation_id": 7, "phase": "transaction", "tx_index": 1, "frame_depth": 0, "component": _opcode(0x40, 20)},
                ],
            }
        ],
    }


class AnchorGapHistogramTests(unittest.TestCase):
    def test_histogram_routes_anchor_only_through_schema4_and_keeps_families_separate(self):
        result = histogram.analyze_anchor_trace(
            _trace(),
            sources=histogram.load_sources(ROOT),
            **HOODI,
        )
        rows = {row["key"]: row for row in result["keys"]}
        self.assertEqual(rows["opcode:0x40"]["event_count"], 1)
        self.assertEqual(rows["opcode:0x40"]["outcome"], "priced")
        self.assertEqual(rows["precompile:0x01"]["native_gas_total"], 3000)
        self.assertEqual(rows["precompile:0x01"]["outcome"], "unmeasured")
        self.assertEqual(rows["opcode:0xf1:confirmed_spawn_wrapper"]["family"], "confirmed_spawn_wrapper")
        self.assertEqual(rows["opcode:0xfa:selected_not_dispatched_spawn"]["outcome"], "intentionally_absent")
        self.assertEqual(rows["opcode:0xfa:selected_not_dispatched_spawn"]["event_count"], 1)
        self.assertEqual(rows["opcode:0xfa:selected_not_dispatched_spawn"]["charge_count"], 0)
        self.assertEqual(rows["opcode:0x30"]["model_source"], "sealed context approximation")
        self.assertEqual(rows["opcode:0x54"]["model_source"], "sealed typed storage model")
        self.assertEqual(result["aggregate"]["anchor_operation_count"], 6)

    def test_histogram_fails_closed_on_incomplete_trace_or_bad_anchor_join(self):
        incomplete = _trace()
        incomplete["status"] = "partial"
        with self.assertRaisesRegex(ValueError, "complete"):
            histogram.analyze_anchor_trace(
                incomplete,
                sources=histogram.load_sources(ROOT),
                **HOODI,
            )
        broken = _trace()
        broken["blocks"][0]["operations"][1]["tx_index"] = 99
        with self.assertRaisesRegex(ValueError, "join"):
            histogram.analyze_anchor_trace(
                broken,
                sources=histogram.load_sources(ROOT),
                **HOODI,
            )

    def test_artifact_replays_aggregate_and_is_create_only(self):
        sources = histogram.load_sources(ROOT)
        record = histogram.analyze_anchor_trace(
            _trace(), sources=sources, **HOODI,
        )
        mainnet_trace = _trace()
        mainnet_trace["guest_input_sha256"] = "0x" + MAINNET["guest_input_sha256"]
        mainnet = histogram.analyze_anchor_trace(mainnet_trace, sources=sources, **MAINNET)
        artifact = histogram.build_analysis_artifact([record, mainnet], sources=sources)
        histogram.verify_analysis_artifact(artifact, repo_root=ROOT)
        with tempfile.TemporaryDirectory() as temporary:
            path = histogram.write_analysis_artifact(artifact, pathlib.Path(temporary))
            self.assertTrue(path.is_file())
            with self.assertRaises(FileExistsError):
                histogram.write_analysis_artifact(artifact, pathlib.Path(temporary))
        altered = dict(artifact)
        altered["aggregate_keys"] = []
        altered["artifact_sha256"] = histogram._sha256(
            {key: value for key, value in altered.items() if key != "artifact_sha256"}
        )
        with self.assertRaisesRegex(ValueError, "aggregate replay"):
            histogram.verify_analysis_artifact(altered)

    def test_same_spawn_opcode_has_distinct_confirmed_and_not_dispatched_rows(self):
        trace = _trace()
        trace["blocks"][0]["operations"][4]["component"]["opcode"] = 0xF1
        result = histogram.analyze_anchor_trace(trace, sources=histogram.load_sources(ROOT), **HOODI)
        rows = {row["key"]: row for row in result["keys"]}
        self.assertEqual(rows["opcode:0xf1:confirmed_spawn_wrapper"]["family"], "confirmed_spawn_wrapper")
        self.assertEqual(rows["opcode:0xf1:selected_not_dispatched_spawn"]["family"], "selected_not_dispatched_spawn")

    def test_analyzer_rejects_trace_schema_or_guest_identity_mismatch(self):
        trace = _trace()
        trace["schema_version"] = 999
        with self.assertRaisesRegex(ValueError, "schema"):
            histogram.analyze_anchor_trace(trace, sources=histogram.load_sources(ROOT), **HOODI)
        trace = _trace()
        trace["guest_input_sha256"] = "0x" + "f" * 64
        with self.assertRaisesRegex(ValueError, "GuestInput"):
            histogram.analyze_anchor_trace(trace, sources=histogram.load_sources(ROOT), **HOODI)

    def test_existing_hoodi_smoke_bundle_is_exactly_joined(self):
        bundle = histogram.load_smoke_bundle(
            ROOT / "experiments/opcode-gas/runs/task-4-integration-smoke/hoodi/record.json",
            repo_root=ROOT,
        )
        self.assertEqual(bundle["identity"], HOODI)
        self.assertEqual(bundle["summary"]["block_count"], len(bundle["trace"]["blocks"]))

    def test_existing_mainnet_smoke_bundle_is_exactly_joined(self):
        bundle = histogram.load_smoke_bundle(
            ROOT / "experiments/opcode-gas/runs/task-4-integration-smoke/mainnet/record.json",
            repo_root=ROOT,
        )
        self.assertEqual(bundle["identity"], MAINNET)
        self.assertEqual(bundle["summary"]["block_count"], len(bundle["trace"]["blocks"]))

    def test_rehashed_semantic_artifact_tampering_is_rejected(self):
        sources = histogram.load_sources(ROOT)
        hoodi = histogram.analyze_anchor_trace(_trace(), sources=sources, **HOODI)
        mainnet_trace = _trace()
        mainnet_trace["guest_input_sha256"] = "0x" + MAINNET["guest_input_sha256"]
        mainnet = histogram.analyze_anchor_trace(mainnet_trace, sources=sources, **MAINNET)
        artifact = histogram.build_analysis_artifact([hoodi, mainnet], sources=sources)
        for mutate, message in (
            (lambda value: value["records"][0]["aggregate"].__setitem__("anchor_operation_count", 999), "record aggregate"),
            (lambda value: value["records"][0].__setitem__("proposal_id", 1), "record identity"),
            (lambda value: value["records"].__setitem__(1, copy.deepcopy(value["records"][0])), "network"),
            (lambda value: value["records"][0]["keys"][0].__setitem__("event_count", -1), "event_count"),
        ):
            altered = copy.deepcopy(artifact)
            mutate(altered)
            altered["artifact_sha256"] = histogram._sha256(
                {key: value for key, value in altered.items() if key != "artifact_sha256"}
            )
            with self.assertRaisesRegex(ValueError, message):
                histogram.verify_analysis_artifact(altered)

    def test_high_precision_prediction_accumulates_exactly(self):
        record = {
            "network": "taiko_hoodi", "proposal_id": HOODI["proposal_id"],
            "input_identity": {key: HOODI[key] for key in HOODI if key.endswith("sha256")},
            "ownership_schema_version": 4,
            "keys": [{
                "key": "opcode:0x01", "family": "opcode_execution", "event_count": 200,
                "charge_count": 200, "raw_gas_total": 200, "native_gas_total": 0,
                "outcome": "priced", "model_source": "test", "reason": "test",
                "predicted_prover_gas": "0.123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890",
            }],
            "aggregate": {"anchor_operation_count": 200, "event_count": 200, "charge_count": 200, "raw_gas_total": 200, "native_gas_total": 0, "unmeasured_key_count": 0},
        }
        records = [copy.deepcopy(record) for _ in range(200)]
        expected = "24.691357802469135780246913578024691357802469135780246913578024691357802469135780246913578024691357802469135780246913578000"
        self.assertEqual(histogram._aggregate_rows(records)[0]["predicted_prover_gas"], expected)

    def test_self_consistent_source_reseal_cannot_replace_reviewed_core(self):
        payload = {"purpose": "tampered"}
        payload["artifact_sha256"] = histogram._sha256(payload)
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "core.json"
            path.write_bytes(histogram._canonical_json(payload))
            with self.assertRaisesRegex(ValueError, "reviewed file SHA256"):
                histogram._reviewed_source_metadata("core", path, payload)


if __name__ == "__main__":
    unittest.main()
