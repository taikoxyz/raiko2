"""Post-freeze SP1 calibration runner for the controlled-block experiment."""

from __future__ import annotations

import argparse
import copy
import ctypes
import errno
import json
import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
from collections.abc import Callable, Mapping, Sequence
from decimal import Decimal, InvalidOperation, localcontext
from typing import Any

import block_comparison
import context_approximation
from opcode_gas import REPO_ROOT, canonical_json, sha256_bytes


CALIBRATION_PURPOSE = "sp1_controlled_block_calibration"
NORMALIZATION_PURPOSE = "sp1_unzen_normalization"
ENGINE_CONFIG = {
    "stage": "controlled-block",
    "proof_type": "sp1",
    "mode": "execute",
    "sp1_prover": "local",
    "sp1_execution_engine": "gas-estimator",
    "primary_metric": "prover_gas",
    "row_timeout_seconds": 600,
}
ROW_TIMEOUT_SECONDS = 600
RUNNER_SOURCE_RELATIVE_PATH = "experiments/opcode-gas/block_calibration.py"
IMPLEMENTATION_SOURCE_PATHS = (
    RUNNER_SOURCE_RELATIVE_PATH,
    "experiments/opcode-gas/block_comparison.py",
    "experiments/opcode-gas/context_approximation.py",
    "experiments/opcode-gas/opcode_gas.py",
    "experiments/opcode-gas/composite_estimator.py",
    "experiments/opcode-gas/hierarchical_model.py",
    "experiments/opcode-gas/calibration_model.py",
)
ELIGIBLE_CANDIDATE_IDENTITY = (
    "c3f24358f8a2b702658a6496431e06d589c54fadaeb9dea34b5aa3805db115aa"
)
ELIGIBLE_MANIFEST_IDENTITY = (
    "658fc415d93188e7ee1e058449558c3041b14bae5e23f0f5f7272763a38efc85"
)
ELIGIBLE_MANIFEST_FILE_SHA256 = (
    "e0658953033ad7067de90b6f11c38f3f6e4d4737b0d6245c74c7b3e0a1c93f75"
)
ELIGIBLE_MANIFEST_RELATIVE_PATH = (
    "experiments/opcode-gas/manifests/sp1-block-comparison-v1/"
    "658fc415d93188e7ee1e0584/manifest.json"
)
NORMALIZATION_FORMULA = "exact_median(observed_prover_gas/finalized_block_zkgas)"
ACCEPTANCE_THRESHOLDS = copy.deepcopy(
    context_approximation.BLOCK_VALIDATION_ACCEPTANCE_THRESHOLDS
)
_REPORT_FIELDS = {
    "stage", "mode", "proof_mode", "sp1_execution_engine",
    "sp1_gas_trace_chunk_threshold", "sp1_gas_trace_chunk_slots", "input",
    "guest_input_sha256", "guest_input_bincode_length", "sp1_proposal_elf_sha256",
    "guest_launcher_sha256", "public_values", "wall_time_ms",
    "primary_workload_metric", "workload_metrics", "exit_code", "gas",
    "total_instruction_count", "total_syscall_count", "touched_memory_addresses",
    "risc0_image_id", "risc0_input_bytes", "risc0_user_cycles", "risc0_padded_cycles",
    "risc0_segment_count", "risc0_po2_counts", "cycle_tracker", "invocation_tracker",
    "opcode_counts", "syscall_counts", "memory_snapshots", "controlled_trace",
    "controlled_overhead", "controlled_block", "controlled_state_holdout",
}


def _is_sha256(value: object) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(
        character in "0123456789abcdef" for character in value
    )


def _canonical_decimal(value: Decimal) -> str:
    if not value.is_finite():
        raise ValueError("calibration Decimal must be finite")
    rendered = format(value, "f")
    if "." in rendered:
        rendered = rendered.rstrip("0").rstrip(".")
    return rendered or "0"


def _decimal(value: object, *, positive: bool = False) -> Decimal:
    if not isinstance(value, str):
        raise ValueError("calibration value must be a canonical Decimal string")
    try:
        parsed = Decimal(value)
    except (InvalidOperation, ValueError) as error:
        raise ValueError("calibration value must be a canonical Decimal string") from error
    if not parsed.is_finite() or _canonical_decimal(parsed) != value:
        raise ValueError("calibration value must be a canonical Decimal string")
    if positive and parsed <= 0:
        raise ValueError("calibration value must be positive")
    return parsed


def _content_address(document: Mapping[str, Any]) -> str:
    return sha256_bytes(
        canonical_json({key: value for key, value in document.items() if key != "artifact_sha256"})
    )


