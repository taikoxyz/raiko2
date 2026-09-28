"""Strict, deterministic evaluator for the SP1 composite block estimator."""

from __future__ import annotations

import hashlib
import json
from decimal import Context, Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext
from pathlib import PurePosixPath
from typing import Any, Mapping

from hierarchical_model import (
    ModelKind,
    ModelSpec,
    OpcodeEvent,
    OpcodeRegistry,
    predict_opcode_event,
)


ESTIMATOR_SCHEMA_VERSION = 2
ESTIMATOR_PURPOSE = "sp1_composite_block_estimator"
TRACE_SCHEMA_VERSION = 3
SP1_GAS_TRACE_CHUNK_THRESHOLD = 134_217_728
SP1_GAS_TRACE_CHUNK_SLOTS = 2
ESTIMATOR_FORMULA = (
    "proposal_startup + blocks*block_base + "
    "started_non_anchor_transactions*tx_base + "
    "committed_native_eoa_transfers*native_value_transfer + "
    "sum(typed_executed_operation_cost)"
)
VERSION_IDENTITY = {
    "taiko_fork": "Unzen",
    "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
    "ethereum_upgrade": "Fusaka",
    "revm_spec_id": "OSAKA",
    "proving_backend": "sp1",
    "primary_metric": "proverGas",
}
FIXED_COST_STATUSES = {
    "proposal_startup": "accepted",
    "block_base": "accepted",
    "tx_base": "accepted",
    "native_value_transfer": "declared_approximation",
}
COARSE_STATE_TRIE = {
    "status": "coarse_model_accepted",
    "maximum_total_ape": "0.1",
    "maximum_effect_ratio": "0.1",
}
COVERAGE_POLICY = {
    "complete_prediction_requires_zero_gaps": True,
    "ape_requires_exact_sp1_report_join": True,
    "gap_fallback_multiplier": None,
    "independent_denominators": [
        "operation_count",
        "raw_gas",
        "spawn_wrapper",
        "precompile",
        "typed_feature",
    ],
}
SOURCE_CODE_PATHS = frozenset(
    {
        "experiments/opcode-gas/opcode_gas.py",
        "experiments/opcode-gas/composite_estimator.py",
        "experiments/opcode-gas/hierarchical_model.py",
        "crates/zkgas-trace/src/inspector.rs",
        "crates/zkgas-trace/src/transactions.rs",
        "crates/zkgas-trace/src/reconstruct.rs",
        "docs/plans/2026-09-26-zkgas-calibration-design.md",
        "docs/plans/2026-09-29-zkgas-typed-storage-promotion-design.md",
    }
)

_DECIMAL_CONTEXT = Context(prec=80, rounding=ROUND_HALF_EVEN, traps=[])
_FIXED_COST_KEYS = frozenset(FIXED_COST_STATUSES)
_SPAWN_OPCODES = frozenset({0xF0, 0xF1, 0xF2, 0xF4, 0xF5, 0xFA})
_STORAGE_OPCODES = frozenset({0x54, 0x55})
_STORAGE_BRANCHES = (
    "noop",
    "set",
    "clear",
    "reset",
    "dirty_rewrite",
    "restore_original",
)
_STORAGE_MODEL_PARAMETER_ORDER = (
    "sload_warm_body",
    "sload_cold_extra",
    *tuple(f"sstore_branch:{branch}" for branch in _STORAGE_BRANCHES),
    "sstore_cold_extra",
)
_EXPECTED_ESTIMATOR_FIELDS = frozenset(
    {
        "schema_version",
        "purpose",
        "status",
        "review_only",
        "production_write",
        "proposal_validation_opened",
        "implementation_revision",
        "version_identity",
        "formula",
        "registry_parameter_basis",
        "trace_schema",
        "registry",
        "storage_model",
        "execution_coverage",
        "ownership_policy",
        "fixed_costs",
        "fixed_cost_statuses",
        "coarse_state_trie",
        "coverage_policy",
        "source_artifacts",
        "artifact_sha256",
    }
)
_OWNERSHIP_SEMANTICS = {
    "system_and_anchor_operations": "block_base",
    "confirmed_spawn_wrapper": "explicit_gap_until_calibrated",
    "selected_not_dispatched_spawn": "diagnostic_no_charge",
    "child_execution": "ordinary_execution_coverage_zero_extra_charge",
}
_TRACE_SELECTOR_KEYS = frozenset(
    {
        "anchor_transaction",
        "child_execution",
        "confirmed_spawn_wrapper",
        "native_value_transfer",
        "non_anchor_started_transaction",
        "opcode_raw_gas_execution",
        "precompile_raw_gas_execution",
        "selected_not_dispatched_spawn",
        "system_operation",
        "transaction_envelope",
    }
)


def _canonical_json(value: Any) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode()


def _sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(
        character in "0123456789abcdef" for character in value
    )


