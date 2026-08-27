"""Shasta regression driver helpers."""

from pathlib import Path
import json
import sys
import subprocess
from typing import Dict, Iterable, List, Optional
import requests
import argparse
import logging
import time


def load_json_file(path: Path, label: str) -> object:
    try:
        return json.loads(path.read_text())
    except (FileNotFoundError, json.JSONDecodeError, OSError) as err:
        raise ValueError(f"{label} is invalid: {path}") from err


def load_config(path: Path) -> dict:
    return load_json_file(path, "config")  # type: ignore[return-value]


def load_chain_specs(spec_path: Path) -> List[Dict]:
    return load_json_file(spec_path, "chain spec")  # type: ignore[return-value]


def resolve_rpc_from_chain_spec(spec_path: Path, chain_name: str) -> Optional[str]:
    return resolve_rpc_from_specs(load_chain_specs(spec_path), chain_name)


def resolve_rpc_from_specs(specs: List[Dict], chain_name: str) -> Optional[str]:
    for entry in specs:
        if entry.get("name") == chain_name:
            return entry.get("rpc")
    return None


def resolve_event_address_from_chain_spec(
    spec_path: Path, chain_name: str, fork: str
) -> Optional[str]:
    specs = load_chain_specs(spec_path)
    return resolve_event_address_from_specs(specs, chain_name, fork)


def resolve_event_address_from_specs(
    specs: List[Dict], chain_name: str, fork: str
) -> Optional[str]:
    for entry in specs:
        if entry.get("name") == chain_name:
            contracts = entry.get("l1_contract", {})
            address = contracts.get(fork)
            if address:
                return address
            configured = [value for value in contracts.values() if value]
            if len(configured) == 1:
                return configured[0]
            return None
    return None


def resolve_chain_id_from_chain_spec(spec_path: Path, chain_name: str) -> Optional[int]:
    specs = load_chain_specs(spec_path)
    return resolve_chain_id_from_specs(specs, chain_name)


def resolve_chain_id_from_specs(specs: List[Dict], chain_name: str) -> Optional[int]:
    for entry in specs:
        if entry.get("name") == chain_name:
            return entry.get("chain_id")
    return None


def resolve_event_address_from_config(
    config: Dict, specs: Optional[List[Dict]] = None
) -> Optional[str]:
    event_address = config.get("event_address")
    if event_address:
        return event_address
    chain_spec_list = config.get("chain_spec_list")
    l1_chain = config.get("l1_chain")
    l2_chain = config.get("l2_chain")
    l1_contract_fork = config.get("l1_contract_fork", "SHASTA")
    if not chain_spec_list and specs is None:
        return None
    if specs is None:
        specs = load_chain_specs(Path(chain_spec_list))
    if l1_chain:
        address = resolve_event_address_from_specs(specs, l1_chain, l1_contract_fork)
        if address:
            return address
    if l2_chain:
        return resolve_event_address_from_specs(specs, l2_chain, l1_contract_fork)
    return None


def preflight_rpc_from_config(config: Dict) -> Optional[str]:
    return config.get("l2_rpc") or config.get("l1_rpc")


def format_progress(
    index: int, total: int, stage: str, proposal_id: Optional[int] = None
) -> str:
    suffix = f" proposal={proposal_id}" if proposal_id is not None else ""
    return f"[{index}/{total}] {stage}{suffix}"


def output_paths(out_dir: Path, proposal_id: int) -> dict:
    return {
        "input": out_dir / f"proposal_{proposal_id}.json",
        "proof": out_dir / f"proposal_{proposal_id}.proof.json",
    }


def check_binaries(preflight: str, guest: str) -> bool:
    return not (Path(preflight).is_file() and Path(guest).is_file())


def select_proposals(
    proposals: List[int],
    range_tuple: Optional[tuple],
    count: Optional[int],
) -> List[int]:
    if range_tuple:
        start, end = range_tuple
        return [p for p in proposals if start <= p <= end]
    if count:
        return proposals[-count:]
    return proposals


def select_all_proposals(proposals: List[int]) -> List[int]:
    return proposals


def group_for_aggregation(proofs: List[Path], size: int) -> List[List[Path]]:
    if size <= 0:
        return []
    return [proofs[i : i + size] for i in range(0, len(proofs), size)]


