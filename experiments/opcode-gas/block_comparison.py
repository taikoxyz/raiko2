"""Frozen, source-only inputs for SP1 controlled-block comparison.

This module deliberately stops before SP1 execution.  It binds host-native
fixtures to one sealed estimator and refuses to seal any row with a coverage
gap, so later calibration cannot silently change its workload population.
"""

from __future__ import annotations

import copy
import argparse
import ctypes
import errno
import json
import pathlib
import os
import stat
import subprocess
import tempfile
import shutil
from collections.abc import Mapping, Sequence
from decimal import Decimal, InvalidOperation
from typing import Any

from composite_estimator import estimate_trace, validate_estimator_artifact
from opcode_gas import canonical_json, sha256_bytes
from opcode_gas import REPO_ROOT


SCHEMA_VERSION = 1
PURPOSE = "sp1_controlled_block_comparison_frozen_inputs"
VERSION_IDENTITY = {
    "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
    "taiko_fork": "Unzen",
    "ethereum_upgrade": "Fusaka",
    "revm_spec_id": "OSAKA",
    "proving_backend": "sp1",
    "primary_metric": "proverGas",
}
REQUIRED_CONTEXT_CLASSES = frozenset(
    {
        "address",
        "caller",
        "callvalue:zero",
        "callvalue:nonzero",
        "calldataload:zero",
        "calldataload:partial",
        "calldataload:full",
        "calldatasize",
        "timestamp:post_unzen_nonzero",
    }
)
REQUIRED_CATEGORIES = frozenset(
    {
        "arithmetic",
        "context_heavy",
        "calldata_boundary",
        "memory_expansion",
        "storage",
        "mixed",
        "native_transfer",
        "block_count",
        "transaction_count",
    }
)
_FORBIDDEN_KEYS = frozenset(
    {
        "actual_prover_gas",
        "observed_prover_gas",
        "fitted_scalar",
        "scalar",
        "normalization_identity",
        "comparison_metric",
        "mape",
        "ape",
        "proposal_id",
        "verdict",
        "validation_verdict",
    }
)


def _scenario(kind: str, **parameters: int) -> dict[str, object]:
    return {"kind": "block_comparison", "scenario": {"kind": kind, **parameters}}


def _request(
    partition: str,
    category: str,
    serial: int,
    program: Mapping[str, object],
    *,
    block_count: int = 1,
    transaction_count: int = 1,
) -> dict[str, object]:
    return {
        "partition": partition,
        "category": category,
        "workload_family": category,
        "split": "fit" if partition == "calibration" else "holdout",
        "block_count": block_count,
        "transaction_count": transaction_count,
        "program": copy.deepcopy(dict(program)),
    }


