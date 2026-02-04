"""Shasta regression driver helpers."""

from pathlib import Path
import json
import subprocess
from typing import Dict, Iterable, List, Optional
import argparse
import logging
import time


def load_config(path: Path) -> dict:
    return json.loads(Path(path).read_text())


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
    l1_rpc = config.get("l1_rpc")
    l2_rpc = config.get("l2_rpc")
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

    # TODO: hook in discovery logic from old stress_shasta_proposal.py
    proposals = []
    selected = select_proposals(proposals, parse_range(args.range_value), args.count)
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
