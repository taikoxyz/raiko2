"""Post-normalization SP1 validation runner for the controlled-block experiment."""

from __future__ import annotations

import argparse
import copy
import json
import pathlib
import stat
import subprocess
import tempfile
from collections.abc import Callable, Mapping, Sequence
from typing import Any

import block_calibration
from opcode_gas import REPO_ROOT, canonical_json, sha256_bytes


PURPOSE = "sp1_controlled_block_validation"
COMPARISON_PURPOSE = "sp1_controlled_block_comparison"
RUNNER_SOURCE_RELATIVE_PATH = "experiments/opcode-gas/block_validation.py"
IMPLEMENTATION_SOURCE_PATHS = (
    RUNNER_SOURCE_RELATIVE_PATH,
    *block_calibration.IMPLEMENTATION_SOURCE_PATHS,
)
ROW_TIMEOUT_SECONDS = block_calibration.ROW_TIMEOUT_SECONDS
ENGINE_CONFIG = copy.deepcopy(block_calibration.ENGINE_CONFIG)
SOURCE_NAMES = (
    "candidate", "launcher", "elf", "vk", "trace", "fixture", "builder"
)

ELIGIBLE_CANDIDATE_IDENTITY = block_calibration.ELIGIBLE_CANDIDATE_IDENTITY
ELIGIBLE_MANIFEST_IDENTITY = block_calibration.ELIGIBLE_MANIFEST_IDENTITY
ELIGIBLE_MANIFEST_FILE_SHA256 = block_calibration.ELIGIBLE_MANIFEST_FILE_SHA256
ELIGIBLE_MANIFEST_RELATIVE_PATH = block_calibration.ELIGIBLE_MANIFEST_RELATIVE_PATH
ELIGIBLE_CALIBRATION_IDENTITY = (
    "c785d807bf97dbf8c46eb66aed27a564ba6b2826b442160a60d2bebfe121651f"
)
ELIGIBLE_CALIBRATION_FILE_SHA256 = (
    "b0139d961fa990996ce5951ea3527bc4b7e6ed7f5ca35a482d3b8fa7f275ff57"
)
ELIGIBLE_CALIBRATION_RELATIVE_PATH = (
    "experiments/opcode-gas/calibrations/sp1-block-comparison-v1/"
    "c785d807bf97dbf8c46eb66a/calibration.json"
)
ELIGIBLE_NORMALIZATION_IDENTITY = (
    "9d97015d76b8c5d66f28f8d220138f91aee740adcd083d62f5938219b5a5470e"
)
ELIGIBLE_NORMALIZATION_FILE_SHA256 = (
    "3856ea6deca594d80c8d68d698cfce1faac037110f02049a6db2c0c6a47b72c7"
)
ELIGIBLE_NORMALIZATION_RELATIVE_PATH = (
    "experiments/opcode-gas/normalizations/sp1-block-comparison-v1/"
    "9d97015d76b8c5d66f28f8d2/normalization.json"
)


def _git(*arguments: str) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", *arguments],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _implementation_identity_at_revision(
    revision: str,
) -> tuple[str, dict[str, str]]:
    revision, identities = block_calibration._implementation_identity_at_revision(
        revision
    )
    relative = RUNNER_SOURCE_RELATIVE_PATH
    path = REPO_ROOT / relative
    try:
        mode = path.lstat().st_mode
        current = path.read_bytes()
    except OSError as error:
        raise ValueError("validation implementation must be a clean tracked file") from error
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
        raise ValueError("validation implementation must be a clean tracked file")
    return revision, {relative: sha256_bytes(current), **identities}


def _current_implementation_identity() -> tuple[str, dict[str, str]]:
    resolved = _git("rev-parse", "--verify", "HEAD^{commit}")
    revision = resolved.stdout.decode().strip()
    if resolved.returncode != 0:
        raise ValueError("validation implementation revision differs")
    return _implementation_identity_at_revision(revision)


def _preflight_pinned_document(
    path: pathlib.Path,
    *,
    relative_path: str,
    file_sha256: str,
    identity: str,
    label: str,
) -> Mapping[str, Any]:
    supplied = pathlib.Path(path)
    value = block_calibration._canonical_file(supplied, label=label)
    try:
        supplied_mode = supplied.lstat().st_mode
        canonical = supplied.resolve(strict=True)
        expected = (REPO_ROOT / relative_path).resolve(strict=True)
        raw = canonical.read_bytes()
    except OSError as error:
        raise ValueError(f"validation requires the eligible {label}") from error
    if (
        supplied.is_symlink()
        or not stat.S_ISREG(supplied_mode)
        or canonical != expected
        or sha256_bytes(raw) != file_sha256
        or value.get("artifact_sha256") != identity
    ):
        raise ValueError(f"validation requires the eligible {label}")
    return value


