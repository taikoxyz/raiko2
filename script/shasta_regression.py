"""Shasta regression driver helpers."""

from pathlib import Path
import json
import subprocess
from typing import Dict, Iterable, List, Optional
import requests
import argparse
import logging
import time


def load_config(path: Path) -> dict:
    return json.loads(Path(path).read_text())


def resolve_rpc_from_chain_spec(spec_path: Path, chain_name: str) -> Optional[str]:
    data = json.loads(Path(spec_path).read_text())
    for entry in data:
        if entry.get("name") == chain_name:
            return entry.get("rpc")
    return None


def resolve_event_address_from_chain_spec(
    spec_path: Path, chain_name: str, fork: str
) -> Optional[str]:
    data = json.loads(Path(spec_path).read_text())
    for entry in data:
        if entry.get("name") == chain_name:
            contracts = entry.get("l1_contract", {})
            return contracts.get(fork)
    return None


def resolve_event_address_from_config(config: Dict) -> Optional[str]:
    event_address = config.get("event_address")
    if event_address:
        return event_address
    chain_spec_list = config.get("chain_spec_list")
    l1_chain = config.get("l1_chain")
    l1_contract_fork = config.get("l1_contract_fork", "SHASTA")
    if not (chain_spec_list and l1_chain):
        return None
    return resolve_event_address_from_chain_spec(
        Path(chain_spec_list), l1_chain, l1_contract_fork
    )


def output_paths(out_dir: Path, proposal_id: int) -> dict:
    return {
        "input": Path(out_dir) / f"proposal_{proposal_id}.json",
        "proof": Path(out_dir) / f"proposal_{proposal_id}.proof.json",
    }


def check_binaries(preflight: str, guest: str) -> bool:
    return not (Path(preflight).is_file() and Path(guest).is_file())


def select_proposals(proposals, range_tuple, count):
    if range_tuple:
        start, end = range_tuple
        return [p for p in proposals if start <= p <= end]
    if count:
        return proposals[-count:]
    return proposals


def group_for_aggregation(proofs, size):
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


def discover_latest_proposals_from_blocks(blocks: Iterable[Dict], count: int) -> List[int]:
    proposals: List[int] = []
    last_id: Optional[int] = None
    for block in blocks:
        proposal_id = extract_proposal_id_from_extradata(block.get("extraData", ""))
        if proposal_id is None:
            continue
        if proposal_id != last_id:
            proposals.append(proposal_id)
            last_id = proposal_id
        if len(proposals) >= count:
            break
    return list(reversed(proposals))


def rpc_call(rpc_url: str, method: str, params: List, timeout: int) -> Dict:
    response = requests.post(
        rpc_url,
        json={"jsonrpc": "2.0", "method": method, "params": params, "id": 1},
        timeout=timeout,
    )
    response.raise_for_status()
    return response.json()


def get_l2_block(rpc_url: str, block_number: int, timeout: int) -> Optional[Dict]:
    payload = rpc_call(rpc_url, "eth_getBlockByNumber", [hex(block_number), False], timeout)
    return payload.get("result")


def get_latest_l2_block_number(rpc_url: str, timeout: int) -> int:
    payload = rpc_call(rpc_url, "eth_blockNumber", [], timeout)
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


def discover_latest_proposals_from_l2(
    rpc_url: str, count: int, timeout: int
) -> List[int]:
    latest = get_latest_l2_block_number(rpc_url, timeout)
    blocks: List[Dict] = []
    for num in range(latest, -1, -1):
        block = get_l2_block(rpc_url, num, timeout)
        if block:
            blocks.append(block)
        proposals = discover_latest_proposals_from_blocks(blocks, count)
        if len(proposals) >= count:
            return proposals
    return discover_latest_proposals_from_blocks(blocks, count)


def run_command(cmd: List[str]) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=False, text=True, capture_output=True)


def run_preflight(
    preflight_bin: str,
    out_path: Path,
    proposal_id: int,
    l1_rpc: str,
    l2_rpc: str,
    event_address: str,
    event_abi: str,
    anchor_abi: Optional[str],
) -> subprocess.CompletedProcess:
    cmd = [
        preflight_bin,
        "--proposal-id",
        str(proposal_id),
        "--l1-rpc",
        l1_rpc,
        "--l2-rpc",
        l2_rpc,
        "--event-address",
        event_address,
        "--event-abi",
        event_abi,
        "--output",
        str(out_path),
    ]
    if anchor_abi:
        cmd.extend(["--anchor-abi", anchor_abi])
    return run_command(cmd)


def run_guest_launcher(guest_bin: str, input_path: Path) -> subprocess.CompletedProcess:
    cmd = [guest_bin, "--input", str(input_path), "--mode", "execute"]
    return run_command(cmd)