def _durable_create(path: pathlib.Path, data: bytes, *, mode: int = 0o444) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    _cleanup_stale_partials(path.parent)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.partial.{os.getpid()}.", dir=path.parent
    )
    temporary = pathlib.Path(temporary_name)
    try:
        os.fchmod(descriptor, mode)
        output = os.fdopen(descriptor, "wb")
        descriptor = -1
        with output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        rename_noreplace = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
        if rename_noreplace is None:
            raise OSError(errno.ENOSYS, "renameat2 is required for atomic publication")
        rename_noreplace.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        rename_noreplace.restype = ctypes.c_int
        if rename_noreplace(
            -100, os.fsencode(temporary), -100, os.fsencode(path), 1
        ) != 0:
            number = ctypes.get_errno()
            if number == errno.EEXIST:
                raise FileExistsError(path)
            raise OSError(number, "atomic file publication failed")
        temporary = None
    finally:
        if descriptor >= 0:
            os.close(descriptor)
        if temporary is not None:
            temporary.unlink(missing_ok=True)
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def _cleanup_stale_partials(directory: pathlib.Path) -> None:
    for path in directory.glob(".*.partial.*.*"):
        marker = path.name.rsplit(".partial.", 1)[-1]
        try:
            process_id = int(marker.split(".", 1)[0])
            os.kill(process_id, 0)
        except ProcessLookupError:
            path.unlink(missing_ok=True)
        except (PermissionError, ValueError):
            continue


def _canonical_file(path: pathlib.Path, *, label: str) -> Mapping[str, Any]:
    try:
        mode = path.lstat().st_mode
        raw = path.read_bytes()
        value = json.loads(raw)
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not canonical JSON") from error
    if not stat.S_ISREG(mode) or path.is_symlink() or raw != canonical_json(value) + b"\n":
        raise ValueError(f"{label} is not canonical JSON")
    if not isinstance(value, Mapping):
        raise ValueError(f"{label} must be an object")
    return value


def _git(*arguments: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *arguments],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _runner_source_path() -> pathlib.Path:
    expected = (REPO_ROOT / RUNNER_SOURCE_RELATIVE_PATH).resolve(strict=True)
    actual = pathlib.Path(__file__).resolve(strict=True)
    if actual != expected or actual.is_symlink() or not actual.is_file():
        raise ValueError("calibration runner is not the canonical implementation")
    return actual


def _implementation_identity_at_revision(
    revision: str,
) -> tuple[str, dict[str, str]]:
    if (
        not isinstance(revision, str)
        or len(revision) != 40
        or any(character not in "0123456789abcdef" for character in revision)
        or _git("cat-file", "-e", f"{revision}^{{commit}}").returncode != 0
    ):
        raise ValueError("calibration implementation revision is not a commit")
    identities: dict[str, str] = {}
    for relative in IMPLEMENTATION_SOURCE_PATHS:
        path = REPO_ROOT / relative
        try:
            mode = path.lstat().st_mode
            current = path.read_bytes()
        except OSError as error:
            raise ValueError("calibration implementation must be clean tracked files") from error
        tracked = _git("ls-files", "--error-unmatch", "--", relative)
        working_diff = _git("diff", "--quiet", "--", relative)
        index_diff = _git("diff", "--cached", "--quiet", "--", relative)
        committed = _git("show", f"{revision}:{relative}")
        if (
            not stat.S_ISREG(mode)
            or path.is_symlink()
            or tracked.returncode != 0
            or working_diff.returncode != 0
            or index_diff.returncode != 0
            or committed.returncode != 0
            or committed.stdout != current
        ):
            raise ValueError("calibration implementation must be clean tracked files")
        identities[relative] = sha256_bytes(current)
    return revision, identities


def _current_implementation_identity() -> tuple[str, dict[str, str]]:
    resolved = _git("rev-parse", "--verify", "HEAD^{commit}")
    revision = resolved.stdout.decode().strip()
    if resolved.returncode != 0:
        raise ValueError("calibration implementation revision differs")
    return _implementation_identity_at_revision(revision)


def _source_snapshot(paths: Mapping[str, pathlib.Path]) -> dict[str, bytes]:
    try:
        return {name: pathlib.Path(path).read_bytes() for name, path in paths.items()}
    except OSError as error:
        raise ValueError("calibration source bytes differ") from error


def _require_source_snapshot(
    paths: Mapping[str, pathlib.Path], expected: Mapping[str, bytes]
) -> None:
    if set(paths) != set(expected) or _source_snapshot(paths) != dict(expected):
        raise ValueError("calibration source changed during execution")


