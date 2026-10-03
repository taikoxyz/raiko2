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
from decimal import Context, Decimal, localcontext
from pathlib import Path
from typing import Any, Mapping, Sequence

import blockhash_campaign
import composite_estimator
import opcode_gas


SCHEMA_VERSION = 1
PURPOSE = "anchor_operation_gap_histogram"
OWNERSHIP_SCHEMA_VERSION = 4
_SHA256_LENGTH = 64
_DECIMAL_CONTEXT = Context(prec=160)
_SOURCE_PATHS = {
    "operation_coverage": "experiments/opcode-gas/manifests/operation-coverage-v7.json",
    "core": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
    "storage_result": "experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0/result.json",
    "context": "experiments/opcode-gas/derivations/1d2758bcb7aa3ae7f09d14eb/context-approximation.json",
    "blockhash_result": "experiments/opcode-gas/calibrations/sp1-blockhash-v2/6af515a6377aaf0c2906b151/result.json",
}
_REVIEWED_SOURCES = {
    "operation_coverage": {
        "file_sha256": "9ef6541f39eaded53e47c0231abb6c062883e211cfea3ae193b7f0c78b203009",
        "artifact_sha256": "4a84dc8289de2ce7ca1e0e93faf8d91611b0e47f12d257ed59f14e7d3aa21353",
    },
    "core": {
        "file_sha256": "0a85afe5f21af2599823cb0031087f4701e598acc3f7ca48896769b32b518234",
        "artifact_sha256": "1c05166e674a66ed51e3b1991c597ede479657e3195774a2960b1ff1cdd5ba4a",
    },
    "storage_result": {
        "file_sha256": "dde37295a7c34bb7bb4621a4e30e3b989352fa30060b1b17c69c73a1b59b493b",
        "artifact_sha256": "185dedb58925304433a0e591016d049abf2bf23aa6807e2165a49e5880ceb953",
    },
    "context": {
        "file_sha256": "346c2ac112357872f4a981737469c39639d7cc845ae97b0cc8bb77ddfc8a5b58",
        "artifact_sha256": "357a6c47bad8e6def3280be77a300e113e350fa673e08c39af13034861aee3d8",
    },
    "blockhash_result": {
        "file_sha256": "180ba27117966710ede6d4cf8f56cfe1beb729e2be72552ad6879ce35a3ed9b8",
        "artifact_sha256": "5fbdf5d26918e57214d70997795b2b41fad3219b8821730a08b0430f890c1d0c",
    },
}
_STORAGE_MODEL_REPORT_SHA256 = "124676b34172bd5715b73107eecc4174c9fb99310298143fe58ce420afeabf89"
_SMOKE_BUNDLES = {
    "taiko_hoodi": {
        "proposal_id": 80907,
        "record_sha256": "27d7f7c49e8edfcfb4efd03cdab640284ab2809b7c6d13d2e00a7334d0acef87",
        "summary_sha256": "cacdfa1fe6489bb1ffe25bdcd23428464abb06b6d247ed68d5c1f9d3634b7034",
        "compressed_trace_sha256": "bb25e90d2bc0bed1b0dec20f24fae41df38bbe63583eabfc8837f7ccd5f425a9",
        "guest_input_sha256": "64083abb0a43a73654b75ce4d8856083fdbc98657856f0bc9886a812ed07d7a2",
    },
    "taiko_mainnet": {
        "proposal_id": 39339,
        "record_sha256": "5d6d9f67ef38bb6661a568949f7a5e625b0a70748b2a24b3caa589265164a750",
        "summary_sha256": "34665671d06a1f9df737d70f58d3d574522f6b0482e6d35a5002e7b1b9bbdbd4",
        "compressed_trace_sha256": "aff03938bc4bc12c172edb7465e141f5059a801348c51681bcd423668d7b7f04",
        "guest_input_sha256": "7e79d122ff315a200b9e5362b2614d961e52cd09ead8935ff875169d3fcb8c0a",
    },
}


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def _sha256(value: Any) -> str:
    return hashlib.sha256(_canonical_json(value)).hexdigest()


