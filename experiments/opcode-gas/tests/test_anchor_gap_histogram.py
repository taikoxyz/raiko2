import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments/opcode-gas"))

import anchor_gap_histogram as histogram


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
            network="taiko_hoodi",
            proposal_id=80907,
            guest_input_sha256="a" * 64,
            compressed_trace_sha256="b" * 64,
            record_sha256="c" * 64,
            summary_sha256="d" * 64,
            sources=histogram.load_sources(ROOT),
        )
        rows = {row["key"]: row for row in result["keys"]}
        self.assertEqual(rows["opcode:0x40"]["event_count"], 1)
        self.assertEqual(rows["opcode:0x40"]["outcome"], "priced")
        self.assertEqual(rows["precompile:0x01"]["native_gas_total"], 3000)
        self.assertEqual(rows["precompile:0x01"]["outcome"], "unmeasured")
        self.assertEqual(rows["opcode:0xf1:spawned"]["family"], "confirmed_spawn_wrapper")
        self.assertEqual(rows["opcode:0xfa:spawned"]["outcome"], "intentionally_absent")
        self.assertEqual(rows["opcode:0xfa:spawned"]["event_count"], 1)
        self.assertEqual(rows["opcode:0xfa:spawned"]["charge_count"], 0)
        self.assertEqual(rows["opcode:0x30"]["model_source"], "sealed context approximation")
        self.assertEqual(rows["opcode:0x54"]["model_source"], "sealed typed storage model")
        self.assertEqual(result["aggregate"]["anchor_operation_count"], 6)

    def test_histogram_fails_closed_on_incomplete_trace_or_bad_anchor_join(self):
        incomplete = _trace()
        incomplete["status"] = "partial"
        with self.assertRaisesRegex(ValueError, "complete"):
            histogram.analyze_anchor_trace(
                incomplete,
                network="taiko_hoodi",
                proposal_id=80907,
                guest_input_sha256="a" * 64,
                compressed_trace_sha256="b" * 64,
                record_sha256="c" * 64,
                summary_sha256="d" * 64,
                sources=histogram.load_sources(ROOT),
            )
        broken = _trace()
        broken["blocks"][0]["operations"][1]["tx_index"] = 99
        with self.assertRaisesRegex(ValueError, "join"):
            histogram.analyze_anchor_trace(
                broken,
                network="taiko_hoodi",
                proposal_id=80907,
                guest_input_sha256="a" * 64,
                compressed_trace_sha256="b" * 64,
                record_sha256="c" * 64,
                summary_sha256="d" * 64,
                sources=histogram.load_sources(ROOT),
            )

    def test_artifact_replays_aggregate_and_is_create_only(self):
        sources = histogram.load_sources(ROOT)
        record = histogram.analyze_anchor_trace(
            _trace(), network="taiko_hoodi", proposal_id=80907,
            guest_input_sha256="a" * 64, compressed_trace_sha256="b" * 64,
            record_sha256="c" * 64, summary_sha256="d" * 64, sources=sources,
        )
        artifact = histogram.build_analysis_artifact([record], sources=sources)
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


if __name__ == "__main__":
    unittest.main()