def row_requests() -> list[dict[str, object]]:
    """Return the frozen 12/20 source matrix, without executing any row."""
    rows: list[dict[str, object]] = []
    parameters = {
        "calibration": [
            ("arithmetic", _scenario("arithmetic", repeat_count=4, seed=11)),
            ("context_heavy", _scenario("context_heavy", repeat_count=2, calldata_length=0, call_value=0, timestamp_delta=1)),
            ("context_heavy", _scenario("context_heavy", repeat_count=3, calldata_length=17, call_value=7, timestamp_delta=17)),
            ("calldata_boundary", _scenario("calldata_memory", repeat_count=3, calldata_length=31, calldata_offset=0, memory_word_offset=1)),
            ("memory_expansion", _scenario("calldata_memory", repeat_count=4, calldata_length=64, calldata_offset=32, memory_word_offset=128)),
            ("storage", _scenario("storage_round_trip", repeat_count=2, slot=1, original_value=0, written_value=9)),
            ("mixed", _scenario("mixed", repeat_count=2, calldata_length=32, calldata_offset=0, memory_word_offset=4, slot=2, original_value=6, written_value=0, call_value=3, timestamp_delta=33)),
            ("native_transfer", {"kind": "native_transfer", "value": 1}),
            ("block_count", {"kind": "empty"}),
            ("transaction_count", _scenario("arithmetic", repeat_count=2, seed=19)),
            ("arithmetic", _scenario("arithmetic", repeat_count=9, seed=23)),
            ("mixed", _scenario("mixed", repeat_count=3, calldata_length=15, calldata_offset=1, memory_word_offset=16, slot=3, original_value=7, written_value=12, call_value=0, timestamp_delta=65)),
        ],
        "validation": [
            ("arithmetic", _scenario("arithmetic", repeat_count=5, seed=101)),
            ("arithmetic", _scenario("arithmetic", repeat_count=13, seed=103)),
            ("context_heavy", _scenario("context_heavy", repeat_count=4, calldata_length=0, call_value=11, timestamp_delta=2)),
            ("context_heavy", _scenario("context_heavy", repeat_count=5, calldata_length=7, call_value=0, timestamp_delta=18)),
            ("context_heavy", _scenario("context_heavy", repeat_count=6, calldata_length=32, call_value=13, timestamp_delta=66)),
            ("calldata_boundary", _scenario("calldata_memory", repeat_count=5, calldata_length=1, calldata_offset=1, memory_word_offset=2)),
            ("calldata_boundary", _scenario("calldata_memory", repeat_count=6, calldata_length=33, calldata_offset=17, memory_word_offset=3)),
            ("memory_expansion", _scenario("calldata_memory", repeat_count=7, calldata_length=96, calldata_offset=64, memory_word_offset=129)),
            ("memory_expansion", _scenario("calldata_memory", repeat_count=8, calldata_length=127, calldata_offset=95, memory_word_offset=192)),
            ("storage", _scenario("storage_round_trip", repeat_count=3, slot=4, original_value=0, written_value=29)),
            ("storage", _scenario("storage_round_trip", repeat_count=5, slot=5, original_value=19, written_value=0)),
            ("mixed", _scenario("mixed", repeat_count=4, calldata_length=31, calldata_offset=15, memory_word_offset=32, slot=6, original_value=21, written_value=35, call_value=15, timestamp_delta=97)),
            ("mixed", _scenario("mixed", repeat_count=5, calldata_length=65, calldata_offset=33, memory_word_offset=130, slot=7, original_value=23, written_value=37, call_value=0, timestamp_delta=129)),
            ("native_transfer", {"kind": "native_transfer", "value": 2}),
            ("native_transfer", {"kind": "native_transfer", "value": 3}),
            ("block_count", {"kind": "empty"}),
            ("block_count", {"kind": "empty"}),
            ("transaction_count", _scenario("arithmetic", repeat_count=3, seed=107)),
            ("transaction_count", _scenario("arithmetic", repeat_count=6, seed=109)),
            ("mixed", _scenario("mixed", repeat_count=7, calldata_length=48, calldata_offset=16, memory_word_offset=64, slot=8, original_value=25, written_value=41, call_value=17, timestamp_delta=257)),
        ],
    }
    for partition, specs in parameters.items():
        for serial, (category, program) in enumerate(specs):
            block_count = 1
            transaction_count = 1
            if category == "block_count":
                block_count = (2, 4, 8)[serial % 3]
                transaction_count = 0
            elif category == "transaction_count":
                transaction_count = (2, 3, 5)[serial % 3]
            rows.append(
                _request(
                    partition,
                    category,
                    serial,
                    program,
                    block_count=block_count,
                    transaction_count=transaction_count,
                )
            )
    return rows


def workload_id(request: Mapping[str, object]) -> str:
    return _digest_payload(
        {
            "program": request["program"],
            "block_count": request["block_count"],
            "transaction_count": request["transaction_count"],
        }
    )


def source_spec(request: Mapping[str, object]) -> dict[str, object]:
    """Construct the complete zero-evidence Rust input for one matrix row."""
    return {
        "row_id": "0" * 64,
        "workload_id": "0" * 64,
        "workload_family": request["workload_family"],
        "split": request["split"],
        "block_count": request["block_count"],
        "transaction_count": request["transaction_count"],
        "program": copy.deepcopy(request["program"]),
        "expected_final_state_root": "0x" + "00" * 32,
        "expected_raw_gas_by_key": {},
        "expected_operation_event_count_by_key": {},
        "expected_context_features": {},
        "expected_features": {},
        "expected_diagnostics": {},
    }


def _canonical_repo_file(path: pathlib.Path, relative: str) -> pathlib.Path:
    supplied = pathlib.Path(path)
    expected = REPO_ROOT / relative
    try:
        mode = supplied.stat(follow_symlinks=False).st_mode
        resolved = supplied.resolve(strict=True)
        expected_resolved = expected.resolve(strict=True)
    except OSError as error:
        raise ValueError(f"source path is missing: {relative}") from error
    if supplied.is_symlink() or not stat.S_ISREG(mode) or resolved != expected_resolved:
        raise ValueError(f"source path is not canonical: {relative}")
    return expected_resolved


def _durable_create(path: pathlib.Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o444)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _run_native_freezes(
    launcher_path: pathlib.Path,
) -> list[dict[str, object]]:
    """Return the exact 32 native launcher outputs; this never invokes SP1."""
    launcher = _canonical_repo_file(launcher_path, "target/release/guest-launcher")
    bundles: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory(prefix="block-comparison-prepare-") as temporary:
        root = pathlib.Path(temporary)
        for index, request in enumerate(row_requests()):
            source = root / f"row-{index:02d}.json"
            result = root / f"row-{index:02d}-identity.json"
            source.write_bytes(canonical_json(source_spec(request)) + b"\n")
            completed = subprocess.run(
                [
                    str(launcher),
                    "--stage", "controlled-block-identity",
                    "--proof-type", "native",
                    "--mode", "execute",
                    "--input", str(source),
                    "--json-out", str(result),
                ],
                cwd=REPO_ROOT,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
                timeout=120,
            )
            if completed.returncode != 0:
                detail = completed.stderr.decode(errors="replace")[-2000:]
                raise ValueError(f"native fixture preparation failed at row {index}: {detail}")
            try:
                bundle = json.loads(result.read_bytes())
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError(f"native fixture output is invalid at row {index}") from error
            bundles.append(bundle)
    return bundles


