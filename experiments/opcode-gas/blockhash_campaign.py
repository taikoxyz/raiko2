#!/usr/bin/env python3
"""Frozen production-guest BLOCKHASH calibration campaign.

This is deliberately independent from the production-context campaign.  It
prepares only controlled `BLOCKHASH`/`ISZERO` proposal rows; it neither reads
the final-60 smoke corpus nor updates an operation table or composite model.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation, localcontext
from typing import Any, Callable, Mapping, Sequence


SCHEMA_VERSION = 1
PURPOSE = "production_blockhash_calibration"
OPERATION_OWNERSHIP_SCHEMA_VERSION = 4
FIT_COUNTS = (0, 1, 2, 4, 8, 16, 32)
CHECKPOINT_COUNT = 64
REPEATS = 3
SEMANTIC_CLASSES = (
    "recent_ancestor_hit_1",
    "recent_ancestor_hit_256",
    "out_of_range_zero",
)
_SEMANTIC_CLASS_OFFSETS = {
    "recent_ancestor_hit_1": 1,
    "recent_ancestor_hit_256": 256,
    "out_of_range_zero": 0,
}
_SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
_ZERO_OUTPUT = "0x" + "00" * 32
CANONICAL_FIT_REGISTRY_SHA256 = (
    "35380cd8e78cc11332e080c322a1af205beb020bd78764f9930b4ff2d242c06a"
)

SOURCES = {
    "operation_coverage_v7": {
        "path": "experiments/opcode-gas/manifests/operation-coverage-v7.json",
        "file_sha256": "9ef6541f39eaded53e47c0231abb6c062883e211cfea3ae193b7f0c78b203009",
        "artifact_sha256": "4a84dc8289de2ce7ca1e0e93faf8d91611b0e47f12d257ed59f14e7d3aa21353",
        "schema_version": 4,
    },
    "production_registry": {
        "path": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
        "file_sha256": "0a85afe5f21af2599823cb0031087f4701e598acc3f7ca48896769b32b518234",
        "artifact_sha256": "1c05166e674a66ed51e3b1991c597ede479657e3195774a2960b1ff1cdd5ba4a",
        "fit_registry_sha256": CANONICAL_FIT_REGISTRY_SHA256,
    },
}

_SEAL_SOURCE_PATHS = (
    "experiments/opcode-gas/blockhash_campaign.py",
    "experiments/opcode-gas/opcode_gas.py",
    "bin/guest-launcher/src/controlled_workload.rs",
    "crates/zkgas-trace/src/reconstruct.rs",
)

EXECUTION = {
    "stage": "controlled-block",
    "proof_type": "sp1",
    "mode": "execute",
    "sp1_prover": "local",
    "sp1_execution_engine": "gas-estimator",
    "launcher_path": "target/release/guest-launcher",
    "production_elf_path": "crates/guests/elf/sp1_shasta_proposal.elf",
    "production_vk_path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
    "production_vk_binding": "provenance_only_not_used_by_gas_estimator",
    "trace_schema_source": "crates/zkgas-trace/src/reconstruct.rs",
    "implementation_revision_binding": "exact_head_bytes_for_campaign_sources",
    "launcher_binding": "binary_sha256_reported_by_launcher_no_source_revision_claim",
    "repeat_contract": "exact_every_field_except_row_identity",
}

QUALITY_GATES = {
    "positive_signal": True,
    "exact_repeats": REPEATS,
    "fit_ape_max": "0.10",
    "checkpoint_ape_max": "0.10",
    "count_zero_execution_delta": "exact_zero",
    "fit": "through_origin_paired_differential",
    "partial_application": "forbidden",
    "failure_status": "unmeasured",
}


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with pathlib.Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _decimal(value: Any, *, label: str) -> Decimal:
    if type(value) not in (str, int):
        raise ValueError(f"{label} must be an exact decimal string or integer")
    try:
        parsed = Decimal(value)
    except (InvalidOperation, ValueError) as error:
        raise ValueError(f"{label} is not a decimal") from error
    if not parsed.is_finite():
        raise ValueError(f"{label} must be finite")
    return parsed


def _decimal_text(value: Decimal) -> str:
    if not value.is_finite():
        raise ValueError("non-finite decimal is not canonical")
    rendered = format(value, "f")
    if "." in rendered:
        rendered = rendered.rstrip("0").rstrip(".")
    return "0" if rendered in {"-0", ""} else rendered


def _reject_floats(value: Any, *, label: str = "payload") -> None:
    if isinstance(value, float):
        raise ValueError(f"{label} contains a binary float")
    if isinstance(value, Mapping):
        for key, member in value.items():
            _reject_floats(member, label=f"{label}.{key}")
    elif isinstance(value, list):
        for index, member in enumerate(value):
            _reject_floats(member, label=f"{label}[{index}]")


def _load_canonical_json(path: pathlib.Path, *, label: str) -> dict[str, Any]:
    raw = pathlib.Path(path).read_bytes()
    try:
        payload = json.loads(raw, parse_float=lambda _: (_ for _ in ()).throw(ValueError()))
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as error:
        raise ValueError(f"{label} is invalid JSON") from error
    _reject_floats(payload, label=label)
    pretty = json.dumps(payload, sort_keys=True, indent=2).encode() + b"\n"
    if not isinstance(payload, dict) or raw not in {canonical_json(payload) + b"\n", pretty}:
        raise ValueError(f"{label} is not canonical")
    return payload


def canonical_blockhash_manifest_payload() -> dict[str, Any]:
    """The immutable task-2 panel; generated rather than copied from context V5."""
    return {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "operation_ownership_schema_version": OPERATION_OWNERSHIP_SCHEMA_VERSION,
        "fit_counts": list(FIT_COUNTS),
        "checkpoint_count": CHECKPOINT_COUNT,
        "repeats": REPEATS,
        "semantic_classes": [
            {
                "semantic_class": name,
                "number_expression": f"NUMBER - {offset}",
                "offset": offset,
                "expected_result": "ancestor_hash"
                if offset in {1, 256}
                else "zero_current_block",
            }
            for name, offset in _SEMANTIC_CLASS_OFFSETS.items()
        ],
        "pair_program": {
            "target": "NUMBER; PUSH3 offset; SUB; BLOCKHASH; POP",
            "control": "NUMBER; PUSH3 offset; SUB; ISZERO; POP",
            "measurement_byte_delta": {"target": "0x40", "control": "0x15"},
            "equal": [
                "loop",
                "stack_shape",
                "bytecode_length",
                "gas_limit",
                "transaction_envelope",
                "block_envelope",
                "non_target_operation_counts",
            ],
            "ancestor_window_headers": 256,
        },
        "sources": json.loads(json.dumps(SOURCES)),
        "execution": json.loads(json.dumps(EXECUTION)),
        "quality_gates": json.loads(json.dumps(QUALITY_GATES)),
    }


def blockhash_manifest_identity(manifest: Mapping[str, Any]) -> str:
    _validate_manifest(manifest)
    return sha256_bytes(canonical_json(manifest))


def _validate_manifest(manifest: Mapping[str, Any]) -> None:
    _reject_floats(manifest, label="BLOCKHASH manifest")
    expected = canonical_blockhash_manifest_payload()
    if canonical_json(manifest) != canonical_json(expected):
        raise ValueError("BLOCKHASH campaign manifest differs from the frozen task-2 panel")


@dataclass(frozen=True)
class BlockHashRowSpec:
    semantic_class: str
    count: int
    split: str
    lane: str
    repeat_index: int
    workload_id: str
    row_id: str


def blockhash_workload_id(manifest: Mapping[str, Any], semantic_class: str, count: int) -> str:
    _validate_manifest(manifest)
    if semantic_class not in SEMANTIC_CLASSES or type(count) is not int or count not in (*FIT_COUNTS, CHECKPOINT_COUNT):
        raise ValueError("BLOCKHASH workload semantic class or count differs")
    return sha256_bytes(
        canonical_json(
            {
                "kind": "blockhash_context_workload_v1",
                "manifest_identity_sha256": blockhash_manifest_identity(manifest),
                "semantic_class": semantic_class,
                "count": count,
            }
        )
    )


def blockhash_row_id(*, workload_id: str, lane: str, repeat_index: int) -> str:
    if _SHA256_RE.fullmatch(workload_id) is None or lane not in {"target", "control"}:
        raise ValueError("BLOCKHASH row identity input differs")
    if type(repeat_index) is not int or repeat_index not in range(REPEATS):
        raise ValueError("BLOCKHASH repeat index is outside the frozen range")
    return sha256_bytes(
        canonical_json(
            {
                "kind": "blockhash_context_row_v1",
                "workload_id": workload_id,
                "lane": lane,
                "repeat_index": repeat_index,
            }
        )
    )


def blockhash_row_specs(manifest: Mapping[str, Any]) -> tuple[BlockHashRowSpec, ...]:
    _validate_manifest(manifest)
    rows = []
    for semantic_class in SEMANTIC_CLASSES:
        for count in (*FIT_COUNTS, CHECKPOINT_COUNT):
            workload_id = blockhash_workload_id(manifest, semantic_class, count)
            split = "fit" if count in FIT_COUNTS else "holdout"
            for lane in ("control", "target"):
                for repeat_index in range(REPEATS):
                    rows.append(
                        BlockHashRowSpec(
                            semantic_class,
                            count,
                            split,
                            lane,
                            repeat_index,
                            workload_id,
                            blockhash_row_id(
                                workload_id=workload_id,
                                lane=lane,
                                repeat_index=repeat_index,
                            ),
                        )
                    )
    return tuple(rows)


def blockhash_fixture_request(row: BlockHashRowSpec) -> dict[str, Any]:
    return {
        "row_id": row.row_id,
        "operation_ownership_schema_version": OPERATION_OWNERSHIP_SCHEMA_VERSION,
        "workload_family": "blockhash_context",
        "split": row.split,
        "block_count": 1,
        "transaction_count": 1,
        "program": {
            "kind": "blockhash_loop",
            "workload_id": row.workload_id,
            "repeat_index": row.repeat_index,
            "lane": row.lane,
            "count": row.count,
            "profile": {"semantic_class": row.semantic_class},
        },
        "expected_final_state_root": _ZERO_OUTPUT,
        "expected_raw_gas_by_key": {},
        "expected_operation_event_count_by_key": {},
        "expected_context_features": {},
        "expected_features": {},
        "expected_diagnostics": {},
    }


def validate_blockhash_sources(repo_root: pathlib.Path) -> dict[str, Any]:
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    resolved: dict[str, Any] = {}
    for name, source in SOURCES.items():
        path = repo_root / source["path"]
        if not path.is_file() or sha256_file(path) != source["file_sha256"]:
            raise ValueError(f"BLOCKHASH source {name} differs")
        payload = _load_canonical_json(path, label=f"BLOCKHASH source {name}")
        if payload.get("artifact_sha256") != source["artifact_sha256"]:
            raise ValueError(f"BLOCKHASH source {name} artifact differs")
        resolved[name] = payload
    coverage = resolved["operation_coverage_v7"]
    if coverage.get("schema_version") != 4 or coverage.get("ownership_predecessor") is None:
        raise ValueError("BLOCKHASH coverage source is not the corrected v7 successor")
    registry = resolved["production_registry"].get("registry")
    if not isinstance(registry, Mapping):
        raise ValueError("BLOCKHASH production registry is missing")
    fit_registry = _fit_registry(registry)
    if (
        sha256_bytes(canonical_json(fit_registry))
        != SOURCES["production_registry"]["fit_registry_sha256"]
    ):
        raise ValueError("BLOCKHASH production fit registry differs")
    return resolved


def _iszero_event_cost(registry: Mapping[str, Any]) -> Decimal:
    try:
        common = _decimal(registry["common_dispatch"], label="common dispatch")
        iszero = registry["models"]["opcode:0x15"]
        if iszero["kind"] != "static_raw_gas":
            raise ValueError("ISZERO is not a static raw-gas model")
        body = _decimal(
            iszero["parameters"]["body_per_raw_gas"], label="ISZERO body per raw gas"
        )
    except (KeyError, TypeError) as error:
        raise ValueError("sealed ISZERO control model is missing") from error
    with localcontext() as context:
        context.prec = 120
        return common + Decimal(3) * body


def _fit_registry(registry: Mapping[str, Any]) -> dict[str, Any]:
    """Keep exactly the sealed registry inputs needed to replay the fit."""
    _iszero_event_cost(registry)
    try:
        body = registry["models"]["opcode:0x15"]["parameters"]["body_per_raw_gas"]
        kind = registry["models"]["opcode:0x15"]["kind"]
        common = registry["common_dispatch"]
    except (KeyError, TypeError) as error:
        raise ValueError("sealed ISZERO control model is missing") from error
    return {
        "common_dispatch": _decimal_text(_decimal(common, label="common dispatch")),
        "models": {
            "opcode:0x15": {
                "kind": kind,
                "parameters": {
                    "body_per_raw_gas": _decimal_text(
                        _decimal(body, label="ISZERO body per raw gas")
                    )
                },
            }
        },
    }


def _signed_delta(target: Mapping[str, Any], control: Mapping[str, Any], *, label: str) -> dict[str, int]:
    if not isinstance(target, Mapping) or not isinstance(control, Mapping):
        raise ValueError(f"BLOCKHASH {label} ledger differs")
    keys = set(target) | set(control)
    result = {}
    for key in sorted(keys):
        left, right = target.get(key, 0), control.get(key, 0)
        if type(left) is not int or type(right) is not int:
            raise ValueError(f"BLOCKHASH {label} ledger must contain exact integers")
        value = left - right
        if value:
            result[key] = value
    return result


def _row_key(row: Mapping[str, Any]) -> tuple[str, int, str, int]:
    try:
        semantic_class = row["semantic_class"]
        count, lane, repeat_index = row["count"], row["lane"], row["repeat_index"]
    except KeyError as error:
        raise ValueError("BLOCKHASH row is missing identity fields") from error
    if (
        semantic_class not in SEMANTIC_CLASSES
        or type(count) is not int
        or count not in (*FIT_COUNTS, CHECKPOINT_COUNT)
        or lane not in {"target", "control"}
        or type(repeat_index) is not int
        or repeat_index not in range(REPEATS)
    ):
        raise ValueError("BLOCKHASH row identity differs")
    return semantic_class, count, lane, repeat_index


def _normalize_rows(rows: Sequence[Mapping[str, Any]]) -> dict[tuple[str, int, str, int], dict[str, Any]]:
    expected = {
        (semantic_class, count, lane, repeat)
        for semantic_class in SEMANTIC_CLASSES
        for count in (*FIT_COUNTS, CHECKPOINT_COUNT)
        for lane in ("target", "control")
        for repeat in range(REPEATS)
    }
    indexed = {}
    for row in rows:
        if not isinstance(row, Mapping):
            raise ValueError("BLOCKHASH row is not an object")
        _reject_floats(row, label="BLOCKHASH row")
        key = _row_key(row)
        if key in indexed:
            raise ValueError("duplicate BLOCKHASH row")
        semantic_class, count, lane, repeat_index = key
        workload_id = blockhash_workload_id(
            canonical_blockhash_manifest_payload(), semantic_class, count
        )
        if (
            row.get("workload_id") != workload_id
            or row.get("row_id")
            != blockhash_row_id(
                workload_id=workload_id,
                lane=lane,
                repeat_index=repeat_index,
            )
            or row.get("split") != ("fit" if count in FIT_COUNTS else "holdout")
            or not isinstance(row.get("input_sha256"), str)
            or _SHA256_RE.fullmatch(row["input_sha256"]) is None
        ):
            raise ValueError("BLOCKHASH row canonical identity differs")
        _decimal(row.get("prover_gas"), label="BLOCKHASH prover gas")
        for field in ("public_output", "backend_input_sha256", "host_trace_sha256"):
            if not isinstance(row.get(field), str):
                raise ValueError(f"BLOCKHASH row {field} differs")
        indexed[key] = dict(row)
    if set(indexed) != expected:
        raise ValueError("BLOCKHASH row panel differs from the frozen campaign")
    return indexed


def _class_fit(
    semantic_class: str,
    rows: Mapping[tuple[str, int, str, int], Mapping[str, Any]],
    *,
    iszero_event_cost: Decimal,
) -> dict[str, Any]:
    reasons: list[str] = []
    observed: dict[int, Decimal] = {}
    for count in (*FIT_COUNTS, CHECKPOINT_COUNT):
        lane_rows = {
            lane: [rows[(semantic_class, count, lane, repeat)] for repeat in range(REPEATS)]
            for lane in ("target", "control")
        }
        for lane, repeated in lane_rows.items():
            baseline = {
                key: value
                for key, value in repeated[0].items()
                if key not in {"repeat_index", "row_id", "input_sha256", "evidence_sha256"}
            }
            if any(
                {
                    key: value
                    for key, value in row.items()
                    if key not in {"repeat_index", "row_id", "input_sha256", "evidence_sha256"}
                }
                != baseline
                for row in repeated[1:]
            ):
                reasons.append(f"{semantic_class}:{count}:{lane}:repeat instability")
        target, control = lane_rows["target"][0], lane_rows["control"][0]
        raw_delta = _signed_delta(
            target.get("actual_raw_gas_by_key"),
            control.get("actual_raw_gas_by_key"),
            label="raw-gas",
        )
        event_delta = _signed_delta(
            target.get("actual_operation_event_count_by_key"),
            control.get("actual_operation_event_count_by_key"),
            label="event",
        )
        expected_raw = {"opcode:0x40": 20 * count, "opcode:0x15": -3 * count}
        expected_event = {"opcode:0x40": count, "opcode:0x15": -count}
        expected_raw = {key: value for key, value in expected_raw.items() if value}
        expected_event = {key: value for key, value in expected_event.items() if value}
        if raw_delta != expected_raw:
            reasons.append(f"{semantic_class}:{count}:raw-gas ledger delta differs")
        if event_delta != expected_event:
            reasons.append(f"{semantic_class}:{count}:event ledger delta differs")
        with localcontext() as context:
            context.prec = 120
            observed[count] = _decimal(
                target["prover_gas"], label="target prover gas"
            ) - _decimal(control["prover_gas"], label="control prover gas")
    if observed[0] != 0:
        reasons.append(f"{semantic_class}:count zero has execution delta")
    with localcontext() as context:
        context.prec = 80
        denominator = sum(Decimal(count * count) for count in FIT_COUNTS if count > 0)
        slope = sum(
            Decimal(count) * observed[count] for count in FIT_COUNTS if count > 0
        ) / denominator
    with localcontext() as context:
        context.prec = 120
        event_cost = slope + iszero_event_cost
    fit_apes = {}
    for count in FIT_COUNTS:
        if count == 0:
            continue
        with localcontext() as context:
            context.prec = 120
            prediction = Decimal(count) * slope
            actual = observed[count]
        if actual <= 0:
            reasons.append(f"{semantic_class}:{count}:nonpositive paired signal")
            continue
        with localcontext() as context:
            context.prec = 120
            ape = abs(prediction - actual) / abs(actual)
        fit_apes[count] = ape
        if ape > Decimal("0.10"):
            reasons.append(f"{semantic_class}:{count}:fit APE exceeds 10%")
    with localcontext() as context:
        context.prec = 120
        checkpoint_prediction = Decimal(CHECKPOINT_COUNT) * slope
    checkpoint_actual = observed[CHECKPOINT_COUNT]
    checkpoint_ape = None
    if checkpoint_actual <= 0:
        reasons.append(f"{semantic_class}:checkpoint nonpositive paired signal")
    else:
        with localcontext() as context:
            context.prec = 120
            checkpoint_ape = abs(checkpoint_prediction - checkpoint_actual) / abs(
                checkpoint_actual
            )
        if checkpoint_ape > Decimal("0.10"):
            reasons.append(f"{semantic_class}:checkpoint APE exceeds 10%")
    if slope <= 0 or event_cost <= 0:
        reasons.append(f"{semantic_class}:positive signal gate failed")
    return {
        "semantic_class": semantic_class,
        "status": "accepted" if not reasons else "unmeasured",
        "slope": slope,
        "event_cost": event_cost,
        "observed": observed,
        "fit_apes": fit_apes,
        "checkpoint_ape": checkpoint_ape,
        "rejection_reasons": sorted(set(reasons)),
    }


def fit_blockhash_rows(
    rows: Sequence[Mapping[str, Any]], *, manifest: Mapping[str, Any], registry: Mapping[str, Any]
) -> dict[str, Any]:
    """Fit only the predeclared paired differences; never apply a partial result."""
    _validate_manifest(manifest)
    indexed = _normalize_rows(rows)
    iszero_event = _iszero_event_cost(registry)
    common_dispatch = _decimal(registry["common_dispatch"], label="common dispatch")
    class_reports = [
        _class_fit(semantic_class, indexed, iszero_event_cost=iszero_event)
        for semantic_class in SEMANTIC_CLASSES
    ]
    for report in class_reports:
        with localcontext() as context:
            context.prec = 120
            report["body_per_raw_gas"] = (
                report["event_cost"] - common_dispatch
            ) / Decimal(20)
    accepted = all(report["status"] == "accepted" for report in class_reports)
    selected = None
    reasons = [reason for report in class_reports for reason in report["rejection_reasons"]]
    if accepted:
        maximum = max(class_reports, key=lambda report: report["event_cost"])
        selected = {
            "key": "opcode:0x40",
            "model_kind": "static_raw_gas",
            "event_cost": _decimal_text(maximum["event_cost"]),
            "body_per_raw_gas": _decimal_text(maximum["body_per_raw_gas"]),
            "selected_semantic_class": maximum["semantic_class"],
            "selection": "maximum_accepted_event_cost",
        }
    result = {
        "schema_version": SCHEMA_VERSION,
        "purpose": "production_blockhash_fit_decisions",
        "manifest_identity_sha256": blockhash_manifest_identity(manifest),
        "status": "accepted" if selected is not None else "unmeasured",
        "control": {
            "key": "opcode:0x15",
            "raw_gas": 3,
            "sealed_event_cost": _decimal_text(iszero_event),
        },
        "classes": [
            {
                "semantic_class": report["semantic_class"],
                "status": report["status"],
                "paired_slope": _decimal_text(report["slope"]),
                "event_cost": _decimal_text(report["event_cost"]),
                "body_per_raw_gas": _decimal_text(report["body_per_raw_gas"]),
                "fit_apes": {
                    str(count): _decimal_text(ape)
                    for count, ape in report["fit_apes"].items()
                },
                "checkpoint_ape": (
                    _decimal_text(report["checkpoint_ape"])
                    if report["checkpoint_ape"] is not None
                    else None
                ),
                "rejection_reasons": report["rejection_reasons"],
            }
            for report in class_reports
        ],
        "selected": selected,
        "rejection_reasons": sorted(set(reasons)),
        "promotion": "forbidden_until_all_semantic_classes_are_accepted",
    }
    result["artifact_sha256"] = sha256_bytes(canonical_json(result))
    return result


def _write_create_only(path: pathlib.Path, payload: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = canonical_json(payload) + b"\n"
    try:
        with path.open("xb") as output:
            output.write(encoded)
    except FileExistsError:
        if path.read_bytes() != encoded:
            raise ValueError(f"create-only output differs: {path}")


def _git_source_identity(
    repo_root: pathlib.Path, implementation_revision: str
) -> dict[str, str]:
    """Require every executable source to equal the exact recorded commit."""
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    if re.fullmatch(r"[0-9a-f]{40}", implementation_revision) is None:
        raise ValueError("BLOCKHASH implementation revision must be an exact git SHA")
    try:
        head = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
    except subprocess.CalledProcessError as error:
        raise ValueError("BLOCKHASH implementation revision cannot be resolved") from error
    if head != implementation_revision:
        raise ValueError("BLOCKHASH implementation revision is not the current HEAD")
    identities = {}
    for relative in _SEAL_SOURCE_PATHS:
        path = repo_root / relative
        if not path.is_file():
            raise ValueError(f"BLOCKHASH implementation source is missing: {relative}")
        try:
            committed = subprocess.run(
                ["git", "-C", str(repo_root), "show", f"{implementation_revision}:{relative}"],
                check=True,
                capture_output=True,
            ).stdout
        except subprocess.CalledProcessError as error:
            raise ValueError(
                f"BLOCKHASH implementation source is absent from the recorded revision: {relative}"
            ) from error
        current_sha256 = sha256_file(path)
        if sha256_bytes(committed) != current_sha256:
            raise ValueError(
                f"BLOCKHASH implementation source differs from the recorded revision: {relative}"
            )
        identities[relative] = current_sha256
    return identities


def _asset_identity(
    path: pathlib.Path,
    *,
    repo_root: pathlib.Path,
    label: str,
    expected_path: str,
) -> dict[str, str]:
    path = pathlib.Path(path).resolve(strict=True)
    expected = (repo_root / expected_path).resolve(strict=True)
    if path != expected:
        raise ValueError(f"BLOCKHASH {label} must use {expected_path}")
    try:
        relative = path.relative_to(repo_root).as_posix()
    except ValueError as error:
        raise ValueError(f"BLOCKHASH {label} must be inside the repository") from error
    return {"path": relative, "sha256": sha256_file(path), "label": label}


def _launcher_asset_identity(path: pathlib.Path) -> dict[str, str]:
    """Bind the shared launcher by content without persisting its machine path."""
    path = pathlib.Path(path).resolve(strict=True)
    if not path.is_file():
        raise ValueError("BLOCKHASH guest launcher is not a file")
    return {
        "path": EXECUTION["launcher_path"],
        "sha256": sha256_file(path),
        "label": "guest launcher",
    }


def prepare_blockhash_run(
    *,
    repo_root: pathlib.Path,
    run: pathlib.Path,
    launcher: pathlib.Path,
    production_elf: pathlib.Path,
    production_vk: pathlib.Path,
    implementation_revision: str,
    identity_runner: Callable[[Mapping[str, Any]], Mapping[str, Any]],
) -> dict[str, Any]:
    """Freeze every native fixture before any gas-estimator execution begins."""
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    if re.fullmatch(r"[0-9a-f]{40}", implementation_revision) is None:
        raise ValueError("BLOCKHASH implementation revision must be an exact git SHA")
    manifest = canonical_blockhash_manifest_payload()
    sources = validate_blockhash_sources(repo_root)
    source_code_sha256s = _git_source_identity(repo_root, implementation_revision)
    launcher = pathlib.Path(launcher).resolve(strict=True)
    assets = {
        "launcher": _launcher_asset_identity(launcher),
        "production_elf": _asset_identity(
            production_elf,
            repo_root=repo_root,
            label="production ELF",
            expected_path=EXECUTION["production_elf_path"],
        ),
        "production_vk": _asset_identity(
            production_vk,
            repo_root=repo_root,
            label="production VK",
            expected_path=EXECUTION["production_vk_path"],
        ),
    }
    rows = []
    run = pathlib.Path(run)
    for row in blockhash_row_specs(manifest):
        request = blockhash_fixture_request(row)
        bundle = identity_runner(request)
        if not isinstance(bundle, Mapping) or bundle.get("spec") is None or bundle.get("observation") is None:
            raise ValueError("BLOCKHASH native fixture identity is incomplete")
        spec = bundle["spec"]
        observation = bundle["observation"]
        if spec.get("row_id") != row.row_id or observation.get("row_id") != row.row_id:
            raise ValueError("BLOCKHASH native fixture identity row differs")
        if spec.get("program") != request["program"]:
            raise ValueError("BLOCKHASH native fixture program differs")
        payload = {
            "row_id": row.row_id,
            "workload_id": row.workload_id,
            "semantic_class": row.semantic_class,
            "count": row.count,
            "split": row.split,
            "lane": row.lane,
            "repeat_index": row.repeat_index,
            "builder_input": spec,
            "fixture_spec_sha256": bundle.get("fixture_spec_sha256"),
        }
        payload["input_sha256"] = sha256_bytes(canonical_json(payload))
        _write_create_only(run / "row-inputs" / f"{row.row_id}.json", payload)
        rows.append({"row_id": row.row_id, "input_sha256": payload["input_sha256"]})
    if _launcher_asset_identity(launcher) != assets["launcher"]:
        raise ValueError("BLOCKHASH guest launcher changed during native fixture freeze")
    identity = {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "manifest": manifest,
        "manifest_identity_sha256": blockhash_manifest_identity(manifest),
        "sources": {
            name: {"artifact_sha256": payload["artifact_sha256"]}
            for name, payload in sources.items()
        },
        "assets": assets,
        "implementation_revision": implementation_revision,
        "source_code_sha256s": source_code_sha256s,
        "rows": rows,
    }
    identity["identity_sha256"] = sha256_bytes(canonical_json(identity))
    _write_create_only(run / "identity.json", identity)
    return identity


def _read_prepared_row(path: pathlib.Path) -> dict[str, Any]:
    payload = _load_canonical_json(path, label="prepared BLOCKHASH row")
    claimed = payload.get("input_sha256")
    unhashed = dict(payload)
    unhashed.pop("input_sha256", None)
    if claimed != sha256_bytes(canonical_json(unhashed)):
        raise ValueError("prepared BLOCKHASH row hash differs")
    return payload


def _validate_prepared_identity(run: pathlib.Path) -> dict[str, Any]:
    identity = _load_canonical_json(run / "identity.json", label="BLOCKHASH run identity")
    if (
        identity.get("schema_version") != SCHEMA_VERSION
        or identity.get("purpose") != PURPOSE
        or identity.get("manifest_identity_sha256")
        != blockhash_manifest_identity(canonical_blockhash_manifest_payload())
        or identity.get("identity_sha256") is None
    ):
        raise ValueError("prepared BLOCKHASH identity differs")
    unsigned = dict(identity)
    claimed = unsigned.pop("identity_sha256")
    if claimed != sha256_bytes(canonical_json(unsigned)):
        raise ValueError("prepared BLOCKHASH identity hash differs")
    expected = [row.row_id for row in blockhash_row_specs(canonical_blockhash_manifest_payload())]
    rows = identity.get("rows")
    if (
        not isinstance(rows, list)
        or any(
            not isinstance(row, Mapping)
            or set(row) != {"row_id", "input_sha256"}
            or _SHA256_RE.fullmatch(row.get("input_sha256", "")) is None
            for row in rows
        )
        or [row["row_id"] for row in rows] != expected
    ):
        raise ValueError("prepared BLOCKHASH row inventory differs")
    for row in rows:
        payload = _read_prepared_row(run / "row-inputs" / f"{row['row_id']}.json")
        if payload.get("input_sha256") != row.get("input_sha256"):
            raise ValueError("prepared BLOCKHASH row identity differs")
    return identity


def _validate_run_assets(
    identity: Mapping[str, Any], *, repo_root: pathlib.Path, launcher: pathlib.Path
) -> pathlib.Path:
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    assets = identity.get("assets")
    if not isinstance(assets, Mapping) or set(assets) != {
        "launcher",
        "production_elf",
        "production_vk",
    }:
        raise ValueError("prepared BLOCKHASH assets differ")
    expected_paths = {
        "launcher": EXECUTION["launcher_path"],
        "production_elf": EXECUTION["production_elf_path"],
        "production_vk": EXECUTION["production_vk_path"],
    }
    for name, expected_path in expected_paths.items():
        asset = assets[name]
        if not isinstance(asset, Mapping) or asset.get("path") != expected_path:
            raise ValueError(f"prepared BLOCKHASH {name} path differs")
        path = pathlib.Path(launcher).resolve(strict=True) if name == "launcher" else repo_root / expected_path
        if not path.is_file() or sha256_file(path) != asset.get("sha256"):
            raise ValueError(f"prepared BLOCKHASH {name} asset differs")
    return pathlib.Path(launcher).resolve(strict=True)


def _default_blockhash_executor(
    row: Mapping[str, Any], *, launcher: pathlib.Path, repo_root: pathlib.Path
) -> Mapping[str, Any]:
    with tempfile.TemporaryDirectory(prefix="raiko2-blockhash-") as directory:
        temporary = pathlib.Path(directory)
        source, output = temporary / "row.json", temporary / "report.jsonl"
        source.write_bytes(canonical_json(row["builder_input"]) + b"\n")
        subprocess.run(
            [
                str(launcher),
                "--stage",
                "controlled-block",
                "--proof-type",
                "sp1",
                "--mode",
                "execute",
                "--sp1-prover",
                "local",
                "--sp1-execution-engine",
                "gas-estimator",
                "--input",
                str(source),
                "--jsonl-out",
                str(output),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
            timeout=3600,
        )
        lines = output.read_text().splitlines()
        if len(lines) != 1:
            raise ValueError("BLOCKHASH launcher emitted an unexpected row count")
        return json.loads(lines[0])


def _normalize_execution_report(
    row: Mapping[str, Any],
    report: Mapping[str, Any],
    *,
    production_elf_sha256: str,
    launcher_sha256: str,
) -> dict[str, Any]:
    try:
        controlled = report["controlled_block"]
        observation = controlled["observation"]
        if (
            report["stage"] != "controlled-block"
            or report["mode"] != "execute"
            or report["sp1_execution_engine"] != "gas-estimator"
            or report["exit_code"] != 0
            or controlled["status"] != "accepted"
            or controlled["row_id"] != row["row_id"]
            or report["sp1_proposal_elf_sha256"] != production_elf_sha256
            or report["guest_launcher_sha256"] != launcher_sha256
            or observation["backend_input_sha256"] != report["guest_input_sha256"].removeprefix("0x")
        ):
            raise ValueError("BLOCKHASH execution report is not accepted")
        evidence = {
            key: row[key]
            for key in (
                "row_id",
                "workload_id",
                "semantic_class",
                "count",
                "split",
                "lane",
                "repeat_index",
                "input_sha256",
            )
        }
        evidence.update(
            {
                "prover_gas": _decimal_text(_decimal(report["gas"], label="BLOCKHASH prover gas")),
                "public_output": report["public_values"],
                "backend_input_sha256": observation["backend_input_sha256"],
                "host_trace_sha256": observation["host_trace_sha256"],
                "actual_raw_gas_by_key": observation["actual_raw_gas_by_key"],
                "actual_operation_event_count_by_key": observation[
                    "actual_operation_event_count_by_key"
                ],
                "sp1_proposal_elf_sha256": production_elf_sha256,
                "guest_launcher_sha256": launcher_sha256,
            }
        )
    except (KeyError, TypeError, AttributeError) as error:
        raise ValueError("BLOCKHASH execution report is incomplete") from error
    evidence["evidence_sha256"] = sha256_bytes(canonical_json(evidence))
    return _validate_execution_evidence(
        row,
        evidence,
        production_elf_sha256=production_elf_sha256,
        launcher_sha256=launcher_sha256,
    )


def _validate_execution_evidence(
    row: Mapping[str, Any],
    evidence: Mapping[str, Any],
    *,
    production_elf_sha256: str,
    launcher_sha256: str,
) -> dict[str, Any]:
    required = {
        "row_id",
        "workload_id",
        "semantic_class",
        "count",
        "split",
        "lane",
        "repeat_index",
        "input_sha256",
        "prover_gas",
        "public_output",
        "backend_input_sha256",
        "host_trace_sha256",
        "actual_raw_gas_by_key",
        "actual_operation_event_count_by_key",
        "sp1_proposal_elf_sha256",
        "guest_launcher_sha256",
        "evidence_sha256",
    }
    if not isinstance(evidence, Mapping) or set(evidence) != required:
        raise ValueError("BLOCKHASH evidence schema differs")
    unhashed = dict(evidence)
    claimed = unhashed.pop("evidence_sha256")
    if claimed != sha256_bytes(canonical_json(unhashed)):
        raise ValueError("BLOCKHASH evidence hash differs")
    for key in (
        "row_id",
        "workload_id",
        "semantic_class",
        "count",
        "split",
        "lane",
        "repeat_index",
        "input_sha256",
    ):
        if evidence[key] != row[key]:
            raise ValueError("BLOCKHASH evidence row binding differs")
    if (
        evidence["sp1_proposal_elf_sha256"] != production_elf_sha256
        or evidence["guest_launcher_sha256"] != launcher_sha256
    ):
        raise ValueError("BLOCKHASH evidence asset binding differs")
    _decimal(evidence["prover_gas"], label="BLOCKHASH evidence prover gas")
    if (
        not isinstance(evidence["public_output"], str)
        or _SHA256_RE.fullmatch(evidence["backend_input_sha256"]) is None
        or _SHA256_RE.fullmatch(evidence["host_trace_sha256"]) is None
    ):
        raise ValueError("BLOCKHASH evidence identity differs")
    _signed_delta(evidence["actual_raw_gas_by_key"], {}, label="raw-gas")
    _signed_delta(evidence["actual_operation_event_count_by_key"], {}, label="event")
    return dict(evidence)


def run_blockhash_campaign(
    run: pathlib.Path,
    *,
    repo_root: pathlib.Path,
    launcher: pathlib.Path,
    executor: Callable[[Mapping[str, Any]], Mapping[str, Any]] | None = None,
) -> dict[str, Any]:
    """Run or exactly resume the frozen panel with the gas-estimator only."""
    run = pathlib.Path(run)
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    identity = _validate_prepared_identity(run)
    if _git_source_identity(repo_root, identity["implementation_revision"]) != identity.get(
        "source_code_sha256s"
    ):
        raise ValueError("prepared BLOCKHASH implementation sources changed before execution")
    sources = validate_blockhash_sources(repo_root)
    if identity.get("sources") != {
        name: {"artifact_sha256": payload["artifact_sha256"]}
        for name, payload in sources.items()
    }:
        raise ValueError("prepared BLOCKHASH source binding differs")
    assets = identity["assets"]
    launcher = _validate_run_assets(identity, repo_root=repo_root, launcher=launcher)
    rows = []
    for entry in identity["rows"]:
        prepared = _read_prepared_row(run / "row-inputs" / f"{entry['row_id']}.json")
        destination = run / "rows" / f"{entry['row_id']}.json"
        if destination.exists():
            evidence = _load_canonical_json(destination, label="BLOCKHASH row evidence")
            evidence = _validate_execution_evidence(
                prepared,
                evidence,
                production_elf_sha256=assets["production_elf"]["sha256"],
                launcher_sha256=assets["launcher"]["sha256"],
            )
        else:
            report = (
                executor(prepared)
                if executor is not None
                else _default_blockhash_executor(
                    prepared,
                    launcher=launcher,
                    repo_root=repo_root,
                )
            )
            evidence = _normalize_execution_report(
                prepared,
                report,
                production_elf_sha256=assets["production_elf"]["sha256"],
                launcher_sha256=assets["launcher"]["sha256"],
            )
            _write_create_only(destination, evidence)
        rows.append({"row_id": entry["row_id"], "evidence_sha256": evidence.get("evidence_sha256")})
    terminal = {
        "schema_version": SCHEMA_VERSION,
        "status": "execution_complete",
        "identity_sha256": identity["identity_sha256"],
        "row_hashes": rows,
    }
    terminal["terminal_sha256"] = sha256_bytes(canonical_json(terminal))
    _write_create_only(run / "execution-complete.json", terminal)
    return terminal


def _load_execution_rows(run: pathlib.Path) -> list[dict[str, Any]]:
    identity = _validate_prepared_identity(run)
    rows = []
    for entry in identity["rows"]:
        evidence = _load_canonical_json(
            run / "rows" / f"{entry['row_id']}.json", label="BLOCKHASH row evidence"
        )
        prepared = _read_prepared_row(run / "row-inputs" / f"{entry['row_id']}.json")
        rows.append(
            _validate_execution_evidence(
                prepared,
                evidence,
                production_elf_sha256=identity["assets"]["production_elf"]["sha256"],
                launcher_sha256=identity["assets"]["launcher"]["sha256"],
            )
        )
    return rows


def _native_identity_runner(*, launcher: pathlib.Path, repo_root: pathlib.Path) -> Callable[[Mapping[str, Any]], Mapping[str, Any]]:
    def run(request: Mapping[str, Any]) -> Mapping[str, Any]:
        with tempfile.TemporaryDirectory(prefix="raiko2-blockhash-identity-") as directory:
            temporary = pathlib.Path(directory)
            source, output = temporary / "row.json", temporary / "identity.json"
            source.write_bytes(canonical_json(request) + b"\n")
            subprocess.run(
                [
                    str(launcher),
                    "--stage",
                    "controlled-block-identity",
                    "--proof-type",
                    "native",
                    "--mode",
                    "execute",
                    "--input",
                    str(source),
                    "--json-out",
                    str(output),
                ],
                cwd=repo_root,
                check=True,
                capture_output=True,
                text=True,
                timeout=300,
            )
            return _load_canonical_json(output, label="BLOCKHASH native fixture identity")

    return run


def _canonical_sealed_run_identity(
    identity: Mapping[str, Any], *, prepared_identity_sha256: str, prepared_rows_sha256: str
) -> dict[str, Any]:
    if (
        not isinstance(prepared_identity_sha256, str)
        or _SHA256_RE.fullmatch(prepared_identity_sha256) is None
        or not isinstance(prepared_rows_sha256, str)
        or _SHA256_RE.fullmatch(prepared_rows_sha256) is None
        or not isinstance(identity["implementation_revision"], str)
        or not re.fullmatch(r"[0-9a-f]{40}", identity["implementation_revision"])
    ):
        raise ValueError("BLOCKHASH run identity hash differs")
    assets = identity["assets"]
    if not isinstance(assets, Mapping) or set(assets) != {
        "launcher",
        "production_elf",
        "production_vk",
    }:
        raise ValueError("BLOCKHASH run identity assets differ")
    for name, asset in assets.items():
        if not isinstance(asset, Mapping) or _SHA256_RE.fullmatch(asset.get("sha256", "")) is None:
            raise ValueError(f"BLOCKHASH {name} asset hash differs")
    expected_paths = {
        "launcher": EXECUTION["launcher_path"],
        "production_elf": EXECUTION["production_elf_path"],
        "production_vk": EXECUTION["production_vk_path"],
    }
    for name, expected_path in expected_paths.items():
        if assets[name].get("path") != expected_path:
            raise ValueError(f"BLOCKHASH {name} asset path differs")
    source_code_sha256s = identity["source_code_sha256s"]
    if (
        not isinstance(source_code_sha256s, Mapping)
        or set(source_code_sha256s) != set(_SEAL_SOURCE_PATHS)
        or any(
            not isinstance(digest, str) or _SHA256_RE.fullmatch(digest) is None
            for digest in source_code_sha256s.values()
        )
    ):
        raise ValueError("BLOCKHASH source-code identity differs")
    sealed = {
        "prepared_identity_sha256": prepared_identity_sha256,
        "prepared_rows_sha256": prepared_rows_sha256,
        "implementation_revision": identity["implementation_revision"],
        "assets": {
            name: {
                "path": assets[name]["path"],
                "sha256": assets[name]["sha256"],
            }
            for name in sorted(assets)
        },
        "source_code_sha256s": {
            name: source_code_sha256s[name] for name in sorted(source_code_sha256s)
        },
    }
    sealed["identity_sha256"] = sha256_bytes(canonical_json(sealed))
    return sealed


def _row_input_inventory(rows: Sequence[Mapping[str, Any]]) -> list[dict[str, str]]:
    expected_rows = [
        row.row_id
        for row in blockhash_row_specs(canonical_blockhash_manifest_payload())
    ]
    inventory = []
    for row in rows:
        if (
            not isinstance(row, Mapping)
            or not isinstance(row.get("row_id"), str)
            or not isinstance(row.get("input_sha256"), str)
            or _SHA256_RE.fullmatch(row["input_sha256"]) is None
        ):
            raise ValueError("BLOCKHASH row input inventory differs")
        inventory.append(
            {"row_id": row["row_id"], "input_sha256": row["input_sha256"]}
        )
    if [row["row_id"] for row in inventory] != expected_rows:
        raise ValueError("BLOCKHASH row input inventory order differs")
    return inventory


def _row_input_inventory_sha256(rows: Sequence[Mapping[str, Any]]) -> str:
    return sha256_bytes(canonical_json(_row_input_inventory(rows)))


def _validate_prepared_run_identity_for_seal(identity: Mapping[str, Any]) -> dict[str, Any]:
    prepared_keys = {
        "schema_version",
        "purpose",
        "manifest",
        "manifest_identity_sha256",
        "sources",
        "assets",
        "implementation_revision",
        "source_code_sha256s",
        "rows",
        "identity_sha256",
    }
    if not isinstance(identity, Mapping) or set(identity) != prepared_keys:
        raise ValueError("prepared BLOCKHASH run identity schema differs at seal")
    unsigned = dict(identity)
    claimed = unsigned.pop("identity_sha256")
    manifest = canonical_blockhash_manifest_payload()
    expected_sources = {
        name: {"artifact_sha256": source["artifact_sha256"]}
        for name, source in SOURCES.items()
    }
    expected_rows = [row.row_id for row in blockhash_row_specs(manifest)]
    rows = identity["rows"]
    if (
        identity["schema_version"] != SCHEMA_VERSION
        or identity["purpose"] != PURPOSE
        or canonical_json(identity["manifest"]) != canonical_json(manifest)
        or identity["manifest_identity_sha256"] != blockhash_manifest_identity(manifest)
        or identity["sources"] != expected_sources
        or not isinstance(rows, list)
        or any(
            not isinstance(row, Mapping)
            or set(row) != {"row_id", "input_sha256"}
            or not isinstance(row["input_sha256"], str)
            or _SHA256_RE.fullmatch(row["input_sha256"]) is None
            for row in rows
        )
        or [row["row_id"] for row in rows] != expected_rows
        or claimed != sha256_bytes(canonical_json(unsigned))
    ):
        raise ValueError("prepared BLOCKHASH run identity differs at seal")
    return _canonical_sealed_run_identity(
        identity,
        prepared_identity_sha256=claimed,
        prepared_rows_sha256=_row_input_inventory_sha256(rows),
    )


def _validate_sealed_run_identity(identity: Mapping[str, Any]) -> dict[str, Any]:
    sealed_keys = {
        "identity_sha256",
        "prepared_identity_sha256",
        "prepared_rows_sha256",
        "implementation_revision",
        "assets",
        "source_code_sha256s",
    }
    if not isinstance(identity, Mapping) or set(identity) != sealed_keys:
        raise ValueError("sealed BLOCKHASH run identity schema differs")
    expected = _canonical_sealed_run_identity(
        identity,
        prepared_identity_sha256=identity["prepared_identity_sha256"],
        prepared_rows_sha256=identity["prepared_rows_sha256"],
    )
    if canonical_json(identity) != canonical_json(expected):
        raise ValueError("sealed BLOCKHASH run identity differs")
    return expected


def seal_blockhash_result(
    *,
    repo_root: pathlib.Path,
    out_root: pathlib.Path,
    run_identity: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
) -> dict[str, Any]:
    """Create a portable, content-addressed candidate without touching production tables."""
    repo_root = pathlib.Path(repo_root).resolve(strict=True)
    sources = validate_blockhash_sources(repo_root)
    identity = _validate_prepared_run_identity_for_seal(run_identity)
    current_source_code_sha256s = _git_source_identity(
        repo_root, identity["implementation_revision"]
    )
    if current_source_code_sha256s != identity["source_code_sha256s"]:
        raise ValueError("BLOCKHASH implementation sources changed after prepare")
    fit_registry = _fit_registry(sources["production_registry"]["registry"])
    execution_rows = [
        _validate_execution_evidence(
            row,
            row,
            production_elf_sha256=identity["assets"]["production_elf"]["sha256"],
            launcher_sha256=identity["assets"]["launcher"]["sha256"],
        )
        for row in rows
    ]
    if (
        _row_input_inventory_sha256(execution_rows)
        != identity["prepared_rows_sha256"]
    ):
        raise ValueError("BLOCKHASH execution rows differ from prepared inputs")
    decision = fit_blockhash_rows(
        execution_rows,
        manifest=canonical_blockhash_manifest_payload(),
        registry=fit_registry,
    )
    envelope = {
        "schema_version": SCHEMA_VERSION,
        "purpose": "production_blockhash_calibration_result",
        "status": decision["status"],
        "run_identity": identity,
        "manifest_identity_sha256": blockhash_manifest_identity(
            canonical_blockhash_manifest_payload()
        ),
        "sources": {
            name: {
                key: SOURCES[name][key]
                for key in (
                    "path",
                    "file_sha256",
                    "artifact_sha256",
                    "fit_registry_sha256",
                )
                if key in SOURCES[name]
            }
            for name, payload in sources.items()
        },
        "source_code_sha256s": identity["source_code_sha256s"],
        "fit_registry": fit_registry,
        "execution_rows": execution_rows,
        "execution_rows_sha256": sha256_bytes(canonical_json(execution_rows)),
        "decision": decision,
        "production_table_updated": False,
        "composite_estimator_updated": False,
        "final_60_opened": False,
    }
    envelope["result_identity_sha256"] = sha256_bytes(canonical_json(envelope))
    envelope["result_id"] = envelope["result_identity_sha256"][:24]
    envelope["artifact_sha256"] = sha256_bytes(canonical_json(envelope))
    directory = pathlib.Path(out_root) / envelope["result_id"]
    _write_create_only(directory / "result.json", envelope)
    return {"result_id": envelope["result_id"], "status": envelope["status"], "directory": str(directory)}


def verify_blockhash_result(directory: pathlib.Path) -> dict[str, Any]:
    directory = pathlib.Path(directory)
    payload = _load_canonical_json(directory / "result.json", label="BLOCKHASH result")
    expected_keys = {
        "schema_version",
        "purpose",
        "status",
        "run_identity",
        "manifest_identity_sha256",
        "sources",
        "source_code_sha256s",
        "fit_registry",
        "execution_rows",
        "execution_rows_sha256",
        "decision",
        "production_table_updated",
        "composite_estimator_updated",
        "final_60_opened",
        "result_identity_sha256",
        "result_id",
        "artifact_sha256",
    }
    if set(payload) != expected_keys or payload["schema_version"] != SCHEMA_VERSION:
        raise ValueError("BLOCKHASH result schema differs")
    unsigned = dict(payload)
    artifact = unsigned.pop("artifact_sha256")
    if artifact != sha256_bytes(canonical_json(unsigned)):
        raise ValueError("BLOCKHASH result artifact hash differs")
    result_id = unsigned.pop("result_id")
    identity = unsigned.pop("result_identity_sha256")
    if (
        identity != sha256_bytes(canonical_json(unsigned))
        or result_id != identity[:24]
        or directory.name != result_id
    ):
        raise ValueError("BLOCKHASH result identity differs")
    if (
        payload["purpose"] != "production_blockhash_calibration_result"
        or payload["manifest_identity_sha256"]
        != blockhash_manifest_identity(canonical_blockhash_manifest_payload())
        or payload["production_table_updated"] is not False
        or payload["composite_estimator_updated"] is not False
        or payload["final_60_opened"] is not False
    ):
        raise ValueError("BLOCKHASH result safety invariants differ")
    expected_sources = {
        name: {
            key: source[key]
            for key in (
                "path",
                "file_sha256",
                "artifact_sha256",
                "fit_registry_sha256",
            )
            if key in source
        }
        for name, source in SOURCES.items()
    }
    if payload["sources"] != expected_sources:
        raise ValueError("BLOCKHASH result source identity differs")
    run_identity = _validate_sealed_run_identity(payload["run_identity"])
    if payload["source_code_sha256s"] != run_identity["source_code_sha256s"]:
        raise ValueError("BLOCKHASH result source-code identity differs")
    execution_rows = payload["execution_rows"]
    if (
        not isinstance(execution_rows, list)
        or payload["execution_rows_sha256"]
        != sha256_bytes(canonical_json(execution_rows))
    ):
        raise ValueError("BLOCKHASH execution rows differ")
    execution_rows = [
        _validate_execution_evidence(
            row,
            row,
            production_elf_sha256=run_identity["assets"]["production_elf"]["sha256"],
            launcher_sha256=run_identity["assets"]["launcher"]["sha256"],
        )
        for row in execution_rows
    ]
    if (
        _row_input_inventory_sha256(execution_rows)
        != run_identity["prepared_rows_sha256"]
    ):
        raise ValueError("BLOCKHASH execution rows differ from prepared inputs")
    fit_registry = _fit_registry(payload["fit_registry"])
    if (
        canonical_json(fit_registry) != canonical_json(payload["fit_registry"])
        or sha256_bytes(canonical_json(fit_registry))
        != CANONICAL_FIT_REGISTRY_SHA256
    ):
        raise ValueError("BLOCKHASH fit registry differs")
    replayed = fit_blockhash_rows(
        execution_rows,
        manifest=canonical_blockhash_manifest_payload(),
        registry=fit_registry,
    )
    if canonical_json(replayed) != canonical_json(payload["decision"]):
        raise ValueError("BLOCKHASH decision does not replay from execution rows")
    if payload["status"] != replayed["status"]:
        raise ValueError("BLOCKHASH result status differs")
    return payload


def main(argv: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("generate-manifest")
    generate.add_argument("--out", type=pathlib.Path, required=True)
    prepare = commands.add_parser("prepare")
    prepare.add_argument("--repo-root", type=pathlib.Path, required=True)
    prepare.add_argument("--run", type=pathlib.Path, required=True)
    prepare.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    prepare.add_argument("--production-elf", type=pathlib.Path, required=True)
    prepare.add_argument("--production-vk", type=pathlib.Path, required=True)
    run = commands.add_parser("run")
    run.add_argument("--repo-root", type=pathlib.Path, required=True)
    run.add_argument("--run", type=pathlib.Path, required=True)
    run.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    fit = commands.add_parser("fit")
    fit.add_argument("--repo-root", type=pathlib.Path, required=True)
    fit.add_argument("--run", type=pathlib.Path, required=True)
    seal = commands.add_parser("seal")
    seal.add_argument("--repo-root", type=pathlib.Path, required=True)
    seal.add_argument("--run", type=pathlib.Path, required=True)
    seal.add_argument("--out-root", type=pathlib.Path, required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("--result", type=pathlib.Path, required=True)
    args = parser.parse_args(argv)
    if args.command == "generate-manifest":
        _write_create_only(args.out, canonical_blockhash_manifest_payload())
        print(blockhash_manifest_identity(canonical_blockhash_manifest_payload()))
    elif args.command == "prepare":
        repo_root = args.repo_root.resolve(strict=True)
        launcher = args.guest_launcher.resolve(strict=True)
        revision = subprocess.run(
            ["git", "-C", str(repo_root), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        identity = prepare_blockhash_run(
            repo_root=repo_root,
            run=args.run,
            launcher=launcher,
            production_elf=args.production_elf,
            production_vk=args.production_vk,
            implementation_revision=revision,
            identity_runner=_native_identity_runner(
                launcher=launcher, repo_root=repo_root
            ),
        )
        print(identity["identity_sha256"])
    elif args.command == "run":
        terminal = run_blockhash_campaign(
            args.run,
            repo_root=args.repo_root,
            launcher=args.guest_launcher,
        )
        print(terminal["terminal_sha256"])
    elif args.command == "fit":
        sources = validate_blockhash_sources(args.repo_root)
        decision = fit_blockhash_rows(
            _load_execution_rows(args.run),
            manifest=canonical_blockhash_manifest_payload(),
            registry=sources["production_registry"]["registry"],
        )
        _write_create_only(args.run / "fit.json", decision)
        print(decision["artifact_sha256"])
    elif args.command == "seal":
        identity = _validate_prepared_identity(args.run)
        sealed = seal_blockhash_result(
            repo_root=args.repo_root,
            out_root=args.out_root,
            run_identity=identity,
            rows=_load_execution_rows(args.run),
        )
        print(sealed["result_id"])
    elif args.command == "verify":
        print(verify_blockhash_result(args.result)["result_id"])


if __name__ == "__main__":
    main()