def run_aggregation(
    guest_bin: str,
    proof_files: Iterable[Path],
    out_path: Path,
) -> subprocess.CompletedProcess:
    cmd = [guest_bin, "--aggregate", *[str(p) for p in proof_files], "--output", str(out_path)]
    return run_command(cmd)


def write_summary(path: Path, summary: Dict) -> None:
    Path(path).write_text(json.dumps(summary, indent=2))


def parse_range(value: Optional[str]) -> Optional[tuple]:
    if not value:
        return None
    start, end = value.split(":")
    return (int(start), int(end))


def setup_logger(log_path: Path) -> logging.Logger:
    logger = logging.getLogger("shasta_regression")
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


def main() -> int:
    parser = argparse.ArgumentParser(description="Shasta regression runner")
    parser.add_argument("--config", required=True, help="Path to JSON config")
    parser.add_argument("--range", dest="range_value", help="L2 range start:end", default=None)
    parser.add_argument("--count", type=int, default=None, help="Most recent N proposals")
    parser.add_argument("--aggregate", type=int, default=0, help="Aggregation group size (0=off)")
    parser.add_argument("--out-dir", default="test/regression/shasta")
    parser.add_argument("--prove-type", default="native")
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

    config = load_config(Path(args.config))
    chain_spec_list = config.get("chain_spec_list")
    l1_chain = config.get("l1_chain")
    l2_chain = config.get("l2_chain")
    l1_rpc = config.get("l1_rpc")
    l2_rpc = config.get("l2_rpc")
    event_address = resolve_event_address_from_config(config)
    if chain_spec_list and (l1_chain or l2_chain):
        spec_path = Path(chain_spec_list)
        if l1_chain and not l1_rpc:
            l1_rpc = resolve_rpc_from_chain_spec(spec_path, l1_chain)
        if l2_chain and not l2_rpc:
            l2_rpc = resolve_rpc_from_chain_spec(spec_path, l2_chain)
    event_address = config.get("event_address")
    event_abi = config.get("event_abi")
    anchor_abi = config.get("anchor_abi")
    timeout = args.timeout or config.get("timeout_sec")
    poll_interval = args.poll_interval or config.get("poll_interval_sec")

    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    logger = setup_logger(out_dir / "regression.log")

    if check_binaries(args.preflight_bin, args.guest_launcher_bin):
        logger.error("Missing binaries. Run script/prepare_regression.sh first.")
        return 2

    if not l2_rpc:
        logger.error("Missing l2_rpc in config.")
        return 2
    if not timeout:
        timeout = 10

    range_tuple = parse_range(args.range_value)
    if range_tuple:
        proposals = discover_proposals_from_l2_range(l2_rpc, range_tuple[0], range_tuple[1], timeout)
    elif args.count:
        proposals = discover_latest_proposals_from_l2(l2_rpc, args.count, timeout)
    else:
        logger.error("Either --range or --count must be provided.")
        return 2

    selected = select_proposals(proposals, range_tuple, args.count)
    summary = {
        "timestamp": int(time.time()),
        "inputs": {
            "range": args.range_value,
            "count": args.count,
            "aggregate": args.aggregate,
            "prove_type": args.prove_type,
        },
        "successes": [],
        "failures": [],
        "errors": {},
    }

    for proposal_id in selected:
        paths = output_paths(out_dir, proposal_id)
        preflight = run_preflight(
            args.preflight_bin,
            paths["input"],
            proposal_id,
            l1_rpc,
            l2_rpc,
            event_address,
            event_abi,
            anchor_abi,
        )
        if preflight.returncode != 0:
            logger.error("preflight failed for %s: %s", proposal_id, preflight.stderr)
            summary["failures"].append(proposal_id)
            summary["errors"][str(proposal_id)] = preflight.stderr
            continue

        guest = run_guest_launcher(args.guest_launcher_bin, paths["input"])
        if guest.returncode != 0:
            logger.error("guest-launcher failed for %s: %s", proposal_id, guest.stderr)
            summary["failures"].append(proposal_id)
            summary["errors"][str(proposal_id)] = guest.stderr
            continue

        summary["successes"].append(proposal_id)

    if args.aggregate and args.aggregate > 0:
        proof_files = [output_paths(out_dir, pid)["proof"] for pid in summary["successes"]]
        groups = group_for_aggregation(proof_files, args.aggregate)
        for idx, group in enumerate(groups):
            out_path = out_dir / f"aggregation_{idx}.proof.json"
            result = run_aggregation(args.guest_launcher_bin, group, out_path)
            if result.returncode != 0:
                logger.error("aggregation failed for group %s: %s", idx, result.stderr)

    write_summary(out_dir / "run_summary.json", summary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
