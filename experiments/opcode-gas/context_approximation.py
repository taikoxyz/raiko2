"""Replayable, non-coefficient SP1 diagnostics for context calibration rows."""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import tempfile
from collections.abc import Iterable, Mapping
from typing import Any

from opcode_gas import REPO_ROOT, canonical_json, sha256_bytes, sha256_file


_SHA256_HEX = frozenset("0123456789abcdef")
_DIAGNOSTIC_FIELDS = frozenset(
    {
        "total_instruction_count",
        "total_syscall_count",
        "touched_memory_addresses",
        "opcode_counts",
        "syscall_counts",
    }
)
_STRICT_SOURCE_RESULT_ID = "a41befb63e663890896ba67d"
_STRICT_SOURCE_RESULT_IDENTITY_SHA256 = (
    "a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943"
)
_STRICT_SOURCE_RESULT_ARTIFACT_SHA256 = (
    "ff48d2285f3f34a87c608780bb98fe344606eab511c6b3f4ba834c68f0e01837"
)
_STRICT_SOURCE_ROWS_SHA256 = (
    "651e7423330b428f346306c8dda52a7c2dff3fd5ae15f82bed02bdbaf7154c06"
)

_FIT_SCENARIOS = (
    "address_canonical",
    "caller_canonical",
    "callvalue_zero",
    "callvalue_nonzero_7",
    "calldataload_empty_offset_0",
    "calldataload_out_of_range_4_offset_64",
    "calldataload_partial_33_offset_17",
    "calldataload_full_32_offset_0",
    "calldatasize_0",
    "calldatasize_64",
    "timestamp_post_unzen_delta_17",
)
_FINAL_HOLDOUT_SCENARIOS = (
    "address_alternate",
    "caller_alternate",
    "callvalue_zero_calldata_1",
    "callvalue_nonzero_4294967297",
    "calldataload_empty_offset_1",
    "calldataload_out_of_range_96_offset_128",
    "calldataload_partial_31_offset_30",
    "calldataload_full_96_offset_32",
    "calldatasize_15",
    "calldatasize_127",
    "timestamp_post_unzen_delta_86400",
)
CONTEXT_DIAGNOSTIC_PANEL = tuple(
    [(scenario, count) for scenario in _FIT_SCENARIOS for count in (0, 16)]
    + [
        (scenario, count)
        for scenario in _FINAL_HOLDOUT_SCENARIOS
        for count in (0, 64)
    ]
)


def _is_sha256(value: object) -> bool:
    return isinstance(value, str) and len(value) == 64 and set(value) <= _SHA256_HEX


def _is_result_id(value: object) -> bool:
    return isinstance(value, str) and len(value) == 24 and set(value) <= _SHA256_HEX


def _jsonl_bytes(rows: Iterable[Mapping[str, Any]]) -> bytes:
    return b"".join(canonical_json(dict(row)) + b"\n" for row in rows)


def _required_nonnegative_int(value: object, *, label: str) -> int:
    if type(value) is not int or value < 0:
        raise ValueError(f"{label} must be a nonnegative integer")
    return value


def _canonical_count_entries(value: object, *, field: str, total: int) -> list[dict[str, object]]:
    if not isinstance(value, list):
        raise ValueError(f"{field} is missing or not an array")
    entries = []
    labels = set()
    for entry in value:
        if not isinstance(entry, Mapping) or set(entry) != {"label", "count"}:
            raise ValueError(f"{field} entry fields differ")
        label = entry["label"]
        if not isinstance(label, str) or not label:
            raise ValueError(f"{field} label differs")
        if label in labels:
            raise ValueError(f"{field} contains a duplicate label")
        labels.add(label)
        entries.append(
            {
                "label": label,
                "count": _required_nonnegative_int(entry["count"], label=f"{field} count"),
            }
        )
    if sum(entry["count"] for entry in entries) != total:
        raise ValueError(f"{field} total does not match its declared total")
    return sorted(entries, key=lambda entry: str(entry["label"]))


