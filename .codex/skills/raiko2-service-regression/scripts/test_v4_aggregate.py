import importlib.util
import json
import os
import tempfile
import threading
import unittest
from contextlib import redirect_stdout
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from io import StringIO
from pathlib import Path
from unittest import mock


SCRIPT_PATH = Path(__file__).with_name("v4_aggregate.py")
SPEC = importlib.util.spec_from_file_location("v4_aggregate", SCRIPT_PATH)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def discovery_payload(*proposal_ids: int) -> dict:
    proposals = []
    l2_start = 100
    for proposal_id in proposal_ids:
        proposals.append(
            {
                "network": "taiko_test",
                "l1_network": "l1_test",
                "proposal_id": proposal_id,
                "l1_inclusion_block_number": proposal_id + 1_000,
                "last_anchor_block_number": l2_start - 1,
                "l2_start": l2_start,
                "l2_end": l2_start + 1,
                "l2_block_numbers": [l2_start, l2_start + 1],
            }
        )
        l2_start += 2
    return {"proposals": proposals}


class AggregateHandler(BaseHTTPRequestHandler):
    responses = []
    requests = []

    def do_POST(self):
        length = int(self.headers["content-length"])
        body = json.loads(self.rfile.read(length))
        type(self).requests.append(
            {"path": self.path, "api_key": self.headers.get("x-api-key"), "body": body}
        )
        response = type(self).responses.pop(0)
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, _format, *_args):
        return


