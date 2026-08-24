#!/usr/bin/env python3
"""Submit and poll a v4 aggregate request from discovered proposal metadata."""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


ACTIVE_STATUSES = {"registered", "work_in_progress"}
TERMINAL_FAILURES = {"failed", "cancelled"}
PROOF_TYPES = ("native", "risc0", "sp1", "sgx", "sgxgeth")


class TransportError(RuntimeError):
    """A retryable failure before a valid HTTP response was received."""


def _required_uint(record: dict[str, Any], key: str) -> int:
    value = record.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"proposal field {key!r} must be a non-negative integer")
    return value


def load_discovered_proposals(
    path: Path,
    expected_proposal_ids: list[int] | None = None,
) -> list[dict[str, Any]]:
    try:
        payload = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read proposal discovery file {path}: {exc}") from exc

    records = payload.get("proposals") if isinstance(payload, dict) else None
    if not isinstance(records, list) or not records:
        raise ValueError("proposal discovery file must contain a non-empty proposals array")

    proposals = []
    for record in records:
        if not isinstance(record, dict):
            raise ValueError("each discovered proposal must be an object")
        l2_start = _required_uint(record, "l2_start")
        l2_end = _required_uint(record, "l2_end")
        if l2_end < l2_start:
            raise ValueError("proposal l2_end must be greater than or equal to l2_start")
        proposals.append(
            {
                "proposal_id": _required_uint(record, "proposal_id"),
                "l1_inclusion_block_number": _required_uint(
                    record, "l1_inclusion_block_number"
                ),
                "l2_block_number_start": l2_start,
                "l2_block_number_end": l2_end,
                "checkpoint": record.get("checkpoint"),
                "last_anchor_block_number": _required_uint(
                    record, "last_anchor_block_number"
                ),
            }
        )

    proposals.sort(key=lambda item: item["proposal_id"])
    for previous, current in zip(proposals, proposals[1:]):
        if current["proposal_id"] != previous["proposal_id"] + 1:
            raise ValueError("aggregate proposal IDs must be strictly increasing and contiguous")
    actual_ids = [proposal["proposal_id"] for proposal in proposals]
    if expected_proposal_ids is not None and actual_ids != expected_proposal_ids:
        raise ValueError(
            f"discovered proposal IDs {actual_ids} do not match requested proposal IDs "
            f"{expected_proposal_ids}"
        )
    return proposals


def build_payload(
    proposals: list[dict[str, Any]], proof_type: str, prover: str
) -> dict[str, Any]:
    return {
        "proposals": proposals,
        "prover": prover,
        "proof_type": proof_type,
        "aggregate": True,
    }