def _is_git_revision(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(
        character in "0123456789abcdef" for character in value
    )


def _decimal_text(value: Decimal) -> str:
    if value == 0:
        return "0"
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _canonical_decimal(
    value: Any, *, label: str, nonnegative: bool = False
) -> Decimal:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a canonical Decimal string")
    try:
        parsed = Decimal(value)
    except (InvalidOperation, ValueError) as error:
        raise ValueError(f"{label} must be a canonical Decimal string") from error
    if not parsed.is_finite() or _decimal_text(parsed) != value:
        raise ValueError(f"{label} must be a canonical Decimal string")
    if nonnegative and parsed < 0:
        raise ValueError(f"{label} must be nonnegative")
    return parsed


def _validate_content_address(artifact: Mapping[str, Any]) -> None:
    digest = artifact.get("artifact_sha256")
    if not _is_sha256(digest):
        raise ValueError("composite estimator artifact SHA256 is invalid")
    payload = {key: value for key, value in artifact.items() if key != "artifact_sha256"}
    if _sha256(_canonical_json(payload)) != digest:
        raise ValueError("composite estimator artifact SHA256 differs")


def load_registry_payload(registry: Any) -> OpcodeRegistry:
    """Load the exact typed opcode registry without numeric coercion."""
    expected_fields = {
        "common_dispatch",
        "invalid_model_id",
        "named_opcode_keys",
        "shared_memory_parameters",
        "models",
        "opcode_model_ids",
    }
    if not isinstance(registry, Mapping) or set(registry) != expected_fields:
        raise ValueError("composite registry schema differs")
    common_dispatch = _canonical_decimal(
        registry["common_dispatch"],
        label="composite common dispatch",
        nonnegative=True,
    )
    invalid_model_id = registry["invalid_model_id"]
    named_keys = registry["named_opcode_keys"]
    slots = registry["opcode_model_ids"]
    model_rows = registry["models"]
    memory_rows = registry["shared_memory_parameters"]
    if (
        not isinstance(invalid_model_id, str)
        or not invalid_model_id
        or not isinstance(named_keys, list)
        or len(named_keys) != 150
        or len(set(named_keys)) != len(named_keys)
        or not isinstance(slots, list)
        or len(slots) != 256
        or not isinstance(model_rows, Mapping)
        or not isinstance(memory_rows, Mapping)
    ):
        raise ValueError("composite registry inventory differs")

    named_opcodes = set()
    for key in named_keys:
        if not isinstance(key, str) or not key.startswith("opcode:0x"):
            raise ValueError("composite registry named opcode key is invalid")
        try:
            opcode = int(key.removeprefix("opcode:0x"), 16)
        except ValueError as error:
            raise ValueError("composite registry named opcode key is invalid") from error
        if not 0 <= opcode <= 0xFF or key != f"opcode:0x{opcode:02x}":
            raise ValueError("composite registry named opcode key is invalid")
        named_opcodes.add(opcode)

    models = {}
    for model_id, row in model_rows.items():
        if (
            not isinstance(model_id, str)
            or not isinstance(row, Mapping)
            or set(row) != {"kind", "parameters"}
            or not isinstance(row["parameters"], Mapping)
        ):
            raise ValueError("composite registry model schema differs")
        try:
            kind = ModelKind(row["kind"])
        except (TypeError, ValueError) as error:
            raise ValueError(
                f"composite registry model kind is invalid: {model_id}"
            ) from error
        parameters = {
            name: _canonical_decimal(
                value,
                label=f"composite registry {model_id} parameter {name}",
            )
            for name, value in row["parameters"].items()
        }
        models[model_id] = ModelSpec(kind=kind, parameters=parameters)
    shared_memory_parameters = {
        name: _canonical_decimal(
            value,
            label=f"composite registry shared memory parameter {name}",
            nonnegative=True,
        )
        for name, value in memory_rows.items()
    }
    return OpcodeRegistry(
        common_dispatch=common_dispatch,
        models=models,
        opcode_model_ids=tuple(slots),
        named_opcodes=frozenset(named_opcodes),
        invalid_model_id=invalid_model_id,
        shared_memory_parameters=shared_memory_parameters,
    )


def _validate_coverage(
    artifact: Mapping[str, Any], registry: OpcodeRegistry
) -> dict[str, Mapping[str, Any]]:
    rows = artifact.get("execution_coverage")
    if not isinstance(rows, list) or len(rows) != 168:
        raise ValueError("composite estimator execution coverage differs")
    by_key = {}
    for row in rows:
        if not isinstance(row, Mapping) or not isinstance(row.get("key"), str):
            raise ValueError("composite estimator execution coverage row differs")
        key = row["key"]
        if key in by_key:
            raise ValueError("composite estimator execution coverage is duplicated")
        by_key[key] = row

    opcode_keys = {f"opcode:0x{opcode:02x}" for opcode in registry.named_opcodes}
    precompile_addresses = (*range(1, 18), 0x100)
    precompile_keys = {
        f"precompile:0x{address:02x}" for address in precompile_addresses
    }
    if set(by_key) != opcode_keys | precompile_keys:
        raise ValueError("composite estimator coverage inventory differs")
    for opcode in registry.named_opcodes:
        key = f"opcode:0x{opcode:02x}"
        row = by_key[key]
        if (
            row.get("component") != "opcode"
            or row.get("identifier") != f"0x{opcode:02x}"
            or row.get("trace_selector_ref") != "opcode_raw_gas_execution"
            or row.get("transaction_scope_selector_ref")
            != "non_anchor_started_transaction"
        ):
            raise ValueError(f"composite estimator opcode coverage differs: {key}")
        model_id = registry.opcode_model_ids[opcode]
        if model_id is None:
            if opcode in _STORAGE_OPCODES:
                source = artifact["source_artifacts"]["stateful_storage"]
                reference = row.get("artifact_ref")
                evidence = row.get("source_evidence")
                machine_evidence = [
                    item
                    for item in evidence
                    if isinstance(item, Mapping)
                    and item.get("kind") == "machine_trace_selector"
                ] if isinstance(evidence, list) else []
                stateful_evidence = [
                    item
                    for item in evidence
                    if isinstance(item, Mapping)
                    and item.get("kind") == "sealed_stateful_model"
                ] if isinstance(evidence, list) else []
                if (
                    row.get("classification") != "structured_storage"
                    or row.get("model_status") != "measured"
                    or not isinstance(reference, Mapping)
                    or set(reference)
                    != {
                        "path",
                        "result_id",
                        "result_identity_sha256",
                        "model_family",
                        "model_sha256",
                    }
                    or reference.get("path") != source.get("path")
                    or reference.get("result_id") != source.get("result_id")
                    or reference.get("result_identity_sha256")
                    != source.get("result_identity_sha256")
                    or reference.get("model_family") != "M_typed"
                    or reference.get("model_sha256") != source.get("model_sha256")
                    or len(machine_evidence) != 1
                    or machine_evidence[0].get("path")
                    != "crates/zkgas-trace/src/inspector.rs"
                    or machine_evidence[0].get("sha256")
                    != artifact["source_artifacts"]["source_code_sha256s"][
                        "crates/zkgas-trace/src/inspector.rs"
                    ]
                    or len(stateful_evidence) != 1
                    or stateful_evidence[0].get("result_id")
                    != source.get("result_id")
                    or stateful_evidence[0].get("result_identity_sha256")
                    != source.get("result_identity_sha256")
                    or stateful_evidence[0].get("model_family") != "M_typed"
                    or stateful_evidence[0].get("model_sha256")
                    != source.get("model_sha256")
                    or "reason" in row
                ):
                    raise ValueError(
                        f"composite estimator storage coverage differs: {key}"
                    )
                continue
            reason = row.get("reason")
            if (
                row.get("classification") != "explicitly_unsupported"
                or row.get("model_status") != "unsupported"
                or not isinstance(reason, Mapping)
                or reason.get("code") != "sealed_registry_unsupported"
                or not isinstance(reason.get("detail"), str)
                or "artifact_ref" in row
            ):
                raise ValueError(
                    f"composite estimator unsupported opcode coverage differs: {key}"
                )
            continue
        model = registry.models[model_id]
        expected_classification = (
            "static_raw_gas"
            if model.kind is ModelKind.STATIC_RAW_GAS
            else "structured_opcode"
        )
        reference = row.get("artifact_ref")
        source = artifact["source_artifacts"]["augmented_core"]
        if (
            row.get("classification") != expected_classification
            or row.get("model_status") != "measured"
            or not isinstance(reference, Mapping)
            or set(reference)
            != {"path", "artifact_sha256", "model_id", "model_kind"}
            or reference.get("path") != source.get("path")
            or reference.get("artifact_sha256") != source.get("artifact_sha256")
            or reference.get("model_id") != model_id
            or reference.get("model_kind") != model.kind.value
            or "reason" in row
        ):
            raise ValueError(f"composite estimator measured opcode coverage differs: {key}")
    for address in precompile_addresses:
        key = f"precompile:0x{address:02x}"
        row = by_key[key]
        reason = row.get("reason")
        if (
            row.get("component") != "precompile"
            or row.get("identifier") != f"0x{address:02x}"
            or row.get("classification") != "direct_precompile"
            or row.get("model_status") != "declared_unmeasured"
            or row.get("trace_selector_ref") != "precompile_raw_gas_execution"
            or row.get("transaction_scope_selector_ref")
            != "non_anchor_started_transaction"
            or not isinstance(reason, Mapping)
            or reason.get("code") != "direct_precompile_cost_not_sealed"
            or not isinstance(reason.get("detail"), str)
            or "artifact_ref" in row
        ):
            raise ValueError(f"composite estimator precompile coverage differs: {key}")
    return by_key


def _validate_sources(sources: Any) -> None:
    if not isinstance(sources, Mapping) or set(sources) != {
        "augmented_core",
        "operation_coverage",
        "corrected_higher_layer",
        "stateful_storage",
        "source_code_sha256s",
    }:
        raise ValueError("composite estimator source artifacts differ")
    for label in ("augmented_core", "operation_coverage"):
        source = sources[label]
        path = source.get("path") if isinstance(source, Mapping) else None
        pure = PurePosixPath(path) if isinstance(path, str) else None
        if (
            not isinstance(source, Mapping)
            or set(source) != {"path", "file_sha256", "artifact_sha256"}
            or pure is None
            or pure.is_absolute()
            or ".." in pure.parts
            or str(pure) != path
            or not _is_sha256(source.get("file_sha256"))
            or not _is_sha256(source.get("artifact_sha256"))
        ):
            raise ValueError(f"composite estimator {label} source differs")
    higher = sources["corrected_higher_layer"]
    higher_path = higher.get("path") if isinstance(higher, Mapping) else None
    higher_pure = PurePosixPath(higher_path) if isinstance(higher_path, str) else None
    if (
        not isinstance(higher, Mapping)
        or set(higher)
        != {
            "path",
            "derivation_id",
            "identity_sha256",
            "file_sha256s",
            "projection_artifact_sha256",
        }
        or higher_pure is None
        or higher_pure.is_absolute()
        or ".." in higher_pure.parts
        or str(higher_pure) != higher_path
        or not isinstance(higher.get("derivation_id"), str)
        or len(higher["derivation_id"]) != 24
        or not _is_sha256(higher.get("identity_sha256"))
        or higher["identity_sha256"][:24] != higher["derivation_id"]
        or not _is_sha256(higher.get("projection_artifact_sha256"))
        or not isinstance(higher.get("file_sha256s"), Mapping)
        or not higher["file_sha256s"]
        or any(not _is_sha256(value) for value in higher["file_sha256s"].values())
    ):
        raise ValueError("composite estimator higher-layer source differs")
    stateful = sources["stateful_storage"]
    stateful_path = stateful.get("path") if isinstance(stateful, Mapping) else None
    stateful_pure = PurePosixPath(stateful_path) if isinstance(stateful_path, str) else None
    if (
        not isinstance(stateful, Mapping)
        or set(stateful)
        != {
            "path",
            "result_id",
            "result_identity_sha256",
            "artifact_sha256",
            "file_sha256s",
            "model_sha256",
        }
        or stateful_pure is None
        or stateful_pure.is_absolute()
        or ".." in stateful_pure.parts
        or str(stateful_pure) != stateful_path
        or not isinstance(stateful.get("result_id"), str)
        or len(stateful["result_id"]) != 24
        or not _is_sha256(stateful.get("result_identity_sha256"))
        or stateful["result_identity_sha256"][:24] != stateful["result_id"]
        or not _is_sha256(stateful.get("artifact_sha256"))
        or not _is_sha256(stateful.get("model_sha256"))
        or not isinstance(stateful.get("file_sha256s"), Mapping)
        or set(stateful["file_sha256s"])
        != {
            "calibration-identity.json",
            "campaign-decisions.json",
            "campaign-decisions.sha256",
            "campaign-identity.json",
            "campaign-manifest.json",
            "model-report.json",
            "result.json",
            "rows.jsonl",
            "source-registry.json",
        }
        or any(not _is_sha256(value) for value in stateful["file_sha256s"].values())
    ):
        raise ValueError("composite estimator stateful-storage source differs")
    source_code = sources["source_code_sha256s"]
    if (
        not isinstance(source_code, Mapping)
        or set(source_code) != SOURCE_CODE_PATHS
        or any(not _is_sha256(value) for value in source_code.values())
    ):
        raise ValueError("composite estimator source-code identities differ")


def _validate_ownership_policy(policy: Any) -> None:
    if not isinstance(policy, Mapping) or set(policy) != {
        "trace_selectors",
        "side_effect_ownership",
        *_OWNERSHIP_SEMANTICS,
    }:
        raise ValueError("composite estimator ownership policy differs")
    for key, value in _OWNERSHIP_SEMANTICS.items():
        if policy.get(key) != value:
            raise ValueError("composite estimator ownership policy differs")
    selectors = policy.get("trace_selectors")
    if not isinstance(selectors, Mapping) or set(selectors) != _TRACE_SELECTOR_KEYS:
        raise ValueError("composite estimator ownership policy selectors differ")
    exact_selectors = {
        "anchor_transaction": {
            "mutually_exclusive_with": ["transaction_envelope", "system_operation"],
            "transaction_is_anchor": True,
            "transaction_started_tx_index": "present",
        },
        "child_execution": {
            "covered_by_execution_coverage": True,
            "derivation": "operation.frame_depth > 0",
            "extra_charge": 0,
            "scope": "matched_execution_coverage",
        },
        "native_value_transfer": {
            "native_value_transfer": True,
            "scope": "transaction_envelope",
            "transaction_disposition": "committed_success",
        },
        "non_anchor_started_transaction": {
            "join": "operation.tx_index == transaction.started_tx_index",
            "operation_phase": "transaction",
            "transaction_disposition_excludes": ["unattempted"],
            "transaction_is_anchor": False,
            "transaction_started_tx_index": "present",
        },
        "opcode_raw_gas_execution": {
            "component_kind": "opcode",
            "dispatch_status": "not_applicable",
            "interpreter_raw_gas": "present_nonnegative_integer",
            "pricing_basis": "raw_gas_slope",
            "scope": "non_anchor_started_transaction",
            "spawned": False,
        },
        "precompile_raw_gas_execution": {
            "component_kind": "precompile",
            "native_gas": "present_nonnegative_integer",
            "pricing_basis": "raw_gas_slope",
            "scope": "non_anchor_started_transaction",
        },
        "system_operation": {
            "mutually_exclusive_with": [
                "non_anchor_started_transaction",
                "anchor_transaction",
            ],
            "operation_phase": "system",
            "operation_tx_index": "absent",
        },
        "confirmed_spawn_wrapper": {
            "component_kind": "opcode",
            "dispatch_status": "confirmed",
            "interpreter_raw_gas": "absent",
            "pricing_basis": "fixed_per_event",
            "scope": "non_anchor_started_transaction",
            "semantics": "substitutes_opcode_raw_gas",
            "spawned": True,
        },
        "selected_not_dispatched_spawn": {
            "component_kind": "opcode",
            "dispatch_status": "selected_not_dispatched",
            "interpreter_raw_gas": "absent",
            "pricing_basis": "absent",
            "scope": "non_anchor_started_transaction",
            "semantics": "diagnostic_no_charge",
            "spawned": True,
        },
        "transaction_envelope": {
            "transaction_disposition_excludes": ["unattempted"],
            "transaction_is_anchor": False,
            "transaction_started_tx_index": "present",
        },
    }
    if any(selectors.get(key) != value for key, value in exact_selectors.items()):
        raise ValueError("composite estimator ownership policy selectors differ")
    side_effects = policy.get("side_effect_ownership")
    if not isinstance(side_effects, list):
        raise ValueError("composite estimator side-effect ownership differs")
    indexed = {
        row.get("event_id"): row
        for row in side_effects
        if isinstance(row, Mapping) and isinstance(row.get("event_id"), str)
    }
    if len(indexed) != len(side_effects):
        raise ValueError("composite estimator side-effect ownership differs")
    expected = {
        "confirmed_spawn_wrapper": ("operation_wrapper", "substitutes_spawn_opcode_raw_gas"),
        "selected_not_dispatched_spawn": ("operation_wrapper", "diagnostic_no_charge"),
        "child_execution": ("operation_wrapper", "covered_by_execution_coverage_zero_extra_charge"),
        "transaction_envelope": ("transaction", "transaction_work"),
        "native_value_transfer": ("transaction", "transaction_work"),
        "anchor_transaction": ("block", "block_work"),
        "system_operation": ("block", "block_work"),
        "account_access": ("state_trie", "next_layer_candidate"),
        "storage_access": ("state_trie", "next_layer_candidate"),
        "dirty_state_update": ("state_trie", "next_layer_candidate"),
        "final_trie_update": ("state_trie", "next_layer_candidate"),
        "witness_state_input_validation": ("state_trie", "next_layer_candidate"),
        "block_context": ("block", "next_layer_candidate"),
    }
    if set(indexed) != set(expected):
        raise ValueError("composite estimator side-effect ownership differs")
    for event_id, (owner, treatment) in expected.items():
        row = indexed.get(event_id)
        if not isinstance(row, Mapping) or (
            row.get("owner"), row.get("cost_treatment")
        ) != (owner, treatment):
            raise ValueError("composite estimator side-effect ownership differs")


def _validate_storage_model(model: Any) -> dict[str, Decimal]:
    if (
        not isinstance(model, Mapping)
        or set(model) != {"family", "parameter_order", "parameters", "ownership"}
        or model.get("family") != "M_typed"
        or model.get("parameter_order") != list(_STORAGE_MODEL_PARAMETER_ORDER)
        or not isinstance(model.get("parameters"), Mapping)
        or set(model["parameters"]) != set(_STORAGE_MODEL_PARAMETER_ORDER)
        or model.get("ownership")
        != {
            "measured": (
                "stateful REVM execution cost including storage execution, journal "
                "updates, and result-state construction"
            ),
            "excluded": [
                "witness materialization",
                "persistent dirty-state commit",
                "trie hashing",
                "final state root",
            ],
        }
    ):
        raise ValueError("composite estimator storage model differs")
    parameters = {
        key: _canonical_decimal(
            value, label=f"composite storage parameter {key}"
        )
        for key, value in model["parameters"].items()
    }
    if any(
        parameters[key] <= 0
        for key in _STORAGE_MODEL_PARAMETER_ORDER
        if key not in {"sload_cold_extra", "sstore_cold_extra"}
    ):
        raise ValueError("composite estimator storage body parameter must be positive")
    if (
        parameters["sload_warm_body"] + parameters["sload_cold_extra"] <= 0
        or any(
            parameters[f"sstore_branch:{branch}"]
            + parameters["sstore_cold_extra"]
            <= 0
            for branch in _STORAGE_BRANCHES
        )
    ):
        raise ValueError("composite estimator storage prediction must be positive")
    return parameters


def validate_estimator_artifact(artifact: Mapping[str, Any]) -> OpcodeRegistry:
    """Validate every semantic field used to evaluate a sealed artifact."""
    if not isinstance(artifact, Mapping):
        raise ValueError("composite estimator must be an object")
    _validate_content_address(artifact)
    if set(artifact) != _EXPECTED_ESTIMATOR_FIELDS:
        raise ValueError("composite estimator schema differs")
    if (
        artifact.get("schema_version") != ESTIMATOR_SCHEMA_VERSION
        or artifact.get("purpose") != ESTIMATOR_PURPOSE
        or artifact.get("status") != "sealed_coverage_qualified_estimator"
        or artifact.get("review_only") is not True
        or artifact.get("production_write") is not False
        or artifact.get("proposal_validation_opened") is not False
        or artifact.get("registry_parameter_basis") != "production_scaled"
        or artifact.get("formula") != ESTIMATOR_FORMULA
        or not _is_git_revision(artifact.get("implementation_revision"))
        or artifact.get("version_identity") != VERSION_IDENTITY
    ):
        raise ValueError("composite estimator header differs")
    trace_schema = artifact.get("trace_schema")
    if (
        not isinstance(trace_schema, Mapping)
        or set(trace_schema) != {"schema_version", "source_path", "source_sha256"}
        or trace_schema.get("schema_version") != TRACE_SCHEMA_VERSION
        or trace_schema.get("source_path") != "crates/zkgas-trace/src/reconstruct.rs"
        or not _is_sha256(trace_schema.get("source_sha256"))
    ):
        raise ValueError("composite estimator trace schema differs")
    fixed = artifact.get("fixed_costs")
    if not isinstance(fixed, Mapping) or set(fixed) != _FIXED_COST_KEYS:
        raise ValueError("composite estimator fixed costs differ")
    for key, value in fixed.items():
        _canonical_decimal(
            value, label=f"composite estimator fixed cost {key}", nonnegative=True
        )
    if artifact.get("fixed_cost_statuses") != FIXED_COST_STATUSES:
        raise ValueError("composite estimator fixed cost statuses differ")
    if artifact.get("coarse_state_trie") != COARSE_STATE_TRIE:
        raise ValueError("composite estimator coarse state/trie status differs")
    if artifact.get("coverage_policy") != COVERAGE_POLICY:
        raise ValueError("composite estimator coverage policy differs")
    _validate_sources(artifact.get("source_artifacts"))
    _validate_ownership_policy(artifact.get("ownership_policy"))
    registry = load_registry_payload(artifact.get("registry"))
    _validate_storage_model(artifact.get("storage_model"))
    if _sha256(_canonical_json(artifact["storage_model"])) != artifact[
        "source_artifacts"
    ]["stateful_storage"]["model_sha256"]:
        raise ValueError("composite estimator storage model source digest differs")
    _validate_coverage(artifact, registry)
    if trace_schema["source_sha256"] != artifact["source_artifacts"][
        "source_code_sha256s"
    ][trace_schema["source_path"]]:
        raise ValueError("composite estimator trace schema source differs")
    return registry


def _nonnegative_int(value: Any, *, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"{label} must be a nonnegative integer")
    return value


def _opcode_event(opcode: int, component: Mapping[str, Any]) -> OpcodeEvent:
    model_input = component.get("model_input")
    if not isinstance(model_input, Mapping) or not isinstance(
        model_input.get("kind"), str
    ):
        raise ValueError("opcode model input is missing")
    kind = model_input["kind"]
    common = {"opcode": opcode}
    if kind == "static_raw_gas":
        if set(model_input) != {"kind", "raw_gas"}:
            raise ValueError("static opcode model input fields differ")
        raw_gas = _nonnegative_int(model_input["raw_gas"], label="static opcode raw gas")
        if component.get("interpreter_raw_gas") != raw_gas:
            raise ValueError("static opcode raw gas differs from interpreter raw gas")
        return OpcodeEvent(**common, raw_gas=raw_gas)
    memory_fields = {
        "memory_growth_event",
        "memory_evm_gas_delta",
        "memory_4k_boundary_event",
    }
    memory = {
        field: _nonnegative_int(model_input[field], label=f"opcode model input {field}")
        for field in memory_fields
        if field in model_input
    }
    if kind == "exp":
        if set(model_input) != {"kind", "exponent_byte_length"}:
            raise ValueError("EXP model input fields differ")
        return OpcodeEvent(
            **common,
            exponent_byte_length=_nonnegative_int(
                model_input["exponent_byte_length"], label="EXP exponent byte length"
            ),
        )
    if kind == "keccak":
        if set(model_input) != {"kind", "input_length", *memory_fields}:
            raise ValueError("KECCAK model input fields differ")
        return OpcodeEvent(
            **common,
            input_length=_nonnegative_int(
                model_input["input_length"], label="KECCAK input length"
            ),
            **memory,
        )
    if kind == "memory_access":
        if set(model_input) != {"kind", *memory_fields}:
            raise ValueError("memory-access model input fields differ")
        return OpcodeEvent(**common, **memory)
    if kind == "memory_copy":
        if set(model_input) != {"kind", "copy_words", *memory_fields}:
            raise ValueError("memory-copy model input fields differ")
        return OpcodeEvent(
            **common,
            copy_words=_nonnegative_int(
                model_input["copy_words"], label="memory-copy words"
            ),
            **memory,
        )
    if kind == "invalid" and set(model_input) == {"kind"}:
        return OpcodeEvent(**common)
    raise ValueError("opcode model input kind is unsupported")


def _storage_cost(
    opcode: int,
    component: Mapping[str, Any],
    parameters: Mapping[str, Decimal],
) -> Decimal:
    model_input = component.get("model_input")
    if not isinstance(model_input, Mapping):
        raise ValueError("storage opcode model input is missing")
    access = model_input.get("access")
    if access not in {"warm", "cold"}:
        raise ValueError("storage access class is invalid")
    if opcode == 0x54:
        if set(model_input) != {"kind", "access"} or model_input.get("kind") != "storage_load":
            raise ValueError("SLOAD model input fields differ")
        cost = parameters["sload_warm_body"]
        if access == "cold":
            cost += parameters["sload_cold_extra"]
        return cost
    if opcode == 0x55:
        if (
            set(model_input) != {"kind", "access", "branch"}
            or model_input.get("kind") != "storage_store"
            or model_input.get("branch") not in _STORAGE_BRANCHES
        ):
            raise ValueError("SSTORE model input fields differ")
        branch = model_input["branch"]
        if access == "cold" and branch in {"dirty_rewrite", "restore_original"}:
            raise ValueError("dirty SSTORE branch must be warm")
        cost = parameters[f"sstore_branch:{branch}"]
        if access == "cold":
            cost += parameters["sstore_cold_extra"]
        return cost
    raise ValueError("storage model received a non-storage opcode")


def _gap(
    gaps: list[dict[str, Any]],
    *,
    reason: str,
    layer: str,
    block_index: int | None = None,
    operation_id: int | None = None,
    execution_key: str | None = None,
    detail: Any = None,
) -> None:
    gap = {"gap_id": len(gaps), "layer": layer, "reason": reason}
    if block_index is not None:
        gap["block_index"] = block_index
    if operation_id is not None:
        gap["operation_id"] = operation_id
    if execution_key is not None:
        gap["execution_key"] = execution_key
    if detail is not None:
        gap["detail"] = detail
    gaps.append(gap)


def _coverage_entry(
    numerator: int | Decimal, denominator: int | Decimal
) -> dict[str, Any]:
    numerator_decimal = Decimal(numerator)
    denominator_decimal = Decimal(denominator)
    ratio = Decimal(1) if denominator_decimal == 0 else numerator_decimal / denominator_decimal
    if isinstance(numerator, Decimal) or isinstance(denominator, Decimal):
        rendered_numerator: int | str = _decimal_text(numerator_decimal)
        rendered_denominator: int | str = _decimal_text(denominator_decimal)
    else:
        rendered_numerator = numerator
        rendered_denominator = denominator
    return {
        "numerator": rendered_numerator,
        "denominator": rendered_denominator,
        "ratio": _decimal_text(ratio),
    }


def _report_join_mismatches(
    trace: Mapping[str, Any], report: Mapping[str, Any]
) -> list[str]:
    mismatches = []
    expected = {
        "stage": "proposal",
        "mode": "execute",
        "sp1_execution_engine": "gas-estimator",
        "sp1_gas_trace_chunk_threshold": SP1_GAS_TRACE_CHUNK_THRESHOLD,
        "sp1_gas_trace_chunk_slots": SP1_GAS_TRACE_CHUNK_SLOTS,
        "guest_input_sha256": trace.get("guest_input_sha256"),
        "guest_input_bincode_length": trace.get("guest_input_bincode_length"),
        "public_values": trace.get("public_output"),
        "exit_code": 0,
    }
    for field, value in expected.items():
        if report.get(field) != value:
            mismatches.append(field)
    gas = report.get("gas")
    if isinstance(gas, bool) or not isinstance(gas, int) or gas <= 0:
        mismatches.append("gas")
    if report.get("primary_workload_metric") != {
        "label": "prover_gas",
        "count": gas,
    }:
        mismatches.append("primary_workload_metric")
    return mismatches


def _parse_address(value: Any) -> int:
    if isinstance(value, bool):
        raise ValueError("precompile address is invalid")
    if isinstance(value, int):
        return value
    if not isinstance(value, str):
        raise ValueError("precompile address is invalid")
    return int(value, 16 if value.lower().startswith("0x") else 10)


def _validate_trace_header(trace: Mapping[str, Any], estimator: Mapping[str, Any]) -> None:
    required = {
        "schema_version",
        "guest_input_sha256",
        "guest_input_bincode_length",
        "status",
        "blocks",
        "partial_blocks",
        "recovery_failures",
        "parity",
    }
    if not isinstance(trace, Mapping) or not required.issubset(trace):
        raise ValueError("composite proposal trace schema is incomplete")
    if set(trace) - (required | {"public_output", "failure"}):
        raise ValueError("composite proposal trace has unknown fields")
    if trace.get("schema_version") != estimator["trace_schema"]["schema_version"]:
        raise ValueError("composite proposal trace schema version differs")
    guest_hash = trace.get("guest_input_sha256")
    parity = trace.get("parity")
    if (
        not isinstance(guest_hash, str)
        or not _is_sha256(guest_hash.removeprefix("0x"))
        or isinstance(trace.get("guest_input_bincode_length"), bool)
        or not isinstance(trace.get("guest_input_bincode_length"), int)
        or trace["guest_input_bincode_length"] <= 0
        or trace.get("status") not in {"complete", "failed"}
        or not isinstance(trace.get("blocks"), list)
        or not isinstance(trace.get("partial_blocks"), list)
        or not isinstance(trace.get("recovery_failures"), list)
        or not isinstance(parity, Mapping)
        or set(parity) != {"passed", "mismatch_fields"}
        or not isinstance(parity.get("passed"), bool)
        or not isinstance(parity.get("mismatch_fields"), list)
        or any(not isinstance(field, str) for field in parity["mismatch_fields"])
    ):
        raise ValueError("composite proposal trace identity differs")
    public_output = trace.get("public_output")
    if trace["status"] == "complete":
        if (
            not isinstance(public_output, str)
            or not public_output.startswith("0x")
            or not _is_sha256(public_output.removeprefix("0x"))
            or "failure" in trace
            or parity["passed"] is not True
            or parity["mismatch_fields"]
        ):
            raise ValueError("complete proposal trace public output or parity differs")
    else:
        failure = trace.get("failure")
        if (
            public_output is not None
            or parity["passed"] is not False
            or not isinstance(failure, Mapping)
            or set(failure) - {"block_index", "stage", "error"}
            or not {"stage", "error"}.issubset(failure)
        ):
            raise ValueError("failed proposal trace failure record differs")


def _validate_spawn_wrapper_component(component: Mapping[str, Any]) -> bool:
    """Validate strict Rust wire shapes before any zero-charge ownership exit."""
    if component.get("kind") != "opcode":
        return False
    dispatch = component.get("dispatch_status")
    if dispatch not in {"confirmed", "selected_not_dispatched"}:
        return False
    opcode = component.get("opcode")
    if (
        isinstance(opcode, bool)
        or not isinstance(opcode, int)
        or opcode not in _SPAWN_OPCODES
    ):
        raise ValueError("spawn wrapper opcode differs from the schema")
    if dispatch == "confirmed":
        expected_fields = {
            "kind",
            "opcode",
            "pricing_basis",
            "spawned",
            "dispatch_status",
        }
        if (
            set(component) != expected_fields
            or component.get("pricing_basis") != "fixed_per_event"
            or component.get("spawned") is not True
        ):
            raise ValueError("confirmed spawn wrapper fields differ from the schema")
    else:
        expected_fields = {"kind", "opcode", "spawned", "dispatch_status"}
        if set(component) != expected_fields or component.get("spawned") is not True:
            raise ValueError(
                "selected-not-dispatched spawn wrapper fields differ from the schema"
            )
    return True


def estimate_trace(
    estimator: Mapping[str, Any],
    trace: Mapping[str, Any],
    *,
    sp1_report: Mapping[str, Any] | None = None,
) -> Mapping[str, Any]:
    """Apply a sealed estimator while preserving subtotal, gaps, and coverage."""
    with localcontext(_DECIMAL_CONTEXT):
        return _estimate_trace(estimator, trace, sp1_report=sp1_report)


def _estimate_trace(
    estimator: Mapping[str, Any],
    trace: Mapping[str, Any],
    *,
    sp1_report: Mapping[str, Any] | None,
) -> Mapping[str, Any]:
    registry = validate_estimator_artifact(estimator)
    storage_parameters = _validate_storage_model(estimator["storage_model"])
    _validate_trace_header(trace, estimator)
    gaps: list[dict[str, Any]] = []
    if trace.get("status") != "complete":
        _gap(gaps, reason="incomplete_trace", layer="proposal")
    if trace["parity"]["passed"] is not True:
        _gap(
            gaps,
            reason="trace_parity_failure",
            layer="proposal",
            detail=trace["parity"]["mismatch_fields"],
        )
    if trace["partial_blocks"]:
        _gap(
            gaps,
            reason="partial_block_trace",
            layer="block",
            detail={"count": len(trace["partial_blocks"])},
        )
    if trace["recovery_failures"]:
        _gap(
            gaps,
            reason="transaction_recovery_failure",
            layer="transaction",
            detail={"count": len(trace["recovery_failures"])},
        )

    fixed = {
        key: _canonical_decimal(
            value, label=f"composite fixed cost {key}", nonnegative=True
        )
        for key, value in estimator["fixed_costs"].items()
    }
    block_count = len(trace["blocks"])
    transaction_count = 0
    native_count = 0
    operation_cost = Decimal(0)
    measured_operation_count = 0
    total_operation_count = 0
    measured_raw_gas = Decimal(0)
    total_raw_gas = Decimal(0)
    measured_spawn_count = 0
    total_spawn_count = 0
    measured_precompile_count = 0
    total_precompile_count = 0
    measured_typed_feature_count = 0
    total_typed_feature_count = 0
    block_owned_operation_count = 0
    coverage_by_key = {row["key"]: row for row in estimator["execution_coverage"]}
    seen_blocks = set()
    for block in trace["blocks"]:
        required_block_fields = {
            "block_index",
            "block_number",
            "input_transaction_count",
            "started_transaction_count",
            "committed_transaction_hashes",
            "attempted_transaction_hashes",
            "unattempted_transaction_hashes",
            "native_value_transfer_count",
            "finalized_block_zkgas",
            "transactions",
            "operations",
        }
        if not isinstance(block, Mapping) or set(block) != required_block_fields:
            raise ValueError("composite block trace schema differs")
        block_index = _nonnegative_int(block["block_index"], label="block index")
        if block_index in seen_blocks:
            raise ValueError("composite block trace contains duplicate block indices")
        seen_blocks.add(block_index)
        _nonnegative_int(block["block_number"], label="block number")
        _nonnegative_int(
            block["input_transaction_count"], label="input transaction count"
        )
        _nonnegative_int(
            block["started_transaction_count"], label="started transaction count"
        )
        _nonnegative_int(
            block["native_value_transfer_count"],
            label="native value transfer count",
        )
        _nonnegative_int(
            block["finalized_block_zkgas"], label="finalized block zkGas"
        )
        transactions = block["transactions"]
        operations = block["operations"]
        hash_fields = (
            "committed_transaction_hashes",
            "attempted_transaction_hashes",
            "unattempted_transaction_hashes",
        )
        if (
            not isinstance(transactions, list)
            or not isinstance(operations, list)
            or any(not isinstance(block[field], list) for field in hash_fields)
            or any(
                not isinstance(tx_hash, str)
                or not tx_hash.startswith("0x")
                or not _is_sha256(tx_hash.removeprefix("0x"))
                for field in hash_fields
                for tx_hash in block[field]
            )
        ):
            raise ValueError("composite block trace rows differ")
        if block["input_transaction_count"] != len(transactions):
            raise ValueError("composite block input transaction count differs")

        operation_transaction_indices = {
            operation.get("tx_index")
            for operation in operations
            if isinstance(operation, Mapping)
            and operation.get("phase") == "transaction"
            and isinstance(operation.get("tx_index"), int)
            and not isinstance(operation.get("tx_index"), bool)
        }
        started_by_index = {}
        recovered_indices = set()
        native_markers = 0
        for transaction in transactions:
            required_transaction_fields = {
                "recovered_index",
                "tx_hash",
                "is_anchor",
                "disposition",
                "native_value_transfer",
            }
            allowed_transaction_fields = required_transaction_fields | {
                "started_tx_index",
                "manifest_index",
            }
            if (
                not isinstance(transaction, Mapping)
                or not required_transaction_fields.issubset(transaction)
                or set(transaction) - allowed_transaction_fields
            ):
                raise ValueError("composite transaction trace schema differs")
            recovered_index = _nonnegative_int(
                transaction["recovered_index"], label="recovered transaction index"
            )
            if recovered_index in recovered_indices:
                raise ValueError("duplicate recovered transaction index")
            recovered_indices.add(recovered_index)
            if recovered_index != len(recovered_indices) - 1:
                raise ValueError("recovered transaction order differs")
            manifest_index = transaction.get("manifest_index")
            if manifest_index is not None:
                _nonnegative_int(manifest_index, label="transaction manifest index")
            tx_hash = transaction["tx_hash"]
            if (
                not isinstance(tx_hash, str)
                or not tx_hash.startswith("0x")
                or not _is_sha256(tx_hash.removeprefix("0x"))
            ):
                raise ValueError("transaction hash differs")
            if transaction.get("is_anchor") is not (recovered_index == 0):
                raise ValueError("transaction anchor identity differs")
            started = transaction.get("started_tx_index")
            disposition = transaction.get("disposition")
            if started is None:
                if disposition != "unattempted":
                    raise ValueError("non-unattempted transaction is missing started index")
                continue
            started = _nonnegative_int(started, label="started transaction index")
            if started in started_by_index:
                raise ValueError("duplicate started transaction index")
            started_by_index[started] = transaction
            if disposition not in {"committed_success", "committed_failure", "attempted"}:
                raise ValueError("started transaction disposition differs")
            if transaction.get("is_anchor") is not True:
                transaction_count += 1
            if transaction.get("native_value_transfer") is True:
                native_markers += 1
                if (
                    transaction.get("is_anchor") is False
                    and disposition == "committed_success"
                    and started not in operation_transaction_indices
                ):
                    native_count += 1
                else:
                    _gap(
                        gaps,
                        reason="invalid_native_transfer_marker",
                        layer="transaction",
                        block_index=block_index,
                        detail={"started_tx_index": started},
                    )
            elif transaction.get("native_value_transfer") is not False:
                raise ValueError("native transfer marker must be a boolean")
        if block["started_transaction_count"] != len(started_by_index):
            raise ValueError("composite block started transaction count differs")
        if set(started_by_index) != set(range(len(started_by_index))):
            raise ValueError("composite block started transaction indices differ")
        if block["native_value_transfer_count"] != native_markers:
            raise ValueError("composite block native transfer count differs")
        dispositions = {
            "committed_transaction_hashes": {"committed_success", "committed_failure"},
            "attempted_transaction_hashes": {"attempted"},
            "unattempted_transaction_hashes": {"unattempted"},
        }
        for field, allowed_dispositions in dispositions.items():
            expected_hashes = [
                transaction["tx_hash"]
                for transaction in transactions
                if transaction["disposition"] in allowed_dispositions
            ]
            if block[field] != expected_hashes:
                raise ValueError(f"composite block {field} differs")

        seen_operations = set()
        for operation in operations:
            required_operation_fields = {"operation_id", "phase", "frame_depth", "component"}
            if (
                not isinstance(operation, Mapping)
                or set(operation) - (required_operation_fields | {"tx_index"})
                or not required_operation_fields.issubset(operation)
                or not isinstance(operation.get("component"), Mapping)
            ):
                raise ValueError("composite operation trace schema differs")
            operation_id = _nonnegative_int(operation["operation_id"], label="operation id")
            _nonnegative_int(operation["frame_depth"], label="operation frame depth")
            if operation_id in seen_operations:
                raise ValueError("composite block has duplicate operation ids")
            seen_operations.add(operation_id)
            phase = operation["phase"]
            tx_index = operation.get("tx_index")
            if phase == "system":
                if tx_index is not None:
                    raise ValueError("system operation must not have a transaction index")
                block_owned_operation_count += 1
                continue
            if phase != "transaction":
                raise ValueError("operation phase differs")
            if isinstance(tx_index, bool) or not isinstance(tx_index, int):
                _gap(
                    gaps,
                    reason="operation_transaction_join_missing",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                )
                continue
            transaction = started_by_index.get(tx_index)
            if transaction is None:
                _gap(
                    gaps,
                    reason="operation_transaction_join_missing",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                )
                continue
            if transaction.get("is_anchor") is True:
                block_owned_operation_count += 1
                continue

            component = operation["component"]
            kind = component.get("kind")
            is_spawn_wrapper = _validate_spawn_wrapper_component(component)
            if is_spawn_wrapper and component["dispatch_status"] == "selected_not_dispatched":
                continue
            total_operation_count += 1
            if kind == "opcode_feature_error":
                raw_gas = _nonnegative_int(
                    component.get("interpreter_raw_gas"),
                    label="feature-error interpreter raw gas",
                )
                total_raw_gas += Decimal(raw_gas)
                total_typed_feature_count += 1
                _gap(
                    gaps,
                    reason="opcode_feature_error",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=f"opcode:0x{component.get('opcode', 0):02x}",
                    detail=component.get("error"),
                )
                continue
            if kind == "precompile":
                total_precompile_count += 1
                native_gas = _nonnegative_int(
                    component.get("native_gas"), label="precompile native gas"
                )
                total_raw_gas += Decimal(native_gas)
                try:
                    address = _parse_address(component.get("address"))
                except (TypeError, ValueError) as error:
                    raise ValueError("precompile trace address differs") from error
                key = f"precompile:0x{address:02x}"
                coverage = coverage_by_key.get(key)
                if component.get("pricing_basis") != "raw_gas_slope" or coverage is None:
                    raise ValueError("precompile trace is outside frozen coverage")
                _gap(
                    gaps,
                    reason="declared_unmeasured_precompile",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                )
                continue
            if kind != "opcode":
                raise ValueError("operation component kind differs")
            opcode = _nonnegative_int(component.get("opcode"), label="opcode")
            if opcode > 0xFF:
                raise ValueError("opcode must fit one byte")
            key = f"opcode:0x{opcode:02x}"
            if is_spawn_wrapper and component["dispatch_status"] == "confirmed":
                total_spawn_count += 1
                _gap(
                    gaps,
                    reason="confirmed_spawn_wrapper_unmeasured",
                    layer="operation_wrapper",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                )
                continue
            expected_opcode_fields = {
                "kind",
                "opcode",
                "pricing_basis",
                "interpreter_raw_gas",
                "model_input",
                "spawned",
                "dispatch_status",
            }
            if (
                set(component) != expected_opcode_fields
                or component.get("pricing_basis") != "raw_gas_slope"
                or component.get("spawned") is not False
                or component.get("dispatch_status") != "not_applicable"
            ):
                raw_gas = component.get("interpreter_raw_gas")
                if isinstance(raw_gas, int) and not isinstance(raw_gas, bool) and raw_gas >= 0:
                    total_raw_gas += Decimal(raw_gas)
                _gap(
                    gaps,
                    reason="opcode_trace_schema_mismatch",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                )
                continue
            raw_gas = _nonnegative_int(
                component.get("interpreter_raw_gas"), label="opcode interpreter raw gas"
            )
            total_raw_gas += Decimal(raw_gas)
            coverage = coverage_by_key.get(key)
            if coverage is None:
                _gap(
                    gaps,
                    reason="opcode_outside_frozen_coverage",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                )
                continue
            if coverage.get("classification") == "structured_storage":
                total_typed_feature_count += 1
                try:
                    predicted = _storage_cost(
                        opcode, component, storage_parameters
                    )
                except (TypeError, ValueError) as error:
                    _gap(
                        gaps,
                        reason="opcode_model_input_incompatible",
                        layer="operation",
                        block_index=block_index,
                        operation_id=operation_id,
                        execution_key=key,
                        detail=str(error),
                    )
                    continue
                operation_cost += predicted
                measured_operation_count += 1
                measured_raw_gas += Decimal(raw_gas)
                measured_typed_feature_count += 1
                continue
            if coverage.get("classification") == "explicitly_unsupported":
                _gap(
                    gaps,
                    reason="explicitly_unsupported_opcode",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                    detail=coverage.get("reason"),
                )
                continue
            total_typed_feature_count += 1
            try:
                event = _opcode_event(opcode, component)
                predicted = predict_opcode_event(registry, event)
            except (TypeError, ValueError) as error:
                _gap(
                    gaps,
                    reason="opcode_model_input_incompatible",
                    layer="operation",
                    block_index=block_index,
                    operation_id=operation_id,
                    execution_key=key,
                    detail=str(error),
                )
                continue
            operation_cost += predicted
            measured_operation_count += 1
            measured_raw_gas += Decimal(raw_gas)
            measured_typed_feature_count += 1

    contributions = {
        "proposal_startup": {"count": 1, "prover_gas": _decimal_text(fixed["proposal_startup"])},
        "block_base": {
            "count": block_count,
            "prover_gas": _decimal_text(Decimal(block_count) * fixed["block_base"]),
        },
        "transaction_base": {
            "count": transaction_count,
            "prover_gas": _decimal_text(Decimal(transaction_count) * fixed["tx_base"]),
        },
        "native_value_transfer": {
            "count": native_count,
            "prover_gas": _decimal_text(Decimal(native_count) * fixed["native_value_transfer"]),
        },
        "operations": {
            "count": measured_operation_count,
            "prover_gas": _decimal_text(operation_cost),
        },
    }
    subtotal = sum(
        (
            _canonical_decimal(row["prover_gas"], label="layer contribution")
            for row in contributions.values()
        ),
        Decimal(0),
    )
    coverage = {
        "operation_count": _coverage_entry(measured_operation_count, total_operation_count),
        "raw_gas": _coverage_entry(measured_raw_gas, total_raw_gas),
        "spawn_wrapper": _coverage_entry(measured_spawn_count, total_spawn_count),
        "precompile": _coverage_entry(measured_precompile_count, total_precompile_count),
        "typed_feature": _coverage_entry(
            measured_typed_feature_count, total_typed_feature_count
        ),
    }
    result = {
        "schema_version": 1,
        "estimator_artifact_sha256": estimator["artifact_sha256"],
        "guest_input_sha256": trace["guest_input_sha256"],
        "modeled_subtotal": _decimal_text(subtotal),
        "layer_contributions": contributions,
        "coverage": coverage,
        "coverage_complete": not gaps,
        "block_owned_operation_count": block_owned_operation_count,
        "gaps": gaps,
        "actual_report_join": "not_provided",
        "validation_status": "not_evaluated",
    }
    if not gaps:
        result["predicted_prover_gas"] = result["modeled_subtotal"]
    if sp1_report is None:
        return result
    if not isinstance(sp1_report, Mapping):
        raise ValueError("SP1 report must be an object")
    mismatches = _report_join_mismatches(trace, sp1_report)
    if mismatches:
        result["actual_report_join"] = "mismatch"
        result["actual_report_join_mismatches"] = mismatches
        result["validation_status"] = "report_join_mismatch"
        return result
    actual = Decimal(sp1_report["gas"])
    result["actual_report_join"] = "joined"
    result["actual_prover_gas"] = _decimal_text(actual)
    if gaps:
        result["validation_status"] = "insufficient_coverage"
        return result
    result["ape"] = _decimal_text(abs(subtotal - actual) / actual)
    result["validation_status"] = "evaluated"
    return result