class V4AggregateTests(unittest.TestCase):
    def write_discovery(self, payload: dict) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "proposals.json"
        path.write_text(json.dumps(payload))
        return path

    def test_loads_and_normalizes_discovered_proposals(self):
        path = self.write_discovery(discovery_payload(41, 42))

        proposals = MODULE.load_discovered_proposals(path)

        self.assertEqual([41, 42], [item["proposal_id"] for item in proposals])
        self.assertEqual(100, proposals[0]["l2_block_number_start"])
        self.assertEqual(103, proposals[1]["l2_block_number_end"])
        self.assertNotIn("network", proposals[0])
        self.assertNotIn("l2_block_numbers", proposals[0])

    def test_rejects_noncontiguous_proposal_ids(self):
        path = self.write_discovery(discovery_payload(41, 43))

        with self.assertRaisesRegex(ValueError, "contiguous"):
            MODULE.load_discovered_proposals(path)

    def test_rejects_incomplete_discovery(self):
        path = self.write_discovery(discovery_payload(41, 42))

        with self.assertRaisesRegex(ValueError, "requested proposal IDs"):
            MODULE.load_discovered_proposals(path, expected_proposal_ids=[41, 42, 43])

    def test_polls_same_v4_aggregate_request_until_completed(self):
        AggregateHandler.requests = []
        AggregateHandler.responses = [
            {
                "status": "ok",
                "proposal_id_start": 41,
                "proposal_id_end": 42,
                "data": {"task_id": "task_example", "status": "registered", "proof": None},
            },
            {
                "status": "ok",
                "proposal_id_start": 41,
                "proposal_id_end": 42,
                "data": {
                    "task_id": "task_example",
                    "status": "completed",
                    "proof": "0x1234",
                },
            },
        ]
        server = ThreadingHTTPServer(("127.0.0.1", 0), AggregateHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        path = self.write_discovery(discovery_payload(41, 42))
        os.environ["TEST_RAIKO2_KEY"] = "secret-value"
        self.addCleanup(os.environ.pop, "TEST_RAIKO2_KEY", None)
        output = StringIO()

        with redirect_stdout(output):
            result = MODULE.run_aggregate(
                raiko_rpc=f"http://127.0.0.1:{server.server_port}",
                proposal_file=path,
                proof_type="sgx",
                prover="0x0000000000000000000000000000000000000000",
                api_key_env="TEST_RAIKO2_KEY",
                poll_interval=0,
                timeout=5,
                request_timeout=2,
                expected_proposal_ids=[41, 42],
                transport_retries=0,
                retry_backoff=0,
            )

        self.assertEqual("completed", result["status"])
        self.assertEqual("task_example", result["task_id"])
        self.assertEqual(2, result["proof_bytes"])
        self.assertGreaterEqual(result["elapsed_seconds"], 0)
        self.assertEqual(2, len(AggregateHandler.requests))
        self.assertTrue(
            all(request["path"] == "/v4/proof/proposal" for request in AggregateHandler.requests)
        )
        self.assertTrue(
            all(request["api_key"] == "secret-value" for request in AggregateHandler.requests)
        )
        self.assertTrue(
            all(request["body"]["aggregate"] is True for request in AggregateHandler.requests)
        )
        self.assertEqual(AggregateHandler.requests[0]["body"], AggregateHandler.requests[1]["body"])
        self.assertNotIn("secret-value", output.getvalue())

    def test_retries_transient_transport_failure(self):
        path = self.write_discovery(discovery_payload(41, 42))
        completed = {
            "status": "ok",
            "proposal_id_start": 41,
            "proposal_id_end": 42,
            "data": {
                "task_id": "task_example",
                "status": "completed",
                "proof": "0x1234",
            },
        }

        with redirect_stdout(StringIO()):
            with mock.patch.object(
                MODULE,
                "_post_json",
                side_effect=[MODULE.TransportError("connection reset"), completed],
            ) as post:
                result = MODULE.run_aggregate(
                    raiko_rpc="http://127.0.0.1:1",
                    proposal_file=path,
                    proof_type="sgx",
                    prover="0x0000000000000000000000000000000000000000",
                    api_key_env="",
                    poll_interval=0,
                    timeout=5,
                    request_timeout=2,
                    expected_proposal_ids=[41, 42],
                    transport_retries=1,
                    retry_backoff=0,
                )

        self.assertEqual("completed", result["status"])
        self.assertEqual(2, post.call_count)

    def test_transport_retry_does_not_cross_overall_deadline(self):
        path = self.write_discovery(discovery_payload(41, 42))
        completed = {
            "status": "ok",
            "proposal_id_start": 41,
            "proposal_id_end": 42,
            "data": {
                "task_id": "task_example",
                "status": "completed",
                "proof": "0x1234",
            },
        }
        clock = [0.0]

        def sleep(seconds):
            clock[0] += seconds

        with redirect_stdout(StringIO()):
            with mock.patch.object(MODULE.time, "monotonic", side_effect=lambda: clock[0]):
                with mock.patch.object(MODULE.time, "sleep", side_effect=sleep):
                    with mock.patch.object(
                        MODULE,
                        "_post_json",
                        side_effect=[MODULE.TransportError("connection reset"), completed],
                    ) as post:
                        with self.assertRaisesRegex(TimeoutError, "timed out"):
                            MODULE.run_aggregate(
                                raiko_rpc="http://127.0.0.1:1",
                                proposal_file=path,
                                proof_type="sgx",
                                prover="0x0000000000000000000000000000000000000000",
                                api_key_env="",
                                poll_interval=0,
                                timeout=5,
                                request_timeout=2,
                                expected_proposal_ids=[41, 42],
                                transport_retries=1,
                                retry_backoff=5,
                            )

        self.assertEqual(1, post.call_count)

    def test_progress_output_is_flushed(self):
        path = self.write_discovery(discovery_payload(41, 42))
        completed = {
            "status": "ok",
            "proposal_id_start": 41,
            "proposal_id_end": 42,
            "data": {
                "task_id": "task_example",
                "status": "completed",
                "proof": "0x1234",
            },
        }

        with mock.patch.object(MODULE, "_post_json", return_value=completed):
            with mock.patch("builtins.print") as printer:
                MODULE.run_aggregate(
                    raiko_rpc="http://127.0.0.1:1",
                    proposal_file=path,
                    proof_type="sgx",
                    prover="0x0000000000000000000000000000000000000000",
                    api_key_env="",
                    poll_interval=0,
                    timeout=5,
                    request_timeout=2,
                    expected_proposal_ids=[41, 42],
                    transport_retries=0,
                    retry_backoff=0,
                )

        self.assertTrue(printer.call_args_list)
        self.assertTrue(all(call.kwargs.get("flush") is True for call in printer.call_args_list))

    def test_rejects_server_echoed_proposal_range_mismatch(self):
        path = self.write_discovery(discovery_payload(41, 42))
        mismatched = {
            "status": "ok",
            "proposal_id_start": 41,
            "proposal_id_end": 43,
            "data": {
                "task_id": "task_example",
                "status": "completed",
                "proof": "0x1234",
            },
        }

        with redirect_stdout(StringIO()):
            with mock.patch.object(MODULE, "_post_json", return_value=mismatched):
                with self.assertRaisesRegex(RuntimeError, "proposal_id_end"):
                    MODULE.run_aggregate(
                        raiko_rpc="http://127.0.0.1:1",
                        proposal_file=path,
                        proof_type="sgx",
                        prover="0x0000000000000000000000000000000000000000",
                        api_key_env="",
                        poll_interval=0,
                        timeout=5,
                        request_timeout=2,
                        expected_proposal_ids=[41, 42],
                        transport_retries=0,
                        retry_backoff=0,
                    )


if __name__ == "__main__":
    unittest.main()