def extract_sp1_diagnostics(report: Mapping[str, object]) -> dict[str, object]:
    """Return the only low-level metrics allowed in a context diagnostic sidecar."""
    if not isinstance(report, Mapping):
        raise ValueError("SP1 diagnostic report must be an object")
    missing = _DIAGNOSTIC_FIELDS - set(report)
    if missing:
        raise ValueError(f"SP1 diagnostic report is missing {sorted(missing)[0]}")
    total_instruction_count = _required_nonnegative_int(
        report["total_instruction_count"], label="total_instruction_count"
    )
    total_syscall_count = _required_nonnegative_int(
        report["total_syscall_count"], label="total_syscall_count"
    )
    return {
        "total_instruction_count": total_instruction_count,
        "total_syscall_count": total_syscall_count,
        "touched_memory_addresses": _required_nonnegative_int(
            report["touched_memory_addresses"], label="touched_memory_addresses"
        ),
        "opcode_counts": _canonical_count_entries(
            report["opcode_counts"],
            field="opcode_counts",
            total=total_instruction_count,
        ),
        "syscall_counts": _canonical_count_entries(
            report["syscall_counts"],
            field="syscall_counts",
            total=total_syscall_count,
        ),
    }


def select_context_diagnostic_rows(
    rows: Iterable[Mapping[str, object]],
) -> list[dict[str, object]]:
    """Select the frozen 88-row panel from immutable portable result rows."""
    panel = {(scenario, count) for scenario, count in CONTEXT_DIAGNOSTIC_PANEL}
    by_key: dict[tuple[str, int, str], dict[str, object]] = {}
    for raw_row in rows:
        if not isinstance(raw_row, Mapping):
            raise ValueError("context diagnostic source row is not an object")
        scenario = raw_row.get("scenario")
        count = raw_row.get("count")
        lane = raw_row.get("lane")
        repeat_index = raw_row.get("repeat_index")
        if (scenario, count) not in panel or repeat_index != 0:
            continue
        if lane not in {"control", "target"}:
            raise ValueError("context diagnostic source row lane differs")
        if not isinstance(scenario, str) or type(count) is not int:
            raise ValueError("context diagnostic source row identity differs")
        expected_split = (
            "fit" if scenario in _FIT_SCENARIOS else "final_holdout"
        )
        if raw_row.get("split") != expected_split:
            raise ValueError("context diagnostic source row split differs")
        row_id = raw_row.get("row_id")
        if not _is_sha256(row_id):
            raise ValueError("context diagnostic source row ID differs")
        key = (scenario, count, lane)
        if key in by_key:
            raise ValueError("context diagnostic source row is duplicated")
        by_key[key] = dict(raw_row)
    selected = []
    for scenario, count in CONTEXT_DIAGNOSTIC_PANEL:
        for lane in ("control", "target"):
            row = by_key.get((scenario, count, lane))
            if row is None:
                raise ValueError("context diagnostic source row is missing")
            selected.append(row)
    if len({row["row_id"] for row in selected}) != len(selected):
        raise ValueError("context diagnostic source row IDs are duplicated")
    return selected


def _source_result_parts(result: Mapping[str, object]) -> tuple[dict[str, object], list[dict[str, object]], str]:
    if not isinstance(result, Mapping):
        raise ValueError("context diagnostic source result must be an object")
    for field in (
        "result_id",
        "result_identity",
        "result_identity_sha256",
        "artifact_sha256",
        "rows",
    ):
        if field not in result:
            raise ValueError(f"context diagnostic source result is missing {field}")
    if (
        not _is_result_id(result["result_id"])
        or not isinstance(result["result_identity"], Mapping)
        or not _is_sha256(result["result_identity_sha256"])
        or not _is_sha256(result["artifact_sha256"])
        or not isinstance(result["rows"], list)
    ):
        raise ValueError("context diagnostic source result identity differs")
    if (
        result["result_id"] != _STRICT_SOURCE_RESULT_ID
        or result["result_identity_sha256"] != _STRICT_SOURCE_RESULT_IDENTITY_SHA256
        or result["artifact_sha256"] != _STRICT_SOURCE_RESULT_ARTIFACT_SHA256
        or sha256_bytes(canonical_json(result["result_identity"]))
        != _STRICT_SOURCE_RESULT_IDENTITY_SHA256
    ):
        raise ValueError("context diagnostic source result differs from the strict source")
    rows = [dict(row) for row in result["rows"] if isinstance(row, Mapping)]
    if len(rows) != len(result["rows"]):
        raise ValueError("context diagnostic source result rows differ")
    source_rows_sha256 = sha256_bytes(_jsonl_bytes(rows))
    if source_rows_sha256 != _STRICT_SOURCE_ROWS_SHA256:
        raise ValueError("context diagnostic source result rows differ")
    return dict(result["result_identity"]), rows, source_rows_sha256