def _file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _load_json_bytes(data: bytes, *, label: str) -> dict[str, Any]:
    try:
        value = json.loads(data)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} JSON is invalid") from error
    if not isinstance(value, dict):
        raise ValueError(f"{label} JSON must be an object")
    return value


def _normalize_guest_input_sha256(value: Any, *, label: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a GuestInput SHA256")
    normalized = value[2:] if value.startswith("0x") else value
    return _require_sha256(normalized, label=label)


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
    value = _load_json_bytes(path.read_bytes(), label=label)
    _content_addressed(value, label=label)
    return value


def _reviewed_source_metadata(name: str, path: Path, payload: Mapping[str, Any]) -> dict[str, str]:
    reviewed = _REVIEWED_SOURCES[name]
    if _file_sha256(path) != reviewed["file_sha256"]:
        raise ValueError(f"{name} reviewed file SHA256 differs")
    if payload.get("artifact_sha256") != reviewed["artifact_sha256"]:
        raise ValueError(f"{name} reviewed artifact differs")
    return {"path": _SOURCE_PATHS[name], **reviewed}


def load_sources(repo_root: Path) -> dict[str, Any]:
    """Load the reviewed, content-addressed models used by this diagnostic."""
    root = Path(repo_root).resolve()
    paths = {name: root / relative for name, relative in _SOURCE_PATHS.items()}
    coverage = _load_json(paths["operation_coverage"], label="operation coverage")
    coverage_metadata = _reviewed_source_metadata("operation_coverage", paths["operation_coverage"], coverage)
    if (
        coverage.get("schema_version") != OWNERSHIP_SCHEMA_VERSION
        or coverage.get("purpose") != "operation_coverage_ownership"
        or coverage.get("candidate_eligible") is not False
    ):
        raise ValueError("corrected operation coverage identity differs")
    core = _load_json(paths["core"], label="core opcode")
    core_metadata = _reviewed_source_metadata("core", paths["core"], core)
    registry = core.get("registry")
    typed_registry = composite_estimator.load_registry_payload(registry)
    storage_result = _load_json(paths["storage_result"], label="typed storage result")
    storage_metadata = _reviewed_source_metadata("storage_result", paths["storage_result"], storage_result)
    storage_report_path = paths["storage_result"].with_name("model-report.json")
    try:
        storage = json.loads(storage_report_path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError("typed storage model report is invalid") from error
    expected_report_hash = storage_result.get("output_hashes", {}).get("model_report_file_sha256")
    if (
        expected_report_hash != _STORAGE_MODEL_REPORT_SHA256
        or _file_sha256(storage_report_path) != _STORAGE_MODEL_REPORT_SHA256
    ):
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
    context_metadata = _reviewed_source_metadata("context", paths["context"], context)
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
    blockhash_metadata = _reviewed_source_metadata("blockhash_result", paths["blockhash_result"], blockhash)
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
            "operation_coverage": coverage_metadata,
            "core": core_metadata,
            "storage_result": {
                **storage_metadata,
                "model_report_file_sha256": _STORAGE_MODEL_REPORT_SHA256,
            },
            "context": context_metadata,
            "blockhash_result": blockhash_metadata,
        },
    }