def extract_proposal_id_from_extradata(extradata: str) -> Optional[int]:
    if not extradata:
        return None
    data = extradata[2:] if extradata.startswith("0x") else extradata
    if len(data) < 14:
        return None
    try:
        proposal_hex = data[2:14]
        return int(proposal_hex, 16)
    except ValueError:
        return None


def discover_proposals_from_blocks(blocks: Iterable[Dict]) -> List[int]:
    proposals: List[int] = []
    last_id: Optional[int] = None
    for block in blocks:
        proposal_id = extract_proposal_id_from_extradata(block.get("extraData", ""))
        if proposal_id is None:
            continue
        if proposal_id != last_id:
            proposals.append(proposal_id)
            last_id = proposal_id
    return proposals


def block_number_from_rpc_block(block: Dict) -> Optional[int]:
    raw_number = block.get("number")
    if isinstance(raw_number, int):
        return raw_number
    if isinstance(raw_number, str):
        try:
            return int(raw_number, 16) if raw_number.startswith("0x") else int(raw_number)
        except ValueError:
            return None
    return None


def discover_proposal_spans_from_blocks(blocks: Iterable[Dict]) -> List[Dict[str, int]]:
    spans: List[Dict[str, int]] = []
    for block in blocks:
        number = block_number_from_rpc_block(block)
        proposal_id = extract_proposal_id_from_extradata(block.get("extraData", ""))
        if proposal_id is None or number is None:
            continue
        if not spans or spans[-1]["proposal_id"] != proposal_id:
            spans.append(
                {
                    "proposal_id": proposal_id,
                    "l2_start": number,
                    "l2_end": number,
                    "block_count": 1,
                }
            )
            continue
        spans[-1]["l2_end"] = number
        spans[-1]["block_count"] += 1
    return spans


def load_proposal_metadata(path: Path) -> List[Dict[str, int]]:
    payload = load_json_file(path, "proposal metadata")
    records = payload.get("proposals") if isinstance(payload, dict) else payload
    if not isinstance(records, list):
        raise ValueError(f"proposal metadata must be a list or contain proposals: {path}")

    required = [
        "proposal_id",
        "l1_inclusion_block_number",
        "last_anchor_block_number",
        "l2_start",
        "l2_end",
    ]
    spans: List[Dict[str, int]] = []
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            raise ValueError(f"proposal metadata record {index} is not an object")
        missing = [key for key in required if key not in record]
        if missing:
            raise ValueError(
                f"proposal metadata record {index} is missing: {', '.join(missing)}"
            )
        l2_start = int(record["l2_start"])
        l2_end = int(record["l2_end"])
        l2_block_numbers = record.get("l2_block_numbers")
        block_count = (
            len(l2_block_numbers)
            if isinstance(l2_block_numbers, list)
            else (l2_end - l2_start) + 1
        )
        spans.append(
            {
                "proposal_id": int(record["proposal_id"]),
                "l1_inclusion_block_number": int(
                    record["l1_inclusion_block_number"]
                ),
                "last_anchor_block_number": int(record["last_anchor_block_number"]),
                "l2_start": l2_start,
                "l2_end": l2_end,
                "block_count": block_count,
            }
        )
    return spans


def missing_preflight_metadata(span: Dict[str, int]) -> List[str]:
    return [
        key
        for key in ("l1_inclusion_block_number", "last_anchor_block_number")
        if key not in span
    ]


def rpc_call(rpc_url: str, method: str, params: List, timeout: int) -> Optional[Dict]:
    try:
        response = requests.post(
            rpc_url,
            json={"jsonrpc": "2.0", "method": method, "params": params, "id": 1},
            timeout=timeout,
        )
        response.raise_for_status()
        return response.json()
    except Exception as err:
        logger = logging.getLogger("proposal_regression")
        logger.debug(
            "RPC call failed: url=%s method=%s params=%s timeout=%s error=%s",
            rpc_url,
            method,
            params,
            timeout,
            err,
        )
        return None


def get_l2_block(rpc_url: str, block_number: int, timeout: int) -> Optional[Dict]:
    payload = rpc_call(
        rpc_url, "eth_getBlockByNumber", [hex(block_number), False], timeout
    )
    if not payload:
        return None
    return payload.get("result")


