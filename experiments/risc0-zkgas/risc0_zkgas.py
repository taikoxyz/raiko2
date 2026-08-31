#!/usr/bin/env python3
"""Finite RISC0 proposal zkGas-to-cycle collection and analysis tools."""

from __future__ import annotations

import argparse
import dataclasses
import hashlib
import json
import math
import os
import pathlib
import statistics
import subprocess
import sys
import tempfile
import time
import tomllib
from collections import Counter
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping, Sequence


SCHEMA_VERSION = 1
SUPPORTED_NETWORKS = {
    "taiko_hoodi": {
        "l1_network": "hoodi",
        "chain_id": 167013,
        "unzen_timestamp": 1781787600,
    },
    "taiko_mainnet": {
        "l1_network": "ethereum",
        "chain_id": 167000,
        "unzen_timestamp": 1786021200,
    },
}
MODEL_FEATURES = {
    "M1": ("total_zkgas",),
    "M2": ("total_zkgas", "block_count"),
    "M3": ("total_zkgas", "block_count", "risc0_input_bytes"),
}
RETRYABLE_PATTERNS = (
    "429",
    "500 internal server error",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
    "connection refused",
    "connection reset",
    "network is unreachable",
    "no route to host",
    "temporary failure",
    "timed out",
    "timeout",
    "unexpected eof",
)


class TerminalSampleError(ValueError):
    """A sample cannot become valid by retrying the same cohort and proposal."""


class RetryableCacheError(RuntimeError):
    """A malformed published cache was removed and can be regenerated on resume."""


@dataclass(frozen=True)
class Candidate:
    proposal_id: int
    split: str
    stratum: str | None = None


@dataclass(frozen=True)
class CollectorConfig:
    network: str
    candidate_manifest: pathlib.Path
    target_count: int
    max_candidates: int
    output_dir: pathlib.Path
    source_revision: str
    image_id: str
    risc0_version: str
    execution_po2: int
    proposal_elf: pathlib.Path
    preflight_bin: pathlib.Path
    guest_launcher_bin: pathlib.Path
    python_bin: pathlib.Path = field(default_factory=lambda: pathlib.Path(sys.executable))
    stress_script: pathlib.Path | None = None
    chain_spec_list: pathlib.Path | None = None
    l1_rpc: str | None = None
    l2_rpc: str | None = None
    determinism_rate: float = 0.10
    command_timeout_secs: int = 3600
    repo_root: pathlib.Path | None = None
    revision_resolver: Callable[[pathlib.Path], str] | None = None


@dataclass(frozen=True)
class LinearModel:
    name: str
    features: tuple[str, ...]
    coefficients: dict[str, float]
    sample_count: int
    diagnostics: dict[str, float]

    def predict(self, row: Mapping[str, Any]) -> float:
        value = self.coefficients["intercept"]
        for feature in self.features:
            value += self.coefficients[feature] * _number(row, feature)
        return value


def _number(mapping: Mapping[str, Any], field_name: str) -> float:
    value = mapping.get(field_name)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field_name} must be numeric")
    value = float(value)
    if not math.isfinite(value):
        raise ValueError(f"{field_name} must be finite")
    return value


