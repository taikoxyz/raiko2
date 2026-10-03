"""Fail-closed Anchor-operation gap analysis for corrected ownership schema 4.

This is deliberately an analysis artifact, not a composite estimator: it only
reports which Anchor operations are already priced by sealed inputs and which
ones still block a corrected higher-layer fit.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from decimal import Decimal
from pathlib import Path
from typing import Any, Mapping, Sequence

import blockhash_campaign
import composite_estimator
import opcode_gas


SCHEMA_VERSION = 1
PURPOSE = "anchor_operation_gap_histogram"
OWNERSHIP_SCHEMA_VERSION = 4
_SHA256_LENGTH = 64
_SOURCE_PATHS = {
    "operation_coverage": "experiments/opcode-gas/manifests/operation-coverage-v7.json",
    "core": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
    "storage_result": "experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0/result.json",
    "context": "experiments/opcode-gas/derivations/1d2758bcb7aa3ae7f09d14eb/context-approximation.json",
    "blockhash_result": "experiments/opcode-gas/calibrations/sp1-blockhash-v2/6af515a6377aaf0c2906b151/result.json",
}


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def _sha256(value: Any) -> str:
    return hashlib.sha256(_canonical_json(value)).hexdigest()


def _file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _require_sha256(value: Any, *, label: str) -> str:
    if not isinstance(value, str) or len(value) != _SHA256_LENGTH:
        raise ValueError(f"{label} must be a SHA256")
    try:
        int(value, 16)
    except ValueError as error:
        raise ValueError(f"{label} must be a SHA256") from error
    return value


def _decimal(value: Any, *, label: str) -> Decimal:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a canonical decimal")
    try:
        parsed = Decimal(value)
    except Exception as error:  # Decimal has several implementation exceptions.
        raise ValueError(f"{label} must be a canonical decimal") from error
    if not parsed.is_finite() or format(parsed, "f") != value:
        raise ValueError(f"{label} must be a canonical decimal")
    return parsed


def _decimal_text(value: Decimal) -> str:
    return format(value, "f")


def _content_addressed(payload: Mapping[str, Any], *, label: str) -> None:
    artifact = payload.get("artifact_sha256")
    _require_sha256(artifact, label=f"{label} artifact")
    expected = _sha256({key: value for key, value in payload.items() if key != "artifact_sha256"})
    if artifact != expected:
        raise ValueError(f"{label} artifact SHA256 differs")


def _load_json(path: Path, *, label: str) -> dict[str, Any]:
    if not path.is_file():
        raise ValueError(f"{label} source is missing")
    try:
        value = json.loads(path.read_text())
    except json.JSONDecodeError as error:
        raise ValueError(f"{label} source JSON is invalid") from error
    if not isinstance(value, dict):
        raise ValueError(f"{label} source must be an object")
    _content_addressed(value, label=label)
    return value


def load_sources(repo_root: Path) -> dict[str, Any]:
    """Load the reviewed, content-addressed models used by this diagnostic."""
    root = Path(repo_root).resolve()
    paths = {name: root / relative for name, relative in _SOURCE_PATHS.items()}
    coverage = _load_json(paths["operation_coverage"], label="operation coverage")
    if (
        coverage.get("schema_version") != OWNERSHIP_SCHEMA_VERSION
        or coverage.get("purpose") != "operation_coverage_ownership"
        or coverage.get("candidate_eligible") is not False
    ):
        raise ValueError("corrected operation coverage identity differs")
    core = _load_json(paths["core"], label="core opcode")
    registry = core.get("registry")
    typed_registry = composite_estimator.load_registry_payload(registry)
    storage_result = _load_json(paths["storage_result"], label="typed storage result")
    storage_report_path = paths["storage_result"].with_name("model-report.json")
    try:
        storage = json.loads(storage_report_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError("typed storage model report is invalid") from error
    expected_report_hash = storage_result.get("output_hashes", {}).get("model_report_file_sha256")
    if _file_sha256(storage_report_path) != expected_report_hash:
        raise ValueError("typed storage model report hash differs")
    selected = storage.get("selection", {}).get("selected_model")
    model = storage.get("frozen_fit_models", {}).get(selected)
    parameters = model.get("parameters_exact") if isinstance(model, Mapping) else None
    if not isinstance(parameters, Mapping):
        raise ValueError("typed storage selected parameters are missing")
    storage_parameters = {
        key: _decimal(value.get("decimal"), label=f"typed storage {key}")
        for key, value in parameters.items()
        if isinstance(value, Mapping)
    }
    required_storage = {
        "sload_warm_body", "sload_cold_extra", "sstore_cold_extra",
        "sstore_branch:noop", "sstore_branch:set", "sstore_branch:clear",
        "sstore_branch:reset", "sstore_branch:dirty_rewrite", "sstore_branch:restore_original",
    }
    if set(storage_parameters) != required_storage:
        raise ValueError("typed storage selected parameter schema differs")
    context = _load_json(paths["context"], label="context approximation")
    context_rows = context.get("classes")
    if not isinstance(context_rows, Mapping):
        raise ValueError("context approximation classes are missing")
    context_costs: dict[str, Decimal] = {}
    for name, row in context_rows.items():
        if not isinstance(name, str) or not isinstance(row, Mapping) or row.get("status") != "declared_approximation":
            raise ValueError("context approximation class differs")
        exact = row.get("cost_exact")
        if not isinstance(exact, Mapping):
            raise ValueError("context approximation cost is missing")
        context_costs[name] = _decimal(exact.get("decimal"), label=f"context {name}")
    blockhash_dir = paths["blockhash_result"].parent
    blockhash = blockhash_campaign.verify_blockhash_result(blockhash_dir)
    selected_blockhash = blockhash.get("decision", {}).get("selected")
    if (
        not isinstance(selected_blockhash, Mapping)
        or selected_blockhash.get("key") != "opcode:0x40"
        or selected_blockhash.get("selection") != "maximum_accepted_event_cost"
    ):
        raise ValueError("BLOCKHASH sealed conservative selection differs")
    blockhash_event_cost = _decimal(selected_blockhash.get("event_cost"), label="BLOCKHASH event cost")
    if blockhash_event_cost <= 0:
        raise ValueError("BLOCKHASH sealed event cost must be positive")
    rows = coverage.get("execution_coverage")
    if not isinstance(rows, list):
        raise ValueError("operation coverage rows are missing")
    coverage_by_key = {row.get("key"): row for row in rows if isinstance(row, Mapping)}
    if len(coverage_by_key) != len(rows) or any(not isinstance(key, str) for key in coverage_by_key):
        raise ValueError("operation coverage keys differ")
    return {
        "coverage": coverage_by_key,
        "registry": typed_registry,
        "storage_parameters": storage_parameters,
        "context_costs": context_costs,
        "blockhash_event_cost": blockhash_event_cost,
        "source_artifacts": {
            name: {
                "path": relative,
                "file_sha256": _file_sha256(paths[name]),
                "artifact_sha256": (
                    blockhash.get("artifact_sha256") if name == "blockhash_result"
                    else {"operation_coverage": coverage, "core": core, "storage_result": storage_result, "context": context}[name]["artifact_sha256"]
                ),
            }
            for name, relative in _SOURCE_PATHS.items()
        },
    }


def _trace_ready(trace: Any) -> list[Mapping[str, Any]]:
    if not isinstance(trace, Mapping) or trace.get("status") != "complete":
        raise ValueError("trace must be complete")
    parity = trace.get("parity")
    if not isinstance(parity, Mapping) or parity.get("passed") is not True or parity.get("mismatch_fields") != []:
        raise ValueError("trace parity must pass exactly")
    if trace.get("partial_blocks") != [] or trace.get("recovery_failures") != []:
        raise ValueError("trace completeness differs")
    blocks = trace.get("blocks")
    if not isinstance(blocks, list) or not blocks:
        raise ValueError("trace blocks are missing")
    return blocks


def _operation_totals(component: Mapping[str, Any]) -> tuple[int, int]:
    raw = component.get("interpreter_raw_gas")
    native = component.get("native_gas")
    raw = 0 if raw is None else raw
    native = 0 if native is None else native
    if isinstance(raw, bool) or not isinstance(raw, int) or raw < 0:
        raise ValueError("operation raw gas is invalid")
    if isinstance(native, bool) or not isinstance(native, int) or native < 0:
        raise ValueError("operation native gas is invalid")
    return raw, native


def _price_opcode(key: str, component: Mapping[str, Any], sources: Mapping[str, Any]) -> tuple[str, str | None, str, Decimal | None]:
    if key == "opcode:0x40":
        return (
            "priced",
            "sp1-blockhash-v2 sealed conservative event cost",
            "sealed maximum accepted event cost; not fit from smoke",
            sources["blockhash_event_cost"],
        )
    opcode = int(key.removeprefix("opcode:0x"), 16)
    try:
        if opcode in {0x54, 0x55}:
            return (
                "priced", "sealed typed storage model", "sealed typed storage model",
                composite_estimator._storage_cost(opcode, component, sources["storage_parameters"]),
            )
        if opcode in {0x30, 0x33, 0x34, 0x35, 0x36, 0x42}:
            return (
                "priced", "sealed context approximation", "declared approximation",
                composite_estimator._declared_context_cost(opcode, component, sources["context_costs"]),
            )
    except (KeyError, ValueError) as error:
        return "unmeasured", None, f"model input rejected: {error}", None
    coverage = sources["coverage"].get(key)
    if not isinstance(coverage, Mapping):
        return "unmeasured", None, "key is absent from corrected operation coverage", None
    classification = coverage.get("classification")
    if coverage.get("model_status") != "measured":
        reason = coverage.get("reason")
        return "unmeasured", None, f"coverage {classification}: {json.dumps(reason, sort_keys=True)}", None
    try:
        if classification in {"static_raw_gas", "structured_opcode"}:
            cost = composite_estimator.predict_opcode_event(sources["registry"], composite_estimator._opcode_event(opcode, component))
            return "priced", "sealed core opcode submodel", "sealed static/raw model", cost
        if classification == "structured_storage":
            cost = composite_estimator._storage_cost(opcode, component, sources["storage_parameters"])
            return "priced", "sealed typed storage model", "sealed typed storage model", cost
        if classification == "declared_context_approximation":
            cost = composite_estimator._declared_context_cost(opcode, component, sources["context_costs"])
            return "priced", "sealed context approximation", "declared approximation", cost
    except (KeyError, ValueError) as error:
        return "unmeasured", None, f"model input rejected: {error}", None
    return "unmeasured", None, f"unsupported corrected coverage classification: {classification}", None


def analyze_anchor_trace(
    trace: Mapping[str, Any], *, network: str, proposal_id: int,
    guest_input_sha256: str, compressed_trace_sha256: str, record_sha256: str,
    summary_sha256: str, sources: Mapping[str, Any],
) -> dict[str, Any]:
    """Classify exactly the Anchor transaction-phase operations in one complete trace."""
    if network not in {"taiko_hoodi", "taiko_mainnet"}:
        raise ValueError("network is unsupported")
    if isinstance(proposal_id, bool) or not isinstance(proposal_id, int) or proposal_id < 0:
        raise ValueError("proposal id is invalid")
    identities = {
        "guest_input_sha256": _require_sha256(guest_input_sha256, label="GuestInput"),
        "compressed_trace_sha256": _require_sha256(compressed_trace_sha256, label="compressed trace"),
        "record_sha256": _require_sha256(record_sha256, label="record"),
        "summary_sha256": _require_sha256(summary_sha256, label="summary"),
    }
    totals: dict[str, dict[str, Any]] = {}
    operation_count = 0
    for block in _trace_ready(trace):
        if not isinstance(block, Mapping) or isinstance(block.get("block_index"), bool) or not isinstance(block.get("block_index"), int):
            raise ValueError("trace block schema differs")
        transactions = block.get("transactions")
        operations = block.get("operations")
        if not isinstance(transactions, list) or not isinstance(operations, list):
            raise ValueError("trace block transactions or operations are missing")
        by_index: dict[int, tuple[Mapping[str, Any], Mapping[str, Any]]] = {}
        for transaction in transactions:
            ownership = opcode_gas.classify_transaction_trace_ownership(transaction)
            started = transaction.get("started_tx_index")
            if started is not None:
                if started in by_index:
                    raise ValueError("trace transaction join is ambiguous")
                by_index[started] = (transaction, ownership)
        seen_operations: set[int] = set()
        for operation in operations:
            if not isinstance(operation, Mapping):
                raise ValueError("operation trace must be an object")
            operation_id = operation.get("operation_id")
            if isinstance(operation_id, bool) or not isinstance(operation_id, int) or operation_id < 0 or operation_id in seen_operations:
                raise ValueError("operation id is invalid")
            seen_operations.add(operation_id)
            # This call is the sole Anchor selector and validates the exact join.
            charge = opcode_gas.classify_operation_trace_charge(
                operation, transactions, ownership_schema_version=OWNERSHIP_SCHEMA_VERSION
            )
            if operation.get("phase") != "transaction":
                continue
            joined = by_index.get(operation.get("tx_index"))
            if joined is None:
                raise ValueError("operation must join exactly one started transaction")
            _transaction, transaction_ownership = joined
            if transaction_ownership["anchor_transaction"] is not True:
                continue
            component = operation.get("component")
            if not isinstance(component, Mapping):
                raise ValueError("operation component is invalid")
            raw_gas, native_gas = _operation_totals(component)
            if charge.get("matches_execution_coverage") is True:
                key = charge["execution_key"]
                if key.startswith("precompile:"):
                    outcome, model_source, reason, predicted = (
                        "unmeasured", None, "direct precompile has no sealed operation model", None
                    )
                    family = "direct_precompile"
                else:
                    outcome, model_source, reason, predicted = _price_opcode(key, component, sources)
                    family = "opcode_execution"
            elif charge.get("side_effect_event") == "confirmed_spawn_wrapper":
                opcode = component["opcode"]
                key = f"opcode:0x{opcode:02x}:spawned"
                outcome, model_source, reason, predicted = (
                    "unmeasured", None, "confirmed spawn wrapper remains a separate operation family", None
                )
                family = "confirmed_spawn_wrapper"
            elif charge.get("side_effect_event") == "selected_not_dispatched_spawn":
                opcode = component["opcode"]
                key = f"opcode:0x{opcode:02x}:spawned"
                outcome, model_source, reason, predicted = (
                    "intentionally_absent", None, "selected_not_dispatched is zero-charge by declared selector", Decimal(0)
                )
                family = "selected_not_dispatched_spawn"
            else:
                raise ValueError("Anchor operation ownership classification differs")
            row = totals.setdefault(key, {
                "key": key, "family": family, "event_count": 0, "charge_count": 0,
                "raw_gas_total": 0, "native_gas_total": 0, "outcome": outcome, "model_source": model_source,
                "reason": reason, "predicted_prover_gas": Decimal(0) if predicted is not None else None,
            })
            if (row["family"], row["outcome"], row["model_source"], row["reason"]) != (family, outcome, model_source, reason):
                raise ValueError("same operation key has conflicting model outcome")
            row["event_count"] += 1
            row["charge_count"] += int(charge.get("charge_count", 0))
            row["raw_gas_total"] += raw_gas
            row["native_gas_total"] += native_gas
            if row["predicted_prover_gas"] is not None:
                row["predicted_prover_gas"] += predicted
            operation_count += 1
    rows = []
    for key in sorted(totals):
        row = totals[key]
        if row["predicted_prover_gas"] is not None:
            # Event models are per operation; static models return the single event prediction.
            row["predicted_prover_gas"] = _decimal_text(row["predicted_prover_gas"])
        rows.append(row)
    return {
        "network": network, "proposal_id": proposal_id, "input_identity": identities,
        "ownership_schema_version": OWNERSHIP_SCHEMA_VERSION, "keys": rows,
        "aggregate": {
            "anchor_operation_count": operation_count,
            "event_count": sum(row["event_count"] for row in rows),
            "charge_count": sum(row["charge_count"] for row in rows),
            "raw_gas_total": sum(row["raw_gas_total"] for row in rows),
            "native_gas_total": sum(row["native_gas_total"] for row in rows),
            "unmeasured_key_count": sum(row["outcome"] == "unmeasured" for row in rows),
        },
    }


def _aggregate_rows(records: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    aggregate: dict[str, dict[str, Any]] = {}
    for record in records:
        for row in record.get("keys", []):
            if not isinstance(row, Mapping):
                raise ValueError("analysis key row is invalid")
            required = {
                "key", "family", "event_count", "charge_count", "raw_gas_total", "native_gas_total",
                "outcome", "model_source", "reason", "predicted_prover_gas",
            }
            if set(row) != required or not isinstance(row["key"], str):
                raise ValueError("analysis key row schema differs")
            output = aggregate.setdefault(row["key"], {
                **row, "event_count": 0, "charge_count": 0, "raw_gas_total": 0, "native_gas_total": 0,
                "predicted_prover_gas": Decimal(0) if row.get("predicted_prover_gas") is not None else None,
            })
            if (output["family"], output["outcome"], output["model_source"], output["reason"]) != (
                row["family"], row["outcome"], row["model_source"], row["reason"]
            ):
                raise ValueError("aggregate key model outcome differs by network")
            for field in ("event_count", "charge_count", "raw_gas_total", "native_gas_total"):
                if isinstance(row[field], bool) or not isinstance(row[field], int) or row[field] < 0:
                    raise ValueError("analysis key numeric total is invalid")
                output[field] += row[field]
            if output["predicted_prover_gas"] is not None:
                output["predicted_prover_gas"] += _decimal(row["predicted_prover_gas"], label="analysis prediction")
    rows = []
    for key in sorted(aggregate):
        row = aggregate[key]
        if row["predicted_prover_gas"] is not None:
            row["predicted_prover_gas"] = _decimal_text(row["predicted_prover_gas"])
        rows.append(row)
    return rows


def build_analysis_artifact(records: Sequence[Mapping[str, Any]], *, sources: Mapping[str, Any]) -> dict[str, Any]:
    if not records:
        raise ValueError("analysis requires at least one network record")
    networks = [record.get("network") for record in records]
    if len(networks) != len(set(networks)):
        raise ValueError("analysis network records must be unique")
    aggregate_rows = _aggregate_rows(records)
    payload = {
        "schema_version": SCHEMA_VERSION, "purpose": PURPOSE, "status": "diagnostic_only",
        "ownership_schema_version": OWNERSHIP_SCHEMA_VERSION,
        "coefficients_fitted": False, "candidate_eligible": False,
        "production_table_updated": False, "composite_estimator_updated": False, "final_60_opened": False,
        "sources": sources["source_artifacts"], "records": list(records), "aggregate_keys": aggregate_rows,
    }
    return {**payload, "artifact_sha256": _sha256(payload)}


def verify_analysis_artifact(artifact: Mapping[str, Any], *, repo_root: Path | None = None) -> None:
    _content_addressed(artifact, label="Anchor gap histogram")
    if (
        artifact.get("schema_version") != SCHEMA_VERSION or artifact.get("purpose") != PURPOSE
        or artifact.get("status") != "diagnostic_only" or artifact.get("ownership_schema_version") != OWNERSHIP_SCHEMA_VERSION
        or artifact.get("coefficients_fitted") is not False or artifact.get("candidate_eligible") is not False
        or artifact.get("production_table_updated") is not False or artifact.get("composite_estimator_updated") is not False
        or artifact.get("final_60_opened") is not False
    ):
        raise ValueError("Anchor gap histogram header differs")
    records = artifact.get("records")
    sources = artifact.get("sources")
    if not isinstance(records, list) or not isinstance(sources, Mapping):
        raise ValueError("Anchor gap histogram records or sources differ")
    expected_source_names = set(_SOURCE_PATHS)
    if set(sources) != expected_source_names:
        raise ValueError("Anchor gap histogram source set differs")
    for name, relative_path in _SOURCE_PATHS.items():
        source = sources[name]
        if not isinstance(source, Mapping) or source.get("path") != relative_path:
            raise ValueError("Anchor gap histogram source path differs")
        _require_sha256(source.get("file_sha256"), label=f"{name} file")
        _require_sha256(source.get("artifact_sha256"), label=f"{name} artifact")
    for record in records:
        if not isinstance(record, Mapping) or record.get("ownership_schema_version") != OWNERSHIP_SCHEMA_VERSION:
            raise ValueError("Anchor gap histogram record schema differs")
        if record.get("network") not in {"taiko_hoodi", "taiko_mainnet"}:
            raise ValueError("Anchor gap histogram record network differs")
        if isinstance(record.get("proposal_id"), bool) or not isinstance(record.get("proposal_id"), int):
            raise ValueError("Anchor gap histogram proposal differs")
        identity = record.get("input_identity")
        if not isinstance(identity, Mapping) or set(identity) != {
            "guest_input_sha256", "compressed_trace_sha256", "record_sha256", "summary_sha256"
        }:
            raise ValueError("Anchor gap histogram record identity differs")
        for name, value in identity.items():
            _require_sha256(value, label=name)
    if _aggregate_rows(records) != artifact.get("aggregate_keys"):
        raise ValueError("Anchor gap histogram aggregate replay differs")
    if repo_root is not None:
        if sources != load_sources(Path(repo_root))["source_artifacts"]:
            raise ValueError("Anchor gap histogram sealed sources differ")


def write_analysis_artifact(artifact: Mapping[str, Any], output_root: Path) -> Path:
    verify_analysis_artifact(artifact)
    directory = Path(output_root) / artifact["artifact_sha256"][:24]
    directory.mkdir(parents=True, exist_ok=False)
    path = directory / "anchor-gap-histogram.json"
    path.write_bytes(_canonical_json(artifact) + b"\n")
    return path


def _load_trace(path: Path) -> Mapping[str, Any]:
    data = gzip.open(path, "rt").read() if path.suffix == ".gz" else path.read_text()
    value = json.loads(data)
    if not isinstance(value, Mapping):
        raise ValueError("trace must be an object")
    return value


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="append", type=Path, help="identity JSON with network/proposal/GuestInput/summary and trace_path")
    parser.add_argument("--output-root", type=Path)
    parser.add_argument("--verify", type=Path, help="verify an existing analysis artifact")
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parents[2]
    if args.verify is not None:
        if args.record or args.output_root is not None:
            parser.error("--verify cannot be combined with analysis inputs")
        verify_analysis_artifact(json.loads(args.verify.read_text()), repo_root=root)
        return 0
    if not args.record or args.output_root is None:
        parser.error("--record and --output-root are required for analysis")
    sources = load_sources(root)
    records = []
    for record_path in args.record:
        record = json.loads(record_path.read_text())
        if not isinstance(record, Mapping) or not isinstance(record.get("trace_path"), str):
            raise ValueError("record must name a trace_path")
        trace_path = (root / record["trace_path"]).resolve()
        records.append(analyze_anchor_trace(
            _load_trace(trace_path), network=record.get("network"), proposal_id=record.get("proposal_id"),
            guest_input_sha256=record.get("guest_input_sha256"), compressed_trace_sha256=_file_sha256(trace_path),
            record_sha256=_file_sha256(record_path), summary_sha256=record.get("summary_sha256"), sources=sources,
        ))
    path = write_analysis_artifact(build_analysis_artifact(records, sources=sources), args.output_root)
    print(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