def _preflight_inputs(
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> tuple[Mapping[str, Any], Mapping[str, Any], Mapping[str, Any]]:
    manifest = _preflight_pinned_document(
        manifest_path,
        relative_path=ELIGIBLE_MANIFEST_RELATIVE_PATH,
        file_sha256=ELIGIBLE_MANIFEST_FILE_SHA256,
        identity=ELIGIBLE_MANIFEST_IDENTITY,
        label="frozen manifest",
    )
    if manifest.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY:
        raise ValueError("validation requires the eligible frozen manifest")
    _preflight_source_paths(manifest, source_paths)
    calibration = _preflight_pinned_document(
        calibration_path,
        relative_path=ELIGIBLE_CALIBRATION_RELATIVE_PATH,
        file_sha256=ELIGIBLE_CALIBRATION_FILE_SHA256,
        identity=ELIGIBLE_CALIBRATION_IDENTITY,
        label="calibration artifact",
    )
    normalization = _preflight_pinned_document(
        normalization_path,
        relative_path=ELIGIBLE_NORMALIZATION_RELATIVE_PATH,
        file_sha256=ELIGIBLE_NORMALIZATION_FILE_SHA256,
        identity=ELIGIBLE_NORMALIZATION_IDENTITY,
        label="normalization artifact",
    )
    if (
        calibration.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or calibration.get("manifest_identity") != ELIGIBLE_MANIFEST_IDENTITY
        or normalization.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or normalization.get("manifest_identity") != ELIGIBLE_MANIFEST_IDENTITY
        or normalization.get("calibration_identity")
        != ELIGIBLE_CALIBRATION_IDENTITY
    ):
        raise ValueError("validation frozen input identities differ")
    return manifest, calibration, normalization


def _preflight_source_paths(
    manifest: Mapping[str, Any], source_paths: Mapping[str, pathlib.Path]
) -> None:
    declared_sources = manifest.get("sources")
    if (
        not isinstance(declared_sources, Mapping)
        or set(declared_sources) != set(SOURCE_NAMES)
        or set(source_paths) != set(SOURCE_NAMES)
    ):
        raise ValueError("validation source inventory differs")
    for name in SOURCE_NAMES:
        declared = declared_sources.get(name)
        supplied = pathlib.Path(source_paths[name])
        if not isinstance(declared, Mapping):
            raise ValueError(f"validation {name} source differs")
        relative = declared.get("path")
        expected_sha256 = declared.get("sha256")
        if (
            not isinstance(relative, str)
            or not relative
            or pathlib.Path(relative).is_absolute()
            or ".." in pathlib.Path(relative).parts
            or not block_calibration._is_sha256(expected_sha256)
        ):
            raise ValueError(f"validation {name} source differs")
        try:
            supplied_mode = supplied.lstat().st_mode
            canonical = supplied.resolve(strict=True)
            expected = (REPO_ROOT / relative).resolve(strict=True)
            source_sha256 = sha256_bytes(canonical.read_bytes())
        except OSError as error:
            raise ValueError(f"validation {name} source differs") from error
        if (
            supplied.is_symlink()
            or not stat.S_ISREG(supplied_mode)
            or canonical != expected
            or source_sha256 != expected_sha256
            or (
                name == "candidate"
                and declared.get("logical_identity")
                != ELIGIBLE_CANDIDATE_IDENTITY
            )
        ):
            raise ValueError(f"validation {name} source differs")


def _verify_frozen_inputs(
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> tuple[Mapping[str, Any], Mapping[str, Any], Mapping[str, Any]]:
    preflight = _preflight_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    normalization = block_calibration.verify_normalization_artifact(
        normalization_path,
        calibration_path=calibration_path,
        manifest_path=manifest_path,
        source_paths=source_paths,
    )
    manifest = json.loads(pathlib.Path(manifest_path).read_bytes())
    calibration = json.loads(pathlib.Path(calibration_path).read_bytes())
    for verified, expected in zip(
        (manifest, calibration, normalization), preflight, strict=True
    ):
        if canonical_json(verified) != canonical_json(expected):
            raise ValueError("validation frozen input changed during exact replay")
    return manifest, calibration, normalization


def _validation_rows(manifest: Mapping[str, Any]) -> list[Mapping[str, Any]]:
    if (
        manifest.get("schema_version") != 2
        or manifest.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or manifest.get("artifact_sha256") != ELIGIBLE_MANIFEST_IDENTITY
    ):
        raise ValueError("controlled-block manifest identity differs")
    rows = manifest.get("rows")
    if not isinstance(rows, list) or len(rows) != 32:
        raise ValueError("controlled-block manifest row population differs")
    calibration = [
        row for row in rows if isinstance(row, Mapping) and row.get("partition") == "calibration"
    ]
    validation = [
        row for row in rows if isinstance(row, Mapping) and row.get("partition") == "validation"
    ]
    row_ids = [row.get("row_id") for row in validation]
    if (
        len(calibration) != 12
        or len(validation) != 20
        or len(set(row_ids)) != 20
        or not all(block_calibration._is_sha256(value) for value in row_ids)
    ):
        raise ValueError("controlled-block validation partition differs")
    return validation


def validate_report(
    report: Mapping[str, Any], row: Mapping[str, Any], manifest: Mapping[str, Any]
) -> dict[str, Any]:
    eligible = {item["row_id"]: item for item in _validation_rows(manifest)}
    if (
        row.get("partition") != "validation"
        or row.get("row_id") not in eligible
        or canonical_json(row) != canonical_json(eligible[row["row_id"]])
    ):
        raise ValueError("controlled-block report is not a validation row")
    calibration_view = copy.deepcopy(dict(row))
    calibration_view["partition"] = "calibration"
    return block_calibration.validate_report(report, calibration_view, manifest)


def _run_identity(
    *,
    manifest: Mapping[str, Any],
    calibration: Mapping[str, Any],
    normalization: Mapping[str, Any],
    revision: str,
    implementation_sources: Mapping[str, str],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "purpose": PURPOSE,
        "source_revision": revision,
        "candidate_identity": manifest["candidate_identity"],
        "manifest_identity": manifest["artifact_sha256"],
        "manifest_file_sha256": ELIGIBLE_MANIFEST_FILE_SHA256,
        "calibration_identity": calibration["artifact_sha256"],
        "calibration_file_sha256": ELIGIBLE_CALIBRATION_FILE_SHA256,
        "normalization_identity": normalization["artifact_sha256"],
        "normalization_file_sha256": ELIGIBLE_NORMALIZATION_FILE_SHA256,
        "runner_source_sha256": implementation_sources[
            RUNNER_SOURCE_RELATIVE_PATH
        ],
        "implementation_sources": dict(implementation_sources),
        "manifest_sources": copy.deepcopy(manifest["sources"]),
        "engine_config": copy.deepcopy(ENGINE_CONFIG),
        "validation_row_ids": [row["row_id"] for row in _validation_rows(manifest)],
    }


def _watched_sources(
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, pathlib.Path]:
    return {
        "manifest": pathlib.Path(manifest_path),
        "calibration": pathlib.Path(calibration_path),
        "normalization": pathlib.Path(normalization_path),
        **{f"manifest:{name}": pathlib.Path(path) for name, path in source_paths.items()},
        **{
            f"implementation:{path}": REPO_ROOT / path
            for path in IMPLEMENTATION_SOURCE_PATHS
        },
    }


def _reject_child_symlinks(directory: pathlib.Path, *, label: str) -> None:
    try:
        children = list(directory.iterdir())
    except OSError as error:
        raise ValueError(f"{label} differs") from error
    if any(child.is_symlink() for child in children):
        raise ValueError(f"{label} differs")


def run_validation(
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    launcher_path: pathlib.Path,
    run_dir: pathlib.Path,
    command_runner: Callable[..., subprocess.CompletedProcess[Any]] = subprocess.run,
) -> list[dict[str, Any]]:
    run_dir = pathlib.Path(run_dir)
    if run_dir.is_symlink() or (run_dir.exists() and not run_dir.is_dir()):
        raise ValueError("validation run root differs")
    try:
        if pathlib.Path(launcher_path).resolve(strict=True) != pathlib.Path(
            source_paths["launcher"]
        ).resolve(strict=True):
            raise ValueError
    except (KeyError, OSError, ValueError) as error:
        raise ValueError("validation launcher path differs") from error
    manifest, calibration, normalization = _verify_frozen_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    rows = _validation_rows(manifest)
    revision, implementation_sources = _current_implementation_identity()
    identity = _run_identity(
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
        revision=revision,
        implementation_sources=implementation_sources,
    )
    watched = _watched_sources(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    snapshot = block_calibration._source_snapshot(watched)
    run_dir.mkdir(parents=True, exist_ok=True)
    _reject_child_symlinks(run_dir, label="validation run inventory")
    for directory in (run_dir / "inputs", run_dir / "rows"):
        if directory.exists():
            if not directory.is_dir():
                raise ValueError("validation row inventory differs")
            _reject_child_symlinks(directory, label="validation row inventory")
    block_calibration._cleanup_stale_partials(run_dir)
    if {path.name for path in run_dir.iterdir()} - {"identity.json", "inputs", "rows"}:
        raise ValueError("validation run inventory differs")
    identity_path = run_dir / "identity.json"
    identity_bytes = canonical_json(identity) + b"\n"
    if identity_path.exists():
        if identity_path.is_symlink() or identity_path.read_bytes() != identity_bytes:
            raise ValueError("validation run identity differs")
    else:
        block_calibration._durable_create(identity_path, identity_bytes)
    inputs = run_dir / "inputs"
    rows_dir = run_dir / "rows"
    for directory in (inputs, rows_dir):
        if directory.is_symlink() or (directory.exists() and not directory.is_dir()):
            raise ValueError("validation row inventory differs")
        directory.mkdir(exist_ok=True)
    expected_names = {f"{row['row_id']}.json" for row in rows}
    for directory in (inputs, rows_dir):
        _reject_child_symlinks(directory, label="validation row inventory")
        block_calibration._cleanup_stale_partials(directory)
        if {path.name for path in directory.iterdir()} - expected_names:
            raise ValueError("validation row inventory differs")
    launcher_bytes = pathlib.Path(launcher_path).read_bytes()
    if sha256_bytes(launcher_bytes) != manifest["sources"]["launcher"]["sha256"]:
        raise ValueError("validation launcher bytes differ")
    reports: list[dict[str, Any]] = []
    for row in rows:
        row_id = row["row_id"]
        input_path = inputs / f"{row_id}.json"
        input_bytes = canonical_json(row["frozen_identity"]["spec"]) + b"\n"
        if input_path.exists():
            if input_path.is_symlink() or input_path.read_bytes() != input_bytes:
                raise ValueError("validation input differs")
        else:
            block_calibration._durable_create(input_path, input_bytes)
        row_path = rows_dir / f"{row_id}.json"
        if row_path.exists():
            stored = block_calibration._canonical_file(
                row_path, label="validation row"
            )
            reports.append(validate_report(stored, row, manifest))
            continue
        with tempfile.TemporaryDirectory(prefix="block-validation-row-") as temporary:
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
                raise ValueError("controlled-block validation launcher timed out") from error
            if completed.returncode != 0:
                raise ValueError("controlled-block validation launcher failed")
            try:
                lines = output_path.read_text().splitlines()
                if len(lines) != 1:
                    raise ValueError(
                        "controlled-block validation launcher emitted duplicate or missing rows"
                    )
                report = json.loads(lines[0])
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError("controlled-block validation report differs") from error
            validated = validate_report(report, row, manifest)
            if pathlib.Path(launcher_path).read_bytes() != launcher_bytes:
                raise ValueError("validation launcher changed during execution")
            block_calibration._require_source_snapshot(watched, snapshot)
            block_calibration._durable_create(
                row_path, canonical_json(validated) + b"\n"
            )
            reports.append(validated)
    if pathlib.Path(launcher_path).read_bytes() != launcher_bytes:
        raise ValueError("validation launcher changed during execution")
    block_calibration._require_source_snapshot(watched, snapshot)
    return reports


def _load_completed_run(
    *,
    run_dir: pathlib.Path,
    manifest: Mapping[str, Any],
    expected_identity: Mapping[str, Any],
) -> list[dict[str, Any]]:
    run_dir = pathlib.Path(run_dir)
    if run_dir.is_symlink() or {path.name for path in run_dir.iterdir()} != {
        "identity.json", "inputs", "rows"
    }:
        raise ValueError("completed validation run inventory differs")
    identity = block_calibration._canonical_file(
        run_dir / "identity.json", label="validation run identity"
    )
    if canonical_json(identity) != canonical_json(expected_identity):
        raise ValueError("completed validation run identity differs")
    rows = _validation_rows(manifest)
    expected_names = {f"{row['row_id']}.json" for row in rows}
    inputs = run_dir / "inputs"
    reports_dir = run_dir / "rows"
    if (
        inputs.is_symlink()
        or reports_dir.is_symlink()
        or {path.name for path in inputs.iterdir()} != expected_names
        or {path.name for path in reports_dir.iterdir()} != expected_names
    ):
        raise ValueError("completed validation row inventory differs")
    reports = []
    for row in rows:
        name = f"{row['row_id']}.json"
        if (inputs / name).is_symlink() or (inputs / name).read_bytes() != (
            canonical_json(row["frozen_identity"]["spec"]) + b"\n"
        ):
            raise ValueError("completed validation input differs")
        report = block_calibration._canonical_file(
            reports_dir / name, label="validation row"
        )
        reports.append(validate_report(report, row, manifest))
    return reports


def build_validation_artifact(
    *,
    manifest: Mapping[str, Any],
    calibration: Mapping[str, Any],
    normalization: Mapping[str, Any],
    reports: Sequence[Mapping[str, Any]],
    source_revision: str,
    implementation_sources: Mapping[str, str],
) -> dict[str, Any]:
    rows = _validation_rows(manifest)
    if len(reports) != 20:
        raise ValueError("validation artifact inputs differ")
    sealed_rows = []
    for source_row, report in zip(rows, reports, strict=True):
        validated = validate_report(report, source_row, manifest)
        sealed_rows.append(
            {
                "row_id": source_row["row_id"],
                "partition": "validation",
                "candidate_identity_sha256": manifest["candidate_identity"],
                "normalization_identity_sha256": normalization["artifact_sha256"],
                "candidate_coverage_complete": source_row[
                    "candidate_coverage_complete"
                ],
                "candidate_predicted_prover_gas": source_row[
                    "candidate_predicted_prover_gas"
                ],
                "observed_prover_gas": str(validated["gas"]),
                "finalized_block_zkgas": str(
                    source_row["frozen_identity"]["observation"][
                        "finalized_block_zkgas"
                    ]
                ),
                "report_sha256": sha256_bytes(canonical_json(validated)),
                "report": validated,
            }
        )
    artifact: dict[str, Any] = {
        **_run_identity(
            manifest=manifest,
            calibration=calibration,
            normalization=normalization,
            revision=source_revision,
            implementation_sources=implementation_sources,
        ),
        "status": "sealed_validation",
        "row_digest": sha256_bytes(canonical_json(sealed_rows)),
        "rows": sealed_rows,
    }
    artifact["artifact_sha256"] = block_calibration._content_address(artifact)
    validate_validation_artifact(artifact)
    return artifact


def validate_validation_artifact(artifact: Mapping[str, Any]) -> None:
    expected = {
        "schema_version", "purpose", "source_revision", "candidate_identity",
        "manifest_identity", "manifest_file_sha256", "calibration_identity",
        "calibration_file_sha256", "normalization_identity",
        "normalization_file_sha256", "runner_source_sha256",
        "implementation_sources", "manifest_sources", "engine_config",
        "validation_row_ids", "status", "row_digest", "rows",
        "artifact_sha256",
    }
    if not isinstance(artifact, Mapping) or set(artifact) != expected:
        raise ValueError("validation artifact schema differs")
    if (
        artifact.get("schema_version") != 1
        or artifact.get("purpose") != PURPOSE
        or artifact.get("status") != "sealed_validation"
        or artifact.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or artifact.get("manifest_identity") != ELIGIBLE_MANIFEST_IDENTITY
        or artifact.get("manifest_file_sha256") != ELIGIBLE_MANIFEST_FILE_SHA256
        or artifact.get("calibration_identity") != ELIGIBLE_CALIBRATION_IDENTITY
        or artifact.get("calibration_file_sha256") != ELIGIBLE_CALIBRATION_FILE_SHA256
        or artifact.get("normalization_identity") != ELIGIBLE_NORMALIZATION_IDENTITY
        or artifact.get("normalization_file_sha256")
        != ELIGIBLE_NORMALIZATION_FILE_SHA256
        or artifact.get("engine_config") != ENGINE_CONFIG
    ):
        raise ValueError("validation artifact header differs")
    revision = artifact.get("source_revision")
    implementation = artifact.get("implementation_sources")
    if (
        not isinstance(revision, str)
        or len(revision) != 40
        or any(character not in "0123456789abcdef" for character in revision)
        or not isinstance(implementation, Mapping)
        or set(implementation) != set(IMPLEMENTATION_SOURCE_PATHS)
        or not all(block_calibration._is_sha256(value) for value in implementation.values())
        or artifact.get("runner_source_sha256")
        != implementation.get(RUNNER_SOURCE_RELATIVE_PATH)
    ):
        raise ValueError("validation implementation identity differs")
    if artifact.get("artifact_sha256") != block_calibration._content_address(artifact):
        raise ValueError("validation artifact content address differs")
    rows = artifact.get("rows")
    if (
        not isinstance(rows, list)
        or len(rows) != 20
        or artifact.get("row_digest") != sha256_bytes(canonical_json(rows))
        or artifact.get("validation_row_ids")
        != [row.get("row_id") for row in rows]
    ):
        raise ValueError("validation artifact rows differ")
    seen = set()
    expected_row_fields = {
        "row_id", "partition", "candidate_identity_sha256",
        "normalization_identity_sha256", "candidate_coverage_complete",
        "candidate_predicted_prover_gas", "observed_prover_gas",
        "finalized_block_zkgas", "report_sha256", "report",
    }
    for row in rows:
        report = row.get("report") if isinstance(row, Mapping) else None
        if (
            not isinstance(row, Mapping)
            or set(row) != expected_row_fields
            or row.get("partition") != "validation"
            or row.get("candidate_identity_sha256") != ELIGIBLE_CANDIDATE_IDENTITY
            or row.get("normalization_identity_sha256")
            != ELIGIBLE_NORMALIZATION_IDENTITY
            or row.get("candidate_coverage_complete") is not True
            or row.get("row_id") in seen
            or not block_calibration._is_sha256(row.get("row_id"))
            or row.get("report_sha256")
            != sha256_bytes(canonical_json(report))
            or not isinstance(report, Mapping)
            or report.get("input") != row.get("row_id")
            or isinstance(report.get("gas"), bool)
            or not isinstance(report.get("gas"), int)
            or report.get("gas") <= 0
            or row.get("observed_prover_gas") != str(report.get("gas"))
        ):
            raise ValueError("validation artifact row differs")
        seen.add(row["row_id"])
        prediction = block_calibration._decimal(
            row.get("candidate_predicted_prover_gas")
        )
        if prediction < 0:
            raise ValueError("validation artifact prediction differs")
        block_calibration._decimal(row.get("observed_prover_gas"), positive=True)
        block_calibration._decimal(row.get("finalized_block_zkgas"), positive=True)


def seal_validation_artifact(
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    run_dir: pathlib.Path,
    output_root: pathlib.Path,
) -> pathlib.Path:
    manifest, calibration, normalization = _verify_frozen_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    stored_identity = block_calibration._canonical_file(
        pathlib.Path(run_dir) / "identity.json", label="validation run identity"
    )
    revision = stored_identity.get("source_revision")
    if not isinstance(revision, str):
        raise ValueError("completed validation source revision differs")
    revision, implementation_sources = _implementation_identity_at_revision(revision)
    expected_identity = _run_identity(
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
        revision=revision,
        implementation_sources=implementation_sources,
    )
    reports = _load_completed_run(
        run_dir=run_dir, manifest=manifest, expected_identity=expected_identity
    )
    artifact = build_validation_artifact(
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
        reports=reports,
        source_revision=revision,
        implementation_sources=implementation_sources,
    )
    return block_calibration._publish_document(
        artifact,
        output_root,
        filename="validation.json",
        validator=validate_validation_artifact,
    )


def _verify_validation_with_frozen_inputs(
    path: pathlib.Path,
    *,
    manifest: Mapping[str, Any],
    calibration: Mapping[str, Any],
    normalization: Mapping[str, Any],
) -> dict[str, Any]:
    artifact = block_calibration._verify_document(
        path, filename="validation.json", validator=validate_validation_artifact
    )
    revision, implementation_sources = _implementation_identity_at_revision(
        artifact["source_revision"]
    )
    rebuilt = build_validation_artifact(
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
        reports=[row["report"] for row in artifact["rows"]],
        source_revision=revision,
        implementation_sources=implementation_sources,
    )
    if canonical_json(rebuilt) != canonical_json(artifact):
        raise ValueError("sealed validation differs from exact replay")
    return artifact


def verify_validation_artifact(
    path: pathlib.Path,
    *,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, Any]:
    manifest, calibration, normalization = _verify_frozen_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    return _verify_validation_with_frozen_inputs(
        path,
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
    )


def _comparison_rows(
    validation: Mapping[str, Any],
) -> list[dict[str, object]]:
    validate_validation_artifact(validation)
    return [
        {
            "row_id": row["row_id"],
            "partition": "validation",
            "candidate_identity_sha256": row["candidate_identity_sha256"],
            "normalization_identity_sha256": row[
                "normalization_identity_sha256"
            ],
            "candidate_coverage_complete": row[
                "candidate_coverage_complete"
            ],
            "candidate_predicted_prover_gas": row[
                "candidate_predicted_prover_gas"
            ],
            "observed_prover_gas": row["observed_prover_gas"],
            "finalized_block_zkgas": row["finalized_block_zkgas"],
        }
        for row in validation["rows"]
    ]


def build_comparison_artifact(
    *,
    validation: Mapping[str, Any],
    normalization: Mapping[str, Any],
) -> dict[str, Any]:
    validate_validation_artifact(validation)
    if (
        normalization.get("artifact_sha256") != ELIGIBLE_NORMALIZATION_IDENTITY
        or normalization.get("acceptance_thresholds")
        != block_calibration.ACCEPTANCE_THRESHOLDS
    ):
        raise ValueError("comparison normalization differs")
    kappa = block_calibration._decimal(
        normalization.get("kappa_unzen"), positive=True
    )
    comparison = block_calibration.context_approximation.compare_block_models(
        _comparison_rows(validation), kappa
    )
    artifact: dict[str, Any] = {
        "schema_version": 1,
        "purpose": COMPARISON_PURPOSE,
        "status": (
            "block_validated_against_unzen"
            if comparison["accepted"]
            else "block_validation_failed"
        ),
        "source_revision": validation["source_revision"],
        "candidate_identity": validation["candidate_identity"],
        "manifest_identity": validation["manifest_identity"],
        "calibration_identity": validation["calibration_identity"],
        "normalization_identity": validation["normalization_identity"],
        "validation_identity": validation["artifact_sha256"],
        "validation_file_sha256": sha256_bytes(
            canonical_json(validation) + b"\n"
        ),
        "implementation_sources": copy.deepcopy(
            validation["implementation_sources"]
        ),
        "comparator_source_sha256": validation["implementation_sources"][
            "experiments/opcode-gas/context_approximation.py"
        ],
        "acceptance_thresholds": copy.deepcopy(
            normalization["acceptance_thresholds"]
        ),
        "comparison": comparison,
    }
    artifact["artifact_sha256"] = block_calibration._content_address(artifact)
    validate_comparison_artifact(artifact)
    return artifact


def _replay_embedded_comparison(artifact: Mapping[str, Any]) -> dict[str, object]:
    comparison = artifact.get("comparison")
    if not isinstance(comparison, Mapping):
        raise ValueError("comparison report differs")
    rows = comparison.get("rows")
    if not isinstance(rows, list):
        raise ValueError("comparison report rows differ")
    replay_rows = []
    for row in rows:
        if not isinstance(row, Mapping):
            raise ValueError("comparison report row differs")
        candidate = row.get("candidate")
        if not isinstance(candidate, Mapping):
            raise ValueError("comparison report candidate row differs")
        replay_rows.append(
            {
                "row_id": row.get("row_id"),
                "partition": "validation",
                "candidate_identity_sha256": artifact.get(
                    "candidate_identity"
                ),
                "normalization_identity_sha256": artifact.get(
                    "normalization_identity"
                ),
                "candidate_coverage_complete": row.get(
                    "candidate_coverage_complete"
                ),
                "candidate_predicted_prover_gas": candidate.get(
                    "predicted_prover_gas"
                ),
                "observed_prover_gas": row.get("observed_prover_gas"),
                "finalized_block_zkgas": row.get("finalized_block_zkgas"),
            }
        )
    return block_calibration.context_approximation.compare_block_models(
        replay_rows,
        block_calibration._decimal(comparison.get("kappa_unzen"), positive=True),
    )


def validate_comparison_artifact(artifact: Mapping[str, Any]) -> None:
    expected = {
        "schema_version", "purpose", "status", "source_revision",
        "candidate_identity", "manifest_identity", "calibration_identity",
        "normalization_identity", "validation_identity",
        "validation_file_sha256", "implementation_sources",
        "comparator_source_sha256", "acceptance_thresholds", "comparison",
        "artifact_sha256",
    }
    if not isinstance(artifact, Mapping) or set(artifact) != expected:
        raise ValueError("comparison artifact schema differs")
    comparison = artifact.get("comparison")
    implementation = artifact.get("implementation_sources")
    if (
        artifact.get("schema_version") != 1
        or artifact.get("purpose") != COMPARISON_PURPOSE
        or artifact.get("candidate_identity") != ELIGIBLE_CANDIDATE_IDENTITY
        or artifact.get("manifest_identity") != ELIGIBLE_MANIFEST_IDENTITY
        or artifact.get("calibration_identity") != ELIGIBLE_CALIBRATION_IDENTITY
        or artifact.get("normalization_identity")
        != ELIGIBLE_NORMALIZATION_IDENTITY
        or not block_calibration._is_sha256(artifact.get("validation_identity"))
        or not block_calibration._is_sha256(
            artifact.get("validation_file_sha256")
        )
        or not isinstance(implementation, Mapping)
        or set(implementation) != set(IMPLEMENTATION_SOURCE_PATHS)
        or not all(
            block_calibration._is_sha256(value)
            for value in implementation.values()
        )
        or artifact.get("comparator_source_sha256")
        != implementation.get("experiments/opcode-gas/context_approximation.py")
        or artifact.get("acceptance_thresholds")
        != block_calibration.ACCEPTANCE_THRESHOLDS
        or not isinstance(comparison, Mapping)
    ):
        raise ValueError("comparison artifact identity differs")
    revision = artifact.get("source_revision")
    if (
        not isinstance(revision, str)
        or len(revision) != 40
        or any(character not in "0123456789abcdef" for character in revision)
    ):
        raise ValueError("comparison source revision differs")
    replayed = _replay_embedded_comparison(artifact)
    if canonical_json(replayed) != canonical_json(comparison):
        raise ValueError("comparison report differs from exact replay")
    expected_status = (
        "block_validated_against_unzen"
        if comparison.get("accepted") is True
        else "block_validation_failed"
    )
    if artifact.get("status") != expected_status:
        raise ValueError("comparison status differs")
    if artifact.get("artifact_sha256") != block_calibration._content_address(
        artifact
    ):
        raise ValueError("comparison artifact content address differs")


def seal_comparison_artifact(
    *,
    validation_path: pathlib.Path,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
    output_root: pathlib.Path,
) -> pathlib.Path:
    manifest, calibration, normalization = _verify_frozen_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    validation = _verify_validation_with_frozen_inputs(
        validation_path,
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
    )
    artifact = build_comparison_artifact(
        validation=validation, normalization=normalization
    )
    return block_calibration._publish_document(
        artifact,
        output_root,
        filename="comparison.json",
        validator=validate_comparison_artifact,
    )


def verify_comparison_artifact(
    path: pathlib.Path,
    *,
    validation_path: pathlib.Path,
    manifest_path: pathlib.Path,
    calibration_path: pathlib.Path,
    normalization_path: pathlib.Path,
    source_paths: Mapping[str, pathlib.Path],
) -> dict[str, Any]:
    manifest, calibration, normalization = _verify_frozen_inputs(
        manifest_path=manifest_path,
        calibration_path=calibration_path,
        normalization_path=normalization_path,
        source_paths=source_paths,
    )
    validation = _verify_validation_with_frozen_inputs(
        validation_path,
        manifest=manifest,
        calibration=calibration,
        normalization=normalization,
    )
    artifact = block_calibration._verify_document(
        path,
        filename="comparison.json",
        validator=validate_comparison_artifact,
    )
    rebuilt = build_comparison_artifact(
        validation=validation, normalization=normalization
    )
    if canonical_json(rebuilt) != canonical_json(artifact):
        raise ValueError("sealed comparison differs from exact replay")
    return artifact


def _source_arguments(parser: argparse.ArgumentParser) -> None:
    for name in ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder"):
        parser.add_argument(f"--{name}", type=pathlib.Path, required=True)


def _source_paths(arguments: argparse.Namespace) -> dict[str, pathlib.Path]:
    return {
        name: getattr(arguments, name)
        for name in ("candidate", "launcher", "elf", "vk", "trace", "fixture", "builder")
    }


def _frozen_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--manifest", type=pathlib.Path, required=True)
    parser.add_argument("--calibration", type=pathlib.Path, required=True)
    parser.add_argument("--normalization", type=pathlib.Path, required=True)
    _source_arguments(parser)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run")
    _frozen_arguments(run)
    run.add_argument("--run-dir", type=pathlib.Path, required=True)
    seal = commands.add_parser("seal-validation")
    _frozen_arguments(seal)
    seal.add_argument("--run-dir", type=pathlib.Path, required=True)
    seal.add_argument("--output-root", type=pathlib.Path, required=True)
    verify = commands.add_parser("verify-validation")
    _frozen_arguments(verify)
    verify.add_argument("--artifact", type=pathlib.Path, required=True)
    compare = commands.add_parser("seal-comparison")
    _frozen_arguments(compare)
    compare.add_argument("--validation", type=pathlib.Path, required=True)
    compare.add_argument("--output-root", type=pathlib.Path, required=True)
    verify_comparison = commands.add_parser("verify-comparison")
    _frozen_arguments(verify_comparison)
    verify_comparison.add_argument(
        "--validation", type=pathlib.Path, required=True
    )
    verify_comparison.add_argument("--artifact", type=pathlib.Path, required=True)
    arguments = parser.parse_args(argv)
    sources = _source_paths(arguments)
    common = {
        "manifest_path": arguments.manifest,
        "calibration_path": arguments.calibration,
        "normalization_path": arguments.normalization,
        "source_paths": sources,
    }
    if arguments.command == "run":
        run_validation(
            **common,
            launcher_path=arguments.launcher,
            run_dir=arguments.run_dir,
        )
        return 0
    if arguments.command == "seal-validation":
        print(
            seal_validation_artifact(
                **common,
                run_dir=arguments.run_dir,
                output_root=arguments.output_root,
            )
        )
        return 0
    if arguments.command == "verify-validation":
        verify_validation_artifact(arguments.artifact, **common)
        return 0
    if arguments.command == "seal-comparison":
        print(
            seal_comparison_artifact(
                **common,
                validation_path=arguments.validation,
                output_root=arguments.output_root,
            )
        )
        return 0
    if arguments.command == "verify-comparison":
        verify_comparison_artifact(
            arguments.artifact,
            **common,
            validation_path=arguments.validation,
        )
        return 0
    raise AssertionError("unreachable")


if __name__ == "__main__":
    raise SystemExit(main())