def get_latest_l2_block_number(rpc_url: str, timeout: int) -> int:
    payload = rpc_call(rpc_url, "eth_blockNumber", [], timeout)
    if not payload:
        return 0
    return int(payload.get("result", "0x0"), 16)


def discover_proposals_from_l2_range(
    rpc_url: str, start: int, end: int, timeout: int
) -> List[int]:
    blocks: List[Dict] = []
    for num in range(start, end + 1):
        block = get_l2_block(rpc_url, num, timeout)
        if block:
            blocks.append(block)
    return discover_proposals_from_blocks(blocks)


def discover_proposal_spans_from_l2_range(
    rpc_url: str, start: int, end: int, timeout: int
) -> List[Dict[str, int]]:
    blocks: List[Dict] = []
    for num in range(start, end + 1):
        block = get_l2_block(rpc_url, num, timeout)
        if block:
            blocks.append(block)
    return discover_proposal_spans_from_blocks(blocks)


def run_command(cmd: List[str]) -> subprocess.CompletedProcess:
    logger = logging.getLogger("proposal_regression")
    logger.info(f"Start running CMD: {' '.join(cmd)}")
    return subprocess.run(cmd, check=False, text=True, capture_output=True)


def run_preflight(
    preflight_bin: str,
    out_path: Path,
    proposal_id: int,
    l1_inclusion_block_number: int,
    last_anchor_block_number: int,
    l2_start: int,
    l2_end: int,
    l2_rpc_url: str,
    l1_rpc_url: str,
    l2_chain_id: int,
    l1_chain_id: int,
    proof_type: str,
) -> subprocess.CompletedProcess:
    cmd = build_preflight_cmd(
        preflight_bin=preflight_bin,
        proposal_id=proposal_id,
        l1_inclusion_block_number=l1_inclusion_block_number,
        last_anchor_block_number=last_anchor_block_number,
        l2_start=l2_start,
        l2_end=l2_end,
        l2_rpc_url=l2_rpc_url,
        l1_rpc_url=l1_rpc_url,
        l2_chain_id=l2_chain_id,
        l1_chain_id=l1_chain_id,
        output_path=out_path,
        proof_type=proof_type,
    )
    return run_command(cmd)


def build_preflight_cmd(
    *,
    preflight_bin: str,
    proposal_id: int,
    l1_inclusion_block_number: int,
    last_anchor_block_number: int,
    l2_start: int,
    l2_end: int,
    l2_rpc_url: str,
    l1_rpc_url: str,
    l2_chain_id: int,
    l1_chain_id: int,
    output_path: Path,
    proof_type: str,
) -> List[str]:
    cmd = [
        preflight_bin,
        "--rpc-url",
        l2_rpc_url,
        "--l1-rpc-url",
        l1_rpc_url,
        "--l2-chain-id",
        str(l2_chain_id),
        "--l1-chain-id",
        str(l1_chain_id),
        "--proposal-id",
        str(proposal_id),
        "--l1-inclusion-block-number",
        str(l1_inclusion_block_number),
        "--last-anchor-block-number",
        str(last_anchor_block_number),
        "--l2-start",
        str(l2_start),
        "--l2-end",
        str(l2_end),
        "--proof-type",
        proof_type,
        "--output",
        str(output_path),
    ]
    return cmd


def build_guest_launcher_cmd(
    guest_bin: str,
    input_path: Path,
    mode: str,
    proof_mode: str,
    output_path: Optional[Path],
    proof_type: str,
) -> List[str]:
    cmd = [
        guest_bin,
        "--input",
        str(input_path),
        "--mode",
        mode,
        "--proof-mode",
        proof_mode,
        "--proof-type",
        proof_type,
    ]
    if output_path:
        cmd.extend(["--output", str(output_path)])
    return cmd


def run_guest_launcher(
    guest_bin: str,
    input_path: Path,
    mode: str,
    proof_mode: str,
    output_path: Optional[Path],
    proof_type: str,
) -> subprocess.CompletedProcess:
    cmd = build_guest_launcher_cmd(
        guest_bin=guest_bin,
        input_path=input_path,
        mode=mode,
        proof_mode=proof_mode,
        output_path=output_path,
        proof_type=proof_type,
    )
    return run_command(cmd)