def _validated_report_for_row(
    report: Mapping[str, object], row: Mapping[str, object]
) -> dict[str, object]:
    try:
        controlled = report["controlled_block"]
        observation = controlled["observation"]
        guest_input_sha256 = str(report["guest_input_sha256"]).removeprefix("0x")
        if (
            report["stage"] != "controlled-block"
            or report["mode"] != "execute"
            or report["sp1_execution_engine"] != "gas-estimator"
            or report["exit_code"] != 0
            or controlled["status"] != "accepted"
            or controlled["row_id"] != row["row_id"]
            or guest_input_sha256 != row["backend_input_sha256"]
            or observation["backend_input_sha256"] != row["backend_input_sha256"]
            or report["sp1_proposal_elf_sha256"] != row["sp1_proposal_elf_sha256"]
            or report["guest_launcher_sha256"] != row["guest_launcher_sha256"]
        ):
            raise ValueError("context diagnostic report source row mismatch")
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError("context diagnostic report source row is incomplete") from error
    return extract_sp1_diagnostics(report)


def _sidecar_without_hashes(
    *,
    result: Mapping[str, object],
    source_identity: Mapping[str, object],
    source_rows_sha256: str,
    selected_rows: list[dict[str, object]],
    reports: Iterable[Mapping[str, object]],
) -> dict[str, object]:
    reports_by_id: dict[str, Mapping[str, object]] = {}
    for report in reports:
        if not isinstance(report, Mapping):
            raise ValueError("context diagnostic report is not an object")
        try:
            row_id = report["controlled_block"]["row_id"]
        except (KeyError, TypeError) as error:
            raise ValueError("context diagnostic report source row is incomplete") from error
        if not _is_sha256(row_id) or row_id in reports_by_id:
            raise ValueError("context diagnostic report source row is duplicated")
        reports_by_id[row_id] = report
    rows: dict[str, object] = {}
    for row in selected_rows:
        row_id = str(row["row_id"])
        report = reports_by_id.pop(row_id, None)
        if report is None:
            raise ValueError("context diagnostic report source row is missing")
        diagnostics = _validated_report_for_row(report, row)
        rows[row_id] = {
            "source_row_sha256": sha256_bytes(canonical_json(row)),
            "scenario": row["scenario"],
            "split": row["split"],
            "count": row["count"],
            "lane": row["lane"],
            "repeat_index": row["repeat_index"],
            "workload_id": row["workload_id"],
            "backend_input_sha256": row["backend_input_sha256"],
            "report_sha256": sha256_bytes(canonical_json(dict(report))),
            "diagnostics": diagnostics,
        }
    if reports_by_id:
        raise ValueError("context diagnostic report source row is not in the frozen panel")
    elf_hashes = {row["sp1_proposal_elf_sha256"] for row in selected_rows}
    launcher_hashes = {row["guest_launcher_sha256"] for row in selected_rows}
    if len(elf_hashes) != 1 or len(launcher_hashes) != 1:
        raise ValueError("context diagnostic source assets differ across rows")
    return {
        "schema_version": 1,
        "purpose": "production_context_sp1_diagnostics",
        "source_result_id": result["result_id"],
        "source_result_identity": dict(source_identity),
        "source_result_identity_sha256": result["result_identity_sha256"],
        "source_result_artifact_sha256": result["artifact_sha256"],
        "source_rows_sha256": source_rows_sha256,
        "guest_launcher_sha256": next(iter(launcher_hashes)),
        "sp1_proposal_elf_sha256": next(iter(elf_hashes)),
        "rows": rows,
    }


