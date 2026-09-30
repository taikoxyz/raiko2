"""Replayable, non-coefficient SP1 diagnostics for context calibration rows."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import tempfile
from collections.abc import Iterable, Mapping
from decimal import Context, Decimal, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
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
_STRICT_SOURCE_RESULT_JSON_SHA256 = (
    "634783f4518151ad16aa5f92022f54cb1e5d258e85a5b16196159bfdc9c285a2"
)
_STRICT_SOURCE_DECISIONS_SHA256 = (
    "2d0c82e67e8ccb86849475fb27df46432fb3c1a1f520d95346e02da326f17fc2"
)
_STRICT_SOURCE_MANIFEST_IDENTITY_SHA256 = (
    "00c375a82a9a5b98a7011af26e603ab53c9779fae83199f81100154382c8be50"
)
_APPROXIMATION_FILE = "context-approximation.json"
_APPROXIMATION_CLASS_BY_MODEL = {
    "address_constant": "address",
    "caller_constant": "caller",
    "callvalue_zero": "callvalue:zero",
    "callvalue_nonzero": "callvalue:nonzero",
    "load_zero": "calldataload:zero",
    "load_partial": "calldataload:partial",
    "load_full": "calldataload:full",
    "calldata_size": "calldatasize",
    "timestamp_nonzero": "timestamp",
}
_APPROXIMATION_CLASSES = frozenset(_APPROXIMATION_CLASS_BY_MODEL.values())
_FRACTION_DECIMAL_CONTEXT = Context(prec=100, rounding=ROUND_HALF_EVEN)

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


def _load_resumed_diagnostic_report(
    path: pathlib.Path, row: Mapping[str, object]
) -> dict[str, object]:
    """Accept only a complete canonical report that is bound to its source row."""
    if path.is_symlink() or not path.is_file():
        raise ValueError("context diagnostic resumed report is not a regular file")
    raw = path.read_bytes()
    try:
        report = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ValueError("context diagnostic resumed report is invalid JSON") from error
    if raw != canonical_json(report) + b"\n":
        raise ValueError("context diagnostic resumed report is not canonical")
    if not isinstance(report, Mapping):
        raise ValueError("context diagnostic resumed report is not an object")
    _validated_report_for_row(report, row)
    return dict(report)


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
        if report_path.exists() or report_path.is_symlink():
            report = _load_resumed_diagnostic_report(report_path, row)
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


def _fraction_decimal_text(value: Fraction) -> str:
    with localcontext(_FRACTION_DECIMAL_CONTEXT):
        projected = Decimal(value.numerator) / Decimal(value.denominator)
    return format(projected, "f")


def _exact_fraction_payload(value: Fraction) -> dict[str, str]:
    if not isinstance(value, Fraction):
        raise TypeError("declared approximation exact value must be a Fraction")
    return {
        "numerator": str(value.numerator),
        "denominator": str(value.denominator),
        "decimal": _fraction_decimal_text(value),
    }


def _replay_exact_fraction(value: object) -> Fraction:
    if not isinstance(value, Mapping) or set(value) != {
        "numerator",
        "denominator",
        "decimal",
    }:
        raise ValueError("declared approximation exact fraction shape differs")
    numerator = value["numerator"]
    denominator = value["denominator"]
    if not isinstance(numerator, str) or not isinstance(denominator, str):
        raise ValueError("declared approximation exact fraction differs")
    try:
        exact = Fraction(int(numerator), int(denominator))
    except (ValueError, ZeroDivisionError) as error:
        raise ValueError("declared approximation exact fraction differs") from error
    if (
        str(exact.numerator) != numerator
        or str(exact.denominator) != denominator
        or value["decimal"] != _fraction_decimal_text(exact)
    ):
        raise ValueError("declared approximation exact fraction is noncanonical")
    return exact


def _fraction_from_decimal(value: Decimal) -> Fraction:
    if not isinstance(value, Decimal) or not value.is_finite():
        raise ValueError("declared approximation Decimal differs")
    return Fraction(value)


def _strict_source_directory() -> pathlib.Path:
    return (
        REPO_ROOT
        / "experiments/opcode-gas/derivations"
        / _STRICT_SOURCE_RESULT_ID
    )


def load_context_approximation_source_result(
    directory: pathlib.Path,
) -> dict[str, object]:
    """Load the one immutable rejected result allowed to feed approximation costs."""
    import context_production_campaign as production

    directory = pathlib.Path(directory)
    try:
        source = load_context_diagnostic_source_result(directory)
        raw = production._read_result_directory(directory)
    except ValueError as error:
        raise ValueError("declared approximation strict source differs") from error
    source_file_sha256s = {name: sha256_bytes(data) for name, data in raw.items()}
    if (
        directory.name != _STRICT_SOURCE_RESULT_ID
        or source.get("result_identity_sha256")
        != _STRICT_SOURCE_RESULT_IDENTITY_SHA256
        or source.get("artifact_sha256") != _STRICT_SOURCE_RESULT_ARTIFACT_SHA256
        or source_file_sha256s.get("result.json")
        != _STRICT_SOURCE_RESULT_JSON_SHA256
        or source_file_sha256s.get("rows.jsonl") != _STRICT_SOURCE_ROWS_SHA256
        or source_file_sha256s.get("campaign-decisions.json")
        != _STRICT_SOURCE_DECISIONS_SHA256
        or set(source_file_sha256s) != production.PRODUCTION_CONTEXT_RESULT_INVENTORY
        or source.get("result_status") != "rejected"
        or source.get("candidate_eligible") is not False
        or source.get("promoted_families") != []
    ):
        raise ValueError("declared approximation strict source differs")
    try:
        decisions = json.loads(raw["campaign-decisions.json"])
    except json.JSONDecodeError as error:  # pragma: no cover - fixed hash already gates this.
        raise ValueError("declared approximation strict source decisions differ") from error
    if canonical_json(decisions) + b"\n" != raw["campaign-decisions.json"]:
        raise ValueError("declared approximation strict source decisions differ")
    return {
        **source,
        "decisions": decisions,
        "source_file_sha256s": source_file_sha256s,
    }


def _strict_rejection_references() -> dict[str, object]:
    source = load_context_approximation_source_result(_strict_source_directory())
    decisions = source["decisions"]
    families = decisions.get("families") if isinstance(decisions, Mapping) else None
    if not isinstance(families, Mapping) or set(families) != {
        "opcode:0x30",
        "opcode:0x33",
        "opcode:0x34",
        "opcode:0x35",
        "opcode:0x36",
        "opcode:0x42",
    }:
        raise ValueError("declared approximation strict rejection inventory differs")
    result = {}
    for key, family in families.items():
        reasons = family.get("rejection_reasons") if isinstance(family, Mapping) else None
        if family.get("status") != "rejected" or not isinstance(reasons, list) or not reasons:
            raise ValueError("declared approximation strict rejection differs")
        result[key] = {
            "status": "rejected",
            "rejection_reasons": list(reasons),
        }
    return result


def _residual_measurement_ledgers(
    row: Mapping[str, object], *, key: str, count: int, opcode: int
) -> tuple[dict[str, int], dict[str, int]]:
    import context_production_campaign as production

    return production._subtract_measurement_contribution(
        row["actual_raw_gas_by_key"],
        row["actual_operation_event_count_by_key"],
        key=key,
        count=count,
        per_event_raw_gas=production._measurement_raw_gas(opcode),
    )


def _replacement_cost(
    row: Mapping[str, object],
    residual_raw: Mapping[str, int],
    residual_events: Mapping[str, int],
    count: int,
    subtotal_model: object,
) -> Fraction:
    import context_production_campaign as production

    raw_delta = {
        key: row["actual_raw_gas_by_key"].get(key, 0) - residual_raw.get(key, 0)
        for key in set(row["actual_raw_gas_by_key"]) | set(residual_raw)
    }
    event_delta = {
        key: row["actual_operation_event_count_by_key"].get(key, 0)
        - residual_events.get(key, 0)
        for key in set(row["actual_operation_event_count_by_key"]) | set(residual_events)
    }
    raw_delta = {key: value for key, value in raw_delta.items() if value}
    event_delta = {key: value for key, value in event_delta.items() if value}
    if (
        set(raw_delta) != set(event_delta)
        or len(raw_delta) != 1
        or next(iter(event_delta.values())) != count
    ):
        raise ValueError("declared approximation replacement ledger differs")
    key = next(iter(raw_delta))
    raw_units, remainder = divmod(raw_delta[key], count)
    if remainder:
        raise ValueError("declared approximation replacement raw gas differs")
    isolated = {
        "actual_features": {name: 0 for name in subtotal_model.fixed_costs},
        "actual_raw_gas_by_key": {key: raw_units},
        "actual_operation_event_count_by_key": {key: 1},
        "actual_typed_opcode_components_by_key": {},
    }
    replacement = _fraction_from_decimal(
        production.evaluate_v5_subtotal(
            isolated, subtotal_model, excluded_target_key=None
        )
    )
    if replacement < 0:
        raise ValueError("declared approximation replacement cost is negative")
    return replacement


def build_declared_context_approximation(
    manifest: object,
    rows: Iterable[Mapping[str, object]],
    subtotal_model: object,
) -> dict[str, object]:
    """Build the conservative review-only context costs from exact paired rows."""
    import context_production_campaign as production

    if (
        not isinstance(manifest, production.ProductionContextManifest)
        or manifest.identity_sha256 != _STRICT_SOURCE_MANIFEST_IDENTITY_SHA256
    ):
        raise ValueError("declared approximation manifest differs from the strict source")
    if not isinstance(subtotal_model, production.ProductionSubtotalModel):
        raise ValueError("declared approximation V5 subtotal model differs")
    raw_rows = list(rows)
    production._reject_forbidden_fit_fields(raw_rows)
    reject_context_diagnostics_from_coefficient_input(raw_rows)
    if any(not isinstance(row, Mapping) for row in raw_rows):
        raise ValueError("declared approximation rows must be objects")
    row_ids = [row.get("row_id") for row in raw_rows]
    if len(row_ids) != len(set(row_ids)):
        raise ValueError("duplicate declared approximation row")
    normalized = [production._normalized_fit_row(row) for row in raw_rows]
    measured_scenarios = {
        scenario.name: scenario
        for scenario in manifest.scenarios
        if scenario.reachability == "measured"
    }
    if any(row["scenario"] not in measured_scenarios for row in normalized):
        raise ValueError("declared approximation row scenario differs")
    grouped: dict[tuple[str, int, str, int], dict[str, object]] = {}
    for row in normalized:
        scenario = measured_scenarios[row["scenario"]]
        if row["split"] != scenario.split or row["count"] not in scenario.counts(manifest):
            raise ValueError("declared approximation row split or count differs")
        key = (row["scenario"], row["count"], row["lane"], row["repeat_index"])
        if key in grouped:
            raise ValueError("duplicate declared approximation pair member")
        grouped[key] = row

    operation_by_key = {operation.key: operation for operation in manifest.operations}
    paired: dict[tuple[str, int, int], dict[str, object]] = {}
    for scenario_name, scenario in measured_scenarios.items():
        operation = operation_by_key[scenario.key]
        control_key = f"opcode:0x{operation.control_opcode:02x}"
        for count in scenario.counts(manifest):
            for repeat in range(manifest.repeats):
                target = grouped.get((scenario_name, count, "target", repeat))
                control = grouped.get((scenario_name, count, "control", repeat))
                if target is None or control is None:
                    missing_lane = "target" if target is None else "control"
                    raise ValueError(
                        f"missing declared approximation row or repeat for {scenario_name} count {count} {missing_lane}"
                    )
                expected_context = (
                    {production._expected_context_feature(operation, scenario): count}
                    if count
                    else {}
                )
                if (
                    target["actual_context_features"] != expected_context
                    or control["actual_context_features"] != {}
                ):
                    raise ValueError("declared approximation exact context event differs")
                target_raw, target_events = _residual_measurement_ledgers(
                    target,
                    key=scenario.key,
                    count=count,
                    opcode=operation.opcode,
                )
                control_raw, control_events = _residual_measurement_ledgers(
                    control,
                    key=control_key,
                    count=count,
                    opcode=operation.control_opcode,
                )
                if (
                    target_raw != control_raw
                    or target_events != control_events
                    or target["actual_features"] != control["actual_features"]
                    or target["actual_diagnostics"] != control["actual_diagnostics"]
                ):
                    raise ValueError("declared approximation residual non-target ledger differs")
                paired[(scenario_name, count, repeat)] = {
                    "target": target,
                    "control": control,
                    "replacement_cost": (
                        _replacement_cost(
                            control,
                            control_raw,
                            control_events,
                            count,
                            subtotal_model,
                        )
                        if count
                        else None
                    ),
                }
    if len(grouped) != sum(
        len(scenario.counts(manifest)) * 2 * manifest.repeats
        for scenario in measured_scenarios.values()
    ):
        raise ValueError("declared approximation row inventory differs")

    scenario_costs: dict[str, object] = {}
    observed_rows: list[dict[str, object]] = []
    for scenario_name in sorted(measured_scenarios):
        scenario = measured_scenarios[scenario_name]
        increments: list[
            tuple[int, int, Fraction, Fraction, Mapping[str, object]]
        ] = []
        for repeat in range(manifest.repeats):
            zero = paired[(scenario_name, 0, repeat)]
            for count in scenario.counts(manifest):
                if count == 0:
                    continue
                point = paired[(scenario_name, count, repeat)]
                target = point["target"]
                control = point["control"]
                delta = (
                    Fraction(target["prover_gas"])
                    - Fraction(control["prover_gas"])
                    - Fraction(zero["target"]["prover_gas"])
                    + Fraction(zero["control"]["prover_gas"])
                )
                replacement = point["replacement_cost"]
                increments.append((repeat, count, delta, replacement, point))
        denominator = sum((count * count for _, count, _, _, _ in increments), 0)
        if denominator == 0:
            raise ValueError("declared approximation scenario has no nonzero count")
        slope = sum(
            (
                Fraction(count) * delta
                for _, count, delta, _, _ in increments
            ),
            Fraction(),
        ) / denominator
        replacements = {replacement for _, _, _, replacement, _ in increments}
        if len(replacements) != 1:
            raise ValueError("declared approximation scenario replacement cost differs")
        replacement = next(iter(replacements))
        scenario_cost = slope + replacement
        class_id = _APPROXIMATION_CLASS_BY_MODEL.get(scenario.model_class)
        if class_id is None:
            raise ValueError("declared approximation scenario class differs")
        scenario_costs[scenario_name] = {
            "class": class_id,
            "production_schedule_key": scenario.key,
            "split": scenario.split,
            "through_origin_slope_exact": _exact_fraction_payload(slope),
            "replacement_cost_exact": _exact_fraction_payload(replacement),
            "scenario_cost_exact": _exact_fraction_payload(scenario_cost),
        }
        for repeat, count, delta, replacement, point in increments:
            observed_rows.append(
                {
                    "scenario": scenario_name,
                    "split": scenario.split,
                    "class": class_id,
                    "count": count,
                    "repeat_index": repeat,
                    "target_row_id": point["target"]["row_id"],
                    "control_row_id": point["control"]["row_id"],
                    "target_prover_gas_exact": _exact_fraction_payload(
                        Fraction(point["target"]["prover_gas"])
                    ),
                    "observed_target_control_marginal_exact": _exact_fraction_payload(
                        delta
                    ),
                    "replacement_marginal_exact": _exact_fraction_payload(
                        Fraction(count) * replacement
                    ),
                    "observed_absolute_marginal_exact": _exact_fraction_payload(
                        delta + Fraction(count) * replacement
                    ),
                }
            )

    classes = {}
    for class_id in sorted(_APPROXIMATION_CLASSES):
        members = sorted(
            name
            for name, cost in scenario_costs.items()
            if cost["class"] == class_id
        )
        if not members:
            raise ValueError("declared approximation class inventory differs")
        selected = max(
            (_replay_exact_fraction(scenario_costs[name]["scenario_cost_exact"]) for name in members),
            default=Fraction(),
        )
        selected = max(Fraction(), selected)
        production_keys = {scenario_costs[name]["production_schedule_key"] for name in members}
        if len(production_keys) != 1:
            raise ValueError("declared approximation class production key differs")
        classes[class_id] = {
            "status": "declared_approximation",
            "production_schedule_key": next(iter(production_keys)),
            "shape": "constant_per_execution",
            "selection": "max_zero_and_scenario_costs",
            "scenario_ids": members,
            "cost_exact": _exact_fraction_payload(selected),
        }
    if set(classes) != _APPROXIMATION_CLASSES:
        raise ValueError("declared approximation class inventory differs")

    for row in observed_rows:
        predicted = Fraction(row["count"]) * _replay_exact_fraction(
            classes[row["class"]]["cost_exact"]
        )
        observed = _replay_exact_fraction(row["observed_absolute_marginal_exact"])
        error = predicted - observed
        target_prover_gas = _replay_exact_fraction(row["target_prover_gas_exact"])
        if target_prover_gas <= 0:
            raise ValueError("declared approximation target prover gas is not positive")
        materiality = abs(error) / target_prover_gas
        row["predicted_marginal_exact"] = _exact_fraction_payload(predicted)
        row["error_exact"] = _exact_fraction_payload(error)
        row["whole_guest_materiality"] = _fraction_decimal_text(materiality)

    return {
        "schema_version": 1,
        "purpose": "declared_context_approximation_model",
        "policy": "declared_approximation",
        "arithmetic": "exact_fraction_with_decimal_projection",
        "calldatasize_semantics": "constant_per_execution_calldata_ingestion_tx_owned",
        "strict_rejections": _strict_rejection_references(),
        "classes": classes,
        "scenario_costs": scenario_costs,
        "controlled_rows": observed_rows,
    }


def _approximation_envelope(source: Mapping[str, object], model: Mapping[str, object]) -> dict[str, object]:
    source_identity = source.get("result_identity")
    source_hashes = source.get("source_file_sha256s")
    if not isinstance(source_identity, Mapping) or not isinstance(source_hashes, Mapping):
        raise ValueError("declared approximation strict source provenance differs")
    base = {
        **dict(model),
        "schema_version": 1,
        "purpose": "sealed_declared_context_approximation",
        "source_result_id": source["result_id"],
        "source_result_identity": dict(source_identity),
        "source_result_identity_sha256": source["result_identity_sha256"],
        "source_result_artifact_sha256": source["artifact_sha256"],
        "source_file_sha256s": dict(source_hashes),
    }
    identity = sha256_bytes(canonical_json(base))
    with_identity = {
        **base,
        "approximation_identity_sha256": identity,
        "approximation_id": identity[:24],
    }
    return {
        **with_identity,
        "artifact_sha256": sha256_bytes(canonical_json(with_identity)),
    }


def _fsync_directory(path: pathlib.Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _publish_approximation_create_only(
    out_root: pathlib.Path, artifact: Mapping[str, object]
) -> pathlib.Path:
    import ctypes
    import errno

    out_root = pathlib.Path(out_root)
    if out_root.exists() and (out_root.is_symlink() or not out_root.is_dir()):
        raise ValueError("declared approximation output root differs")
    out_root.mkdir(parents=True, exist_ok=True)
    artifact_id = artifact["approximation_id"]
    if not _is_result_id(artifact_id):
        raise ValueError("declared approximation identity differs")
    destination = out_root / artifact_id
    if destination.exists() or destination.is_symlink():
        raise ValueError(f"declared approximation directory already exists: {destination}")
    temporary = pathlib.Path(tempfile.mkdtemp(prefix=f".{artifact_id}.", dir=out_root))
    try:
        path = temporary / _APPROXIMATION_FILE
        _write_create_only(path, canonical_json(dict(artifact)) + b"\n")
        path.chmod(0o444)
        descriptor = os.open(path, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        _fsync_directory(temporary)
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
        if rename_noreplace(
            -100, os.fsencode(temporary), -100, os.fsencode(destination), 1
        ) != 0:
            error_number = ctypes.get_errno()
            if error_number == errno.EEXIST:
                raise ValueError(
                    f"declared approximation directory already exists: {destination}"
                )
            raise OSError(error_number, "declared approximation create-only publish failed")
        temporary = None
        _fsync_directory(out_root)
        return destination
    finally:
        if temporary is not None and temporary.exists():
            shutil.rmtree(temporary)


def seal_context_approximation(
    *, source_result: pathlib.Path, out_root: pathlib.Path
) -> dict[str, object]:
    """Replay the strict source and create one immutable approximation directory."""
    import context_production_campaign as production

    source = load_context_approximation_source_result(source_result)
    subtotal = production.load_production_v5_subtotal_model(source["manifest"], REPO_ROOT)
    model = build_declared_context_approximation(
        source["manifest"], source["rows"], subtotal
    )
    artifact = _approximation_envelope(source, model)
    destination = _publish_approximation_create_only(out_root, artifact)
    return {
        "status": "sealed",
        "approximation_id": artifact["approximation_id"],
        "artifact_sha256": artifact["artifact_sha256"],
        "directory": str(destination),
    }


def verify_context_approximation_path(path: pathlib.Path) -> dict[str, object]:
    """Verify content addressing and exact replay from the immutable strict source."""
    import context_production_campaign as production

    directory = pathlib.Path(path)
    if (
        directory.is_symlink()
        or not directory.is_dir()
        or {entry.name for entry in directory.iterdir()} != {_APPROXIMATION_FILE}
    ):
        raise ValueError("declared approximation directory inventory differs")
    artifact_path = directory / _APPROXIMATION_FILE
    if artifact_path.is_symlink() or not artifact_path.is_file():
        raise ValueError("declared approximation file differs")
    raw = artifact_path.read_bytes()
    try:
        artifact = json.loads(raw)
    except json.JSONDecodeError as error:
        raise ValueError("declared approximation JSON differs") from error
    if raw != canonical_json(artifact) + b"\n":
        raise ValueError("declared approximation JSON is not canonical")
    if (
        artifact.get("schema_version") != 1
        or artifact.get("purpose") != "sealed_declared_context_approximation"
    ):
        raise ValueError("declared approximation schema differs")
    artifact_without_hash = dict(artifact)
    claimed_artifact = artifact_without_hash.pop("artifact_sha256", None)
    claimed_id = artifact_without_hash.pop("approximation_id", None)
    claimed_identity = artifact_without_hash.pop("approximation_identity_sha256", None)
    if (
        not _is_sha256(claimed_artifact)
        or not _is_result_id(claimed_id)
        or not _is_sha256(claimed_identity)
        or claimed_identity != sha256_bytes(canonical_json(artifact_without_hash))
        or claimed_id != claimed_identity[:24]
        or claimed_artifact
        != sha256_bytes(
            canonical_json(
                {
                    **artifact_without_hash,
                    "approximation_identity_sha256": claimed_identity,
                    "approximation_id": claimed_id,
                }
            )
        )
        or directory.name != claimed_id
    ):
        raise ValueError("declared approximation identity differs")
    for section in (artifact.get("classes"), artifact.get("scenario_costs")):
        if not isinstance(section, Mapping):
            raise ValueError("declared approximation exact fraction inventory differs")
        for entry in section.values():
            if not isinstance(entry, Mapping):
                raise ValueError("declared approximation exact fraction inventory differs")
            for field, value in entry.items():
                if field.endswith("_exact"):
                    _replay_exact_fraction(value)
    for row in artifact.get("controlled_rows", ()):
        if not isinstance(row, Mapping):
            raise ValueError("declared approximation controlled row differs")
        for field, value in row.items():
            if field.endswith("_exact"):
                _replay_exact_fraction(value)
    source = load_context_approximation_source_result(_strict_source_directory())
    subtotal = production.load_production_v5_subtotal_model(source["manifest"], REPO_ROOT)
    expected = _approximation_envelope(
        source,
        build_declared_context_approximation(source["manifest"], source["rows"], subtotal),
    )
    if canonical_json(expected) != canonical_json(artifact):
        raise ValueError("declared approximation exact replay differs")
    return dict(artifact)