def _checked_count(value: Any, *, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{label} is invalid")
    return value


def _bundle_identity(network: str) -> dict[str, Any]:
    expected = _SMOKE_BUNDLES[network]
    return {"network": network, **expected}


def load_smoke_bundle(record_path: Path, *, repo_root: Path) -> dict[str, Any]:
    """Read one reviewed smoke bundle once and join its record, summary, and trace."""
    root = Path(repo_root).resolve()
    path = Path(record_path).resolve()
    for network, expected in _SMOKE_BUNDLES.items():
        canonical = (root / "experiments/opcode-gas/runs/task-4-integration-smoke" /
                     network.removeprefix("taiko_") / "record.json").resolve()
        if path != canonical:
            continue
        record_bytes = path.read_bytes()
        record = _load_json_bytes(record_bytes, label="smoke record")
        if (
            _sha256_bytes(record_bytes) != expected["record_sha256"]
            or record.get("network") != network
            or record.get("proposal_id") != expected["proposal_id"]
            or record.get("purpose") != "integration_smoke"
        ):
            raise ValueError("smoke record identity differs")
        summary_path = path.with_name("run.proposal-trace.summary.json")
        trace_path = path.with_name("run.proposal-trace.json.gz")
        summary_bytes = summary_path.read_bytes()
        trace_bytes = trace_path.read_bytes()
        summary = _load_json_bytes(summary_bytes, label="smoke trace summary")
        if _sha256_bytes(summary_bytes) != expected["summary_sha256"]:
            raise ValueError("smoke trace summary SHA256 differs")
        if _sha256_bytes(trace_bytes) != expected["compressed_trace_sha256"]:
            raise ValueError("smoke compressed trace SHA256 differs")
        try:
            trace = _load_json_bytes(gzip.decompress(trace_bytes), label="smoke trace")
        except OSError as error:
            raise ValueError("smoke trace gzip is invalid") from error
        guest = expected["guest_input_sha256"]
        summary_fields = {
            "schema_version": 4,
            "full_trace_encoding": "json+gzip",
            "status": "complete",
            "parity_passed": True,
            "partial_block_count": 0,
            "recovery_failure_count": 0,
        }
        if any(summary.get(key) != value for key, value in summary_fields.items()):
            raise ValueError("smoke trace summary schema differs")
        if _normalize_guest_input_sha256(summary.get("guest_input_sha256"), label="summary GuestInput") != guest:
            raise ValueError("smoke trace summary GuestInput differs")
        if (
            trace.get("schema_version") != 4
            or trace.get("status") != "complete"
            or _normalize_guest_input_sha256(trace.get("guest_input_sha256"), label="trace GuestInput") != guest
            or trace.get("guest_input_bincode_length") != summary.get("guest_input_bincode_length")
            or trace.get("public_output") != summary.get("public_output")
            or trace.get("parity", {}).get("passed") is not True
            or trace.get("parity", {}).get("mismatch_fields") != []
        ):
            raise ValueError("smoke trace and summary join differs")
        blocks = trace.get("blocks")
        if not isinstance(blocks, list):
            raise ValueError("smoke trace blocks are missing")
        if len(blocks) != _checked_count(summary.get("block_count"), label="summary block count"):
            raise ValueError("smoke trace block count differs")
        operation_count = 0
        for block in blocks:
            if not isinstance(block, Mapping) or not isinstance(block.get("operations"), list):
                raise ValueError("smoke trace block operation schema differs")
            operation_count += len(block["operations"])
        if operation_count != _checked_count(summary.get("operation_count"), label="summary operation count"):
            raise ValueError("smoke trace operation count differs")
        return {
            "record": record,
            "summary": summary,
            "trace": trace,
            "identity": _bundle_identity(network),
        }
    raise ValueError("record path is not a reviewed integration-smoke bundle")


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
    expected = _SMOKE_BUNDLES.get(network)
    if expected is None or proposal_id != expected["proposal_id"]:
        raise ValueError("reviewed smoke record identity differs")
    identities = {
        "guest_input_sha256": _normalize_guest_input_sha256(guest_input_sha256, label="GuestInput"),
        "compressed_trace_sha256": _require_sha256(compressed_trace_sha256, label="compressed trace"),
        "record_sha256": _require_sha256(record_sha256, label="record"),
        "summary_sha256": _require_sha256(summary_sha256, label="summary"),
    }
    for key, value in identities.items():
        if value != expected[key]:
            raise ValueError("reviewed smoke input identity differs")
    if (
        trace.get("schema_version") != 4
        or _normalize_guest_input_sha256(trace.get("guest_input_sha256"), label="trace GuestInput")
        != identities["guest_input_sha256"]
    ):
        raise ValueError("trace schema or GuestInput identity differs")
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
                key = f"opcode:0x{opcode:02x}:confirmed_spawn_wrapper"
                outcome, model_source, reason, predicted = (
                    "unmeasured", None, "confirmed spawn wrapper remains a separate operation family", None
                )
                family = "confirmed_spawn_wrapper"
            elif charge.get("side_effect_event") == "selected_not_dispatched_spawn":
                opcode = component["opcode"]
                key = f"opcode:0x{opcode:02x}:selected_not_dispatched_spawn"
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
                with localcontext(_DECIMAL_CONTEXT):
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
                with localcontext(_DECIMAL_CONTEXT):
                    output["predicted_prover_gas"] += _decimal(row["predicted_prover_gas"], label="analysis prediction")
    rows = []
    for key in sorted(aggregate):
        row = aggregate[key]
        if row["predicted_prover_gas"] is not None:
            row["predicted_prover_gas"] = _decimal_text(row["predicted_prover_gas"])
        rows.append(row)
    return rows


_KEY_FIELDS = frozenset({
    "key", "family", "event_count", "charge_count", "raw_gas_total", "native_gas_total",
    "outcome", "model_source", "reason", "predicted_prover_gas",
})
_AGGREGATE_FIELDS = frozenset({
    "anchor_operation_count", "event_count", "charge_count", "raw_gas_total",
    "native_gas_total", "unmeasured_key_count",
})


def _validate_key_row(row: Mapping[str, Any]) -> None:
    if set(row) != _KEY_FIELDS or not isinstance(row.get("key"), str) or not row["key"]:
        raise ValueError("analysis key row schema differs")
    for field in ("event_count", "charge_count", "raw_gas_total", "native_gas_total"):
        _checked_count(row.get(field), label=f"analysis {field}")
    if row["charge_count"] > row["event_count"]:
        raise ValueError("analysis key charge count exceeds event count")
    family = row.get("family")
    outcome = row.get("outcome")
    if not isinstance(row.get("reason"), str) or not row["reason"]:
        raise ValueError("analysis key reason differs")
    if family == "confirmed_spawn_wrapper":
        if not row["key"].endswith(":confirmed_spawn_wrapper") or outcome != "unmeasured":
            raise ValueError("confirmed wrapper key outcome differs")
    elif family == "selected_not_dispatched_spawn":
        if (
            not row["key"].endswith(":selected_not_dispatched_spawn")
            or outcome != "intentionally_absent"
            or row["charge_count"] != 0
        ):
            raise ValueError("selected-not-dispatched key outcome differs")
    elif family not in {"opcode_execution", "direct_precompile"}:
        raise ValueError("analysis key family differs")
    if outcome == "priced":
        if not isinstance(row.get("model_source"), str) or not row["model_source"]:
            raise ValueError("priced key model source differs")
        if _decimal(row.get("predicted_prover_gas"), label="priced prediction") < 0:
            raise ValueError("priced prediction must be nonnegative")
    elif outcome == "unmeasured":
        if row.get("model_source") is not None or row.get("predicted_prover_gas") is not None:
            raise ValueError("unmeasured key cannot carry a model prediction")
    elif outcome == "intentionally_absent":
        if row.get("model_source") is not None or row.get("predicted_prover_gas") != "0":
            raise ValueError("intentionally absent key must be zero-charge")
    else:
        raise ValueError("analysis key outcome differs")


def _record_aggregate(record: Mapping[str, Any]) -> dict[str, int]:
    keys = record.get("keys")
    if not isinstance(keys, list) or not keys:
        raise ValueError("analysis record keys are missing")
    seen = set()
    for row in keys:
        if not isinstance(row, Mapping):
            raise ValueError("analysis key row is invalid")
        _validate_key_row(row)
        if row["key"] in seen:
            raise ValueError("analysis record keys must be unique")
        seen.add(row["key"])
    if [row["key"] for row in keys] != sorted(seen):
        raise ValueError("analysis record keys must be sorted")
    return {
        "anchor_operation_count": sum(row["event_count"] for row in keys),
        "event_count": sum(row["event_count"] for row in keys),
        "charge_count": sum(row["charge_count"] for row in keys),
        "raw_gas_total": sum(row["raw_gas_total"] for row in keys),
        "native_gas_total": sum(row["native_gas_total"] for row in keys),
        "unmeasured_key_count": sum(row["outcome"] == "unmeasured" for row in keys),
    }


def _validate_records(records: Sequence[Mapping[str, Any]]) -> None:
    if not isinstance(records, list) or len(records) != len(_SMOKE_BUNDLES):
        raise ValueError("analysis must contain exactly the reviewed two-network smoke pair")
    networks = []
    for record in records:
        if not isinstance(record, Mapping) or set(record) != {
            "network", "proposal_id", "input_identity", "ownership_schema_version", "keys", "aggregate"
        }:
            raise ValueError("analysis record schema differs")
        network = record.get("network")
        expected = _SMOKE_BUNDLES.get(network)
        if expected is None or record.get("proposal_id") != expected["proposal_id"]:
            raise ValueError("analysis record identity differs")
        identity = record.get("input_identity")
        expected_identity = {key: expected[key] for key in expected if key.endswith("sha256")}
        if record.get("ownership_schema_version") != OWNERSHIP_SCHEMA_VERSION or identity != expected_identity:
            raise ValueError("analysis record input identity differs")
        aggregate = record.get("aggregate")
        if not isinstance(aggregate, Mapping) or set(aggregate) != _AGGREGATE_FIELDS:
            raise ValueError("analysis record aggregate schema differs")
        expected_aggregate = _record_aggregate(record)
        if aggregate != expected_aggregate:
            raise ValueError("analysis record aggregate replay differs")
        networks.append(network)
    if networks != sorted(_SMOKE_BUNDLES) or len(set(networks)) != len(networks):
        raise ValueError("analysis record network order differs")


def build_analysis_artifact(records: Sequence[Mapping[str, Any]], *, sources: Mapping[str, Any]) -> dict[str, Any]:
    _validate_records(records)
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
        expected_fields = {"path", "file_sha256", "artifact_sha256"}
        if name == "storage_result":
            expected_fields.add("model_report_file_sha256")
        if (
            not isinstance(source, Mapping)
            or set(source) != expected_fields
            or source.get("path") != relative_path
            or source.get("file_sha256") != _REVIEWED_SOURCES[name]["file_sha256"]
            or source.get("artifact_sha256") != _REVIEWED_SOURCES[name]["artifact_sha256"]
        ):
            raise ValueError("Anchor gap histogram source path differs")
        _require_sha256(source.get("file_sha256"), label=f"{name} file")
        _require_sha256(source.get("artifact_sha256"), label=f"{name} artifact")
        if name == "storage_result" and source.get("model_report_file_sha256") != _STORAGE_MODEL_REPORT_SHA256:
            raise ValueError("Anchor gap histogram storage model report differs")
    _validate_records(records)
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


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", action="append", type=Path, help="reviewed task-4 integration-smoke record.json")
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
        bundle = load_smoke_bundle(record_path, repo_root=root)
        identity = bundle["identity"]
        records.append(analyze_anchor_trace(
            bundle["trace"], sources=sources, **identity,
        ))
    path = write_analysis_artifact(build_analysis_artifact(records, sources=sources), args.output_root)
    print(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