def build_context_diagnostic_sidecar(
    result: Mapping[str, object], reports: Iterable[Mapping[str, object]]
) -> dict[str, object]:
    """Bind exactly one 88-row diagnostic panel to an immutable result identity."""
    source_identity, source_rows, source_rows_sha256 = _source_result_parts(result)
    sidecar = _sidecar_without_hashes(
        result=result,
        source_identity=source_identity,
        source_rows_sha256=source_rows_sha256,
        selected_rows=select_context_diagnostic_rows(source_rows),
        reports=reports,
    )
    sidecar["diagnostic_identity_sha256"] = sha256_bytes(canonical_json(sidecar))
    sidecar["diagnostic_id"] = sidecar["diagnostic_identity_sha256"][:24]
    sidecar["artifact_sha256"] = sha256_bytes(canonical_json(sidecar))
    return sidecar


def reject_context_diagnostics_from_coefficient_input(value: object) -> None:
    """Fail closed if post-execution diagnostics enter a coefficient input."""
    if isinstance(value, Mapping):
        for key, child in value.items():
            normalized = str(key).lower().replace("-", "_")
            if normalized == "diagnostics" or normalized in _DIAGNOSTIC_FIELDS:
                raise ValueError("diagnostic fields are unavailable in coefficient inputs")
            reject_context_diagnostics_from_coefficient_input(child)
    elif isinstance(value, (list, tuple)):
        for child in value:
            reject_context_diagnostics_from_coefficient_input(child)


def verify_context_diagnostic_sidecar(
    sidecar: Mapping[str, object], result: Mapping[str, object] | None = None
) -> dict[str, object]:
    """Validate sidecar content addressing and, when supplied, its full source result."""
    if not isinstance(sidecar, Mapping):
        raise ValueError("context diagnostic sidecar must be an object")
    required = {
        "schema_version",
        "purpose",
        "source_result_id",
        "source_result_identity",
        "source_result_identity_sha256",
        "source_result_artifact_sha256",
        "source_rows_sha256",
        "guest_launcher_sha256",
        "sp1_proposal_elf_sha256",
        "rows",
        "diagnostic_identity_sha256",
        "diagnostic_id",
        "artifact_sha256",
    }
    if set(sidecar) != required or sidecar.get("schema_version") != 1 or sidecar.get("purpose") != "production_context_sp1_diagnostics":
        raise ValueError("context diagnostic sidecar schema differs")
    unhashed = dict(sidecar)
    claimed_artifact = unhashed.pop("artifact_sha256")
    claimed_id = unhashed.pop("diagnostic_id")
    claimed_identity = unhashed.pop("diagnostic_identity_sha256")
    if (
        not _is_sha256(claimed_identity)
        or not _is_result_id(claimed_id)
        or not _is_sha256(claimed_artifact)
        or sha256_bytes(canonical_json(unhashed)) != claimed_identity
        or claimed_id != claimed_identity[:24]
        or sha256_bytes(canonical_json({**unhashed, "diagnostic_identity_sha256": claimed_identity, "diagnostic_id": claimed_id})) != claimed_artifact
    ):
        raise ValueError("context diagnostic sidecar identity differs")
    strict_result = load_context_diagnostic_source_result(
        REPO_ROOT
        / "experiments/opcode-gas/derivations"
        / _STRICT_SOURCE_RESULT_ID
    )
    strict_identity, strict_rows, strict_rows_sha256 = _source_result_parts(
        strict_result
    )
    if (
        sidecar["source_result_id"] != _STRICT_SOURCE_RESULT_ID
        or canonical_json(sidecar["source_result_identity"])
        != canonical_json(strict_identity)
        or sidecar["source_result_identity_sha256"]
        != _STRICT_SOURCE_RESULT_IDENTITY_SHA256
        or sidecar["source_result_artifact_sha256"]
        != _STRICT_SOURCE_RESULT_ARTIFACT_SHA256
        or sidecar["source_rows_sha256"] != strict_rows_sha256
    ):
        raise ValueError("context diagnostic sidecar strict source identity differs")
    strict_selected_rows = select_context_diagnostic_rows(strict_rows)
    rows = sidecar["rows"]
    if (
        not isinstance(rows, Mapping)
        or set(rows) != {row["row_id"] for row in strict_selected_rows}
    ):
        raise ValueError("context diagnostic sidecar row inventory differs")
    labels = []
    for row_id, entry in rows.items():
        if not _is_sha256(row_id) or not isinstance(entry, Mapping):
            raise ValueError("context diagnostic sidecar row identity differs")
        if set(entry) != {
            "source_row_sha256",
            "scenario",
            "split",
            "count",
            "lane",
            "repeat_index",
            "workload_id",
            "backend_input_sha256",
            "report_sha256",
            "diagnostics",
        } or any(
            not _is_sha256(entry[field])
            for field in (
                "source_row_sha256",
                "workload_id",
                "backend_input_sha256",
                "report_sha256",
            )
        ):
            raise ValueError("context diagnostic sidecar row schema differs")
        diagnostics = entry.get("diagnostics")
        if not isinstance(diagnostics, Mapping) or extract_sp1_diagnostics(diagnostics) != dict(diagnostics):
            raise ValueError("context diagnostic sidecar metrics differ")
        labels.append((entry.get("scenario"), entry.get("count"), entry.get("lane")))
        source_row = next(
            row for row in strict_selected_rows if row["row_id"] == row_id
        )
        expected_source_fields = {
            "source_row_sha256": sha256_bytes(canonical_json(source_row)),
            "scenario": source_row["scenario"],
            "split": source_row["split"],
            "count": source_row["count"],
            "lane": source_row["lane"],
            "repeat_index": source_row["repeat_index"],
            "workload_id": source_row["workload_id"],
            "backend_input_sha256": source_row["backend_input_sha256"],
        }
        if any(entry[field] != value for field, value in expected_source_fields.items()):
            raise ValueError("context diagnostic sidecar source row differs")
    expected_labels = [
        (scenario, count, lane)
        for scenario, count in CONTEXT_DIAGNOSTIC_PANEL
        for lane in ("control", "target")
    ]
    if len(set(labels)) != len(expected_labels) or set(labels) != set(expected_labels):
        raise ValueError("context diagnostic sidecar panel differs")
    if result is not None:
        source_identity, source_rows, source_rows_sha256 = _source_result_parts(result)
        if (
            sidecar["source_result_id"] != result["result_id"]
            or canonical_json(sidecar["source_result_identity"]) != canonical_json(source_identity)
            or sidecar["source_result_identity_sha256"] != result["result_identity_sha256"]
            or sidecar["source_result_artifact_sha256"] != result["artifact_sha256"]
            or sidecar["source_rows_sha256"] != source_rows_sha256
        ):
            raise ValueError("context diagnostic source result mismatch")
        selected = select_context_diagnostic_rows(source_rows)
        if set(rows) != {row["row_id"] for row in selected}:
            raise ValueError("context diagnostic source row mismatch")
        for row in selected:
            entry = rows[row["row_id"]]
            if entry.get("source_row_sha256") != sha256_bytes(canonical_json(row)):
                raise ValueError("context diagnostic source row mismatch")
    return dict(sidecar)