def _post_json(
    url: str,
    payload: dict[str, Any],
    *,
    api_key: str | None,
    timeout: float,
) -> dict[str, Any]:
    headers = {"content-type": "application/json"}
    if api_key:
        headers["x-api-key"] = api_key
    request = urllib.request.Request(
        url,
        data=json.dumps(payload, separators=(",", ":")).encode(),
        headers=headers,
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read()
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        raise RuntimeError(f"v4 aggregate request returned HTTP {exc.code}: {detail}") from exc
    except (urllib.error.URLError, TimeoutError, ConnectionError) as exc:
        reason = getattr(exc, "reason", exc)
        raise TransportError(f"v4 aggregate transport failed: {reason}") from exc

    try:
        decoded = json.loads(body)
    except json.JSONDecodeError as exc:
        raise RuntimeError("v4 aggregate response was not valid JSON") from exc
    if not isinstance(decoded, dict):
        raise RuntimeError("v4 aggregate response must be a JSON object")
    return decoded


def _proof_hex(data: dict[str, Any]) -> str | None:
    proof = data.get("proof")
    if isinstance(proof, str):
        return proof
    if isinstance(proof, dict) and isinstance(proof.get("proof"), str):
        return proof["proof"]
    return None


def _proof_size(proof: str) -> int:
    value = proof.removeprefix("0x")
    try:
        return len(bytes.fromhex(value))
    except ValueError as exc:
        raise RuntimeError("completed aggregate proof is not valid hex") from exc


def run_aggregate(
    *,
    raiko_rpc: str,
    proposal_file: Path,
    proof_type: str,
    prover: str,
    api_key_env: str,
    poll_interval: float,
    timeout: float,
    request_timeout: float,
    expected_proposal_ids: list[int],
    transport_retries: int,
    retry_backoff: float,
    heartbeat_interval: float = 60,
) -> dict[str, Any]:
    proposals = load_discovered_proposals(
        proposal_file,
        expected_proposal_ids=expected_proposal_ids,
    )
    payload = build_payload(proposals, proof_type, prover)
    endpoint = f"{raiko_rpc.rstrip('/')}/v4/proof/proposal"
    api_key = os.environ.get(api_key_env) if api_key_env else None
    started_at = time.monotonic()
    deadline = started_at + timeout
    task_id = None
    last_status = None
    last_log_at = started_at

    while True:
        if time.monotonic() >= deadline:
            raise TimeoutError(
                f"timed out waiting for v4 aggregate task {task_id or 'registration'}"
            )

        transport_attempt = 0
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(
                    f"timed out waiting for v4 aggregate task {task_id or 'registration'}"
                )
            try:
                response = _post_json(
                    endpoint,
                    payload,
                    api_key=api_key,
                    timeout=min(request_timeout, remaining),
                )
                if time.monotonic() >= deadline:
                    raise TimeoutError(
                        f"timed out waiting for v4 aggregate task "
                        f"{task_id or 'registration'}"
                    )
                break
            except TransportError as exc:
                if transport_attempt >= transport_retries:
                    raise TransportError(
                        f"v4 aggregate transport failed after "
                        f"{transport_attempt + 1} attempt(s): {exc}"
                    ) from exc
                transport_attempt += 1
                delay = retry_backoff * (2 ** (transport_attempt - 1))
                print(
                    f"aggregate task_id={task_id or 'registration'} "
                    f"transport_retry={transport_attempt}/{transport_retries} "
                    f"delay_seconds={delay:g}",
                    flush=True,
                )
                if delay > 0:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise TimeoutError(
                            f"timed out waiting for v4 aggregate task "
                            f"{task_id or 'registration'}"
                        )
                    time.sleep(min(delay, remaining))

        if response.get("status") != "ok":
            code = response.get("error", "unknown_error")
            message = response.get("message", "no diagnostic message")
            raise RuntimeError(f"v4 aggregate rejected request: {code}: {message}")

        expected_start = proposals[0]["proposal_id"]
        expected_end = proposals[-1]["proposal_id"]
        if response.get("proposal_id_start") != expected_start:
            raise RuntimeError(
                "v4 aggregate response proposal_id_start does not match the request"
            )
        if response.get("proposal_id_end") != expected_end:
            raise RuntimeError(
                "v4 aggregate response proposal_id_end does not match the request"
            )

        data = response.get("data")
        if not isinstance(data, dict):
            raise RuntimeError("v4 aggregate success response is missing data")
        current_task_id = data.get("task_id")
        if not isinstance(current_task_id, str) or not current_task_id:
            raise RuntimeError("v4 aggregate response is missing task_id")
        if task_id is not None and current_task_id != task_id:
            raise RuntimeError(
                f"v4 aggregate task_id changed while polling: {task_id} -> {current_task_id}"
            )
        task_id = current_task_id
        status = data.get("status")
        if status != last_status:
            print(
                f"aggregate proposals={proposals[0]['proposal_id']}.."
                f"{proposals[-1]['proposal_id']} proof_type={proof_type} "
                f"task_id={task_id} status={status}",
                flush=True,
            )
            last_status = status
            last_log_at = time.monotonic()
        elif time.monotonic() - last_log_at >= heartbeat_interval:
            print(
                f"aggregate proposals={expected_start}..{expected_end} "
                f"proof_type={proof_type} task_id={task_id} status={status} heartbeat=true",
                flush=True,
            )
            last_log_at = time.monotonic()

        if status == "completed":
            proof = _proof_hex(data)
            if proof is None:
                raise RuntimeError("completed aggregate response is missing its final proof")
            return {
                "task_id": task_id,
                "status": status,
                "proof_type": proof_type,
                "proposal_id_start": proposals[0]["proposal_id"],
                "proposal_id_end": proposals[-1]["proposal_id"],
                "proof_bytes": _proof_size(proof),
                "elapsed_seconds": round(time.monotonic() - started_at, 3),
            }
        if status in TERMINAL_FAILURES:
            detail = data.get("error") or "no diagnostic message"
            raise RuntimeError(f"v4 aggregate task {task_id} ended as {status}: {detail}")
        if status not in ACTIVE_STATUSES:
            raise RuntimeError(f"v4 aggregate task {task_id} returned unknown status {status!r}")
        time.sleep(poll_interval)


def parse_proposal_ids(value: str) -> list[int]:
    try:
        proposal_ids = sorted({int(part.strip()) for part in value.split(",") if part.strip()})
    except ValueError as exc:
        raise argparse.ArgumentTypeError("proposal IDs must be comma-separated integers") from exc
    if not proposal_ids or proposal_ids[0] < 0:
        raise argparse.ArgumentTypeError("proposal IDs must be non-negative")
    for previous, current in zip(proposal_ids, proposal_ids[1:]):
        if current != previous + 1:
            raise argparse.ArgumentTypeError("proposal IDs must be contiguous")
    return proposal_ids


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Submit and poll a v4 aggregate from stress discovery JSON"
    )
    parser.add_argument("--raiko-rpc", required=True, help="raiko2 host base URL")
    parser.add_argument("--proposal-file", required=True, type=Path)
    parser.add_argument(
        "--expect-proposal-ids",
        required=True,
        type=parse_proposal_ids,
        help="exact comma-separated proposal IDs expected in the discovery file",
    )
    parser.add_argument("--proof-type", required=True, choices=PROOF_TYPES)
    parser.add_argument("--prover", required=True)
    parser.add_argument(
        "--api-key-env",
        default="RAIKO2_API_KEY",
        help="environment variable containing the optional x-api-key value",
    )
    parser.add_argument("--poll-interval", type=float, default=15)
    parser.add_argument("--timeout", type=float, default=3600)
    parser.add_argument("--request-timeout", type=float, default=10)
    parser.add_argument("--transport-retries", type=int, default=3)
    parser.add_argument("--retry-backoff", type=float, default=2)
    parser.add_argument("--heartbeat-interval", type=float, default=60)
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate input and print the request without sending it",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        if args.dry_run:
            proposals = load_discovered_proposals(
                args.proposal_file,
                expected_proposal_ids=args.expect_proposal_ids,
            )
            print(json.dumps(build_payload(proposals, args.proof_type, args.prover), indent=2))
            return 0
        result = run_aggregate(
            raiko_rpc=args.raiko_rpc,
            proposal_file=args.proposal_file,
            proof_type=args.proof_type,
            prover=args.prover,
            api_key_env=args.api_key_env,
            poll_interval=args.poll_interval,
            timeout=args.timeout,
            request_timeout=args.request_timeout,
            expected_proposal_ids=args.expect_proposal_ids,
            transport_retries=args.transport_retries,
            retry_backoff=args.retry_backoff,
            heartbeat_interval=args.heartbeat_interval,
        )
        print(json.dumps(result, sort_keys=True))
        return 0
    except (RuntimeError, TimeoutError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