def _run_bound_native_freezes(
    launcher_path: pathlib.Path, launcher_bytes: bytes
) -> list[dict[str, object]]:
    """Run the launcher only while its canonical bytes equal the declared source."""
    canonical_launcher = _canonical_repo_file(
        launcher_path, "target/release/guest-launcher"
    )
    if _read(canonical_launcher) != launcher_bytes:
        raise ValueError("executed launcher bytes differ from supplied source")
    bundles = _run_native_freezes(canonical_launcher)
    if _read(canonical_launcher) != launcher_bytes:
        raise ValueError("executed launcher changed during native replay")
    return bundles


def prepare_bundles(
    *, launcher_path: pathlib.Path, output_path: pathlib.Path
) -> list[dict[str, object]]:
    """Run all 32 native identity freezes and create one prepared bundle file."""
    if output_path.exists() or output_path.is_symlink():
        raise FileExistsError(output_path)
    bundles = _run_native_freezes(launcher_path)
    _durable_create(output_path, canonical_json(bundles) + b"\n")
    return bundles


def covered_context_classes(rows: Sequence[Mapping[str, object]]) -> frozenset[str]:
    covered: set[str] = set()
    for row in rows:
        program = row.get("program")
        if not isinstance(program, Mapping) or program.get("kind") != "block_comparison":
            continue
        scenario = program.get("scenario")
        if not isinstance(scenario, Mapping) or scenario.get("kind") not in {"context_heavy", "mixed"}:
            continue
        covered.update({"address", "caller", "calldatasize", "timestamp:post_unzen_nonzero"})
        covered.add("callvalue:zero" if scenario.get("call_value") == 0 else "callvalue:nonzero")
        length = scenario.get("calldata_length")
        if length == 0:
            covered.add("calldataload:zero")
        elif isinstance(length, int) and length < 32:
            covered.add("calldataload:partial")
        elif isinstance(length, int):
            covered.add("calldataload:full")
    return frozenset(covered)