def load_context_diagnostic_source_result(directory: pathlib.Path) -> dict[str, object]:
    """Read one portable result without changing its source rows or resealing it."""
    import context_production_campaign as production

    directory = pathlib.Path(directory)
    raw = production._read_result_directory(directory)
    envelope = production._load_canonical_json_bytes(raw["result.json"], label="context diagnostic result")
    manifest = production.ProductionContextManifest.from_mapping(
        production._load_canonical_json_bytes(
            raw["campaign-manifest.json"], label="context diagnostic manifest"
        )
    )
    rows_raw, rows = production._load_canonical_rows(directory / "rows.jsonl")
    identity = envelope.get("result_identity")
    envelope_without_artifact = dict(envelope)
    claimed_artifact = envelope_without_artifact.pop("artifact_sha256", None)
    if (
        not isinstance(identity, Mapping)
        or raw["rows.jsonl"] != rows_raw
        or not _is_result_id(envelope.get("result_id"))
        or not _is_sha256(envelope.get("result_identity_sha256"))
        or envelope.get("result_identity_sha256") != sha256_bytes(canonical_json(identity))
        or envelope.get("result_id") != envelope.get("result_identity_sha256")[:24]
        or not _is_sha256(claimed_artifact)
        or claimed_artifact != sha256_bytes(canonical_json(envelope_without_artifact))
        or not isinstance(identity.get("file_sha256s"), Mapping)
        or any(
            name not in raw or digest != sha256_bytes(raw[name])
            for name, digest in identity["file_sha256s"].items()
        )
        or identity.get("file_sha256s", {}).get("rows.jsonl") != sha256_bytes(rows_raw)
        or envelope.get("result_id") != _STRICT_SOURCE_RESULT_ID
        or envelope.get("result_identity_sha256")
        != _STRICT_SOURCE_RESULT_IDENTITY_SHA256
        or claimed_artifact != _STRICT_SOURCE_RESULT_ARTIFACT_SHA256
        or sha256_bytes(rows_raw) != _STRICT_SOURCE_ROWS_SHA256
    ):
        raise ValueError("context diagnostic source result rows differ")
    return {
        **envelope,
        "rows": rows,
        "source_rows_sha256": sha256_bytes(rows_raw),
        "manifest": manifest,
    }


