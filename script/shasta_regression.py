"""Shasta regression driver helpers."""

from pathlib import Path
import json


def load_config(path: Path) -> dict:
    return json.loads(Path(path).read_text())


def output_paths(out_dir: Path, proposal_id: int) -> dict:
    return {
        "input": Path(out_dir) / f"proposal_{proposal_id}.json",
        "proof": Path(out_dir) / f"proposal_{proposal_id}.proof.json",
    }


def check_binaries(preflight: str, guest: str) -> bool:
    return not (Path(preflight).is_file() and Path(guest).is_file())