def _walk_forbidden(value: object, path: str = "manifest") -> None:
    if isinstance(value, Mapping):
        for key, child in value.items():
            if key in _FORBIDDEN_KEYS:
                raise ValueError(f"forbidden pre-data-open field at {path}.{key}")
            _walk_forbidden(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _walk_forbidden(child, f"{path}[{index}]")


def _digest_payload(value: Mapping[str, object]) -> str:
    return sha256_bytes(canonical_json(dict(value)))


def _is_sha256(value: object) -> bool:
    return (
        isinstance(value, str)
        and len(value) == 64
        and all(character in "0123456789abcdef" for character in value)
    )


def _validated_bundle(
    bundle: Mapping[str, Any], request: Mapping[str, object]
) -> tuple[Mapping[str, Any], Mapping[str, Any], Mapping[str, Any]]:
    if set(bundle) != {
        "schema_version",
        "fixture_spec_sha256",
        "spec",
        "observation",
        "candidate_trace",
    } or bundle.get("schema_version") != 1:
        raise ValueError("frozen bundle schema differs")
    spec = bundle.get("spec")
    observation = bundle.get("observation")
    trace = bundle.get("candidate_trace")
    if not all(isinstance(value, Mapping) for value in (spec, observation, trace)):
        raise ValueError("frozen bundle evidence is incomplete")
    required_spec = {
        "row_id", "workload_id", "workload_family", "split", "block_count", "transaction_count", "program",
        "expected_final_state_root", "expected_raw_gas_by_key",
        "expected_operation_event_count_by_key", "expected_context_features", "expected_features",
        "expected_diagnostics", "expected_backend_input_sha256", "expected_host_trace_sha256",
        "expected_finalized_block_zkgas",
    }
    if not required_spec.issubset(spec) or set(spec) - (required_spec | {"expected_storage_features"}):
        raise ValueError("frozen spec schema differs")
    required_observation = {
        "row_id", "workload_family", "split", "backend_input_sha256", "host_trace_sha256",
        "guest_input_bincode_length", "public_output", "actual_final_state_root",
        "actual_raw_gas_by_key", "actual_operation_event_count_by_key", "actual_context_features",
        "actual_features", "actual_diagnostics", "unzen_activation_timestamp",
        "minimum_block_timestamp", "operation_phase_ownership", "system_operation_ownership",
        "anchor_operation_ownership", "finalized_block_zkgas",
    }
    if not required_observation.issubset(observation) or set(observation) - (
        required_observation | {"actual_storage_features"}
    ):
        raise ValueError("frozen observation schema differs")
    if bundle.get("fixture_spec_sha256") != sha256_bytes(canonical_json(spec)):
        raise ValueError("frozen fixture spec digest differs")
    for field in ("workload_family", "split", "block_count", "transaction_count", "program"):
        if spec.get(field) != request[field]:
            raise ValueError(f"frozen spec {field} differs from matrix")
    equalities = (
        ("row_id", "row_id"),
        ("workload_family", "workload_family"),
        ("split", "split"),
        ("expected_final_state_root", "actual_final_state_root"),
        ("expected_raw_gas_by_key", "actual_raw_gas_by_key"),
        ("expected_operation_event_count_by_key", "actual_operation_event_count_by_key"),
        ("expected_context_features", "actual_context_features"),
        ("expected_features", "actual_features"),
        ("expected_diagnostics", "actual_diagnostics"),
        ("expected_backend_input_sha256", "backend_input_sha256"),
        ("expected_host_trace_sha256", "host_trace_sha256"),
        ("expected_finalized_block_zkgas", "finalized_block_zkgas"),
    )
    for expected, actual in equalities:
        if spec.get(expected) != observation.get(actual):
            raise ValueError(f"frozen {expected} evidence differs")
    if spec.get("expected_storage_features", {}) != observation.get("actual_storage_features", {}):
        raise ValueError("frozen storage feature evidence differs")
    row_id = spec.get("row_id")
    row_semantics = dict(spec)
    row_semantics.pop("row_id", None)
    if spec["program"].get("kind") == "block_comparison":
        for field in (
            "workload_id", "expected_final_state_root", "expected_raw_gas_by_key",
            "expected_operation_event_count_by_key", "expected_context_features",
            "expected_storage_features", "expected_features", "expected_diagnostics",
            "expected_backend_input_sha256", "expected_host_trace_sha256",
            "expected_finalized_block_zkgas",
        ):
            row_semantics.pop(field, None)
    else:
        if not row_semantics.get("expected_context_features"):
            row_semantics.pop("expected_context_features", None)
        if not row_semantics.get("expected_storage_features"):
            row_semantics.pop("expected_storage_features", None)
    if row_id != _digest_payload(row_semantics):
        raise ValueError("frozen row identity differs from canonical Rust semantics")
    if spec.get("workload_id") != workload_id(request):
        raise ValueError("frozen workload identity differs")
    backend_id = observation.get("backend_input_sha256")
    host_trace_id = observation.get("host_trace_sha256")
    if not all(_is_sha256(value) for value in (row_id, backend_id, host_trace_id)):
        raise ValueError("frozen identity is not lowercase SHA256")
    finalized_zkgas = observation.get("finalized_block_zkgas")
    if isinstance(finalized_zkgas, bool) or not isinstance(finalized_zkgas, int) or finalized_zkgas <= 0:
        raise ValueError("frozen finalized zkGas must be positive")
    if sha256_bytes(canonical_json(trace)) != host_trace_id:
        raise ValueError("candidate trace does not match frozen host trace")
    if trace.get("guest_input_sha256") not in {backend_id, f"0x{backend_id}"}:
        raise ValueError("candidate trace backend input identity differs")
    return spec, observation, trace


def build_manifest(
    *,
    candidate: Mapping[str, Any],
    candidate_file_bytes: bytes,
    source_bytes: Mapping[str, bytes],
    frozen_bundles: Sequence[Mapping[str, Any]],
    launcher_path: pathlib.Path,
) -> dict[str, object]:
    """Build and validate a manifest from already-frozen native bundles."""
    validate_estimator_artifact(candidate)
    if candidate.get("schema_version") != 5:
        raise ValueError("block comparison requires the schema-5 successor candidate")
    try:
        parsed_candidate = json.loads(candidate_file_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("candidate file is not valid JSON") from error
    if parsed_candidate != candidate:
        raise ValueError("candidate file does not contain the supplied candidate")
    if candidate_file_bytes != (json.dumps(candidate, indent=2, sort_keys=True) + "\n").encode():
        raise ValueError("candidate file must be canonical JSON with one trailing newline")
    if candidate.get("version_identity") != VERSION_IDENTITY:
        raise ValueError("candidate version identity differs")
    required_sources = {"launcher", "elf", "vk", "trace", "fixture", "builder"}
    if set(source_bytes) != required_sources | {"candidate"}:
        raise ValueError("source byte inventory differs")
    if source_bytes["candidate"] != candidate_file_bytes:
        raise ValueError("candidate file bytes differ")
    execution = candidate["source_artifacts"]["execution_artifacts"]
    proposal = execution["sp1_proposal_guest"]
    expected_hashes = {
        "launcher": execution["guest_launcher"]["file_sha256"],
        "elf": proposal["elf_sha256"],
        "vk": proposal["vk_sha256"],
        "trace": candidate["trace_schema"]["source_sha256"],
        "fixture": candidate["source_artifacts"]["source_code_sha256s"][
            "bin/guest-launcher/src/controlled_workload.rs"
        ],
        "builder": candidate["source_artifacts"]["source_code_sha256s"][
            "experiments/opcode-gas/block_comparison.py"
        ],
    }
    for name, expected in expected_hashes.items():
        if sha256_bytes(source_bytes[name]) != expected:
            raise ValueError(f"{name} source hash differs from candidate")

    replay = _run_bound_native_freezes(launcher_path, source_bytes["launcher"])
    if canonical_json(list(replay)) != canonical_json(list(frozen_bundles)):
        raise ValueError("prepared bundles differ from exact native launcher replay")

    requests = row_requests()
    if len(frozen_bundles) != len(requests):
        raise ValueError("frozen bundle count differs from fixed matrix")
    rows: list[dict[str, object]] = []
    backend_ids: set[str] = set()
    row_ids: set[str] = set()
    for request, bundle in zip(requests, frozen_bundles, strict=True):
        if not isinstance(bundle, Mapping):
            raise ValueError("frozen bundle must be an object")
        frozen_row, observation, trace = _validated_bundle(bundle, request)
        row_id = frozen_row.get("row_id")
        backend_id = observation.get("backend_input_sha256")
        if row_id in row_ids:
            raise ValueError("duplicate row identity")
        if backend_id in backend_ids:
            raise ValueError("duplicate backend input identity")
        row_ids.add(row_id)
        backend_ids.add(backend_id)
        estimate = estimate_trace(candidate, trace)
        if estimate.get("coverage_complete") is not True or estimate.get("gaps") != [] or "predicted_prover_gas" not in estimate:
            raise ValueError("every frozen row requires complete candidate coverage")
        rows.append(
            {
                "partition": request["partition"],
                "category": request["category"],
                "candidate_identity": candidate["artifact_sha256"],
                "workload_id": frozen_row["workload_id"],
                "row_id": row_id,
                "backend_input_sha256": backend_id,
                "candidate_coverage_complete": True,
                "candidate_predicted_prover_gas": estimate["predicted_prover_gas"],
                "frozen_bundle": copy.deepcopy(dict(bundle)),
            }
        )
    logical_paths = {
        "launcher": execution["guest_launcher"]["path"],
        "elf": proposal["elf_path"],
        "vk": proposal["vk_path"],
        "trace": candidate["trace_schema"]["source_path"],
        "fixture": "bin/guest-launcher/src/controlled_workload.rs",
        "builder": "experiments/opcode-gas/block_comparison.py",
    }
    sources = {
        name: {"path": logical_paths[name], "sha256": sha256_bytes(source_bytes[name])}
        for name in sorted(required_sources)
    }
    sources["candidate"] = {
        "path": f"experiments/opcode-gas/estimators/{candidate['artifact_sha256'][:24]}/estimator.json",
        "sha256": sha256_bytes(candidate_file_bytes),
        "logical_identity": candidate["artifact_sha256"],
    }
    payload: dict[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "status": "frozen_before_data_open",
        "version_identity": copy.deepcopy(VERSION_IDENTITY),
        "candidate_identity": candidate["artifact_sha256"],
        "sources": sources,
        "matrix_sha256": _digest_payload({"rows": requests}),
        "rows": rows,
    }
    payload["artifact_sha256"] = _digest_payload(payload)
    validate_manifest(payload)
    return payload


def validate_manifest(manifest: Mapping[str, Any]) -> None:
    if not isinstance(manifest, Mapping):
        raise ValueError("manifest must be an object")
    _walk_forbidden(manifest)
    expected = {
        "schema_version", "purpose", "status", "version_identity", "candidate_identity",
        "sources", "matrix_sha256", "rows", "artifact_sha256",
    }
    if set(manifest) != expected:
        raise ValueError("manifest schema differs")
    if manifest.get("schema_version") != SCHEMA_VERSION or manifest.get("purpose") != PURPOSE or manifest.get("status") != "frozen_before_data_open" or manifest.get("version_identity") != VERSION_IDENTITY:
        raise ValueError("manifest header differs")
    payload = {key: value for key, value in manifest.items() if key != "artifact_sha256"}
    if manifest.get("artifact_sha256") != _digest_payload(payload):
        raise ValueError("manifest content address differs")
    if manifest.get("matrix_sha256") != _digest_payload({"rows": row_requests()}):
        raise ValueError("manifest fixed matrix identity differs")
    candidate_identity = manifest.get("candidate_identity")
    if not _is_sha256(candidate_identity) or not _is_sha256(manifest.get("artifact_sha256")) or not _is_sha256(manifest.get("matrix_sha256")):
        raise ValueError("manifest identity is not lowercase SHA256")
    sources = manifest.get("sources")
    if not isinstance(sources, Mapping) or sources.get("candidate", {}).get("logical_identity") != candidate_identity:
        raise ValueError("candidate source identity differs")
    expected_hashes = {"launcher", "elf", "vk", "trace", "fixture", "builder", "candidate"}
    if set(sources) != expected_hashes:
        raise ValueError("source inventory differs")
    expected_paths = {
        "candidate": f"experiments/opcode-gas/estimators/{candidate_identity[:24]}/estimator.json",
        "launcher": "target/release/guest-launcher",
        "elf": "crates/guests/elf/sp1_shasta_proposal.elf",
        "vk": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
        "trace": "crates/zkgas-trace/src/reconstruct.rs",
        "fixture": "bin/guest-launcher/src/controlled_workload.rs",
        "builder": "experiments/opcode-gas/block_comparison.py",
    }
    for name, source in sources.items():
        expected_fields = {"path", "sha256", "logical_identity"} if name == "candidate" else {"path", "sha256"}
        path = source.get("path") if isinstance(source, Mapping) else None
        pure = pathlib.PurePosixPath(path) if isinstance(path, str) else None
        if not isinstance(source, Mapping) or set(source) != expected_fields or not _is_sha256(source.get("sha256")) or pure is None or pure.is_absolute() or ".." in pure.parts or str(pure) != path:
            raise ValueError(f"{name} source hash differs")
        if path != expected_paths[name]:
            raise ValueError(f"{name} source path differs")
    rows = manifest.get("rows")
    if not isinstance(rows, list) or len(rows) != 32:
        raise ValueError("manifest row count differs")
    partitions = [row.get("partition") for row in rows if isinstance(row, Mapping)]
    if partitions.count("calibration") != 12 or partitions.count("validation") != 20:
        raise ValueError("manifest partition counts differ")
    categories = {row.get("category") for row in rows if isinstance(row, Mapping)}
    if not REQUIRED_CATEGORIES <= categories:
        raise ValueError("manifest category coverage differs")
    backend_ids: set[str] = set()
    row_ids: set[str] = set()
    workload_ids: set[str] = set()
    for request, row in zip(row_requests(), rows, strict=True):
        required = {"partition", "category", "candidate_identity", "workload_id", "row_id", "backend_input_sha256", "candidate_coverage_complete", "candidate_predicted_prover_gas", "frozen_bundle"}
        if not isinstance(row, Mapping) or set(row) != required:
            raise ValueError("manifest row schema differs")
        if row["candidate_identity"] != candidate_identity or row["candidate_coverage_complete"] is not True:
            raise ValueError("manifest candidate identity or coverage differs")
        if row["partition"] != request["partition"] or row["category"] != request["category"]:
            raise ValueError("manifest row differs from fixed matrix")
        frozen_row, observation, trace = _validated_bundle(row["frozen_bundle"], request)
        if frozen_row.get("row_id") != row["row_id"]:
            raise ValueError("manifest frozen row identity differs")
        if frozen_row.get("workload_id") != row["workload_id"]:
            raise ValueError("manifest frozen workload identity differs")
        if frozen_row.get("expected_backend_input_sha256") != row["backend_input_sha256"] or observation.get("backend_input_sha256") != row["backend_input_sha256"]:
            raise ValueError("manifest frozen backend input identity differs")
        if trace.get("guest_input_sha256") not in {row["backend_input_sha256"], f"0x{row['backend_input_sha256']}"}:
            raise ValueError("manifest candidate trace backend identity differs")
        prediction = row["candidate_predicted_prover_gas"]
        try:
            parsed_prediction = Decimal(prediction)
        except (InvalidOperation, TypeError) as error:
            raise ValueError("manifest candidate prediction differs") from error
        if not parsed_prediction.is_finite() or parsed_prediction < 0 or str(parsed_prediction) != prediction:
            raise ValueError("manifest candidate prediction differs")
        if row["row_id"] in row_ids:
            raise ValueError("duplicate row identity")
        if row["workload_id"] in workload_ids:
            raise ValueError("duplicate workload identity")
        if row["backend_input_sha256"] in backend_ids:
            raise ValueError("duplicate backend input identity")
        row_ids.add(row["row_id"])
        workload_ids.add(row["workload_id"])
        backend_ids.add(row["backend_input_sha256"])
    if row_ids & workload_ids or row_ids & backend_ids or workload_ids & backend_ids:
        raise ValueError("manifest identity domains overlap")


def _publish_manifest(
    manifest: Mapping[str, Any], output_root: pathlib.Path
) -> pathlib.Path:
    """Durably publish a manifest already checked by the exact replay gate."""
    output_root = pathlib.Path(output_root)
    if output_root.exists() and (output_root.is_symlink() or not output_root.is_dir()):
        raise ValueError("manifest output root differs")
    output_root.mkdir(parents=True, exist_ok=True)
    target = output_root / str(manifest["artifact_sha256"])[:24]
    if target.exists() or target.is_symlink():
        raise FileExistsError(target)
    temporary = pathlib.Path(tempfile.mkdtemp(prefix=f".{target.name}.", dir=output_root))
    try:
        path = temporary / "manifest.json"
        _durable_create(path, canonical_json(dict(manifest)) + b"\n")
        directory = os.open(temporary, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        rename_noreplace = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
        if rename_noreplace is None:
            raise OSError(errno.ENOSYS, "renameat2 is required for create-only sealing")
        rename_noreplace.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        rename_noreplace.restype = ctypes.c_int
        if rename_noreplace(-100, os.fsencode(temporary), -100, os.fsencode(target), 1) != 0:
            number = ctypes.get_errno()
            if number == errno.EEXIST:
                raise FileExistsError(target)
            raise OSError(number, "manifest create-only publication failed")
        temporary = None
        root_descriptor = os.open(output_root, os.O_RDONLY)
        try:
            os.fsync(root_descriptor)
        finally:
            os.close(root_descriptor)
        return target / "manifest.json"
    finally:
        if temporary is not None and temporary.exists():
            shutil.rmtree(temporary)


def _read_bound_sources(
    source_paths: Mapping[str, pathlib.Path], manifest: Mapping[str, Any]
) -> dict[str, bytes]:
    return {
        name: _read(
            _canonical_repo_file(source_paths[name], manifest["sources"][name]["path"])
        )
        for name in source_paths
    }


def _validate_exact_replay(
    manifest: Mapping[str, Any], source_paths: Mapping[str, pathlib.Path]
) -> None:
    """Re-evaluate every publication input before sealing or accepting it."""
    validate_manifest(manifest)
    if set(source_paths) != set(manifest["sources"]):
        raise ValueError("replay source path inventory differs")
    source_bytes = _read_bound_sources(source_paths, manifest)
    if set(source_bytes) != set(manifest["sources"]):
        raise ValueError("replay source inventory differs")
    for name, data in source_bytes.items():
        if sha256_bytes(data) != manifest["sources"][name]["sha256"]:
            raise ValueError(f"replay {name} source hash differs")
    candidate = json.loads(source_bytes["candidate"])
    if source_bytes["candidate"] != (
        json.dumps(candidate, indent=2, sort_keys=True) + "\n"
    ).encode():
        raise ValueError("replay candidate file is not canonical JSON")
    validate_estimator_artifact(candidate)
    if candidate.get("schema_version") != 5:
        raise ValueError("replay candidate is not the schema-5 successor")
    if candidate.get("artifact_sha256") != manifest["candidate_identity"]:
        raise ValueError("replay candidate logical identity differs")
    execution = candidate["source_artifacts"]["execution_artifacts"]
    guest = execution["sp1_proposal_guest"]
    declared = {
        "launcher": execution["guest_launcher"]["file_sha256"],
        "elf": guest["elf_sha256"],
        "vk": guest["vk_sha256"],
        "trace": candidate["trace_schema"]["source_sha256"],
        "fixture": candidate["source_artifacts"]["source_code_sha256s"][
            "bin/guest-launcher/src/controlled_workload.rs"
        ],
        "builder": candidate["source_artifacts"]["source_code_sha256s"][
            "experiments/opcode-gas/block_comparison.py"
        ],
    }
    for name, digest in declared.items():
        if manifest["sources"][name]["sha256"] != digest:
            raise ValueError(f"replay {name} candidate source differs")
    native = _run_bound_native_freezes(
        source_paths["launcher"], source_bytes["launcher"]
    )
    stored = [row["frozen_bundle"] for row in manifest["rows"]]
    if canonical_json(list(native)) != canonical_json(stored):
        raise ValueError("sealed bundles differ from exact native launcher replay")
    for row in manifest["rows"]:
        replay = estimate_trace(candidate, row["frozen_bundle"]["candidate_trace"])
        if replay.get("coverage_complete") is not True or replay.get("gaps") != []:
            raise ValueError("replay candidate coverage differs")
        if replay.get("predicted_prover_gas") != row["candidate_predicted_prover_gas"]:
            raise ValueError("replay candidate prediction differs")


def seal_manifest(
    manifest: Mapping[str, Any],
    output_root: pathlib.Path,
    *,
    source_paths: Mapping[str, pathlib.Path],
) -> pathlib.Path:
    """Exact-replay a manifest, then publish it atomically and create-only."""
    _validate_exact_replay(manifest, source_paths)
    return _publish_manifest(manifest, output_root)


def verify_sealed_manifest(
    path: pathlib.Path,
    *,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, object]:
    try:
        file_mode = path.lstat().st_mode
        parent_mode = path.parent.lstat().st_mode
    except OSError as error:
        raise ValueError("sealed manifest inventory differs") from error
    if not stat.S_ISREG(file_mode) or stat.S_IMODE(file_mode) != 0o444 or not stat.S_ISDIR(parent_mode) or path.is_symlink() or path.parent.is_symlink() or {item.name for item in path.parent.iterdir()} != {"manifest.json"}:
        raise ValueError("sealed manifest inventory differs")
    raw = path.read_bytes()
    manifest = json.loads(raw)
    if raw != canonical_json(manifest) + b"\n":
        raise ValueError("sealed manifest is not canonical JSON")
    if path.parent.name != manifest["artifact_sha256"][:24]:
        raise ValueError("sealed manifest directory identity differs")
    _validate_exact_replay(manifest, source_paths)
    return manifest


def _read(path: pathlib.Path) -> bytes:
    return path.read_bytes()


def _source_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--candidate", type=pathlib.Path, required=True)
    parser.add_argument("--launcher", type=pathlib.Path, required=True)
    parser.add_argument("--elf", type=pathlib.Path, required=True)
    parser.add_argument("--vk", type=pathlib.Path, required=True)
    parser.add_argument("--trace", type=pathlib.Path, required=True)
    parser.add_argument("--fixture", type=pathlib.Path, required=True)
    parser.add_argument("--builder", type=pathlib.Path, default=pathlib.Path(__file__))


def _cli_sources(arguments: argparse.Namespace) -> dict[str, bytes]:
    supplied = _cli_source_paths(arguments)
    candidate_raw = _read(supplied["candidate"])
    candidate = json.loads(candidate_raw)
    expected = {
        "candidate": f"experiments/opcode-gas/estimators/{candidate['artifact_sha256'][:24]}/estimator.json",
        "launcher": "target/release/guest-launcher",
        "elf": "crates/guests/elf/sp1_shasta_proposal.elf",
        "vk": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
        "trace": "crates/zkgas-trace/src/reconstruct.rs",
        "fixture": "bin/guest-launcher/src/controlled_workload.rs",
        "builder": "experiments/opcode-gas/block_comparison.py",
    }
    return {
        name: _read(_canonical_repo_file(path, expected[name]))
        for name, path in supplied.items()
    }


def _cli_source_paths(arguments: argparse.Namespace) -> dict[str, pathlib.Path]:
    return {
        name: getattr(arguments, name)
        for name in ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder")
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)
    matrix = subcommands.add_parser("matrix", help="print the fixed source row matrix")
    matrix.add_argument("--output", type=pathlib.Path)
    prepare = subcommands.add_parser("prepare", help="freeze all rows with the native launcher")
    prepare.add_argument("--launcher", type=pathlib.Path, required=True)
    prepare.add_argument("--output", type=pathlib.Path, required=True)
    seal = subcommands.add_parser("seal", help="seal already-frozen native bundles")
    _source_arguments(seal)
    seal.add_argument("--bundles", type=pathlib.Path, required=True)
    seal.add_argument("--output-root", type=pathlib.Path, required=True)
    verify = subcommands.add_parser("verify", help="verify a sealed manifest and exact sources")
    _source_arguments(verify)
    verify.add_argument("--manifest", type=pathlib.Path, required=True)
    arguments = parser.parse_args(argv)
    if arguments.command == "matrix":
        data = canonical_json({"rows": row_requests()}) + b"\n"
        if arguments.output is None:
            print(data.decode(), end="")
        else:
            arguments.output.write_bytes(data)
        return 0
    if arguments.command == "prepare":
        prepare_bundles(launcher_path=arguments.launcher, output_path=arguments.output)
        return 0
    sources = _cli_sources(arguments)
    if arguments.command == "verify":
        verify_sealed_manifest(
            arguments.manifest,
            source_paths=_cli_source_paths(arguments),
        )
        return 0
    candidate = json.loads(sources["candidate"])
    bundles = json.loads(arguments.bundles.read_bytes())
    manifest = build_manifest(
        candidate=candidate,
        candidate_file_bytes=sources["candidate"],
        source_bytes=sources,
        frozen_bundles=bundles,
        launcher_path=arguments.launcher,
    )
    path = seal_manifest(
        manifest,
        arguments.output_root,
        source_paths=_cli_source_paths(arguments),
    )
    print(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