def _write_create_only(path: pathlib.Path, payload: bytes) -> None:
    import context_production_campaign as production

    value = json.loads(payload)
    if canonical_json(value) + b"\n" != payload:
        raise ValueError("context diagnostic output is not canonical JSON")
    production._write_json_create_only(path, value)


def _remove_interrupted_atomic_temps(
    directory: pathlib.Path, *, final_names: Iterable[str]
) -> None:
    """Remove only same-directory temp files left before an atomic hard-link publish."""
    prefixes = tuple(f".{name}." for name in final_names)
    for entry in directory.iterdir():
        if not (
            entry.name.endswith(".tmp")
            and entry.name.startswith(prefixes)
        ):
            continue
        if entry.is_symlink() or not entry.is_file():
            raise ValueError("context diagnostic interrupted temporary output differs")
        entry.unlink()


def _run_diagnostic_report(
    *,
    launcher: pathlib.Path,
    manifest: object,
    row: Mapping[str, object],
    output: pathlib.Path,
) -> Mapping[str, object]:
    import context_production_campaign as production

    row_input = production._reconstruct_portable_row_input(
        manifest,
        row,
    )
    with tempfile.TemporaryDirectory(prefix="raiko2-context-diagnostic-") as temporary:
        source = pathlib.Path(temporary) / "source-row.json"
        report_path = pathlib.Path(temporary) / "report.jsonl"
        source.write_bytes(canonical_json(row_input["builder_input"]) + b"\n")
        subprocess.run(
            [
                str(launcher), "--stage", "controlled-block", "--proof-type", "sp1",
                "--mode", "execute", "--sp1-prover", "local",
                "--sp1-execution-engine", "gas-estimator", "--input", str(source),
                "--jsonl-out", str(report_path),
            ],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
            timeout=3600,
        )
        lines = report_path.read_bytes().splitlines()
        if len(lines) != 1:
            raise ValueError("context diagnostic launcher emitted an unexpected row count")
        report = json.loads(lines[0])
    _validated_report_for_row(report, row)
    _write_create_only(output, canonical_json(report) + b"\n")
    return report