def run_aggregation(
    guest_bin: str,
    proof_files: Iterable[Path],
    out_path: Path,
    proof_type: str,
) -> subprocess.CompletedProcess:
    cmd = [
        guest_bin,
        "--aggregate",
        *[str(p) for p in proof_files],
        "--mode",
        "prove",
        "--proof-type",
        proof_type,
        "--output",
        str(out_path),
    ]
    return run_command(cmd)


def write_summary(path: Path, summary: Dict) -> None:
    path.write_text(json.dumps(summary, indent=2))


def parse_range(value: Optional[str]) -> Optional[tuple]:
    if not value:
        return None
    try:
        start, end = value.split(":")
        return (int(start), int(end))
    except (ValueError, TypeError):
        return None


def parse_boolish(value: Optional[str]) -> Optional[bool]:
    if value is None:
        return None
    lowered = value.strip().lower()
    if lowered in {"1", "true", "t", "yes", "y"}:
        return True
    if lowered in {"0", "false", "f", "no", "n"}:
        return False
    return None


def validate_count_option(count: Optional[int], max_count: int = 5) -> tuple:
    if count is None:
        return True, ""
    if count <= 0:
        return False, "count must be positive"
    if count > max_count:
        return False, f"count exceeds max {max_count}"
    return True, ""


def expand_range_to_proposal_boundaries(
    rpc_url: str, start: int, end: int, timeout: int
) -> tuple:
    start_pid = proposal_id_for_block(rpc_url, start, timeout)
    end_pid = proposal_id_for_block(rpc_url, end, timeout)
    if start_pid is None or end_pid is None:
        return start, end, start_pid, end_pid

    expanded_start = start
    while expanded_start > 0:
        candidate = expanded_start - 1
        pid = proposal_id_for_block(rpc_url, candidate, timeout)
        if pid is None or pid != start_pid:
            break
        expanded_start = candidate

    latest = get_latest_l2_block_number(rpc_url, timeout)
    expanded_end = end
    while expanded_end < latest:
        candidate = expanded_end + 1
        pid = proposal_id_for_block(rpc_url, candidate, timeout)
        if pid is None or pid != end_pid:
            break
        expanded_end = candidate

    return expanded_start, expanded_end, start_pid, end_pid


MAX_SCAN_DEFAULT = 5000


def discover_latest_completed_proposals_from_l2(
    rpc_url: str,
    count: int,
    timeout: int,
    max_scan: int = MAX_SCAN_DEFAULT,
) -> List[int]:
    latest = get_latest_l2_block_number(rpc_url, timeout)

    def pid_at(num: int) -> Optional[int]:
        return proposal_id_for_block(rpc_url, num, timeout)

    current_pid = pid_at(latest)
    if current_pid is None:
        return []

    # Find first block of current proposal via binary search within lookback.
    low, high = max(0, latest - max_scan), latest
    while low < high:
        mid = (low + high) // 2
        pid = pid_at(mid)
        if pid == current_pid:
            high = mid
        else:
            low = mid + 1
    boundary = low

    proposals: List[int] = []
    cursor = boundary - 1

    while cursor >= 0 and len(proposals) < count and (boundary - cursor) <= max_scan:
        pid = pid_at(cursor)
        if pid is None:
            cursor -= 1
            continue
        # Lower bound for this pid
        low, high = max(0, cursor - max_scan), cursor
        while low < high:
            mid = (low + high) // 2
            mid_pid = pid_at(mid)
            if mid_pid == pid:
                high = mid
            else:
                low = mid + 1
        start_pid_block = low
        # Upper bound for this pid
        low, high = cursor, min(latest, cursor + (boundary - start_pid_block))
        while low < high:
            mid = (low + high + 1) // 2
            mid_pid = pid_at(mid)
            if mid_pid == pid:
                low = mid
            else:
                high = mid - 1
        end_pid_block = low

        proposals.append(pid)
        cursor = start_pid_block - 1

    proposals.reverse()
    return proposals


def discover_latest_completed_proposal_spans_from_l2(
    rpc_url: str,
    count: int,
    timeout: int,
    max_scan: int = MAX_SCAN_DEFAULT,
) -> List[Dict[str, int]]:
    proposal_ids = discover_latest_completed_proposals_from_l2(
        rpc_url, count, timeout, max_scan
    )
    spans: List[Dict[str, int]] = []
    for proposal_id in proposal_ids:
        l2_range = derive_block_range_for_proposal(
            rpc_url, proposal_id, timeout, max_scan
        )
        if l2_range is None:
            continue
        l2_start, l2_end = l2_range
        spans.append(
            {
                "proposal_id": proposal_id,
                "l2_start": l2_start,
                "l2_end": l2_end,
                "block_count": (l2_end - l2_start) + 1,
            }
        )
    return spans