def _positive_int(value: Any, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ValueError(f"{field_name} must be a positive integer")
    return value


def _parse_quantity(value: Any, field_name: str) -> int:
    if isinstance(value, bool):
        raise TerminalSampleError(f"{field_name} must be an integer quantity")
    if isinstance(value, int):
        return value
    if isinstance(value, str):
        try:
            return int(value, 0)
        except ValueError as error:
            raise TerminalSampleError(f"{field_name} is not an integer quantity") from error
    raise TerminalSampleError(f"{field_name} must be an integer quantity")


def validate_candidate_manifest(
    manifest: Mapping[str, Any], network: str, *, max_candidates: int
) -> list[Candidate]:
    if network not in SUPPORTED_NETWORKS:
        raise ValueError(f"unsupported network: {network}")
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise ValueError(f"candidate manifest schema_version must be {SCHEMA_VERSION}")
    if manifest.get("network") != network:
        raise ValueError(
            f"candidate manifest network {manifest.get('network')!r} does not match {network!r}"
        )
    _positive_int(max_candidates, "max_candidates")
    raw_candidates = manifest.get("candidates")
    if not isinstance(raw_candidates, list) or not raw_candidates:
        raise ValueError("candidate manifest must contain a finite non-empty candidates array")

    allowed_splits = (
        {"fit", "calibration"} if network == "taiko_hoodi" else {"holdout"}
    )
    selected: list[Candidate] = []
    seen: set[int] = set()
    for index, raw in enumerate(raw_candidates):
        if not isinstance(raw, Mapping):
            raise ValueError(f"candidate {index} must be an object")
        proposal_id = _positive_int(raw.get("proposal_id"), f"candidate {index} proposal_id")
        if proposal_id in seen:
            raise ValueError(f"duplicate proposal_id {proposal_id}")
        seen.add(proposal_id)
        split = raw.get("split")
        if split not in allowed_splits:
            raise ValueError(
                f"candidate {proposal_id} split must be one of {sorted(allowed_splits)}"
            )
        stratum = raw.get("stratum")
        if stratum is not None and (not isinstance(stratum, str) or not stratum.strip()):
            raise ValueError(f"candidate {proposal_id} stratum must be a non-empty string")
        selected.append(Candidate(proposal_id, split, stratum))
    return selected[:max_candidates]


def validate_split_targets(
    manifest: Mapping[str, Any],
    network: str,
    *,
    target_count: int,
    candidates: Sequence[Candidate],
) -> dict[str, int]:
    required_splits = (
        ("fit", "calibration") if network == "taiko_hoodi" else ("holdout",)
    )
    raw_targets = manifest.get("split_targets")
    if not isinstance(raw_targets, Mapping):
        raise ValueError("candidate manifest must contain split_targets")
    if set(raw_targets) != set(required_splits):
        raise ValueError(
            f"split_targets must contain exactly {list(required_splits)} for {network}"
        )
    targets = {
        split: _positive_int(raw_targets[split], f"split_targets {split}")
        for split in required_splits
    }
    if sum(targets.values()) != target_count:
        raise ValueError("split_targets must sum to target_count")
    available = Counter(candidate.split for candidate in candidates)
    for split, quota in targets.items():
        if available[split] < quota:
            raise ValueError(
                f"not enough {split} candidates: need {quota}, found {available[split]}"
            )
    return targets


def _nearest_rank(values: Sequence[int], percentile: float) -> int:
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def extract_guest_input_features(
    guest_input: Mapping[str, Any],
    *,
    network: str,
    proposal_id: int,
    l2_start: int,
    l2_end: int,
    guest_input_json_bytes: int,
    unzen_timestamp: int,
) -> dict[str, Any]:
    if network not in SUPPORTED_NETWORKS:
        raise TerminalSampleError(f"unsupported network: {network}")
    taiko = guest_input.get("taiko")
    if not isinstance(taiko, Mapping):
        raise TerminalSampleError("GuestInput is missing taiko metadata")
    if taiko.get("proposal_id") != proposal_id:
        raise TerminalSampleError("GuestInput proposal_id does not match discovered proposal")
    chain_spec = taiko.get("chain_spec")
    if not isinstance(chain_spec, Mapping) or chain_spec.get("name") != network:
        raise TerminalSampleError("GuestInput network does not match collector network")
    if chain_spec.get("is_taiko") is not True:
        raise TerminalSampleError("GuestInput chain spec must identify a Taiko chain")
    chain_id = _parse_quantity(chain_spec.get("chain_id"), "GuestInput chain_id")
    if chain_id != SUPPORTED_NETWORKS[network]["chain_id"]:
        raise TerminalSampleError("GuestInput chain_id does not match collector network")
    expected_activation = SUPPORTED_NETWORKS[network]["unzen_timestamp"]
    if unzen_timestamp != expected_activation:
        raise TerminalSampleError(
            "resolved UNZEN timestamp does not match the expected network schedule"
        )
    activation = unzen_timestamp
    witnesses = guest_input.get("witnesses")
    if not isinstance(witnesses, list) or not witnesses:
        raise TerminalSampleError("GuestInput witnesses must be non-empty")
    expected_numbers = list(range(l2_start, l2_end + 1))
    if len(witnesses) != len(expected_numbers):
        raise TerminalSampleError("GuestInput witness count does not match discovered L2 range")

    timestamps: list[int] = []
    zkgas_values: list[int] = []
    block_numbers: list[int] = []
    transaction_count = 0
    witness_state_node_count = 0
    witness_state_index_count = 0
    witness_code_count = 0
    for index, witness in enumerate(witnesses):
        if not isinstance(witness, Mapping):
            raise TerminalSampleError(f"GuestInput witness {index} must be an object")
        block = witness.get("block")
        header = block.get("header") if isinstance(block, Mapping) else None
        if not isinstance(header, Mapping):
            raise TerminalSampleError(f"GuestInput witness {index} is missing block header")
        number = _parse_quantity(header.get("number"), f"witness {index} block number")
        timestamp = _parse_quantity(header.get("timestamp"), f"witness {index} timestamp")
        difficulty = _parse_quantity(header.get("difficulty"), f"witness {index} difficulty")
        if timestamp < activation:
            raise TerminalSampleError(
                f"GuestInput contains pre-Unzen block {number} at timestamp {timestamp}"
            )
        if difficulty <= 0:
            raise TerminalSampleError(
                f"GuestInput block {number} must have non-zero difficulty after Unzen"
            )
        body = block.get("body")
        transactions = body.get("transactions") if isinstance(body, Mapping) else None
        if not isinstance(transactions, list):
            raise TerminalSampleError(f"GuestInput block {number} is missing transactions")
        witness_data = witness.get("witness")
        if not isinstance(witness_data, Mapping):
            raise TerminalSampleError(f"GuestInput witness {index} is missing witness data")
        for field_name in ("state", "state_indices", "codes"):
            if not isinstance(witness_data.get(field_name), list):
                raise TerminalSampleError(
                    f"GuestInput witness {index} {field_name} must be an array"
                )
        block_numbers.append(number)
        timestamps.append(timestamp)
        zkgas_values.append(difficulty)
        transaction_count += len(transactions)
        witness_state_node_count += len(witness_data["state"])
        witness_state_index_count += len(witness_data["state_indices"])
        witness_code_count += len(witness_data["codes"])

    if block_numbers != expected_numbers:
        raise TerminalSampleError("GuestInput block numbers do not match discovered L2 range")
    proposal_state_nodes = guest_input.get("proposal_state_nodes")
    proposal_ancestor_headers = guest_input.get("proposal_ancestor_headers")
    if not isinstance(proposal_state_nodes, list):
        raise TerminalSampleError("GuestInput proposal_state_nodes must be an array")
    if not isinstance(proposal_ancestor_headers, list):
        raise TerminalSampleError("GuestInput proposal_ancestor_headers must be an array")

    return {
        "first_l2_timestamp": timestamps[0],
        "last_l2_timestamp": timestamps[-1],
        "unzen_timestamp": activation,
        "block_count": len(witnesses),
        "total_zkgas": sum(zkgas_values),
        "min_block_zkgas": min(zkgas_values),
        "median_block_zkgas": statistics.median(zkgas_values),
        "p95_block_zkgas": _nearest_rank(zkgas_values, 0.95),
        "max_block_zkgas": max(zkgas_values),
        "transaction_count": transaction_count,
        "guest_input_json_bytes": guest_input_json_bytes,
        "proposal_state_node_count": len(proposal_state_nodes),
        "proposal_ancestor_header_count": len(proposal_ancestor_headers),
        "witness_state_node_count": witness_state_node_count,
        "witness_state_index_count": witness_state_index_count,
        "witness_code_count": witness_code_count,
    }


def evaluated_mcycles(user_cycles: int) -> int:
    _positive_int(user_cycles, "risc0_user_cycles")
    return (user_cycles + 999_999) // 1_000_000


def quote_bucket(mcycles: float) -> int:
    if not math.isfinite(float(mcycles)):
        raise ValueError("mcycles must be finite")
    rounded = math.ceil(max(0.0, float(mcycles)) / 1000.0) * 1000
    return max(2000, rounded)


def parse_launcher_report(report: Mapping[str, Any]) -> dict[str, Any]:
    if report.get("stage") != "proposal" or report.get("mode") != "execute":
        raise TerminalSampleError("guest-launcher report is not proposal execute mode")
    image_id = report.get("risc0_image_id")
    if not isinstance(image_id, str) or not image_id:
        raise TerminalSampleError("risc0_image_id must be a non-empty string")
    try:
        user_cycles = _positive_int(report.get("risc0_user_cycles"), "risc0_user_cycles")
        input_bytes = _positive_int(report.get("risc0_input_bytes"), "risc0_input_bytes")
        parsed = {
            "risc0_image_id": image_id,
            "risc0_input_bytes": input_bytes,
            "risc0_user_cycles": user_cycles,
            "risc0_padded_cycles": _positive_int(
                report.get("risc0_padded_cycles"), "risc0_padded_cycles"
            ),
            "risc0_segment_count": _positive_int(
                report.get("risc0_segment_count"), "risc0_segment_count"
            ),
            "execution_duration_ms": _positive_int(
                report.get("wall_time_ms"), "wall_time_ms"
            ),
        }
    except ValueError as error:
        raise TerminalSampleError(str(error)) from error
    parsed["evaluated_mcycles_count"] = evaluated_mcycles(user_cycles)
    parsed["current_quote_bucket_mcycles"] = quote_bucket(
        parsed["evaluated_mcycles_count"]
    )
    return parsed


def read_json(path: pathlib.Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def read_jsonl(
    path: pathlib.Path, *, repair_truncated_tail: bool = False
) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    payload = path.read_bytes()
    lines = payload.splitlines(keepends=True)
    rows: list[dict[str, Any]] = []
    offset = 0
    for index, raw_line in enumerate(lines):
        line_number = index + 1
        complete = raw_line.endswith((b"\n", b"\r"))
        if not raw_line.strip():
            offset += len(raw_line)
            continue
        try:
            value = json.loads(raw_line)
        except (UnicodeDecodeError, json.JSONDecodeError):
            is_torn_tail = index == len(lines) - 1 and not complete
            if not repair_truncated_tail or not is_torn_tail:
                raise
            removed_bytes = len(payload) - offset
            with path.open("r+b") as handle:
                handle.truncate(offset)
                handle.flush()
                os.fsync(handle.fileno())
            _fsync_directory(path.parent)
            print(
                f"recovered {path}: truncated {removed_bytes} torn tail bytes",
                file=sys.stderr,
            )
            break
        if not isinstance(value, dict):
            raise ValueError(f"{path}:{line_number}: JSONL row must be an object")
        rows.append(value)
        if repair_truncated_tail and index == len(lines) - 1 and not complete:
            with path.open("ab") as handle:
                handle.write(b"\n")
                handle.flush()
                os.fsync(handle.fileno())
            _fsync_directory(path.parent)
            print(
                f"recovered {path}: terminated complete final JSONL record",
                file=sys.stderr,
            )
        offset += len(raw_line)
    return rows


def _fsync_directory(path: pathlib.Path) -> None:
    try:
        descriptor = os.open(path, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def atomic_write_json(path: pathlib.Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary_path = pathlib.Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, path)
        _fsync_directory(path.parent)
    finally:
        if temporary_path.exists():
            temporary_path.unlink()


def append_jsonl_fsync(path: pathlib.Path, row: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n"
    with path.open("a", encoding="utf-8") as handle:
        handle.write(encoded)
        handle.flush()
        os.fsync(handle.fileno())


def _repo_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[2]


def _safe_stem(value: str) -> str:
    readable = "".join(character for character in value if character.isalnum())[:16]
    digest = hashlib.sha256(value.encode()).hexdigest()[:12]
    return f"{readable or 'value'}-{digest}"


def _sample_key(network: str, proposal_id: int, image_id: str) -> str:
    return f"{network}:{proposal_id}:{image_id}"


def _resolve_path(root: pathlib.Path, path: pathlib.Path) -> pathlib.Path:
    return path if path.is_absolute() else root / path


def _sha256_file(path: pathlib.Path, label: str) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError as error:
        raise ValueError(f"cannot read {label} at {path}: {error}") from error


def _git_head(root: pathlib.Path) -> str:
    result = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        raise ValueError(f"cannot resolve repository HEAD: {result.stderr.strip()}")
    return result.stdout.strip()


def _risc0_version_from_lock(path: pathlib.Path) -> str:
    try:
        payload = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"cannot parse Cargo.lock at {path}: {error}") from error
    versions = {
        package.get("version")
        for package in payload.get("package", [])
        if package.get("name") == "risc0-zkvm"
    }
    if len(versions) != 1 or not all(isinstance(value, str) for value in versions):
        raise ValueError("Cargo.lock must contain exactly one risc0-zkvm version")
    return next(iter(versions))


def _resolve_unzen_timestamp(path: pathlib.Path, network: str) -> int:
    expected = SUPPORTED_NETWORKS[network]["unzen_timestamp"]
    payload = read_json(path)
    if not isinstance(payload, list):
        raise ValueError("resolved chain spec list must be a JSON array")

    l1_network = SUPPORTED_NETWORKS[network]["l1_network"]
    l2_entries = [
        entry
        for entry in payload
        if isinstance(entry, Mapping) and entry.get("name") == network
    ]
    l1_entries = [
        entry
        for entry in payload
        if isinstance(entry, Mapping) and entry.get("name") == l1_network
    ]
    if not l2_entries:
        raise ValueError(f"missing L2 chain spec entry named {network!r}")
    if len(l2_entries) > 1:
        raise ValueError(f"duplicate L2 chain spec entries named {network!r}")
    if not l1_entries:
        raise ValueError(f"missing L1 chain spec entry named {l1_network!r}")
    if len(l1_entries) > 1:
        raise ValueError(f"duplicate L1 chain spec entries named {l1_network!r}")

    hard_forks = l2_entries[0].get("hard_forks")
    if not isinstance(hard_forks, Mapping) or "UNZEN" not in hard_forks:
        return expected
    unzen = hard_forks["UNZEN"]
    if not isinstance(unzen, Mapping) or "Timestamp" not in unzen:
        raise ValueError("custom chain spec UNZEN fork must contain Timestamp")
    try:
        resolved = _parse_quantity(unzen["Timestamp"], "custom UNZEN timestamp")
    except TerminalSampleError as error:
        raise ValueError(str(error)) from error
    if resolved != expected:
        raise ValueError(
            f"custom UNZEN timestamp {resolved} does not match expected {expected}"
        )
    return resolved


def resolve_cohort_identity(config: CollectorConfig) -> dict[str, Any]:
    root = (config.repo_root or _repo_root()).resolve()
    paths = {
        "proposal_elf": _resolve_path(root, config.proposal_elf).resolve(),
        "preflight_binary": _resolve_path(root, config.preflight_bin).resolve(),
        "guest_launcher_binary": _resolve_path(
            root, config.guest_launcher_bin
        ).resolve(),
        "stress_discovery_script": _resolve_path(
            root,
            config.stress_script
            or pathlib.Path("scripts/regression/stress_shasta_proposal.py"),
        ).resolve(),
        "cargo_lock": (root / "Cargo.lock").resolve(),
        "chain_spec": _resolve_path(
            root,
            config.chain_spec_list
            or pathlib.Path("config/chain_spec_list_default.json"),
        ).resolve(),
        "collector_script": pathlib.Path(__file__).resolve(),
    }
    artifact_hashes = {
        f"{name}_sha256": _sha256_file(path, name.replace("_", " "))
        for name, path in paths.items()
    }
    resolved_risc0_version = _risc0_version_from_lock(paths["cargo_lock"])
    if resolved_risc0_version != config.risc0_version:
        raise ValueError(
            f"risc0_version {config.risc0_version!r} does not match Cargo.lock "
            f"version {resolved_risc0_version!r}"
        )
    resolver = config.revision_resolver or _git_head
    resolved_revision = resolver(root).strip()
    if resolved_revision != config.source_revision:
        raise ValueError(
            f"source_revision {config.source_revision!r} does not match repository "
            f"HEAD {resolved_revision!r}"
        )
    unzen_timestamp = _resolve_unzen_timestamp(
        paths["chain_spec"],
        config.network,
    )
    return {
        "repo_root": str(root),
        "resolved_paths": {name: str(path) for name, path in paths.items()},
        "artifact_hashes": artifact_hashes,
        "resolved_source_revision": resolved_revision,
        "resolved_risc0_version": resolved_risc0_version,
        "unzen_timestamp": unzen_timestamp,
    }


def _run_manifest(
    config: CollectorConfig,
    candidates: Sequence[Candidate],
    split_targets: Mapping[str, int],
    identity: Mapping[str, Any],
) -> dict[str, Any]:
    candidate_bytes = config.candidate_manifest.read_bytes()
    return {
        "schema_version": SCHEMA_VERSION,
        "network": config.network,
        "l1_network": SUPPORTED_NETWORKS[config.network]["l1_network"],
        "target_count": config.target_count,
        "target_counts_by_split": dict(split_targets),
        "max_candidates": config.max_candidates,
        "selected_candidate_count": len(candidates),
        "candidate_manifest_sha256": hashlib.sha256(candidate_bytes).hexdigest(),
        "selected_candidates": [dataclasses.asdict(candidate) for candidate in candidates],
        "source_revision": config.source_revision,
        "resolved_source_revision": identity["resolved_source_revision"],
        "image_id": config.image_id,
        "risc0_version": config.risc0_version,
        "resolved_risc0_version": identity["resolved_risc0_version"],
        "artifact_hashes": identity["artifact_hashes"],
        "resolved_paths": identity["resolved_paths"],
        "execution_po2": config.execution_po2,
        "proposal_elf": str(config.proposal_elf),
        "determinism_rate": config.determinism_rate,
        "unzen_timestamp": identity["unzen_timestamp"],
    }


def _initialize_run(
    config: CollectorConfig,
    candidates: Sequence[Candidate],
    split_targets: Mapping[str, int],
    identity: Mapping[str, Any],
) -> None:
    config.output_dir.mkdir(parents=True, exist_ok=True)
    path = config.output_dir / "run-manifest.json"
    expected = _run_manifest(config, candidates, split_targets, identity)
    if path.exists():
        if read_json(path) != expected:
            raise ValueError("existing run manifest does not match requested cohort")
        return
    atomic_write_json(path, expected)


def _command_failure_stage(
    error: BaseException | subprocess.CompletedProcess[str],
) -> tuple[str, str]:
    if isinstance(error, subprocess.TimeoutExpired):
        return "retryable_failure", f"command timed out after {error.timeout} seconds"
    if isinstance(error, OSError):
        return "retryable_failure", str(error)
    diagnostic = f"{error.stdout or ''}\n{error.stderr or ''}".strip()
    lowered = diagnostic.lower()
    status = (
        "retryable_failure"
        if any(pattern in lowered for pattern in RETRYABLE_PATTERNS)
        else "terminal_failure"
    )
    return status, diagnostic or f"command exited {error.returncode}"


def _invoke(
    runner: Callable[..., subprocess.CompletedProcess[str]],
    command: Sequence[object],
    *,
    timeout: int,
) -> subprocess.CompletedProcess[str]:
    result = runner(
        [str(part) for part in command],
        check=False,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    if result.returncode != 0:
        raise CommandFailure(result)
    return result


def _run_json_command_atomic(
    output: pathlib.Path,
    command_factory: Callable[[pathlib.Path], Sequence[object]],
    runner: Callable[..., subprocess.CompletedProcess[str]],
    *,
    timeout: int,
    validator: Callable[[Any], Any],
) -> Any:
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{output.name}.", suffix=".tmp", dir=output.parent
    )
    os.close(descriptor)
    temporary_path = pathlib.Path(temporary_name)
    try:
        _invoke(runner, command_factory(temporary_path), timeout=timeout)
        payload = read_json(temporary_path)
        validated = validator(payload)
        os.replace(temporary_path, output)
        _fsync_directory(output.parent)
        return validated
    finally:
        if temporary_path.exists():
            temporary_path.unlink()


class CommandFailure(RuntimeError):
    def __init__(self, result: subprocess.CompletedProcess[str]):
        super().__init__(f"command exited {result.returncode}")
        self.result = result


def _validate_discovery_payload(
    payload: Any, config: CollectorConfig, candidate: Candidate
) -> dict[str, Any]:
    proposals = payload.get("proposals") if isinstance(payload, Mapping) else None
    if not isinstance(proposals, list) or len(proposals) != 1:
        raise TerminalSampleError("discover-only output must contain exactly one proposal")
    proposal = proposals[0]
    if not isinstance(proposal, dict) or proposal.get("proposal_id") != candidate.proposal_id:
        raise TerminalSampleError("discover-only output proposal_id mismatch")
    if proposal.get("network") != config.network:
        raise TerminalSampleError("discover-only output network mismatch")
    required = (
        "l1_inclusion_block_number",
        "last_anchor_block_number",
        "l2_start",
        "l2_end",
    )
    for field_name in required:
        _positive_int(proposal.get(field_name), f"discovered {field_name}")
    if proposal["l2_start"] > proposal["l2_end"]:
        raise TerminalSampleError("discovered L2 range is reversed")
    return proposal


def _validate_guest_input_payload(payload: Any) -> Mapping[str, Any]:
    if not isinstance(payload, Mapping):
        raise TerminalSampleError("preflight output must be a GuestInput JSON object")
    return payload


def _read_valid_cache(
    output: pathlib.Path,
    validator: Callable[[Any], Any],
    *,
    label: str,
) -> Any:
    try:
        return validator(read_json(output))
    except (ValueError, OSError) as error:
        try:
            output.unlink()
        except OSError:
            pass
        raise RetryableCacheError(
            f"removed malformed cached {label}; retry to regenerate it: {error}"
        ) from error


def _discover_tuple(
    config: CollectorConfig,
    candidate: Candidate,
    output: pathlib.Path,
    runner: Callable[..., subprocess.CompletedProcess[str]],
) -> dict[str, Any]:
    validator = lambda payload: _validate_discovery_payload(payload, config, candidate)
    if output.exists():
        return _read_valid_cache(output, validator, label="discovery output")
    stress_script = config.stress_script or (
        _repo_root() / "scripts/regression/stress_shasta_proposal.py"
    )

    def command(temporary_output: pathlib.Path) -> Sequence[object]:
        command: list[object] = [
            config.python_bin,
            stress_script,
            "--network",
            config.network,
            "--l1-network",
            SUPPORTED_NETWORKS[config.network]["l1_network"],
            "--proposal-id",
            candidate.proposal_id,
            "--discover-only",
            "--proposal-out",
            temporary_output,
            "--log-file",
            "none",
        ]
        if config.chain_spec_list is not None:
            command.extend(("--chain-spec-list", config.chain_spec_list))
        if config.l1_rpc is not None:
            command.extend(("--l1-rpc", config.l1_rpc))
        if config.l2_rpc is not None:
            command.extend(("--l2-rpc", config.l2_rpc))
        return command

    return _run_json_command_atomic(
        output,
        command,
        runner,
        timeout=config.command_timeout_secs,
        validator=validator,
    )


def _run_preflight(
    config: CollectorConfig,
    proposal: Mapping[str, Any],
    output: pathlib.Path,
    runner: Callable[..., subprocess.CompletedProcess[str]],
) -> Mapping[str, Any]:
    if output.exists():
        return _read_valid_cache(
            output, _validate_guest_input_payload, label="preflight output"
        )

    def command(temporary_output: pathlib.Path) -> Sequence[object]:
        command: list[object] = [
            config.preflight_bin,
            "--network",
            config.network,
            "--l1-network",
            SUPPORTED_NETWORKS[config.network]["l1_network"],
            "--proposal-id",
            proposal["proposal_id"],
            "--l1-inclusion-block-number",
            proposal["l1_inclusion_block_number"],
            "--last-anchor-block-number",
            proposal["last_anchor_block_number"],
            "--l2-start",
            proposal["l2_start"],
            "--l2-end",
            proposal["l2_end"],
            "--proof-type",
            "risc0",
            "--output",
            temporary_output,
        ]
        if config.l1_rpc is not None:
            command.extend(("--l1-rpc-url", config.l1_rpc))
        if config.l2_rpc is not None:
            command.extend(("--rpc-url", config.l2_rpc))
        if config.chain_spec_list is not None:
            command.extend(("--chain-spec-file", config.chain_spec_list))
        return command

    return _run_json_command_atomic(
        output,
        command,
        runner,
        timeout=config.command_timeout_secs,
        validator=_validate_guest_input_payload,
    )


def _run_launcher(
    config: CollectorConfig,
    guest_input: pathlib.Path,
    report_path: pathlib.Path,
    runner: Callable[..., subprocess.CompletedProcess[str]],
) -> dict[str, Any]:
    command: list[object] = [
        config.guest_launcher_bin,
        "--stage",
        "proposal",
        "--proof-type",
        "risc0",
        "--mode",
        "execute",
        "--input",
        guest_input,
        "--elf",
        config.proposal_elf,
        "--risc0-execution-po2",
        config.execution_po2,
        "--json-out",
        report_path,
    ]
    _invoke(runner, command, timeout=config.command_timeout_secs)
    parsed = parse_launcher_report(read_json(report_path))
    if parsed["risc0_image_id"] != config.image_id:
        raise TerminalSampleError(
            "guest-launcher risc0_image_id does not match the pinned run manifest"
        )
    return parsed


def _determinism_selected(config: CollectorConfig, candidate: Candidate) -> bool:
    digest = hashlib.sha256(
        _sample_key(config.network, candidate.proposal_id, config.image_id).encode()
    ).digest()
    fraction = int.from_bytes(digest[:8], "big") / float(1 << 64)
    return fraction < config.determinism_rate


def _write_progress(
    config: CollectorConfig,
    candidates: Sequence[Candidate],
    split_targets: Mapping[str, int],
    rows: Sequence[Mapping[str, Any]],
    *,
    exhausted: bool,
) -> dict[str, Any]:
    cohort_rows = [
        row
        for row in rows
        if row.get("network") == config.network and row.get("image_id") == config.image_id
    ]
    latest = {str(row["sample_key"]): row for row in cohort_rows}
    successful_by_split = {
        split: sum(
            row.get("status") == "success" and row.get("split") == split
            for row in latest.values()
        )
        for split in split_targets
    }
    successful = sum(successful_by_split.values())
    terminal = sum(row.get("status") == "terminal_failure" for row in latest.values())
    retryable = sum(row.get("status") == "retryable_failure" for row in latest.values())
    shortfall_by_split = {
        split: max(0, target - successful_by_split[split])
        for split, target in split_targets.items()
    }
    target_reached = all(shortfall == 0 for shortfall in shortfall_by_split.values())
    progress = {
        "schema_version": SCHEMA_VERSION,
        "network": config.network,
        "image_id": config.image_id,
        "target_count": config.target_count,
        "target_counts_by_split": dict(split_targets),
        "selected_candidate_count": len(candidates),
        "successful_samples": successful,
        "successful_samples_by_split": successful_by_split,
        "terminal_failures": terminal,
        "retryable_failures": retryable,
        "target_reached": target_reached,
        "manifest_exhausted": exhausted and not target_reached,
        "shortfall": sum(shortfall_by_split.values()),
        "shortfall_by_split": shortfall_by_split,
        "attempt_rows": len(cohort_rows),
    }
    atomic_write_json(config.output_dir / "progress.json", progress)
    return progress


def collect(
    config: CollectorConfig,
    *,
    runner: Callable[..., subprocess.CompletedProcess[str]] = subprocess.run,
) -> int:
    if config.network not in SUPPORTED_NETWORKS:
        raise ValueError(f"unsupported network: {config.network}")
    _positive_int(config.target_count, "target_count")
    _positive_int(config.max_candidates, "max_candidates")
    _positive_int(config.execution_po2, "execution_po2")
    _positive_int(config.command_timeout_secs, "command_timeout_secs")
    for field_name in ("source_revision", "image_id", "risc0_version"):
        value = getattr(config, field_name)
        if not value.strip():
            raise ValueError(f"{field_name} must be a non-empty string")
    if not 0.0 <= config.determinism_rate <= 1.0:
        raise ValueError("determinism_rate must be between zero and one")
    identity = resolve_cohort_identity(config)
    resolved_paths = identity["resolved_paths"]
    config = dataclasses.replace(
        config,
        proposal_elf=pathlib.Path(resolved_paths["proposal_elf"]),
        preflight_bin=pathlib.Path(resolved_paths["preflight_binary"]),
        guest_launcher_bin=pathlib.Path(resolved_paths["guest_launcher_binary"]),
        stress_script=pathlib.Path(resolved_paths["stress_discovery_script"]),
        chain_spec_list=pathlib.Path(resolved_paths["chain_spec"]),
    )
    manifest = read_json(config.candidate_manifest)
    candidates = validate_candidate_manifest(
        manifest, config.network, max_candidates=config.max_candidates
    )
    split_targets = validate_split_targets(
        manifest,
        config.network,
        target_count=config.target_count,
        candidates=candidates,
    )
    _initialize_run(config, candidates, split_targets, identity)
    results_path = config.output_dir / "samples.jsonl"
    rows = read_jsonl(results_path, repair_truncated_tail=True)
    final_by_key = {
        str(row["sample_key"]): row
        for row in rows
        if row.get("status") in {"success", "terminal_failure"}
    }
    initial_progress = _write_progress(
        config, candidates, split_targets, rows, exhausted=False
    )
    if initial_progress["target_reached"]:
        return 0

    image_stem = _safe_stem(config.image_id)
    for candidate in candidates:
        if (
            initial_progress["successful_samples_by_split"][candidate.split]
            >= split_targets[candidate.split]
        ):
            continue
        key = _sample_key(config.network, candidate.proposal_id, config.image_id)
        if key in final_by_key:
            continue
        proposal_stem = f"{config.network}-{candidate.proposal_id}-{image_stem}"
        discovery_path = config.output_dir / "discovery" / f"{proposal_stem}.json"
        guest_input_path = config.output_dir / "inputs" / f"{proposal_stem}.json"
        report_path = config.output_dir / "launcher-reports" / f"{proposal_stem}.json"
        discovery_path.parent.mkdir(parents=True, exist_ok=True)
        guest_input_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.parent.mkdir(parents=True, exist_ok=True)
        attempt = 1 + sum(row.get("sample_key") == key for row in rows)
        base_row: dict[str, Any] = {
            "schema_version": SCHEMA_VERSION,
            "sample_key": key,
            "network": config.network,
            "proposal_id": candidate.proposal_id,
            "image_id": config.image_id,
            "split": candidate.split,
            "stratum": candidate.stratum,
            "attempt": attempt,
            "source_revision": config.source_revision,
            "risc0_version": config.risc0_version,
            "execution_po2": config.execution_po2,
            "unzen_timestamp": identity["unzen_timestamp"],
            "artifact_hashes": identity["artifact_hashes"],
        }
        wall_start = time.perf_counter()
        stage = "discovery"
        discovery_ms = 0
        preflight_ms = 0
        try:
            started = time.perf_counter()
            proposal = _discover_tuple(config, candidate, discovery_path, runner)
            discovery_ms = round((time.perf_counter() - started) * 1000)
            base_row.update(
                {
                    "l1_inclusion_block": proposal["l1_inclusion_block_number"],
                    "last_anchor_block": proposal["last_anchor_block_number"],
                    "l2_start": proposal["l2_start"],
                    "l2_end": proposal["l2_end"],
                }
            )

            stage = "preflight"
            started = time.perf_counter()
            _run_preflight(config, proposal, guest_input_path, runner)
            preflight_ms = round((time.perf_counter() - started) * 1000)
            guest_input_json_bytes = guest_input_path.stat().st_size
            features = extract_guest_input_features(
                read_json(guest_input_path),
                network=config.network,
                proposal_id=candidate.proposal_id,
                l2_start=proposal["l2_start"],
                l2_end=proposal["l2_end"],
                guest_input_json_bytes=guest_input_json_bytes,
                unzen_timestamp=identity["unzen_timestamp"],
            )

            stage = "execution"
            launcher = _run_launcher(config, guest_input_path, report_path, runner)
            determinism_checked = _determinism_selected(config, candidate)
            determinism_match = None
            repeated_cycles = None
            if determinism_checked:
                repeat_path = report_path.with_name(f"{report_path.stem}.repeat.json")
                repeated = _run_launcher(config, guest_input_path, repeat_path, runner)
                repeated_cycles = repeated["risc0_user_cycles"]
                determinism_match = (
                    repeated_cycles == launcher["risc0_user_cycles"]
                    and repeated["risc0_input_bytes"] == launcher["risc0_input_bytes"]
                )
                base_row["determinism_checked"] = True
                base_row["determinism_match"] = determinism_match
                base_row["determinism_repeat_user_cycles"] = repeated_cycles
                if not determinism_match:
                    raise TerminalSampleError("RISC0 deterministic repeat did not match")
            row = {
                **base_row,
                **features,
                **launcher,
                "status": "success",
                "failure_stage": None,
                "failure_class": None,
                "error": None,
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round((time.perf_counter() - wall_start) * 1000),
                "determinism_checked": determinism_checked,
                "determinism_match": determinism_match,
                "determinism_repeat_user_cycles": repeated_cycles,
            }
        except CommandFailure as error:
            status, diagnostic = _command_failure_stage(error.result)
            row = {
                **base_row,
                "status": status,
                "failure_stage": stage,
                "failure_class": "infrastructure" if status == "retryable_failure" else "command",
                "error": diagnostic,
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round((time.perf_counter() - wall_start) * 1000),
            }
        except (subprocess.TimeoutExpired, OSError) as error:
            status, diagnostic = _command_failure_stage(error)
            row = {
                **base_row,
                "status": status,
                "failure_stage": stage,
                "failure_class": "infrastructure",
                "error": diagnostic,
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round((time.perf_counter() - wall_start) * 1000),
            }
        except RetryableCacheError as error:
            row = {
                **base_row,
                "status": "retryable_failure",
                "failure_stage": stage,
                "failure_class": "cache",
                "error": str(error),
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round(
                    (time.perf_counter() - wall_start) * 1000
                ),
            }
        except TerminalSampleError as error:
            row = {
                **base_row,
                "status": "terminal_failure",
                "failure_stage": stage,
                "failure_class": "invalid_sample",
                "error": str(error),
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round((time.perf_counter() - wall_start) * 1000),
            }
        except ValueError as error:
            row = {
                **base_row,
                "status": "terminal_failure",
                "failure_stage": stage,
                "failure_class": "invalid_output",
                "error": str(error),
                "preflight_duration_ms": preflight_ms,
                "discovery_duration_ms": discovery_ms,
                "wall_clock_duration_ms": round(
                    (time.perf_counter() - wall_start) * 1000
                ),
            }
        append_jsonl_fsync(results_path, row)
        rows.append(row)
        if row["status"] in {"success", "terminal_failure"}:
            final_by_key[key] = row
        progress = _write_progress(
            config, candidates, split_targets, rows, exhausted=False
        )
        initial_progress = progress
        if progress["target_reached"]:
            return 0

    _write_progress(config, candidates, split_targets, rows, exhausted=True)
    return 3


def _solve_linear_system(matrix: list[list[float]], vector: list[float]) -> list[float]:
    size = len(vector)
    augmented = [matrix[row][:] + [vector[row]] for row in range(size)]
    for column in range(size):
        pivot = max(range(column, size), key=lambda row: abs(augmented[row][column]))
        if abs(augmented[pivot][column]) < 1e-12:
            raise ValueError("model feature matrix is singular")
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        augmented[column] = [value / divisor for value in augmented[column]]
        for row in range(size):
            if row == column:
                continue
            factor = augmented[row][column]
            augmented[row] = [
                current - factor * pivot_value
                for current, pivot_value in zip(augmented[row], augmented[column])
            ]
    return [augmented[row][-1] for row in range(size)]


def _metrics(actual: Sequence[float], predicted: Sequence[float]) -> dict[str, float]:
    residuals = [
        actual_value - predicted_value
        for actual_value, predicted_value in zip(actual, predicted)
    ]
    mean_actual = statistics.fmean(actual)
    ss_total = sum((value - mean_actual) ** 2 for value in actual)
    ss_residual = sum(value**2 for value in residuals)
    nonzero_percent_errors = [
        abs(residual / actual_value)
        for residual, actual_value in zip(residuals, actual)
        if actual_value != 0
    ]
    return {
        "r_squared": (
            1.0 - ss_residual / ss_total
            if ss_total
            else (1.0 if not ss_residual else 0.0)
        ),
        "mae": statistics.fmean(abs(value) for value in residuals),
        "mape": statistics.fmean(nonzero_percent_errors) if nonzero_percent_errors else 0.0,
    }


def fit_model(name: str, rows: Sequence[Mapping[str, Any]]) -> LinearModel:
    try:
        features = MODEL_FEATURES[name]
    except KeyError as error:
        raise ValueError(f"unknown model {name}") from error
    if len(rows) <= len(features):
        raise ValueError(f"{name} requires more than {len(features)} fitting samples")
    columns = [[_number(row, feature) for row in rows] for feature in features]
    target = [_number(row, "evaluated_mcycles_count") for row in rows]
    means = [statistics.fmean(column) for column in columns]
    scales = [max(abs(value - mean) for value in column) for column, mean in zip(columns, means)]
    if any(scale == 0 for scale in scales):
        raise ValueError(f"{name} feature matrix is singular")
    normalized = [
        [(value - mean) / scale for value in column]
        for column, mean, scale in zip(columns, means, scales)
    ]
    centered_target = [value - statistics.fmean(target) for value in target]
    gram = [
        [
            sum(left * right for left, right in zip(normalized[i], normalized[j]))
            for j in range(len(features))
        ]
        for i in range(len(features))
    ]
    rhs = [
        sum(
            value * target_value
            for value, target_value in zip(column, centered_target)
        )
        for column in normalized
    ]
    normalized_coefficients = _solve_linear_system(gram, rhs)
    slopes = [value / scale for value, scale in zip(normalized_coefficients, scales)]
    intercept = statistics.fmean(target) - sum(slope * mean for slope, mean in zip(slopes, means))
    coefficients = {"intercept": intercept, **dict(zip(features, slopes))}
    provisional = LinearModel(name, features, coefficients, len(rows), {})
    diagnostics = _metrics(target, [provisional.predict(row) for row in rows])
    return dataclasses.replace(provisional, diagnostics=diagnostics)


def largest_positive_residual(model: LinearModel, rows: Sequence[Mapping[str, Any]]) -> float:
    if not rows:
        raise ValueError("calibration rows must be non-empty")
    return max(
        0.0,
        max(_number(row, "evaluated_mcycles_count") - model.predict(row) for row in rows),
    )


def evaluate_holdout(
    model: LinearModel,
    calibration_margin: float,
    rows: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    if not rows:
        raise ValueError("Mainnet holdout rows must be non-empty")
    evaluations = []
    for row in rows:
        actual_mcycles = _number(row, "evaluated_mcycles_count")
        predicted = model.predict(row)
        safe = predicted + calibration_margin
        quoted = quote_bucket(safe)
        actual_bucket = quote_bucket(actual_mcycles)
        evaluations.append(
            {
                "proposal_id": row.get("proposal_id"),
                "actual_mcycles": actual_mcycles,
                "prediction_mcycles": predicted,
                "safe_mcycles": safe,
                "quoted_mcycles": quoted,
                "actual_quote_bucket_mcycles": actual_bucket,
                "underquote": quoted < actual_mcycles,
                "exact_bucket_match": quoted == actual_bucket,
                "quote_overhead_mcycles": quoted - actual_bucket,
                "residual_mcycles": actual_mcycles - predicted,
            }
        )
    underquote_count = sum(row["underquote"] for row in evaluations)
    match_rate = sum(row["exact_bucket_match"] for row in evaluations) / len(evaluations)
    p95_overhead = _nearest_rank(
        [int(row["quote_overhead_mcycles"]) for row in evaluations], 0.95
    )
    actual = [float(row["actual_mcycles"]) for row in evaluations]
    predicted = [float(row["prediction_mcycles"]) for row in evaluations]
    return {
        **_metrics(actual, predicted),
        "sample_count": len(evaluations),
        "underquote_count": underquote_count,
        "exact_bucket_match_rate": match_rate,
        "p95_quote_overhead_mcycles": p95_overhead,
        "gates": {
            "zero_underquotes": underquote_count == 0,
            "exact_bucket_match_rate_at_least_90_percent": match_rate >= 0.90,
            "p95_quote_overhead_at_most_1000_mcycles": p95_overhead <= 1000,
        },
        "samples": evaluations,
    }


def _pearson(left: Sequence[float], right: Sequence[float]) -> float:
    if len(left) != len(right) or not left:
        raise ValueError("correlation inputs must have the same non-zero length")
    left_mean = statistics.fmean(left)
    right_mean = statistics.fmean(right)
    left_centered = [value - left_mean for value in left]
    right_centered = [value - right_mean for value in right]
    left_norm = math.sqrt(sum(value * value for value in left_centered))
    right_norm = math.sqrt(sum(value * value for value in right_centered))
    if left_norm < 1e-9 or right_norm < 1e-9:
        return 0.0
    return sum(
        left_value * right_value
        for left_value, right_value in zip(left_centered, right_centered)
    ) / (left_norm * right_norm)


def _residual_correlation(
    model: LinearModel, rows: Sequence[Mapping[str, Any]], feature: str
) -> float:
    residuals = [
        _number(row, "evaluated_mcycles_count") - model.predict(row) for row in rows
    ]
    if max(residuals) - min(residuals) < 1e-7:
        return 0.0
    return _pearson([_number(row, feature) for row in rows], residuals)


def _require_exact_count(rows: Sequence[Any], expected: int, label: str) -> None:
    if len(rows) != expected:
        raise ValueError(f"expected {expected} {label} samples, found {len(rows)}")


def _cohort_invariants(rows: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    invariant_fields = (
        "source_revision",
        "image_id",
        "risc0_version",
        "execution_po2",
        "artifact_hashes",
    )
    cohort = {}
    for field_name in invariant_fields:
        raw_values = [row.get(field_name) for row in rows]
        if any(value is None for value in raw_values):
            raise ValueError(f"successful samples cross cohort invariant {field_name}")
        values = {
            json.dumps(value, sort_keys=True, separators=(",", ":"))
            for value in raw_values
        }
        if len(values) != 1:
            raise ValueError(f"successful samples cross cohort invariant {field_name}")
        cohort[field_name] = raw_values[0]
    return cohort


def analyze_experiment(
    rows: Sequence[Mapping[str, Any]],
    *,
    expected_fit_count: int = 80,
    expected_calibration_count: int = 40,
    expected_holdout_count: int = 60,
    material_correlation_threshold: float = 0.20,
) -> dict[str, Any]:
    if not 0.0 < material_correlation_threshold < 1.0:
        raise ValueError("material_correlation_threshold must be between zero and one")
    successful = [row for row in rows if row.get("status") == "success"]
    if not successful:
        raise ValueError("sample file contains no successful samples")
    for row in successful:
        network = row.get("network")
        if network not in SUPPORTED_NETWORKS:
            raise ValueError("successful sample contains an unsupported network")
        if row.get("unzen_timestamp") != SUPPORTED_NETWORKS[network]["unzen_timestamp"]:
            raise ValueError("successful sample has an invalid resolved UNZEN timestamp")
    keys = [row.get("sample_key") for row in successful]
    if any(not isinstance(key, str) or not key for key in keys):
        raise ValueError("every successful row must have a sample_key")
    if len(keys) != len(set(keys)):
        raise ValueError("successful samples contain duplicate cohort keys")
    cohort = _cohort_invariants(successful)
    fit_rows = [
        row
        for row in successful
        if row.get("network") == "taiko_hoodi" and row.get("split") == "fit"
    ]
    calibration_rows = [
        row
        for row in successful
        if row.get("network") == "taiko_hoodi"
        and row.get("split") == "calibration"
    ]
    holdout_rows = [
        row
        for row in successful
        if row.get("network") == "taiko_mainnet" and row.get("split") == "holdout"
    ]
    if len(fit_rows) + len(calibration_rows) + len(holdout_rows) != len(successful):
        raise ValueError("successful rows contain an unsupported network or split")
    _require_exact_count(fit_rows, expected_fit_count, "Hoodi fitting")
    _require_exact_count(calibration_rows, expected_calibration_count, "Hoodi calibration")
    _require_exact_count(holdout_rows, expected_holdout_count, "Mainnet holdout")

    models = {name: fit_model(name, fit_rows) for name in MODEL_FEATURES}
    m1_block_trend = _residual_correlation(models["M1"], fit_rows, "block_count")
    selected_name = "M1"
    selection_reasons = [
        f"M1 fit residual/block_count correlation={m1_block_trend:.6f}"
    ]
    m2_input_trend = None
    if abs(m1_block_trend) >= material_correlation_threshold:
        selected_name = "M2"
        m2_input_trend = _residual_correlation(
            models["M2"], fit_rows, "risc0_input_bytes"
        )
        selection_reasons.append(
            f"M2 fit residual/risc0_input_bytes correlation={m2_input_trend:.6f}"
        )
        if abs(m2_input_trend) >= material_correlation_threshold:
            selected_name = "M3"
    selected_model = models[selected_name]
    margins = {
        name: largest_positive_residual(model, calibration_rows)
        for name, model in models.items()
    }
    calibration_margin = margins[selected_name]

    # This is the first operation in the analysis that reads Mainnet target values. Every candidate
    # model and one-sided margin is already fixed entirely from Hoodi above.
    holdout_evaluations = {
        name: evaluate_holdout(model, margins[name], holdout_rows)
        for name, model in models.items()
    }
    holdout = holdout_evaluations[selected_name]
    residual_trends = {
        feature: _residual_correlation(selected_model, holdout_rows, feature)
        for feature in ("total_zkgas", "block_count", "risc0_input_bytes")
    }
    cross_network_rows = [*calibration_rows, *holdout_rows]
    cross_network_residuals = [
        _number(row, "evaluated_mcycles_count") - selected_model.predict(row)
        for row in cross_network_rows
    ]
    if max(cross_network_residuals) - min(cross_network_residuals) < 1e-7:
        network_trend = 0.0
    else:
        network_trend = _pearson(
            [1.0 if row["network"] == "taiko_mainnet" else 0.0 for row in cross_network_rows],
            cross_network_residuals,
        )
    residual_trends["network"] = network_trend
    no_material_trends = all(
        abs(value) < material_correlation_threshold for value in residual_trends.values()
    )
    repeated = [row for row in rows if row.get("determinism_checked") is True]
    determinism_mismatches = sum(
        row.get("determinism_match") is False for row in repeated
    )
    determinism_matches = bool(repeated) and all(
        row.get("determinism_match") is True for row in repeated
    )
    holdout["gates"]["no_material_residual_trends"] = no_material_trends
    holdout["gates"]["deterministic_repeats_match"] = determinism_matches
    recommendation = all(holdout["gates"].values())
    failures = Counter(
        f"{row.get('status')}:{row.get('failure_stage') or 'unknown'}"
        for row in rows
        if row.get("status") != "success"
    )
    feature_envelope = {
        feature: {
            "min": min(_number(row, feature) for row in [*fit_rows, *calibration_rows]),
            "max": max(_number(row, feature) for row in [*fit_rows, *calibration_rows]),
        }
        for feature in MODEL_FEATURES["M3"]
    }
    candidate_models = {
        name: {
            "features": list(model.features),
            "coefficients": model.coefficients,
            "fit_diagnostics": model.diagnostics,
            "calibration_margin_mcycles": margins[name],
            "mainnet_holdout": {
                key: value
                for key, value in holdout_evaluations[name].items()
                if key != "samples"
            },
        }
        for name, model in models.items()
    }
    analysis: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "input_rows_sha256": hashlib.sha256(
            json.dumps(list(rows), sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        "cohort": cohort,
        "sample_counts": {
            "hoodi_fit": len(fit_rows),
            "hoodi_calibration": len(calibration_rows),
            "mainnet_holdout": len(holdout_rows),
            "determinism_repeats": len(repeated),
            "determinism_mismatches": determinism_mismatches,
        },
        "selected_model": selected_name,
        "selected_features": list(selected_model.features),
        "coefficients": selected_model.coefficients,
        "candidate_models": candidate_models,
        "selection_reasons": selection_reasons,
        "material_correlation_threshold": material_correlation_threshold,
        "calibration_method": "largest_positive_residual",
        "calibration_margin_mcycles": calibration_margin,
        "quote_policy": {"step_mcycles": 1000, "minimum_mcycles": 2000},
        "feature_envelope": feature_envelope,
        "mainnet_holdout": holdout,
        "residual_correlations": residual_trends,
        "accounted_failures": dict(sorted(failures.items())),
        "recommend_shadow_mode": recommendation,
    }
    model_identity = hashlib.sha256(
        json.dumps(analysis, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()
    analysis["model_id"] = f"risc0-zkgas-{model_identity[:16]}"
    return analysis


def render_markdown_report(analysis: Mapping[str, Any]) -> str:
    cohort = analysis["cohort"]
    counts = analysis["sample_counts"]
    holdout = analysis["mainnet_holdout"]
    lines = [
        "# RISC0 zkGas to Cycle Estimation Report",
        "",
        "## Cohort",
        "",
        f"- Model ID: `{analysis['model_id']}`",
        f"- Source revision: `{cohort['source_revision']}`",
        f"- RISC0 image ID: `{cohort['image_id']}`",
        f"- RISC0 version: `{cohort['risc0_version']}`",
        f"- `execution_po2`: `{cohort['execution_po2']}`",
        "- Samples: "
        f"Hoodi fit {counts['hoodi_fit']}, "
        f"Hoodi calibration {counts['hoodi_calibration']}, "
        f"Mainnet holdout {counts['mainnet_holdout']}",
        f"- Determinism repeats: {counts['determinism_repeats']}",
        f"- Determinism mismatches: {counts['determinism_mismatches']}",
        "",
        "### Objective artifact identities",
        "",
        *[
            f"- `{name}`: `{digest}`"
            for name, digest in sorted(cohort["artifact_hashes"].items())
        ],
        "",
        "## Candidate Models",
        "",
        "| Model | Features | R2 | MAE | MAPE | Calibration margin | "
        "Mainnet underquotes | Exact bucket match | p95 overhead |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for name in ("M1", "M2", "M3"):
        candidate = analysis["candidate_models"][name]
        diagnostics = candidate["fit_diagnostics"]
        candidate_holdout = candidate["mainnet_holdout"]
        lines.append(
            f"| {name} | {', '.join(candidate['features'])} | "
            f"{diagnostics['r_squared']:.6f} | {diagnostics['mae']:.6f} | "
            f"{diagnostics['mape']:.6f} | {candidate['calibration_margin_mcycles']:.6f} | "
            f"{candidate_holdout['underquote_count']} | "
            f"{candidate_holdout['exact_bucket_match_rate']:.2%} | "
            f"{candidate_holdout['p95_quote_overhead_mcycles']} |"
        )
    lines.extend(
        [
            "",
            "## Selection and Calibration",
            "",
            f"Selected `{analysis['selected_model']}` before reading Mainnet outcomes.",
            *[f"- {reason}" for reason in analysis["selection_reasons"]],
            f"- Calibration method: `{analysis['calibration_method']}`",
            f"- Calibration margin: `{analysis['calibration_margin_mcycles']:.6f}` mcycles",
            "- Quote rule: `max(2000, ceil_to_multiple(prediction + margin, 1000))`",
            "",
            "## Mainnet Holdout Gates",
            "",
            "| Gate | Result |",
            "| --- | --- |",
        ]
    )
    for gate, passed in holdout["gates"].items():
        lines.append(f"| `{gate}` | {'PASS' if passed else 'FAIL'} |")
    lines.extend(
        [
            "",
            f"- Underquotes: {holdout['underquote_count']}",
            f"- Exact bucket match rate: {holdout['exact_bucket_match_rate']:.2%}",
            f"- p95 quote overhead: {holdout['p95_quote_overhead_mcycles']} mcycles",
            f"- R2: {holdout['r_squared']:.6f}",
            f"- MAE: {holdout['mae']:.6f}",
            f"- MAPE: {holdout['mape']:.6f}",
            "",
            "## Residual Diagnostics",
            "",
            "| Dimension | Pearson correlation | Material |",
            "| --- | ---: | --- |",
        ]
    )
    threshold = analysis["material_correlation_threshold"]
    for feature, correlation in analysis["residual_correlations"].items():
        material = abs(correlation) >= threshold
        lines.append(
            f"| `{feature}` | {correlation:.6f} | {'yes' if material else 'no'} |"
        )
    lines.extend(
        [
            "",
            "### Mainnet residuals",
            "",
            "| Proposal | Actual | Prediction | Safe | Quoted bucket | Actual bucket | Residual |",
            "| ---: | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    for sample in holdout["samples"]:
        lines.append(
            "| {proposal_id} | {actual_mcycles:.6f} | {prediction_mcycles:.6f} | "
            "{safe_mcycles:.6f} | {quoted_mcycles} | {actual_quote_bucket_mcycles} | "
            "{residual_mcycles:.6f} |".format(**sample)
        )
    lines.extend(["", "## Accounted Exclusions and Failures", ""])
    if analysis["accounted_failures"]:
        for name, count in analysis["accounted_failures"].items():
            lines.append(f"- `{name}`: {count}")
    else:
        lines.append("- None")
    lines.extend(["", "## Recommendation", ""])
    if analysis["recommend_shadow_mode"]:
        lines.append(
            "Proceed to the 1,000-proposal shadow stage; do not change production quoting yet."
        )
    else:
        lines.append(
            "Retain local pre-execution. The offline estimator did not satisfy every "
            "safety and efficiency gate."
        )
    lines.append("")
    return "\n".join(lines)


def write_analysis_outputs(
    analysis: Mapping[str, Any], model_path: pathlib.Path, report_path: pathlib.Path
) -> None:
    atomic_write_json(model_path, analysis)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{report_path.name}.", dir=report_path.parent
    )
    temporary_path = pathlib.Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(render_markdown_report(analysis))
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, report_path)
        _fsync_directory(report_path.parent)
    finally:
        if temporary_path.exists():
            temporary_path.unlink()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    collect_parser = subparsers.add_parser("collect", help="collect a finite RISC0 cycle cohort")
    collect_parser.add_argument("--network", choices=sorted(SUPPORTED_NETWORKS), required=True)
    collect_parser.add_argument("--candidate-manifest", type=pathlib.Path, required=True)
    collect_parser.add_argument("--target-count", type=int, required=True)
    collect_parser.add_argument("--max-candidates", type=int, required=True)
    collect_parser.add_argument("--output-dir", type=pathlib.Path, required=True)
    collect_parser.add_argument("--source-revision", required=True)
    collect_parser.add_argument("--image-id", required=True)
    collect_parser.add_argument("--risc0-version", required=True)
    collect_parser.add_argument("--execution-po2", type=int, required=True)
    collect_parser.add_argument("--proposal-elf", type=pathlib.Path, required=True)
    collect_parser.add_argument("--preflight-bin", type=pathlib.Path, required=True)
    collect_parser.add_argument("--guest-launcher-bin", type=pathlib.Path, required=True)
    collect_parser.add_argument("--chain-spec-list", type=pathlib.Path)
    collect_parser.add_argument("--l1-rpc")
    collect_parser.add_argument("--l2-rpc")
    collect_parser.add_argument("--determinism-rate", type=float, default=0.10)
    collect_parser.add_argument("--command-timeout-secs", type=int, default=3600)
    fit_parser = subparsers.add_parser("fit", help="fit and evaluate M1/M2/M3")
    fit_parser.add_argument(
        "--samples",
        type=pathlib.Path,
        action="append",
        required=True,
        help="append-only sample JSONL; repeat for separate Hoodi/Mainnet runs",
    )
    fit_parser.add_argument("--model-out", type=pathlib.Path, required=True)
    fit_parser.add_argument("--report-out", type=pathlib.Path, required=True)
    fit_parser.add_argument("--expected-fit-count", type=int, default=80)
    fit_parser.add_argument("--expected-calibration-count", type=int, default=40)
    fit_parser.add_argument("--expected-holdout-count", type=int, default=60)
    fit_parser.add_argument("--material-correlation-threshold", type=float, default=0.20)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "collect":
        config = CollectorConfig(
            network=args.network,
            candidate_manifest=args.candidate_manifest,
            target_count=args.target_count,
            max_candidates=args.max_candidates,
            output_dir=args.output_dir,
            source_revision=args.source_revision,
            image_id=args.image_id,
            risc0_version=args.risc0_version,
            execution_po2=args.execution_po2,
            proposal_elf=args.proposal_elf,
            preflight_bin=args.preflight_bin,
            guest_launcher_bin=args.guest_launcher_bin,
            chain_spec_list=args.chain_spec_list,
            l1_rpc=args.l1_rpc,
            l2_rpc=args.l2_rpc,
            determinism_rate=args.determinism_rate,
            command_timeout_secs=args.command_timeout_secs,
        )
        return collect(config)
    if args.command == "fit":
        rows = [row for path in args.samples for row in read_jsonl(path)]
        analysis = analyze_experiment(
            rows,
            expected_fit_count=args.expected_fit_count,
            expected_calibration_count=args.expected_calibration_count,
            expected_holdout_count=args.expected_holdout_count,
            material_correlation_threshold=args.material_correlation_threshold,
        )
        write_analysis_outputs(analysis, args.model_out, args.report_out)
        return 0 if analysis["recommend_shadow_mode"] else 4
    raise AssertionError(f"unhandled command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main())