def run_context_diagnostics(
    *, result_directory: pathlib.Path, launcher: pathlib.Path, out: pathlib.Path
) -> dict[str, object]:
    """Execute or exactly resume the bounded diagnostics panel, preserving source rows."""
    result_directory = pathlib.Path(result_directory).resolve(strict=True)
    launcher = pathlib.Path(launcher).resolve(strict=True)
    out = pathlib.Path(out).resolve(strict=False)
    if not launcher.is_file():
        raise ValueError("context diagnostic launcher does not exist")
    if out == result_directory or out.is_relative_to(result_directory):
        raise ValueError("context diagnostic output overlaps source result")
    source = load_context_diagnostic_source_result(result_directory)
    _source_identity, _source_rows, source_rows_sha256 = _source_result_parts(source)
    selected = select_context_diagnostic_rows(source["rows"])
    production_elf = REPO_ROOT / "crates/guests/elf/sp1_shasta_proposal.elf"
    source_elf_sha256s = {row["sp1_proposal_elf_sha256"] for row in selected}
    if (
        not production_elf.is_file()
        or len(source_elf_sha256s) != 1
        or sha256_file(production_elf) != next(iter(source_elf_sha256s))
    ):
        raise ValueError("context diagnostic production ELF differs from source result")
    source_launcher_sha256s = {row["guest_launcher_sha256"] for row in selected}
    if (
        len(source_launcher_sha256s) != 1
        or sha256_file(launcher) != next(iter(source_launcher_sha256s))
    ):
        raise ValueError("context diagnostic launcher differs from source result")
    state = {
        "source_result_identity_sha256": source["result_identity_sha256"],
        "source_rows_sha256": source_rows_sha256,
        "guest_launcher_sha256": sha256_file(launcher),
    }
    if out.exists() or out.is_symlink():
        if out.is_symlink():
            raise ValueError("context diagnostic output must not be a symlink")
        state_path = out / "run.json"
        reports_directory = out / "reports"
        if not out.is_dir() or (out / "diagnostics.json").exists():
            raise ValueError("context diagnostic output already exists")
        _remove_interrupted_atomic_temps(
            out, final_names=("run.json", "diagnostics.json")
        )
        entries = {entry.name for entry in out.iterdir()}
        if entries - {"reports", "run.json"}:
            raise ValueError("context diagnostic output inventory differs")
        if not state_path.exists():
            if reports_directory.exists() and (
                reports_directory.is_symlink()
                or not reports_directory.is_dir()
                or any(reports_directory.iterdir())
            ):
                raise ValueError("context diagnostic interrupted initialization differs")
            reports_directory.mkdir(mode=0o700, exist_ok=True)
            _write_create_only(state_path, canonical_json(state) + b"\n")
        elif not state_path.is_file() or json.loads(state_path.read_bytes()) != state:
            raise ValueError("context diagnostic resume identity differs")
        if reports_directory.is_symlink() or not reports_directory.is_dir():
            raise ValueError("context diagnostic reports directory differs")
        _remove_interrupted_atomic_temps(
            reports_directory,
            final_names=(f"{row['row_id']}.json" for row in selected),
        )
        expected_report_names = {f"{row['row_id']}.json" for row in selected}
        if {entry.name for entry in reports_directory.iterdir()} - expected_report_names:
            raise ValueError("context diagnostic report inventory differs")
    else:
        out.mkdir(parents=True, mode=0o700)
        (out / "reports").mkdir(mode=0o700)
        _write_create_only(out / "run.json", canonical_json(state) + b"\n")
    reports = []
    for row in selected:
        report_path = out / "reports" / f"{row['row_id']}.json"
        if report_path.exists():
            report = json.loads(report_path.read_bytes())
        else:
            report = _run_diagnostic_report(
                launcher=launcher,
                manifest=source["manifest"],
                row=row,
                output=report_path,
            )
        reports.append(report)
    sidecar = build_context_diagnostic_sidecar(source, reports)
    _write_create_only(out / "diagnostics.json", canonical_json(sidecar) + b"\n")
    return sidecar


def verify_context_diagnostics_path(path: pathlib.Path) -> dict[str, object]:
    path = pathlib.Path(path)
    sidecar_path = path / "diagnostics.json" if path.is_dir() else path
    sidecar = json.loads(sidecar_path.read_bytes())
    verified = verify_context_diagnostic_sidecar(sidecar)
    reports_directory = sidecar_path.parent / "reports"
    if path.is_dir():
        if reports_directory.is_symlink() or not reports_directory.is_dir():
            raise ValueError("context diagnostic reports directory is missing")
        expected_reports = {f"{row_id}.json" for row_id in sidecar["rows"]}
        if {entry.name for entry in reports_directory.iterdir()} != expected_reports:
            raise ValueError("context diagnostic report inventory differs")
        for row_id, entry in sidecar["rows"].items():
            report_path = reports_directory / f"{row_id}.json"
            if report_path.is_symlink() or not report_path.is_file():
                raise ValueError("context diagnostic report is missing")
            raw_report = report_path.read_bytes()
            report = json.loads(raw_report)
            if raw_report != canonical_json(report) + b"\n":
                raise ValueError("context diagnostic report is not canonical")
            if entry["report_sha256"] != sha256_bytes(canonical_json(report)):
                raise ValueError("context diagnostic report hash differs")
            diagnostics = _validated_report_for_row(
                report,
                {
                    "row_id": row_id,
                    "backend_input_sha256": entry["backend_input_sha256"],
                    "sp1_proposal_elf_sha256": sidecar["sp1_proposal_elf_sha256"],
                    "guest_launcher_sha256": sidecar["guest_launcher_sha256"],
                },
            )
            if canonical_json(diagnostics) != canonical_json(entry["diagnostics"]):
                raise ValueError("context diagnostic report diagnostics differ")
    return verified