def derive_block_range_for_proposal(
    rpc_url: str, proposal_id: int, timeout: int, lookback: int, step: int = 128
) -> Optional[tuple]:
    """
    Derive the contiguous L2 block range for a proposal via stepped + binary search:
    1) Locate a block with the target proposal by stepping back from the tip, halving step when we pass it.
    2) Binary search downward to find the lower boundary.
    3) Binary search upward to find the upper boundary.
    """
    logger = logging.getLogger("proposal_regression")
    latest = get_latest_l2_block_number(rpc_url, timeout)

    logger.info(f"latest proposal is {latest}")

    def pid_at(num: int) -> Optional[int]:
        block = get_l2_block(rpc_url, num, timeout)
        if not block:
            return None
        return extract_proposal_id_from_extradata(block.get("extraData", ""))

    scanned = 0

    # 1) Locate a block with target proposal_id using stepping with halving step when we overshoot.
    pos = latest
    s = step
    found = None
    last_pid = None
    while pos >= 0 and scanned <= lookback:
        pid = pid_at(pos)
        scanned += 1
        if pid == proposal_id:
            found = pos
            break
        if (
            pid is not None
            and last_pid is not None
            and last_pid > proposal_id
            and pid < proposal_id
        ):
            s = max(1, s // 2)
        last_pid = pid
        pos = max(0, pos - s)
    if found is None:
        return None

    # 2) Binary search lower boundary [low, found]
    low, high = max(0, found - lookback), found
    while low < high:
        mid = (low + high) // 2
        pid = pid_at(mid)
        scanned += 1
        if pid == proposal_id:
            high = mid
        else:
            low = mid + 1
        if scanned > lookback:
            return None
    start = low

    # 3) Binary search upper boundary [found, found+lookback]
    low, high = found, min(latest, found + lookback)
    while low < high:
        mid = (low + high + 1) // 2
        pid = pid_at(mid)
        scanned += 1
        if pid == proposal_id:
            low = mid
        else:
            high = mid - 1
        if scanned > lookback:
            return None
    end = low
    logger.info(f"found blocks for proposal {proposal_id}, start: {start}, end: {end}")
    return (start, end)


def setup_logger(log_path: Path) -> logging.Logger:
    logger = logging.getLogger("proposal_regression")
    logger.setLevel(logging.INFO)
    logger.handlers.clear()
    formatter = logging.Formatter("%(asctime)s - %(levelname)s - %(message)s")
    file_handler = logging.FileHandler(log_path)
    file_handler.setFormatter(formatter)
    stream_handler = logging.StreamHandler()
    stream_handler.setFormatter(formatter)
    logger.addHandler(file_handler)
    logger.addHandler(stream_handler)
    return logger


def proposal_id_for_block(
    rpc_url: str, block_number: int, timeout: int
) -> Optional[int]:
    block = get_l2_block(rpc_url, block_number, timeout)
    if not block:
        return None
    return extract_proposal_id_from_extradata(block.get("extraData", ""))


def main() -> int:
    parser = argparse.ArgumentParser(description="Shasta regression runner")
    parser.add_argument("--config", required=True, help="Path to JSON config")
    parser.add_argument(
        "--range", dest="range_value", help="L2 range start:end", default=None
    )
    parser.add_argument(
        "--count",
        type=int,
        default=None,
        help="Most recent N completed proposals (max 5; skips current)",
    )
    parser.add_argument(
        "--discover-only",
        action="store_true",
        help="Print resolved proposal spans as JSON and exit without running binaries",
    )
    parser.add_argument(
        "--proposal-metadata",
        type=Path,
        default=None,
        help="Path to proposal metadata JSON produced by stress_proposal.py --proposal-out",
    )
    parser.add_argument(
        "--aggregate", type=int, default=0, help="Aggregation group size (0=off)"
    )
    parser.add_argument("--out-dir", default="test/regression/proposal")
    parser.add_argument(
        "--proof-type",
        default=None,
        help="Proof backend to use (native or sp1). Defaults to config value or native.",
    )
    parser.add_argument("--timeout", type=int, default=None)
    parser.add_argument("--poll-interval", type=int, default=None)
    parser.add_argument(
        "--preflight-bin",
        default="target/release/preflight",
        help="Path to preflight binary",
    )
    parser.add_argument(
        "--guest-launcher-bin",
        default="target/release/guest-launcher",
        help="Path to guest-launcher binary",
    )
    args = parser.parse_args()

    try:
        config = load_config(Path(args.config))
    except ValueError as err:
        print(f"ERROR: {err}")
        return 2
    chain_spec_list = config.get("chain_spec_list")
    l1_chain = config.get("l1_chain")
    l2_chain = config.get("l2_chain")
    l1_rpc = config.get("l1_rpc")
    l2_rpc = config.get("l2_rpc")
    l1_chain_id = config.get("l1_chain_id")
    l2_chain_id = config.get("l2_chain_id")
    specs = None
    if chain_spec_list:
        try:
            specs = load_chain_specs(Path(chain_spec_list))
        except ValueError as err:
            print(f"ERROR: {err}")
            return 2
    event_address = resolve_event_address_from_config(config, specs)
    if chain_spec_list and (l1_chain or l2_chain):
        if specs is None:
            specs = load_chain_specs(Path(chain_spec_list))
        if l1_chain and not l1_rpc:
            l1_rpc = resolve_rpc_from_specs(specs, l1_chain)
        if l2_chain and not l2_rpc:
            l2_rpc = resolve_rpc_from_specs(specs, l2_chain)
        if l1_chain and not l1_chain_id:
            l1_chain_id = resolve_chain_id_from_specs(specs, l1_chain)
        if l2_chain and not l2_chain_id:
            l2_chain_id = resolve_chain_id_from_specs(specs, l2_chain)
    # TODO: Use event_abi/anchor_abi when adding L1 event-based discovery.
    event_abi = config.get("event_abi")
    anchor_abi = config.get("anchor_abi")
    timeout = args.timeout or config.get("timeout_sec")
    proof_type = args.proof_type or config.get("proof_type") or "native"

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    logger = setup_logger(out_dir / "regression.log")

    if not args.discover_only and check_binaries(
        args.preflight_bin, args.guest_launcher_bin
    ):
        logger.error(
            "Missing binaries. Run scripts/regression/prepare_regression.sh first."
        )
        return 2

    logger.info(
        "Starting discovery (range=%s, count=%s, proposal_metadata=%s, proof_type=%s)",
        args.range_value,
        args.count,
        args.proposal_metadata,
        proof_type,
    )

    if not l2_rpc:
        logger.error("Missing l2_rpc in config or chain spec lookup.")
        return 2
    if not timeout:
        timeout = 10

    range_tuple = parse_range(args.range_value)
    ok, message = validate_count_option(args.count)
    if not ok:
        logger.error("%s", message)
        return 2

    if args.proposal_metadata:
        try:
            selected = load_proposal_metadata(args.proposal_metadata)
        except ValueError as err:
            logger.error("%s", err)
            return 2
        summary_range = None
        summary_proposals = [span["proposal_id"] for span in selected]
    elif range_tuple:
        expanded_start, expanded_end, start_pid, end_pid = (
            expand_range_to_proposal_boundaries(
                l2_rpc, range_tuple[0], range_tuple[1], timeout
            )
        )
        logger.info(
            "Resolved range %s:%s (proposals %s..%s)",
            expanded_start,
            expanded_end,
            start_pid,
            end_pid,
        )
        selected = discover_proposal_spans_from_l2_range(
            l2_rpc, expanded_start, expanded_end, timeout
        )
        summary_range = f"{expanded_start}:{expanded_end}"
        summary_proposals = [span["proposal_id"] for span in selected]
    elif args.count:
        selected = discover_latest_completed_proposal_spans_from_l2(
            l2_rpc, args.count, timeout, MAX_SCAN_DEFAULT
        )
        summary_range = None
        summary_proposals = [span["proposal_id"] for span in selected]
    else:
        logger.error("Either --range, --count, or --proposal-metadata must be provided.")
        return 2

    logger.info("Selected %s proposals", len(selected))
    summary = {
        "timestamp": int(time.time()),
        "inputs": {
            "range": args.range_value,
            "resolved_range": summary_range,
            "resolved_proposals": summary_proposals,
            "resolved_spans": selected,
            "proposal_metadata": str(args.proposal_metadata)
            if args.proposal_metadata
            else None,
            "count": args.count,
            "aggregate": args.aggregate,
            "proof_type": proof_type,
        },
        "successes": [],
        "failures": [],
        "errors": {},
    }

    if args.discover_only:
        json.dump(summary["inputs"], sys.stdout, indent=2)
        print()
        return 0

    preflight_l2_rpc = preflight_rpc_from_config({"l1_rpc": l1_rpc, "l2_rpc": l2_rpc})
    if not preflight_l2_rpc:
        logger.error("Missing l2_rpc in config or chain spec lookup.")
        return 2
    if not l1_rpc:
        logger.error("Missing l1_rpc in config or chain spec lookup.")
        return 2
    if not l2_chain_id:
        logger.error("Missing l2_chain_id in config or chain spec lookup.")
        return 2
    if not l1_chain_id:
        l1_chain_id = 1
    if not event_address:
        logger.error("Missing event_address (resolve from chain spec).")
        return 2
    incomplete_spans = [
        (span["proposal_id"], missing_preflight_metadata(span))
        for span in selected
        if missing_preflight_metadata(span)
    ]
    if incomplete_spans:
        formatted = ", ".join(
            f"{proposal_id} missing {missing}" for proposal_id, missing in incomplete_spans
        )
        logger.error(
            "Selected proposal spans do not include current preflight metadata: %s. "
            "Run scripts/regression/stress_proposal.py --discover-only --proposal-out "
            "and pass the JSON with --proposal-metadata.",
            formatted,
        )
        return 2

    guest_mode = "prove" if proof_type == "native" else "execute"
    proof_mode = "plonk"
    if args.aggregate and args.aggregate > 0:
        if proof_type != "sp1":
            logger.error("Aggregation requires proof_type=sp1.")
            return 2
        if guest_mode != "prove":
            logger.warning(
                "Aggregation requested; switching guest-launcher to prove mode."
            )
        guest_mode = "prove"
        proof_mode = "compressed"

    for idx, span in enumerate(selected, start=1):
        proposal_id = span["proposal_id"]
        logger.info(format_progress(idx, len(selected), "preflight", proposal_id))
        paths = output_paths(out_dir, proposal_id)
        l2_start = span["l2_start"]
        l2_end = span["l2_end"]

        preflight = run_preflight(
            args.preflight_bin,
            paths["input"],
            proposal_id,
            span["l1_inclusion_block_number"],
            span["last_anchor_block_number"],
            l2_start,
            l2_end,
            preflight_l2_rpc,
            l1_rpc,
            l2_chain_id,
            l1_chain_id,
            proof_type,
        )
        if preflight.returncode != 0:
            logger.error("preflight failed for %s: %s", proposal_id, preflight.stderr)
            summary["failures"].append(proposal_id)
            summary["errors"][str(proposal_id)] = preflight.stderr
            continue

        logger.info(format_progress(idx, len(selected), "guest-launcher", proposal_id))
        guest = run_guest_launcher(
            args.guest_launcher_bin,
            paths["input"],
            guest_mode,
            proof_mode,
            paths["proof"] if guest_mode == "prove" else None,
            proof_type,
        )
        if guest.returncode != 0:
            logger.error("guest-launcher failed for %s: %s", proposal_id, guest.stderr)
            summary["failures"].append(proposal_id)
            summary["errors"][str(proposal_id)] = guest.stderr
            continue

        summary["successes"].append(proposal_id)

    if args.aggregate and args.aggregate > 0:
        proof_files = [
            output_paths(out_dir, pid)["proof"] for pid in summary["successes"]
        ]
        groups = group_for_aggregation(proof_files, args.aggregate)
        for idx, group in enumerate(groups, start=1):
            logger.info(format_progress(idx, len(groups), "aggregate"))
            out_path = out_dir / f"aggregation_{idx}.proof.json"
            result = run_aggregation(args.guest_launcher_bin, group, out_path, "sp1")
            if result.returncode != 0:
                logger.error("aggregation failed for group %s: %s", idx, result.stderr)

    write_summary(out_dir / "run_summary.json", summary)
    logger.info(
        "Done: %s success, %s failures",
        len(summary["successes"]),
        len(summary["failures"]),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
