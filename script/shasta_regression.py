"""Shasta regression driver helpers."""

from pathlib import Path
import json
import subprocess
from typing import Dict, Iterable, List, Optional


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
