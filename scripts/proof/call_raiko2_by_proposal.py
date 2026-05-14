#!/usr/bin/env python3
"""Build and submit a Raiko2 v3 proof request for a Shasta proposal."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any

ROOT_DIR = Path(__file__).resolve().parents[2]
DEFAULT_CHAIN_SPEC_FILE = ROOT_DIR / "config" / "chain_spec_list_default.json"
PROPOSED_TOPIC0 = "0x7c4c4523e17533e451df15762a093e0693a2cd8b279fe54c6cd3777ed5771213"
DEFAULT_PROVER = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8"
DEFAULT_GRAFFITI = "8008500000000000000000000000000000000000000000000000000000000000"
TERMINAL_STATUSES = {"completed", "failed", "cancelled", "error", "expired"}


def fail(message: str) -> None:
    print(f"[ERROR] {message}", file=sys.stderr)
    raise SystemExit(1)


def log_event(event: str, **fields: Any) -> None:
    print(json.dumps({"event": event, **fields}, sort_keys=True), file=sys.stderr, flush=True)


def parse_quantity(value: str) -> int:
    return int(value, 16)


def topic_u256(value: int) -> str:
    return "0x" + value.to_bytes(32, "big", signed=False).hex()


def redact_url(value: str) -> str:
    parsed = urllib.parse.urlsplit(value)
    if not parsed.query:
        return value
    query = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    safe_query = [
        (key, "***" if any(token in key.lower() for token in ("key", "token", "secret")) else val)
        for key, val in query
    ]
    return urllib.parse.urlunsplit(parsed._replace(query=urllib.parse.urlencode(safe_query)))


def rpc_call(rpc_url: str, method: str, params: list[Any], timeout_sec: int) -> Any:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = urllib.request.Request(
        rpc_url,
        data=payload,
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_sec) as response:
            decoded = json.loads(response.read())
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        fail(f"RPC {method} returned HTTP {exc.code}: {detail}")
    except urllib.error.URLError as exc:
        fail(f"RPC {method} failed: {exc.reason}")
    except TimeoutError:
        fail(f"RPC {method} timed out after {timeout_sec}s")
    if decoded.get("error"):
        fail(f"RPC {method} returned an error: {json.dumps(decoded['error'])}")
    return decoded.get("result")


def rpc_batch_call(
    rpc_url: str,
    calls: list[tuple[str, list[Any]]],
    timeout_sec: int,
) -> list[Any]:
    payload = [
        {"jsonrpc": "2.0", "id": index, "method": method, "params": params}
        for index, (method, params) in enumerate(calls)
    ]
    request = urllib.request.Request(
        rpc_url,
        data=json.dumps(payload).encode(),
        headers={"content-type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_sec) as response:
            decoded = json.loads(response.read())
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode(errors="replace")
        fail(f"RPC batch returned HTTP {exc.code}: {detail}")
    except urllib.error.URLError as exc:
        fail(f"RPC batch failed: {exc.reason}")
    if not isinstance(decoded, list):
        fail("RPC batch returned a non-array response")
    by_id = {item.get("id"): item for item in decoded if isinstance(item, dict)}
    results: list[Any] = []
    for index in range(len(calls)):
        item = by_id.get(index)
        if item is None:
            fail(f"RPC batch response is missing id {index}")
        if item.get("error"):
            fail(f"RPC batch item {index} returned an error: {json.dumps(item['error'])}")
        results.append(item.get("result"))
    return results


def http_json(
    url: str,
    *,
    method: str,
    headers: dict[str, str],
    payload: dict[str, Any] | None,
    timeout_sec: int,
) -> tuple[int, Any]:
    data = json.dumps(payload).encode() if payload is not None else None
    request_headers = {"accept": "application/json", **headers}
    if payload is not None:
        request_headers["content-type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=request_headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=timeout_sec) as response:
            body = response.read()
            status = response.status
    except urllib.error.HTTPError as exc:
        body = exc.read()
        status = exc.code
    except urllib.error.URLError as exc:
        fail(f"HTTP {method} request failed: {exc.reason}")
    try:
        decoded = json.loads(body) if body else None
    except json.JSONDecodeError:
        decoded = body.decode(errors="replace")
    return status, decoded


def latest_block_number(rpc_url: str, timeout_sec: int) -> int:
    result = rpc_call(rpc_url, "eth_blockNumber", [], timeout_sec)
    if not isinstance(result, str):
        fail("eth_blockNumber returned a non-hex result")
    return parse_quantity(result)


def fetch_block(rpc_url: str, block_number: int, full: bool, timeout_sec: int) -> dict[str, Any] | None:
    block = rpc_call(rpc_url, "eth_getBlockByNumber", [hex(block_number), full], timeout_sec)
    if block is None:
        return None
    if not isinstance(block, dict):
        fail(f"eth_getBlockByNumber returned unexpected payload for block {block_number}")
    return block


def fetch_blocks(
    rpc_url: str,
    block_numbers: list[int],
    full: bool,
    timeout_sec: int,
) -> list[dict[str, Any] | None]:
    calls = [("eth_getBlockByNumber", [hex(block_number), full]) for block_number in block_numbers]
    results = rpc_batch_call(rpc_url, calls, timeout_sec)
    blocks: list[dict[str, Any] | None] = []
    for block_number, result in zip(block_numbers, results, strict=True):
        if result is not None and not isinstance(result, dict):
            fail(f"eth_getBlockByNumber returned unexpected payload for block {block_number}")
        blocks.append(result)
    return blocks


def proposal_id_from_extra_data(extra_data: str) -> int | None:
    raw = bytes.fromhex(extra_data[2:] if extra_data.startswith("0x") else extra_data)
    if len(raw) < 7:
        return None
    return int.from_bytes(raw[1:7], "big")


def proposal_id_for_block(rpc_url: str, block_number: int, timeout_sec: int) -> int | None:
    block = fetch_block(rpc_url, block_number, False, timeout_sec)
    if not block:
        return None
    try:
        return proposal_id_from_extra_data(str(block.get("extraData", "0x")))
    except ValueError:
        return None


def find_seed_block(args: argparse.Namespace) -> int:
    if args.l2_block_number is not None:
        observed = proposal_id_for_block(args.l2_witness_rpc_url, args.l2_block_number, args.timeout_sec)
        if observed != args.proposal_id:
            fail(f"L2 block {args.l2_block_number} belongs to proposal {observed}, not {args.proposal_id}")
        return args.l2_block_number

    latest = latest_block_number(args.l2_witness_rpc_url, args.timeout_sec)
    lower_bound = max(0, latest - args.l2_max_lookback + 1)
    upper = latest
    while upper >= lower_bound:
        lower = max(lower_bound, upper - args.l2_scan_page_size + 1)
        block_numbers = list(range(lower, upper + 1))
        blocks = fetch_blocks(args.l2_witness_rpc_url, block_numbers, False, args.timeout_sec)
        for block_number, block in reversed(list(zip(block_numbers, blocks, strict=True))):
            if not block:
                continue
            try:
                proposal_id = proposal_id_from_extra_data(str(block.get("extraData", "0x")))
            except ValueError:
                continue
            if proposal_id == args.proposal_id:
                return block_number
        if lower == lower_bound:
            break
        upper = lower - 1
    fail(f"proposal {args.proposal_id} not found in latest {args.l2_max_lookback} L2 blocks")


def expand_l2_range(args: argparse.Namespace, seed_block: int) -> tuple[int, int]:
    start = seed_block
    while start > 0 and proposal_id_for_block(args.l2_witness_rpc_url, start - 1, args.timeout_sec) == args.proposal_id:
        start -= 1
    latest = latest_block_number(args.l2_witness_rpc_url, args.timeout_sec)
    end = seed_block
    while end < latest and proposal_id_for_block(args.l2_witness_rpc_url, end + 1, args.timeout_sec) == args.proposal_id:
        end += 1
    return start, end


def decode_anchor_number(tx_input: str) -> int | None:
    raw = bytes.fromhex(tx_input[2:] if tx_input.startswith("0x") else tx_input)
    if len(raw) < 4 + 32 * 3:
        return None
    checkpoint_block_word = raw[4 : 4 + 32]
    return int.from_bytes(checkpoint_block_word[26:32], "big")


def derive_last_anchor_block_number(args: argparse.Namespace, l2_start: int) -> int:
    if args.last_anchor_block_number is not None:
        return args.last_anchor_block_number
    if l2_start == 0:
        return 0
    block = fetch_block(args.l2_witness_rpc_url, l2_start - 1, True, args.timeout_sec)
    if not block:
        fail(f"failed to fetch parent L2 block {l2_start - 1}")
    transactions = block.get("transactions") or []
    if not transactions or not isinstance(transactions[0], dict):
        fail(f"parent L2 block {l2_start - 1} has no decodable anchor transaction")
    try:
        anchor = decode_anchor_number(str(transactions[0].get("input", "0x")))
    except ValueError:
        anchor = None
    if anchor is None:
        fail(f"failed to derive last_anchor_block_number from parent L2 block {l2_start - 1}")
    return anchor


def resolve_event_address(args: argparse.Namespace) -> str:
    if args.event_address:
        return args.event_address
    chain_spec_file = Path(args.chain_spec_file).expanduser().resolve()
    try:
        specs = json.loads(chain_spec_file.read_text())
    except FileNotFoundError:
        fail(f"chain spec file not found: {chain_spec_file}; pass --event-address")
    except json.JSONDecodeError as exc:
        fail(f"invalid chain spec file {chain_spec_file}: {exc}")
    for spec in specs:
        if not isinstance(spec, dict) or spec.get("name") != args.network:
            continue
        l1_contract = spec.get("l1_contract") or {}
        for fork in ("SHASTA", "UNZEN", "CANCUN", "PACAYA"):
            address = l1_contract.get(fork)
            if address and address != "0x0000000000000000000000000000000000000000":
                return str(address)
    fail(f"network {args.network} does not expose a usable event address in {chain_spec_file}")


def lookup_l1_inclusion(args: argparse.Namespace) -> int:
    if args.l1_inclusion_block_number is not None:
        return args.l1_inclusion_block_number
    event_address = resolve_event_address(args)
    latest = latest_block_number(args.l1_rpc_url, args.timeout_sec)
    lower_bound = max(0, latest - args.l1_max_lookback + 1)
    upper = latest
    while upper >= lower_bound:
        lower = max(lower_bound, upper - args.l1_scan_page_size + 1)
        logs = rpc_call(
            args.l1_rpc_url,
            "eth_getLogs",
            [
                {
                    "address": event_address,
                    "fromBlock": hex(lower),
                    "toBlock": hex(upper),
                    "topics": [PROPOSED_TOPIC0, topic_u256(args.proposal_id)],
                }
            ],
            args.timeout_sec,
        )
        if isinstance(logs, list) and logs:
            block_numbers = sorted(
                {
                    parse_quantity(log["blockNumber"])
                    for log in logs
                    if isinstance(log, dict) and isinstance(log.get("blockNumber"), str)
                }
            )
            if len(block_numbers) != 1:
                fail(f"proposal {args.proposal_id} resolved to L1 blocks {block_numbers}")
            return block_numbers[0]
        if lower == lower_bound:
            break
        upper = lower - 1
    fail(f"no L1 Proposed log found for proposal {args.proposal_id} in latest {args.l1_max_lookback} blocks")


def build_payload(args: argparse.Namespace) -> dict[str, Any]:
    seed = find_seed_block(args)
    l2_start, l2_end = expand_l2_range(args, seed)
    payload: dict[str, Any] = {
        "proposals": [
            {
                "proposal_id": args.proposal_id,
                "l1_inclusion_block_number": lookup_l1_inclusion(args),
                "l2_block_numbers": list(range(l2_start, l2_end + 1)),
                "last_anchor_block_number": derive_last_anchor_block_number(args, l2_start),
            }
        ],
        "aggregate": args.aggregate,
        "proof_type": args.proof_type,
        "network": args.network,
        "l1_network": args.l1_network,
    }
    if args.prover:
        payload["prover"] = args.prover
    if args.graffiti:
        payload["graffiti"] = args.graffiti
    if args.blob_proof_type:
        payload["blob_proof_type"] = args.blob_proof_type
    log_event(
        "request_built",
        proposal_id=args.proposal_id,
        network=args.network,
        l1_network=args.l1_network,
        proof_type=args.proof_type,
        aggregate=args.aggregate,
        l2_start=l2_start,
        l2_end=l2_end,
        last_anchor_block_number=payload["proposals"][0]["last_anchor_block_number"],
        l1_inclusion_block_number=payload["proposals"][0]["l1_inclusion_block_number"],
        l1_rpc=redact_url(args.l1_rpc_url),
        l2_rpc=redact_url(args.l2_witness_rpc_url),
    )
    return payload


def write_json(path: str | None, payload: Any) -> None:
    if not path:
        return
    output = Path(path).expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
    log_event("wrote_json", path=str(output))


def response_status(response: Any) -> str:
    if not isinstance(response, dict):
        return "unknown"
    data = response.get("data")
    if isinstance(data, dict):
        if data.get("proof"):
            return "completed"
        status = data.get("status")
        if isinstance(status, str):
            return status
        if isinstance(status, dict) and status:
            return next(iter(status.keys()))
    status = response.get("status")
    return status if isinstance(status, str) else "unknown"


def task_summary(task: Any) -> dict[str, Any]:
    if not isinstance(task, dict):
        return {"status": "unknown"}
    if isinstance(task.get("data"), dict):
        task = task["data"]
    proposal = ((task.get("proposals") or [{}])[0] or {}) if isinstance(task.get("proposals"), list) else {}
    runtime = proposal.get("runtime") if isinstance(proposal.get("runtime"), dict) else {}
    stages = task.get("stages") if isinstance(task.get("stages"), list) else []
    latest_stage = stages[-1] if stages and isinstance(stages[-1], dict) else {}
    proof = latest_stage.get("proof") if isinstance(latest_stage, dict) else {}
    provider = proof.get("provider") if isinstance(proof, dict) and isinstance(proof.get("provider"), dict) else {}
    return {
        "task_id": task.get("task_id"),
        "status": task.get("status"),
        "route": task.get("route"),
        "proposal_id": proposal.get("proposal_id"),
        "last_event": latest_stage.get("last_event") or (task.get("runtime") or {}).get("last_event"),
        "provider_request_id": provider.get("request_id") or runtime.get("provider_request_id"),
        "remote_tx_hash": provider.get("remote_tx_hash"),
    }


def find_task_id(report: Any, payload: dict[str, Any], proof_type: str) -> str | None:
    if not isinstance(report, list):
        return None
    route_prefix = {"risc0": "risc0/", "sp1": "sp1/", "native": "native/", "zk_any": None}.get(proof_type)
    proposal_id = payload["proposals"][0]["proposal_id"]
    matches: list[dict[str, Any]] = []
    for task in report:
        if not isinstance(task, dict) or task.get("network") != payload["network"]:
            continue
        if route_prefix and not str(task.get("route", "")).startswith(route_prefix):
            continue
        proposals = task.get("proposals")
        if isinstance(proposals, list) and any(
            isinstance(item, dict) and item.get("proposal_id") == proposal_id for item in proposals
        ):
            matches.append(task)
    if not matches:
        return None
    matches.sort(key=lambda item: str((item.get("runtime") or {}).get("updated_at", "")))
    task_id = matches[-1].get("task_id")
    return task_id if isinstance(task_id, str) and task_id else None


def resolve_task_id(
    base_url: str,
    headers: dict[str, str],
    response: Any,
    payload: dict[str, Any],
    timeout_sec: int,
) -> str | None:
    if isinstance(response, dict):
        data = response.get("data")
        if isinstance(data, dict) and isinstance(data.get("task_id"), str):
            return data["task_id"]
    selected = response.get("proof_type") if isinstance(response, dict) else payload["proof_type"]
    proof_type = selected if isinstance(selected, str) and selected else payload["proof_type"]
    for _ in range(10):
        status, report = http_json(
            f"{base_url}/v3/proof/report",
            method="GET",
            headers=headers,
            payload=None,
            timeout_sec=timeout_sec,
        )
        if 200 <= status < 300:
            task_id = find_task_id(report, payload, proof_type)
            if task_id:
                return task_id
        time.sleep(2)
    return None


def raiko2_headers(args: argparse.Namespace) -> dict[str, str]:
    headers: dict[str, str] = {}
    if args.api_key_env:
        api_key = os.environ.get(args.api_key_env)
        if not api_key:
            fail(f"--api-key-env {args.api_key_env} is not set")
        headers[args.api_key_header] = api_key
    return headers


def submit_and_watch(args: argparse.Namespace, payload: dict[str, Any]) -> int:
    if not args.raiko2_url:
        fail("--raiko2-url is required unless --dry-run")
    base_url = args.raiko2_url.rstrip("/")
    headers = raiko2_headers(args)
    status, response = http_json(
        f"{base_url}/v3/proof/batch/shasta",
        method="POST",
        headers=headers,
        payload=payload,
        timeout_sec=args.timeout_sec,
    )
    write_json(args.response_output, response)
    log_event("submit_response", http_status=status, response_status=response_status(response))
    if not (200 <= status < 300):
        print(json.dumps(response, indent=2, sort_keys=True), file=sys.stderr)
        return 1
    if not args.watch:
        print(json.dumps(response, indent=2, sort_keys=True))
        return 0
    task_id = resolve_task_id(base_url, headers, response, payload, args.timeout_sec)
    if not task_id:
        fail("submitted successfully, but failed to resolve task id from response or proof report")
    final_status = "unknown"
    for check in range(1, args.watch_max_checks + 1):
        status, task = http_json(
            f"{base_url}/v3/tasks/{task_id}",
            method="GET",
            headers=headers,
            payload=None,
            timeout_sec=args.timeout_sec,
        )
        summary = task_summary(task)
        final_status = str(summary.get("status") or "unknown")
        log_event("task_status", check=check, http_status=status, **summary)
        if final_status in TERMINAL_STATUSES:
            break
        time.sleep(args.watch_interval_sec)
    return 0 if final_status == "completed" else 2


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proposal-id", type=int, required=True)
    parser.add_argument("--network", required=True, help="Raiko2 network name, for example taiko_masaya")
    parser.add_argument("--l1-network", required=True, help="L1 network name, for example hoodi")
    parser.add_argument("--l1-rpc-url", required=True)
    parser.add_argument("--l2-rpc-url", required=True)
    parser.add_argument("--l2-witness-rpc-url", help="Defaults to --l2-rpc-url")
    parser.add_argument("--raiko2-url", help="Raiko2 base URL; required unless --dry-run")
    parser.add_argument("--api-key-env", help="Environment variable containing the Raiko2 API key")
    parser.add_argument("--api-key-header", default="x-api-key")
    parser.add_argument("--proof-type", choices=("zk_any", "risc0", "sp1", "native"), default="risc0")
    parser.add_argument("--aggregate", action="store_true")
    parser.add_argument("--l2-block-number", type=int, help="Seed L2 block for faster lookup")
    parser.add_argument("--last-anchor-block-number", type=int)
    parser.add_argument("--l1-inclusion-block-number", type=int)
    parser.add_argument("--event-address")
    parser.add_argument("--chain-spec-file", default=str(DEFAULT_CHAIN_SPEC_FILE))
    parser.add_argument("--l1-max-lookback", type=int, default=100_000)
    parser.add_argument("--l1-scan-page-size", type=int, default=2_048)
    parser.add_argument("--l2-max-lookback", type=int, default=20_000)
    parser.add_argument("--l2-scan-page-size", type=int, default=128)
    parser.add_argument("--timeout-sec", type=int, default=10)
    parser.add_argument("--prover", default=DEFAULT_PROVER)
    parser.add_argument("--graffiti", default=DEFAULT_GRAFFITI)
    parser.add_argument("--blob-proof-type", default="proof_of_equivalence")
    parser.add_argument("--output", help="Write generated request JSON")
    parser.add_argument("--response-output", help="Write Raiko2 submit response JSON")
    parser.add_argument("--dry-run", action="store_true", help="Build request only")
    parser.add_argument("--watch", action="store_true")
    parser.add_argument("--watch-interval-sec", type=int, default=15)
    parser.add_argument("--watch-max-checks", type=int, default=80)
    args = parser.parse_args()
    args.l2_witness_rpc_url = args.l2_witness_rpc_url or args.l2_rpc_url
    if args.aggregate and args.proof_type == "zk_any":
        fail("--aggregate requires proof_type risc0 or sp1")
    for name in ("l1_max_lookback", "l1_scan_page_size", "l2_max_lookback", "l2_scan_page_size", "timeout_sec"):
        if getattr(args, name) <= 0:
            fail(f"--{name.replace('_', '-')} must be positive")
    if args.api_key_env and not args.raiko2_url:
        fail("--api-key-env requires --raiko2-url")
    return args


def main() -> int:
    args = parse_args()
    payload = build_payload(args)
    write_json(args.output, payload)
    if args.dry_run:
        print(json.dumps(payload, indent=2, sort_keys=True))
        return 0
    return submit_and_watch(args, payload)


if __name__ == "__main__":
    raise SystemExit(main())
