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
DEFAULT_PROVER = "0x0000000000000000000000000000000000000000"


def _required_uint(record: dict[str, Any], key: str) -> int:
    value = record.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"proposal field {key!r} must be a non-negative integer")
    return value


def load_discovered_proposals(path: Path) -> list[dict[str, Any]]:
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
    except urllib.error.URLError as exc:
        raise RuntimeError(f"v4 aggregate request failed: {exc.reason}") from exc

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
) -> dict[str, Any]:
    proposals = load_discovered_proposals(proposal_file)
    payload = build_payload(proposals, proof_type, prover)
    endpoint = f"{raiko_rpc.rstrip('/')}/v4/proof/proposal"
    api_key = os.environ.get(api_key_env) if api_key_env else None
    deadline = time.monotonic() + timeout
    task_id = None
    last_status = None

    while True:
        response = _post_json(
            endpoint,
            payload,
            api_key=api_key,
            timeout=request_timeout,
        )
        if response.get("status") != "ok":
            code = response.get("error", "unknown_error")
            message = response.get("message", "no diagnostic message")
            raise RuntimeError(f"v4 aggregate rejected request: {code}: {message}")

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
                f"task_id={task_id} status={status}"
            )
            last_status = status

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
            }
        if status in TERMINAL_FAILURES:
            detail = data.get("error") or "no diagnostic message"
            raise RuntimeError(f"v4 aggregate task {task_id} ended as {status}: {detail}")
        if status not in ACTIVE_STATUSES:
            raise RuntimeError(f"v4 aggregate task {task_id} returned unknown status {status!r}")
        if time.monotonic() >= deadline:
            raise TimeoutError(f"timed out waiting for v4 aggregate task {task_id}")
        time.sleep(poll_interval)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Submit and poll a v4 aggregate from stress discovery JSON"
    )
    parser.add_argument("--raiko-rpc", required=True, help="raiko2 host base URL")
    parser.add_argument("--proposal-file", required=True, type=Path)
    parser.add_argument("--proof-type", required=True, choices=PROOF_TYPES)
    parser.add_argument("--prover", default=DEFAULT_PROVER)
    parser.add_argument(
        "--api-key-env",
        default="RAIKO2_API_KEY",
        help="environment variable containing the optional x-api-key value",
    )
    parser.add_argument("--poll-interval", type=float, default=5)
    parser.add_argument("--timeout", type=float, default=3600)
    parser.add_argument("--request-timeout", type=float, default=10)
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
            proposals = load_discovered_proposals(args.proposal_file)
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
        )
        print(json.dumps(result, sort_keys=True))
        return 0
    except (RuntimeError, TimeoutError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