def _calibration_rows(manifest: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    if manifest.get("schema_version") != 2:
        raise ValueError("calibration requires the sealed schema-2 manifest")
    rows = manifest.get("rows")
    if not isinstance(rows, list) or len(rows) != 32:
        raise ValueError("controlled-block manifest row population differs")
    calibration = [
        row
        for row in rows
        if isinstance(row, Mapping) and row.get("partition") == "calibration"
    ]
    validation = [
        row
        for row in rows
        if isinstance(row, Mapping) and row.get("partition") == "validation"
    ]
    if len(calibration) != 12 or len(validation) != 20 or len(calibration) + len(validation) != 32:
        raise ValueError("controlled-block manifest partition population differs")
    row_ids = [row.get("row_id") for row in calibration]
    if len(set(row_ids)) != 12 or not all(_is_sha256(value) for value in row_ids):
        raise ValueError("calibration row identities differ")
    return calibration


def _require_eligible_manifest(
    manifest_path: pathlib.Path, manifest: Mapping[str, Any]
) -> None:
    try:
        supplied_path = pathlib.Path(manifest_path)
        supplied_mode = supplied_path.lstat().st_mode
        canonical_path = supplied_path.resolve(strict=True)
        expected_path = (REPO_ROOT / ELIGIBLE_MANIFEST_RELATIVE_PATH).resolve(
            strict=True
        )
        file_sha256 = sha256_bytes(canonical_path.read_bytes())
    except OSError as error:
        raise ValueError("calibration requires the eligible frozen manifest") from error
    if (
        canonical_path != expected_path
        or supplied_path.is_symlink()
        or not stat.S_ISREG(supplied_mode)
        or manifest.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or manifest.get("artifact_sha256") != ELIGIBLE_MANIFEST_IDENTITY
        or file_sha256 != ELIGIBLE_MANIFEST_FILE_SHA256
    ):
        raise ValueError("calibration requires the eligible frozen manifest")


def _preflight_eligible_manifest(
    manifest_path: pathlib.Path,
) -> Mapping[str, Any]:
    manifest = _canonical_file(
        pathlib.Path(manifest_path), label="eligible frozen manifest"
    )
    _require_eligible_manifest(manifest_path, manifest)
    return manifest


def _verify_eligible_manifest(
    manifest_path: pathlib.Path, source_paths: Mapping[str, pathlib.Path]
) -> Mapping[str, Any]:
    preflight = _preflight_eligible_manifest(manifest_path)
    verified = block_comparison.verify_sealed_manifest(
        manifest_path, source_paths=source_paths
    )
    if canonical_json(verified) != canonical_json(preflight):
        raise ValueError("eligible manifest changed during exact replay")
    return verified


def validate_report(
    report: Mapping[str, Any], row: Mapping[str, Any], manifest: Mapping[str, Any]
) -> dict[str, Any]:
    if not isinstance(report, Mapping) or set(report) != _REPORT_FIELDS:
        raise ValueError("controlled-block report schema differs")
    if row.get("partition") != "calibration" or row.get(
        "candidate_identity"
    ) != manifest.get("candidate_identity"):
        raise ValueError("controlled-block report is not a calibration row")
    if (
        report.get("stage") != "controlled-block"
        or report.get("mode") != "execute"
        or report.get("proof_mode") != "compressed"
        or report.get("sp1_execution_engine") != "gas-estimator"
        or report.get("sp1_gas_trace_chunk_threshold") != 134_217_728
        or report.get("sp1_gas_trace_chunk_slots") != 2
    ):
        raise ValueError("controlled-block report execution context differs")
    gas = report.get("gas")
    if isinstance(gas, bool) or not isinstance(gas, int) or gas <= 0:
        raise ValueError("controlled-block report prover gas differs")
    if report.get("primary_workload_metric") != {"label": "prover_gas", "count": gas}:
        raise ValueError("controlled-block report primary metric differs")
    metrics = report.get("workload_metrics")
    if not isinstance(metrics, list) or not all(isinstance(item, Mapping) for item in metrics):
        raise ValueError("controlled-block report workload metrics differ")
    labels = [item.get("label") for item in metrics]
    instruction_count = report.get("total_instruction_count")
    syscall_count = report.get("total_syscall_count")
    touched_memory = report.get("touched_memory_addresses")
    expected_metrics = [
        {"label": "prover_gas", "count": gas},
        {"label": "sp1_total_instruction_count", "count": instruction_count},
        {"label": "sp1_total_syscall_count", "count": syscall_count},
        {"label": "sp1_touched_memory_addresses", "count": touched_memory},
    ]
    if labels != [item["label"] for item in expected_metrics] or metrics != expected_metrics:
        raise ValueError("controlled-block report workload metrics differ")
    if (
        isinstance(instruction_count, bool)
        or not isinstance(instruction_count, int)
        or instruction_count <= 0
        or isinstance(syscall_count, bool)
        or not isinstance(syscall_count, int)
        or syscall_count < 0
        or isinstance(touched_memory, bool)
        or not isinstance(touched_memory, int)
        or touched_memory < 0
    ):
        raise ValueError("controlled-block instruction diagnostics differ")
    if report.get("exit_code") != 0:
        raise ValueError("controlled-block report guest execution failed")
    identity = row.get("frozen_identity")
    if not isinstance(identity, Mapping) or not isinstance(identity.get("observation"), Mapping):
        raise ValueError("controlled-block manifest identity differs")
    observation = identity["observation"]
    controlled = report.get("controlled_block")
    if not isinstance(controlled, Mapping) or set(controlled) != {
        "status",
        "row_id",
        "reasons",
        "observation",
    }:
        raise ValueError("controlled-block report result differs")
    if (
        controlled.get("status") != "accepted"
        or controlled.get("reasons") != []
        or controlled.get("row_id") != row.get("row_id")
        or controlled.get("observation") != observation
    ):
        raise ValueError("controlled-block report result differs")
    if report.get("input") != row.get("row_id"):
        raise ValueError("controlled-block report input differs")
    if report.get("guest_input_sha256") != f"0x{row.get('backend_input_sha256')}":
        raise ValueError("controlled-block report guest input differs")
    if report.get("guest_input_bincode_length") != observation.get("guest_input_bincode_length"):
        raise ValueError("controlled-block report guest input length differs")
    if report.get("public_values") != str(observation.get("public_output")).lower():
        raise ValueError("controlled-block report public output differs")
    sources = manifest.get("sources")
    if not isinstance(sources, Mapping):
        raise ValueError("controlled-block manifest sources differ")
    if report.get("sp1_proposal_elf_sha256") != sources.get("elf", {}).get("sha256"):
        raise ValueError("controlled-block report ELF differs")
    if report.get("guest_launcher_sha256") != sources.get("launcher", {}).get("sha256"):
        raise ValueError("controlled-block report launcher differs")
    if (
        report.get("controlled_trace") is not None
        or report.get("controlled_overhead") is not None
        or report.get("controlled_state_holdout") is not None
    ):
        raise ValueError("controlled-block report contains another workload")
    return copy.deepcopy(dict(report))


def _run_identity(
    manifest: Mapping[str, Any],
    manifest_bytes: bytes,
    runner_bytes: bytes,
    revision: str,
    implementation_sources: Mapping[str, str],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "purpose": CALIBRATION_PURPOSE,
        "source_revision": revision,
        "candidate_identity": manifest["candidate_identity"],
        "manifest_identity": manifest["artifact_sha256"],
        "manifest_file_sha256": sha256_bytes(manifest_bytes),
        "runner_source_sha256": sha256_bytes(runner_bytes),
        "implementation_sources": dict(implementation_sources),
        "manifest_sources": copy.deepcopy(manifest["sources"]),
        "engine_config": copy.deepcopy(ENGINE_CONFIG),
        "calibration_row_ids": [row["row_id"] for row in _calibration_rows(manifest)],
    }


def run_calibration(
    *,
    manifest_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    launcher_path: pathlib.Path,
    run_dir: pathlib.Path,
    command_runner: Callable[..., subprocess.CompletedProcess[Any]] = subprocess.run,
) -> list[dict[str, Any]]:
    run_dir = pathlib.Path(run_dir)
    if run_dir.exists() and (run_dir.is_symlink() or not run_dir.is_dir()):
        raise ValueError("calibration run root differs")
    try:
        if pathlib.Path(launcher_path).resolve(strict=True) != pathlib.Path(
            source_paths["launcher"]
        ).resolve(strict=True):
            raise ValueError("calibration launcher path differs")
    except (KeyError, OSError) as error:
        raise ValueError("calibration launcher path differs") from error
    manifest = _verify_eligible_manifest(manifest_path, source_paths)
    calibration = _calibration_rows(manifest)
    manifest_bytes = pathlib.Path(manifest_path).read_bytes()
    runner_source_path = _runner_source_path()
    runner_bytes = pathlib.Path(runner_source_path).read_bytes()
    revision, implementation_sources = _current_implementation_identity()
    identity = _run_identity(
        manifest,
        manifest_bytes,
        runner_bytes,
        revision,
        implementation_sources,
    )
    watched_sources = {
        "manifest": pathlib.Path(manifest_path),
        "runner": pathlib.Path(runner_source_path),
        **{f"manifest:{name}": pathlib.Path(path) for name, path in source_paths.items()},
        **{
            f"implementation:{path}": REPO_ROOT / path
            for path in IMPLEMENTATION_SOURCE_PATHS
        },
    }
    source_snapshot = _source_snapshot(watched_sources)
    run_dir.mkdir(parents=True, exist_ok=True)
    _cleanup_stale_partials(run_dir)
    allowed = {"identity.json", "inputs", "rows"}
    if {path.name for path in run_dir.iterdir()} - allowed:
        raise ValueError("calibration run inventory differs")
    identity_path = run_dir / "identity.json"
    identity_bytes = canonical_json(identity) + b"\n"
    if identity_path.exists():
        if identity_path.is_symlink() or identity_path.read_bytes() != identity_bytes:
            raise ValueError("calibration run identity differs")
    else:
        _durable_create(identity_path, identity_bytes)
    inputs = run_dir / "inputs"
    rows_dir = run_dir / "rows"
    for directory in (inputs, rows_dir):
        if directory.exists() and (
            directory.is_symlink() or not directory.is_dir()
        ):
            raise ValueError("calibration row inventory differs")
        directory.mkdir(exist_ok=True)
    expected_names = {f"{row['row_id']}.json" for row in calibration}
    for directory in (inputs, rows_dir):
        _cleanup_stale_partials(directory)
        if {path.name for path in directory.iterdir()} - expected_names:
            raise ValueError("calibration row inventory differs")
    launcher_bytes = pathlib.Path(launcher_path).read_bytes()
    if sha256_bytes(launcher_bytes) != manifest["sources"]["launcher"]["sha256"]:
        raise ValueError("calibration launcher bytes differ")
    reports: list[dict[str, Any]] = []
    for row in calibration:
        row_id = row["row_id"]
        spec = row["frozen_identity"]["spec"]
        input_path = inputs / f"{row_id}.json"
        input_bytes = canonical_json(spec) + b"\n"
        if input_path.exists():
            if input_path.is_symlink() or input_path.read_bytes() != input_bytes:
                raise ValueError("calibration input differs")
        else:
            _durable_create(input_path, input_bytes)
        row_path = rows_dir / f"{row_id}.json"
        if row_path.exists():
            stored = _canonical_file(row_path, label="calibration row")
            reports.append(validate_report(stored, row, manifest))
            continue
        with tempfile.TemporaryDirectory(prefix="block-calibration-row-") as temporary:
            output_path = pathlib.Path(temporary) / "report.jsonl"
            command = [
                str(launcher_path), "--stage", "controlled-block", "--proof-type", "sp1",
                "--mode", "execute", "--sp1-prover", "local",
                "--sp1-execution-engine", "gas-estimator", "--input", str(input_path),
                "--jsonl-out", str(output_path),
            ]
            try:
                completed = command_runner(
                    command,
                    cwd=REPO_ROOT,
                    stdin=subprocess.DEVNULL,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                    timeout=ROW_TIMEOUT_SECONDS,
                )
            except subprocess.TimeoutExpired as error:
                raise ValueError("controlled-block launcher timed out") from error
            if completed.returncode != 0:
                raise ValueError("controlled-block launcher failed")
            try:
                lines = output_path.read_text().splitlines()
                if len(lines) != 1:
                    raise ValueError("controlled-block launcher emitted duplicate or missing rows")
                report = json.loads(lines[0])
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError("controlled-block launcher report differs") from error
            validated = validate_report(report, row, manifest)
            if pathlib.Path(launcher_path).read_bytes() != launcher_bytes:
                raise ValueError("calibration launcher changed during execution")
            _require_source_snapshot(watched_sources, source_snapshot)
            _durable_create(row_path, canonical_json(validated) + b"\n")
            reports.append(validated)
    if pathlib.Path(launcher_path).read_bytes() != launcher_bytes:
        raise ValueError("calibration launcher changed during execution")
    _require_source_snapshot(watched_sources, source_snapshot)
    return reports


def build_calibration_artifact(
    *, manifest: Mapping[str, Any], manifest_file_bytes: bytes,
    reports: Sequence[Mapping[str, Any]], runner_source_bytes: bytes,
    source_revision: str, implementation_sources: Mapping[str, str],
) -> dict[str, Any]:
    calibration = _calibration_rows(manifest)
    try:
        manifest_file_value = json.loads(manifest_file_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("calibration manifest file differs") from error
    if (
        manifest_file_bytes != canonical_json(manifest_file_value) + b"\n"
        or manifest_file_value != manifest
        or len(reports) != 12
        or len(source_revision) != 40
        or any(character not in "0123456789abcdef" for character in source_revision)
    ):
        raise ValueError("calibration artifact inputs differ")
    rows = []
    for source_row, report in zip(calibration, reports, strict=True):
        validated = validate_report(report, source_row, manifest)
        gas = validated["gas"]
        zkgas = source_row["frozen_identity"]["observation"]["finalized_block_zkgas"]
        rows.append(
            {
                "row_id": source_row["row_id"],
                "partition": "calibration",
                "candidate_identity_sha256": manifest["candidate_identity"],
                "observed_prover_gas": str(gas),
                "finalized_block_zkgas": str(zkgas),
                "report_sha256": sha256_bytes(canonical_json(validated)),
                "report": validated,
            }
        )
    artifact: dict[str, Any] = {
        "schema_version": 1,
        "purpose": CALIBRATION_PURPOSE,
        "status": "sealed_calibration",
        "source_revision": source_revision,
        "candidate_identity": manifest["candidate_identity"],
        "manifest_identity": manifest["artifact_sha256"],
        "manifest_file_sha256": sha256_bytes(manifest_file_bytes),
        "manifest_sources": copy.deepcopy(manifest["sources"]),
        "runner_source_sha256": sha256_bytes(runner_source_bytes),
        "implementation_sources": dict(implementation_sources),
        "engine_config": copy.deepcopy(ENGINE_CONFIG),
        "row_digest": sha256_bytes(canonical_json(rows)),
        "rows": rows,
    }
    artifact["artifact_sha256"] = _content_address(artifact)
    validate_calibration_artifact(artifact)
    return artifact


def _load_completed_run(
    *, run_dir: pathlib.Path, manifest: Mapping[str, Any], expected_identity: Mapping[str, Any]
) -> list[dict[str, Any]]:
    run_dir = pathlib.Path(run_dir)
    if run_dir.is_symlink() or {path.name for path in run_dir.iterdir()} != {
        "identity.json", "inputs", "rows"
    }:
        raise ValueError("completed calibration run inventory differs")
    identity = _canonical_file(run_dir / "identity.json", label="calibration run identity")
    if canonical_json(identity) != canonical_json(expected_identity):
        raise ValueError("completed calibration run identity differs")
    calibration = _calibration_rows(manifest)
    expected_names = {f"{row['row_id']}.json" for row in calibration}
    inputs = run_dir / "inputs"
    rows_dir = run_dir / "rows"
    if (
        inputs.is_symlink()
        or rows_dir.is_symlink()
        or {path.name for path in inputs.iterdir()} != expected_names
        or {path.name for path in rows_dir.iterdir()} != expected_names
    ):
        raise ValueError("completed calibration row inventory differs")
    reports = []
    for row in calibration:
        name = f"{row['row_id']}.json"
        expected_input = canonical_json(row["frozen_identity"]["spec"]) + b"\n"
        input_path = inputs / name
        if input_path.is_symlink() or input_path.read_bytes() != expected_input:
            raise ValueError("completed calibration input differs")
        report = _canonical_file(rows_dir / name, label="calibration row")
        reports.append(validate_report(report, row, manifest))
    return reports


def seal_calibration_artifact(
    *,
    manifest_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    run_dir: pathlib.Path,
    output_root: pathlib.Path,
) -> pathlib.Path:
    manifest = _verify_eligible_manifest(manifest_path, source_paths)
    runner_source_path = _runner_source_path()
    manifest_bytes = pathlib.Path(manifest_path).read_bytes()
    runner_bytes = pathlib.Path(runner_source_path).read_bytes()
    stored_identity = _canonical_file(
        pathlib.Path(run_dir) / "identity.json", label="calibration run identity"
    )
    revision = stored_identity.get("source_revision")
    if not isinstance(revision, str):
        raise ValueError("completed calibration source revision differs")
    revision, implementation_sources = _implementation_identity_at_revision(revision)
    expected_identity = _run_identity(
        manifest,
        manifest_bytes,
        runner_bytes,
        revision,
        implementation_sources,
    )
    reports = _load_completed_run(
        run_dir=run_dir, manifest=manifest, expected_identity=expected_identity
    )
    artifact = build_calibration_artifact(
        manifest=manifest,
        manifest_file_bytes=manifest_bytes,
        reports=reports,
        runner_source_bytes=runner_bytes,
        source_revision=revision,
        implementation_sources=implementation_sources,
    )
    return _publish_document(
        artifact,
        output_root,
        filename="calibration.json",
        validator=validate_calibration_artifact,
    )


def validate_calibration_artifact(artifact: Mapping[str, Any]) -> None:
    expected = {
        "schema_version", "purpose", "status", "source_revision", "candidate_identity",
        "manifest_identity", "manifest_file_sha256", "manifest_sources",
        "runner_source_sha256", "implementation_sources", "engine_config",
        "row_digest", "rows", "artifact_sha256",
    }
    if not isinstance(artifact, Mapping) or set(artifact) != expected:
        raise ValueError("calibration artifact schema differs")
    revision = artifact.get("source_revision")
    if (
        artifact.get("schema_version") != 1
        or artifact.get("purpose") != CALIBRATION_PURPOSE
        or artifact.get("status") != "sealed_calibration"
        or artifact.get("engine_config") != ENGINE_CONFIG
        or not isinstance(revision, str)
        or len(revision) != 40
        or any(character not in "0123456789abcdef" for character in revision)
    ):
        raise ValueError("calibration artifact header differs")
    for field in (
        "candidate_identity",
        "manifest_identity",
        "manifest_file_sha256",
        "runner_source_sha256",
        "row_digest",
        "artifact_sha256",
    ):
        if not _is_sha256(artifact.get(field)):
            raise ValueError("calibration artifact identity differs")
    implementation_sources = artifact.get("implementation_sources")
    if (
        not isinstance(implementation_sources, Mapping)
        or set(implementation_sources) != set(IMPLEMENTATION_SOURCE_PATHS)
        or not all(_is_sha256(value) for value in implementation_sources.values())
        or artifact["runner_source_sha256"]
        != implementation_sources[RUNNER_SOURCE_RELATIVE_PATH]
    ):
        raise ValueError("calibration implementation identity differs")
    if artifact["artifact_sha256"] != _content_address(artifact):
        raise ValueError("calibration artifact content address differs")
    rows = artifact.get("rows")
    if (
        not isinstance(rows, list)
        or len(rows) != 12
        or artifact["row_digest"] != sha256_bytes(canonical_json(rows))
    ):
        raise ValueError("calibration artifact rows differ")
    seen = set()
    for row in rows:
        fields = {
            "row_id",
            "partition",
            "candidate_identity_sha256",
            "observed_prover_gas",
            "finalized_block_zkgas",
            "report_sha256",
            "report",
        }
        if (
            not isinstance(row, Mapping)
            or set(row) != fields
            or row.get("partition") != "calibration"
            or row.get("candidate_identity_sha256") != artifact["candidate_identity"]
        ):
            raise ValueError("calibration artifact row differs")
        if row.get("row_id") in seen or not _is_sha256(row.get("row_id")):
            raise ValueError("calibration artifact row identity differs")
        seen.add(row["row_id"])
        _decimal(row.get("observed_prover_gas"), positive=True)
        _decimal(row.get("finalized_block_zkgas"), positive=True)
        if row.get("report_sha256") != sha256_bytes(canonical_json(row.get("report"))):
            raise ValueError("calibration report digest differs")


def verify_calibration_artifact(
    path: pathlib.Path,
    *,
    manifest_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, Any]:
    manifest = _verify_eligible_manifest(manifest_path, source_paths)
    artifact = _verify_document(
        path,
        filename="calibration.json",
        validator=validate_calibration_artifact,
    )
    runner_source_path = _runner_source_path()
    revision, implementation_sources = _implementation_identity_at_revision(
        artifact["source_revision"]
    )
    rebuilt = build_calibration_artifact(
        manifest=manifest,
        manifest_file_bytes=pathlib.Path(manifest_path).read_bytes(),
        reports=[row["report"] for row in artifact["rows"]],
        runner_source_bytes=pathlib.Path(runner_source_path).read_bytes(),
        source_revision=revision,
        implementation_sources=implementation_sources,
    )
    if canonical_json(rebuilt) != canonical_json(artifact):
        raise ValueError("sealed calibration differs from exact replay")
    return artifact


def build_normalization_artifact(
    *, calibration: Mapping[str, Any], calibration_file_bytes: bytes
) -> dict[str, Any]:
    validate_calibration_artifact(calibration)
    if calibration_file_bytes != canonical_json(calibration) + b"\n":
        raise ValueError("normalization calibration file differs")
    scalar_rows = [
        {
            "row_id": row["row_id"],
            "partition": "calibration",
            "candidate_identity_sha256": calibration["candidate_identity"],
            "observed_prover_gas": row["observed_prover_gas"],
            "finalized_block_zkgas": row["finalized_block_zkgas"],
        }
        for row in calibration["rows"]
    ]
    with localcontext() as context:
        context.prec = 100
        kappa = context_approximation.fit_unzen_scalar(scalar_rows)
    artifact: dict[str, Any] = {
        "schema_version": 1,
        "purpose": NORMALIZATION_PURPOSE,
        "status": "sealed_before_validation",
        "candidate_identity": calibration["candidate_identity"],
        "manifest_identity": calibration["manifest_identity"],
        "calibration_identity": calibration["artifact_sha256"],
        "calibration_file_sha256": sha256_bytes(calibration_file_bytes),
        "row_digest": calibration["row_digest"],
        "formula": NORMALIZATION_FORMULA,
        "kappa_unzen": _canonical_decimal(kappa),
        "acceptance_thresholds": copy.deepcopy(ACCEPTANCE_THRESHOLDS),
        "calibration_rows": scalar_rows,
    }
    artifact["artifact_sha256"] = _content_address(artifact)
    validate_normalization_artifact(artifact)
    return artifact


def validate_normalization_artifact(artifact: Mapping[str, Any]) -> None:
    expected = {
        "schema_version", "purpose", "status", "candidate_identity", "manifest_identity",
        "calibration_identity", "calibration_file_sha256", "row_digest", "formula",
        "kappa_unzen", "acceptance_thresholds", "calibration_rows", "artifact_sha256",
    }
    if not isinstance(artifact, Mapping) or set(artifact) != expected:
        raise ValueError("normalization artifact schema differs")
    if (
        artifact.get("schema_version") != 1
        or artifact.get("purpose") != NORMALIZATION_PURPOSE
        or artifact.get("status") != "sealed_before_validation"
        or artifact.get("formula") != NORMALIZATION_FORMULA
        or artifact.get("acceptance_thresholds") != ACCEPTANCE_THRESHOLDS
    ):
        raise ValueError("normalization artifact header differs")
    for field in (
        "candidate_identity",
        "manifest_identity",
        "calibration_identity",
        "calibration_file_sha256",
        "row_digest",
        "artifact_sha256",
    ):
        if not _is_sha256(artifact.get(field)):
            raise ValueError("normalization artifact identity differs")
    if artifact["artifact_sha256"] != _content_address(artifact):
        raise ValueError("normalization artifact content address differs")
    rows = artifact.get("calibration_rows")
    if not isinstance(rows, list) or len(rows) != 12:
        raise ValueError("normalization rows differ")
    with localcontext() as context:
        context.prec = 100
        expected_kappa = context_approximation.fit_unzen_scalar(rows)
    if any(row.get("candidate_identity_sha256") != artifact["candidate_identity"] for row in rows):
        raise ValueError("normalization candidate identity differs")
    if artifact.get("kappa_unzen") != _canonical_decimal(expected_kappa):
        raise ValueError("normalization scalar differs")


def verify_normalization_artifact(
    path: pathlib.Path,
    *,
    calibration_path: pathlib.Path,
    manifest_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, Any]:
    artifact = _verify_document(
        path,
        filename="normalization.json",
        validator=validate_normalization_artifact,
    )
    calibration = verify_calibration_artifact(
        calibration_path,
        manifest_path=manifest_path,
        source_paths=source_paths,
    )
    rebuilt = build_normalization_artifact(
        calibration=calibration,
        calibration_file_bytes=pathlib.Path(calibration_path).read_bytes(),
    )
    if canonical_json(rebuilt) != canonical_json(artifact):
        raise ValueError("sealed normalization differs from exact replay")
    return artifact


def seal_normalization_artifact(
    *,
    calibration_path: pathlib.Path,
    manifest_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    output_root: pathlib.Path,
) -> pathlib.Path:
    calibration = verify_calibration_artifact(
        calibration_path,
        manifest_path=manifest_path,
        source_paths=source_paths,
    )
    artifact = build_normalization_artifact(
        calibration=calibration,
        calibration_file_bytes=pathlib.Path(calibration_path).read_bytes(),
    )
    return _publish_document(
        artifact,
        output_root,
        filename="normalization.json",
        validator=validate_normalization_artifact,
    )


def _publish_document(
    document: Mapping[str, Any], output_root: pathlib.Path, *, filename: str,
    validator: Callable[[Mapping[str, Any]], None],
) -> pathlib.Path:
    validator(document)
    output_root = pathlib.Path(output_root)
    if output_root.exists() and (output_root.is_symlink() or not output_root.is_dir()):
        raise ValueError("artifact output root differs")
    output_root.mkdir(parents=True, exist_ok=True)
    target = output_root / str(document["artifact_sha256"])[:24]
    if target.exists() or target.is_symlink():
        raise FileExistsError(target)
    temporary = pathlib.Path(tempfile.mkdtemp(prefix=f".{target.name}.", dir=output_root))
    try:
        _durable_create(temporary / filename, canonical_json(document) + b"\n")
        descriptor = os.open(temporary, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        rename_noreplace = getattr(ctypes.CDLL(None, use_errno=True), "renameat2", None)
        if rename_noreplace is None:
            raise OSError(errno.ENOSYS, "renameat2 is required for create-only sealing")
        rename_noreplace.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        rename_noreplace.restype = ctypes.c_int
        if rename_noreplace(-100, os.fsencode(temporary), -100, os.fsencode(target), 1) != 0:
            number = ctypes.get_errno()
            if number == errno.EEXIST:
                raise FileExistsError(target)
            raise OSError(number, "artifact create-only publication failed")
        temporary = None
        root_descriptor = os.open(output_root, os.O_RDONLY)
        try:
            os.fsync(root_descriptor)
        finally:
            os.close(root_descriptor)
        return target / filename
    finally:
        if temporary is not None and temporary.exists():
            shutil.rmtree(temporary)


def _verify_document(
    path: pathlib.Path, *, filename: str,
    validator: Callable[[Mapping[str, Any]], None],
) -> dict[str, Any]:
    path = pathlib.Path(path)
    try:
        mode = path.lstat().st_mode
        parent_mode = path.parent.lstat().st_mode
    except OSError as error:
        raise ValueError("sealed artifact inventory differs") from error
    if (
        path.name != filename or path.is_symlink() or path.parent.is_symlink()
        or not stat.S_ISREG(mode) or stat.S_IMODE(mode) not in {0o444, 0o644}
        or not stat.S_ISDIR(parent_mode)
        or {item.name for item in path.parent.iterdir()} != {filename}
    ):
        raise ValueError("sealed artifact inventory differs")
    raw = path.read_bytes()
    value = json.loads(raw)
    if raw != canonical_json(value) + b"\n" or path.parent.name != value.get(
        "artifact_sha256", ""
    )[:24]:
        raise ValueError("sealed artifact bytes differ")
    validator(value)
    return dict(value)


def _source_arguments(parser: argparse.ArgumentParser) -> None:
    for name in ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder"):
        parser.add_argument(f"--{name}", type=pathlib.Path, required=True)


def _source_paths(arguments: argparse.Namespace) -> dict[str, pathlib.Path]:
    return {
        name: getattr(arguments, name)
        for name in ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder")
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    run.add_argument("--manifest", type=pathlib.Path, required=True)
    run.add_argument("--run-dir", type=pathlib.Path, required=True)
    _source_arguments(run)
    seal_calibration = commands.add_parser(
        "seal-calibration", help="seal a completed 12-row calibration run"
    )
    seal_calibration.add_argument("--manifest", type=pathlib.Path, required=True)
    seal_calibration.add_argument("--run-dir", type=pathlib.Path, required=True)
    seal_calibration.add_argument("--output-root", type=pathlib.Path, required=True)
    _source_arguments(seal_calibration)
    verify_calibration = commands.add_parser(
        "verify-calibration", help="exactly replay a sealed calibration artifact"
    )
    verify_calibration.add_argument("--artifact", type=pathlib.Path, required=True)
    verify_calibration.add_argument("--manifest", type=pathlib.Path, required=True)
    _source_arguments(verify_calibration)
    seal_normalization = commands.add_parser(
        "seal-normalization", help="seal normalization before validation opens"
    )
    seal_normalization.add_argument("--calibration", type=pathlib.Path, required=True)
    seal_normalization.add_argument("--manifest", type=pathlib.Path, required=True)
    seal_normalization.add_argument("--output-root", type=pathlib.Path, required=True)
    _source_arguments(seal_normalization)
    verify_normalization = commands.add_parser(
        "verify-normalization", help="exactly replay a sealed normalization artifact"
    )
    verify_normalization.add_argument("--artifact", type=pathlib.Path, required=True)
    verify_normalization.add_argument("--calibration", type=pathlib.Path, required=True)
    verify_normalization.add_argument("--manifest", type=pathlib.Path, required=True)
    _source_arguments(verify_normalization)
    arguments = parser.parse_args(argv)
    sources = _source_paths(arguments)
    if arguments.command == "run":
        run_calibration(
            manifest_path=arguments.manifest,
            source_paths=sources,
            launcher_path=arguments.launcher,
            run_dir=arguments.run_dir,
        )
        return 0
    if arguments.command == "seal-calibration":
        path = seal_calibration_artifact(
            manifest_path=arguments.manifest,
            source_paths=sources,
            run_dir=arguments.run_dir,
            output_root=arguments.output_root,
        )
        print(path)
        return 0
    if arguments.command == "verify-calibration":
        verify_calibration_artifact(
            arguments.artifact,
            manifest_path=arguments.manifest,
            source_paths=sources,
        )
        return 0
    if arguments.command == "seal-normalization":
        path = seal_normalization_artifact(
            calibration_path=arguments.calibration,
            manifest_path=arguments.manifest,
            source_paths=sources,
            output_root=arguments.output_root,
        )
        print(path)
        return 0
    if arguments.command == "verify-normalization":
        verify_normalization_artifact(
            arguments.artifact,
            calibration_path=arguments.calibration,
            manifest_path=arguments.manifest,
            source_paths=sources,
        )
        return 0
    raise AssertionError("unreachable")


if __name__ == "__main__":
    raise SystemExit(main())
