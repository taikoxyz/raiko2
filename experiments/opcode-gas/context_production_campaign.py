#!/usr/bin/env python3
"""Frozen contract for production-guest context-opcode calibration."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re
import stat
import subprocess
import tempfile
from dataclasses import dataclass
from decimal import Context, Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from types import MappingProxyType
from typing import Any, Callable, Iterable, Mapping, Sequence

from opcode_gas import canonical_json, sha256_bytes


SCHEMA_VERSION = 1
PURPOSE = "production_context_opcode_calibration"
FIT_COUNTS = (0, 1, 2, 4, 8, 16)
VALIDATION_COUNTS = (0, 32, 64)
REPEATS = 3
KEYS = (
    "opcode:0x30",
    "opcode:0x33",
    "opcode:0x34",
    "opcode:0x35",
    "opcode:0x36",
    "opcode:0x42",
)
MODEL_SELECTION_ORDER = ("calldatasize_length", "calldatasize_boundary")
_SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")
_GIT_REVISION_RE = re.compile(r"[0-9a-f]{40}\Z")
_DECIMAL_CONTEXT = Context(prec=80, rounding=ROUND_HALF_EVEN)

SOURCES = {
    "operation_coverage_v5": {
        "path": "experiments/opcode-gas/manifests/operation-coverage-v5.json",
        "file_sha256": "4c67852165636d1991e5dd21716dc225e8df8bbf79245763e5c6bca2ce505b89",
        "artifact_sha256": "5e3f9aae0d3d9ae10a9b05bfbe877b2f9c4ab146bd7d6df66f915211c4a9108d",
    },
    "higher_layer": {
        "path": "experiments/opcode-gas/derivations/3e4de6eecb5e92aa59a6a4b9/higher-layer-calibration.json",
        "file_sha256": "e06e90546f48b59f52670ceb320c63a5da64e910c7154efc95bafc4fa5579a54",
        "model_identity_sha256": "91d461f0a817446a802237987c08f1290c571d19b245e7655084600ba05b68ff",
        "directory_identity_path": "experiments/opcode-gas/derivations/3e4de6eecb5e92aa59a6a4b9/identity.json",
        "directory_identity_file_sha256": "b68c892a25cec824cc5805b9f20baee163287caa91b0d146261e4497b042391e",
        "directory_identity_sha256": "3e4de6eecb5e92aa59a6a4b9d380765b94ddf9b1b8198479659393a878694f24",
    },
    "discovery": {
        "path": "experiments/opcode-gas/derivations/79dcfe2d5be2c3432987a671/result.json",
        "file_sha256": "b8524de5dfa392cec021433ca499408aa56bf016bcb6c2363b790d9819677686",
        "result_identity_sha256": "79dcfe2d5be2c3432987a671f92ae94b44361c4a0ae914218c149efe7eaa8734",
        "artifact_sha256": "b5c1e0efdf22d6c0bb1fef07e0a42785af6d49ae68ebf2c36fafb6399b541675",
        "numeric_parameter_authority": False,
        "use": "function_shape_and_feature_vocabulary_only",
    },
}

EXECUTION = {
    "stage": "controlled-block",
    "proof_type": "sp1",
    "mode": "execute",
    "sp1_prover": "local",
    "sp1_execution_engine": "gas-estimator",
    "launcher_path": "target/release/guest-launcher",
    "launcher_binding": "sha256_at_clean_prepare",
    "production_elf_path": "crates/guests/elf/sp1_shasta_proposal.elf",
    "production_vk_path": "crates/guests/elf/sp1_shasta_proposal.vk.bin",
    "guest_artifact_binding": "sha256_at_clean_prepare",
    "implementation_revision_binding": "clean_git_head_at_prepare",
    "trace_schema_source": "crates/zkgas-trace/src/reconstruct.rs",
    "trace_schema_version": 4,
    "trace_source_binding": "sha256_at_clean_prepare",
}

VERSION_IDENTITY = {
    "taiko_fork": "Unzen",
    "ethereum_upgrade": "Fusaka",
    "revm_spec_id": "OSAKA",
    "production_schedule": "UNZEN_ZK_GAS_SCHEDULE",
    "primary_metric": "proverGas",
    "proving_backend": "sp1",
    "sp1_sdk_version": "6.3.0",
    "timestamp_semantics": "strictly_post_unzen_activation",
}

PRODUCTION_SCHEDULE_SHA256 = (
    "b27c29fb5fe482de4b3e22784de0cc6c49a5f1f0160ec8a6eaa6997869864927"
)

FIT_EQUATIONS = {
    "target_observed": "Y_s(n) = [P_s(n) - P_s(0)] - [K_s(n) - K_s(0)]",
    "target_predicted": "Y_hat_s(n) = n * f_s(context_features)",
    "control_residual": "C_s(n) = [P_control_s(n) - P_control_s(0)] - [K_control_s(n) - K_control_s(0)]",
    "ape": "abs(predicted_increment - observed_increment) / abs(observed_increment)",
}

QUALITY_GATES = {
    "signal_evaluation_count": 16,
    "signal_min_formula": "max(1000, 20 * 143)",
    "signal_min_prover_gas": 2860,
    "r2_min": "0.995",
    "relative_coefficient_stderr_max": "0.05",
    "fit_residual_signal_max": "0.02",
    "control_residual_abs_max_prover_gas": 2860,
    "control_residual_abs_max_formula": "max(1000, 20 * 143)",
    "count_holdout_ape_max": "0.10",
    "extrapolation_ape_max": "0.10",
    "final_scenario_row_ape_max": "0.10",
    "final_scenario_family_mape_max": "0.05",
    "sibling_slope_relative_difference_max": "0.05",
    "finite_nonnegative_coefficients_and_predictions": True,
}

REPEAT_CONTRACT = {
    "exact_repeats": REPEATS,
    "equality_fields": [
        "prover_gas",
        "public_output",
        "backend_input_sha256",
        "host_trace_sha256",
    ],
    "mismatch_outcome": "reject_scenario",
}

FAMILY_PROMOTION_CONTRACT = {
    "required_evidence": "all_required_scenarios_classes_and_gates",
    "required_gates": [
        "exact_event_matching",
        "control_lane_contamination",
        "repeat",
        "signal",
        "fit",
        "count_holdout",
        "extrapolation",
        "scenario_holdout",
    ],
    "partial_application": "forbidden",
    "failed_family_status": "explicit_gap",
    "final_holdout_parameter_influence": "forbidden",
    "final_holdout_model_switching": "forbidden",
}


def _candidate(name: str, key: str, formula: str, terms: tuple[str, ...]) -> dict[str, Any]:
    return {"name": name, "key": key, "formula": formula, "terms": list(terms)}


MODEL_CANDIDATES = (
    _candidate("address_constant", "opcode:0x30", "address_constant", ("address_constant",)),
    _candidate("caller_constant", "opcode:0x33", "caller_constant", ("caller_constant",)),
    _candidate(
        "callvalue_classes",
        "opcode:0x34",
        "callvalue_zero * I(value_class = zero) + callvalue_nonzero * I(value_class = nonzero)",
        ("callvalue_zero", "callvalue_nonzero"),
    ),
    _candidate(
        "calldataload_access_classes",
        "opcode:0x35",
        "load_zero * I(access_class = zero) + load_partial * I(access_class = partial) + load_full * I(access_class = full)",
        ("load_zero", "load_partial", "load_full"),
    ),
    _candidate(
        "calldatasize_length",
        "opcode:0x36",
        "beta_0 + beta_length * input_length",
        ("beta_0", "beta_length"),
    ),
    _candidate(
        "calldatasize_boundary",
        "opcode:0x36",
        "beta_0 + beta_words * ceil(input_length / 32) + beta_partial * I(input_length mod 32 != 0)",
        ("beta_0", "beta_words", "beta_partial"),
    ),
    _candidate(
        "timestamp_nonzero",
        "opcode:0x42",
        "timestamp_nonzero * I(value_class = nonzero)",
        ("timestamp_nonzero",),
    ),
)


def _scenario(
    name: str,
    key: str,
    opcode: int,
    control_opcode: int,
    split: str,
    model_class: str,
    context: Mapping[str, Any],
    *,
    reachability: str = "measured",
) -> dict[str, Any]:
    return {
        "name": name,
        "key": key,
        "opcode": opcode,
        "control_opcode": control_opcode,
        "split": split,
        "model_class": model_class,
        "context": dict(context),
        "reachability": reachability,
    }


def _canonical_scenarios() -> list[dict[str, Any]]:
    rows = [
        _scenario("address_canonical", "opcode:0x30", 0x30, 0x5F, "fit", "address_constant", {"address_profile": "canonical"}),
        _scenario("caller_canonical", "opcode:0x33", 0x33, 0x5F, "fit", "caller_constant", {"caller_profile": "canonical"}),
        _scenario("callvalue_zero", "opcode:0x34", 0x34, 0x5F, "fit", "callvalue_zero", {"value": "0", "value_class": "zero"}),
        _scenario("callvalue_nonzero_7", "opcode:0x34", 0x34, 0x5F, "fit", "callvalue_nonzero", {"value": "7", "value_class": "nonzero"}),
        _scenario("calldataload_empty_offset_0", "opcode:0x35", 0x35, 0x90, "fit", "load_zero", {"input_length": 0, "offset": 0, "access_class": "zero"}),
        _scenario("calldataload_full_32_offset_0", "opcode:0x35", 0x35, 0x90, "fit", "load_full", {"input_length": 32, "offset": 0, "access_class": "full"}),
        _scenario("calldataload_partial_33_offset_17", "opcode:0x35", 0x35, 0x90, "fit", "load_partial", {"input_length": 33, "offset": 17, "access_class": "partial"}),
        _scenario("calldataload_out_of_range_4_offset_64", "opcode:0x35", 0x35, 0x90, "fit", "load_zero", {"input_length": 4, "offset": 64, "access_class": "zero"}),
    ]
    rows.extend(
        _scenario(f"calldatasize_{length}", "opcode:0x36", 0x36, 0x5F, "fit", "calldata_size", {"input_length": length})
        for length in (0, 1, 31, 32, 33, 64)
    )
    rows.append(_scenario("timestamp_post_unzen_delta_17", "opcode:0x42", 0x42, 0x5F, "fit", "timestamp_nonzero", {"timestamp_delta": 17, "value_class": "nonzero"}))
    rows.extend(
        _scenario(f"calldatasize_{length}", "opcode:0x36", 0x36, 0x5F, "model_selection", "calldata_size", {"input_length": length})
        for length in (2, 63, 65, 96)
    )
    rows.extend(
        [
            _scenario("address_alternate", "opcode:0x30", 0x30, 0x5F, "final_holdout", "address_constant", {"address_profile": "alternate"}),
            _scenario("caller_alternate", "opcode:0x33", 0x33, 0x5F, "final_holdout", "caller_constant", {"caller_profile": "alternate"}),
            _scenario("callvalue_nonzero_4294967297", "opcode:0x34", 0x34, 0x5F, "final_holdout", "callvalue_nonzero", {"value": "4294967297", "value_class": "nonzero"}),
            _scenario("calldataload_partial_31_offset_30", "opcode:0x35", 0x35, 0x90, "final_holdout", "load_partial", {"input_length": 31, "offset": 30, "access_class": "partial"}),
            _scenario("calldataload_full_96_offset_32", "opcode:0x35", 0x35, 0x90, "final_holdout", "load_full", {"input_length": 96, "offset": 32, "access_class": "full"}),
        ]
    )
    rows.extend(
        _scenario(f"calldatasize_{length}", "opcode:0x36", 0x36, 0x5F, "final_holdout", "calldata_size", {"input_length": length})
        for length in (15, 47, 127, 255)
    )
    rows.extend(
        [
            _scenario("timestamp_post_unzen_delta_86400", "opcode:0x42", 0x42, 0x5F, "final_holdout", "timestamp_nonzero", {"timestamp_delta": 86400, "value_class": "nonzero"}),
            _scenario("timestamp_zero_unreachable", "opcode:0x42", 0x42, 0x5F, "unreachable", "timestamp_zero", {"timestamp_delta": 0}, reachability="unreachable_under_version_identity"),
        ]
    )
    return rows


def _operation(
    mnemonic: str,
    key: str,
    opcode: int,
    trace_input_kind: str,
    control_opcode: int,
    candidate_ids: tuple[str, ...],
    required_scenario_ids: tuple[str, ...],
    diagnostic_scenario_ids: tuple[str, ...] = (),
) -> dict[str, Any]:
    return {
        "mnemonic": mnemonic,
        "key": key,
        "opcode": opcode,
        "production_schedule": VERSION_IDENTITY["production_schedule"],
        "production_schedule_sha256": PRODUCTION_SCHEDULE_SHA256,
        "component_kind": "opcode",
        "trace_selector_ref": "opcode_raw_gas_execution",
        "transaction_scope_selector_ref": "non_anchor_started_transaction",
        "pricing_basis": "raw_gas_slope",
        "dispatch_status": "not_applicable",
        "spawned": False,
        "trace_input_kind": trace_input_kind,
        "control_opcode": control_opcode,
        "candidate_ids": list(candidate_ids),
        "required_scenario_ids": list(required_scenario_ids),
        "diagnostic_scenario_ids": list(diagnostic_scenario_ids),
    }


def _canonical_operations() -> list[dict[str, Any]]:
    return [
        _operation(
            "ADDRESS",
            "opcode:0x30",
            0x30,
            "context_fixed",
            0x5F,
            ("address_constant",),
            ("address_canonical", "address_alternate"),
        ),
        _operation(
            "CALLER",
            "opcode:0x33",
            0x33,
            "context_fixed",
            0x5F,
            ("caller_constant",),
            ("caller_canonical", "caller_alternate"),
        ),
        _operation(
            "CALLVALUE",
            "opcode:0x34",
            0x34,
            "context_value",
            0x5F,
            ("callvalue_classes",),
            (
                "callvalue_zero",
                "callvalue_nonzero_7",
                "callvalue_nonzero_4294967297",
            ),
        ),
        _operation(
            "CALLDATALOAD",
            "opcode:0x35",
            0x35,
            "calldata_load",
            0x90,
            ("calldataload_access_classes",),
            (
                "calldataload_empty_offset_0",
                "calldataload_full_32_offset_0",
                "calldataload_partial_33_offset_17",
                "calldataload_out_of_range_4_offset_64",
                "calldataload_partial_31_offset_30",
                "calldataload_full_96_offset_32",
            ),
        ),
        _operation(
            "CALLDATASIZE",
            "opcode:0x36",
            0x36,
            "calldata_size",
            0x5F,
            ("calldatasize_length", "calldatasize_boundary"),
            tuple(
                f"calldatasize_{length}"
                for length in (0, 1, 31, 32, 33, 64, 2, 63, 65, 96, 15, 47, 127, 255)
            ),
        ),
        _operation(
            "TIMESTAMP",
            "opcode:0x42",
            0x42,
            "context_value",
            0x5F,
            ("timestamp_nonzero",),
            (
                "timestamp_post_unzen_delta_17",
                "timestamp_post_unzen_delta_86400",
            ),
            ("timestamp_zero_unreachable",),
        ),
    ]


def canonical_production_context_manifest_payload() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "keys": list(KEYS),
        "fit_counts": list(FIT_COUNTS),
        "validation_counts": list(VALIDATION_COUNTS),
        "repeats": REPEATS,
        "sources": json.loads(canonical_json(SOURCES)),
        "execution": dict(EXECUTION),
        "version_identity": dict(VERSION_IDENTITY),
        "fit_equations": dict(FIT_EQUATIONS),
        "quality_gates": dict(QUALITY_GATES),
        "repeat_contract": json.loads(canonical_json(REPEAT_CONTRACT)),
        "family_promotion_contract": json.loads(
            canonical_json(FAMILY_PROMOTION_CONTRACT)
        ),
        "model_selection_order": list(MODEL_SELECTION_ORDER),
        "model_candidates": json.loads(canonical_json(MODEL_CANDIDATES)),
        "operations": _canonical_operations(),
        "scenarios": _canonical_scenarios(),
    }


def _reject_binary_floats(value: Any, path: str = "manifest") -> None:
    if isinstance(value, float):
        raise ValueError(f"{path} contains a binary float")
    if isinstance(value, Mapping):
        for key, child in value.items():
            _reject_binary_floats(child, f"{path}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _reject_binary_floats(child, f"{path}[{index}]")


def _require_relative_path(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value or pathlib.PurePath(value).is_absolute():
        raise ValueError(f"{field} must be a repository-relative path")
    path = pathlib.PurePosixPath(value)
    if ".." in path.parts or str(path) != value:
        raise ValueError(f"{field} must be a normalized repository-relative path")
    return value


@dataclass(frozen=True)
class ProductionContextModelCandidate:
    name: str
    key: str
    formula: str
    terms: tuple[str, ...]

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "ProductionContextModelCandidate":
        if not isinstance(value, Mapping) or set(value) != {"name", "key", "formula", "terms"}:
            raise ValueError("production context model candidate fields differ")
        if not isinstance(value["terms"], list) or not value["terms"] or len(set(value["terms"])) != len(value["terms"]):
            raise ValueError("production context model terms must be unique and nonempty")
        return cls(value["name"], value["key"], value["formula"], tuple(value["terms"]))


@dataclass(frozen=True)
class ProductionContextOperation:
    mnemonic: str
    key: str
    opcode: int
    production_schedule: str
    production_schedule_sha256: str
    component_kind: str
    trace_selector_ref: str
    transaction_scope_selector_ref: str
    pricing_basis: str
    dispatch_status: str
    spawned: bool
    trace_input_kind: str
    control_opcode: int
    candidate_ids: tuple[str, ...]
    required_scenario_ids: tuple[str, ...]
    diagnostic_scenario_ids: tuple[str, ...]

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "ProductionContextOperation":
        fields = {
            "mnemonic",
            "key",
            "opcode",
            "production_schedule",
            "production_schedule_sha256",
            "component_kind",
            "trace_selector_ref",
            "transaction_scope_selector_ref",
            "pricing_basis",
            "dispatch_status",
            "spawned",
            "trace_input_kind",
            "control_opcode",
            "candidate_ids",
            "required_scenario_ids",
            "diagnostic_scenario_ids",
        }
        if not isinstance(value, Mapping) or set(value) != fields:
            raise ValueError("production context operation fields differ")
        sequence_fields = (
            "candidate_ids",
            "required_scenario_ids",
            "diagnostic_scenario_ids",
        )
        if any(
            not isinstance(value[field], list)
            or len(value[field]) != len(set(value[field]))
            for field in sequence_fields
        ):
            raise ValueError("production context operation references must be unique lists")
        return cls(
            mnemonic=value["mnemonic"],
            key=value["key"],
            opcode=value["opcode"],
            production_schedule=value["production_schedule"],
            production_schedule_sha256=value["production_schedule_sha256"],
            component_kind=value["component_kind"],
            trace_selector_ref=value["trace_selector_ref"],
            transaction_scope_selector_ref=value[
                "transaction_scope_selector_ref"
            ],
            pricing_basis=value["pricing_basis"],
            dispatch_status=value["dispatch_status"],
            spawned=value["spawned"],
            trace_input_kind=value["trace_input_kind"],
            control_opcode=value["control_opcode"],
            candidate_ids=tuple(value["candidate_ids"]),
            required_scenario_ids=tuple(value["required_scenario_ids"]),
            diagnostic_scenario_ids=tuple(value["diagnostic_scenario_ids"]),
        )


@dataclass(frozen=True)
class ProductionContextScenario:
    name: str
    key: str
    opcode: int
    control_opcode: int
    split: str
    model_class: str
    context: Mapping[str, Any]
    reachability: str

    def counts(self, manifest: "ProductionContextManifest") -> tuple[int, ...]:
        if self.split == "fit":
            return manifest.fit_counts
        if self.split in {"model_selection", "final_holdout"}:
            return manifest.validation_counts
        if self.split == "unreachable":
            return ()
        raise ValueError(f"unknown production context split: {self.split}")


def _validate_internal_joins(
    *,
    keys: tuple[str, ...],
    production_schedule: str,
    repeats: int,
    repeat_contract: Mapping[str, Any],
    candidates: tuple[ProductionContextModelCandidate, ...],
    operations: tuple[ProductionContextOperation, ...],
    scenarios: tuple[ProductionContextScenario, ...],
) -> None:
    if repeat_contract.get("exact_repeats") != repeats:
        raise ValueError("production context repeat contract differs")
    if tuple(operation.key for operation in operations) != keys:
        raise ValueError("production context operation key inventory differs")
    if len({operation.mnemonic for operation in operations}) != len(operations):
        raise ValueError("production context operation mnemonic inventory differs")

    candidate_by_name = {candidate.name: candidate for candidate in candidates}
    scenario_by_name = {scenario.name: scenario for scenario in scenarios}
    if len(candidate_by_name) != len(candidates):
        raise ValueError("production context model candidate IDs differ")
    if len(scenario_by_name) != len(scenarios):
        raise ValueError("production context scenario IDs differ")

    assigned_candidates: list[str] = []
    assigned_scenarios: list[str] = []
    for operation in operations:
        if (
            operation.key != f"opcode:0x{operation.opcode:02x}"
            or operation.production_schedule != production_schedule
            or operation.production_schedule_sha256
            != PRODUCTION_SCHEDULE_SHA256
            or operation.component_kind != "opcode"
            or operation.trace_selector_ref != "opcode_raw_gas_execution"
            or operation.transaction_scope_selector_ref
            != "non_anchor_started_transaction"
            or operation.pricing_basis != "raw_gas_slope"
            or operation.dispatch_status != "not_applicable"
            or operation.spawned is not False
            or operation.trace_input_kind
            not in {"context_fixed", "context_value", "calldata_load", "calldata_size"}
        ):
            raise ValueError("production context operation trace identity differs")
        if not operation.candidate_ids or not operation.required_scenario_ids:
            raise ValueError("production context operation evidence is incomplete")
        if set(operation.required_scenario_ids) & set(
            operation.diagnostic_scenario_ids
        ):
            raise ValueError("production context required and diagnostic scenarios overlap")
        for candidate_id in operation.candidate_ids:
            candidate = candidate_by_name.get(candidate_id)
            if candidate is None or candidate.key != operation.key:
                raise ValueError("production context operation model join differs")
            assigned_candidates.append(candidate_id)
        for scenario_id in operation.required_scenario_ids:
            scenario = scenario_by_name.get(scenario_id)
            if (
                scenario is None
                or scenario.key != operation.key
                or scenario.opcode != operation.opcode
                or scenario.control_opcode != operation.control_opcode
                or scenario.split == "unreachable"
                or scenario.reachability != "measured"
            ):
                raise ValueError("production context required scenario join differs")
            assigned_scenarios.append(scenario_id)
        for scenario_id in operation.diagnostic_scenario_ids:
            scenario = scenario_by_name.get(scenario_id)
            if (
                scenario is None
                or scenario.key != operation.key
                or scenario.opcode != operation.opcode
                or scenario.control_opcode != operation.control_opcode
                or scenario.split != "unreachable"
                or scenario.reachability != "unreachable_under_version_identity"
            ):
                raise ValueError("production context diagnostic scenario join differs")
            assigned_scenarios.append(scenario_id)

    if assigned_candidates != [candidate.name for candidate in candidates]:
        raise ValueError("production context operation candidate ownership differs")
    if sorted(assigned_scenarios) != sorted(scenario_by_name):
        raise ValueError("production context operation scenario ownership differs")


def _deep_freeze(value: Any) -> Any:
    if isinstance(value, Mapping):
        return MappingProxyType(
            {key: _deep_freeze(member) for key, member in value.items()}
        )
    if isinstance(value, list):
        return tuple(_deep_freeze(member) for member in value)
    return value


@dataclass(frozen=True)
class ProductionContextManifest:
    schema_version: int
    purpose: str
    keys: tuple[str, ...]
    fit_counts: tuple[int, ...]
    validation_counts: tuple[int, ...]
    repeats: int
    sources: Mapping[str, Mapping[str, Any]]
    execution: Mapping[str, Any]
    version_identity: Mapping[str, Any]
    fit_equations: Mapping[str, str]
    quality_gates: Mapping[str, Any]
    repeat_contract: Mapping[str, Any]
    family_promotion_contract: Mapping[str, Any]
    model_selection_order: tuple[str, ...]
    model_candidates: tuple[ProductionContextModelCandidate, ...]
    operations: tuple[ProductionContextOperation, ...]
    scenarios: tuple[ProductionContextScenario, ...]
    identity_sha256: str

    def scenario(self, name: str) -> ProductionContextScenario:
        for scenario in self.scenarios:
            if scenario.name == name:
                return scenario
        raise ValueError(f"undeclared production context scenario: {name}")

    def model_candidate(self, name: str) -> ProductionContextModelCandidate:
        for candidate in self.model_candidates:
            if candidate.name == name:
                return candidate
        raise ValueError(f"undeclared production context model candidate: {name}")

    def operation(self, key: str) -> ProductionContextOperation:
        for operation in self.operations:
            if operation.key == key:
                return operation
        raise ValueError(f"undeclared production context operation: {key}")

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "ProductionContextManifest":
        if not isinstance(value, Mapping):
            raise ValueError("production context manifest must be an object")
        _reject_binary_floats(value)
        try:
            candidates = tuple(
                ProductionContextModelCandidate.from_mapping(row)
                for row in value["model_candidates"]
            )
            operations = tuple(
                ProductionContextOperation.from_mapping(row)
                for row in value["operations"]
            )
            scenarios = tuple(
                ProductionContextScenario(
                    name=row["name"],
                    key=row["key"],
                    opcode=row["opcode"],
                    control_opcode=row["control_opcode"],
                    split=row["split"],
                    model_class=row["model_class"],
                    context=MappingProxyType(dict(row["context"])),
                    reachability=row["reachability"],
                )
                for row in value["scenarios"]
            )
            _validate_internal_joins(
                keys=tuple(value["keys"]),
                production_schedule=value["version_identity"][
                    "production_schedule"
                ],
                repeats=value["repeats"],
                repeat_contract=value["repeat_contract"],
                candidates=candidates,
                operations=operations,
                scenarios=scenarios,
            )
        except (KeyError, TypeError) as error:
            raise ValueError("production context manifest structure differs") from error

        expected = canonical_production_context_manifest_payload()
        if canonical_json(value) != canonical_json(expected):
            raise ValueError("production context manifest differs from the frozen contract")

        for source_name, source in value["sources"].items():
            _require_relative_path(source["path"], f"sources.{source_name}.path")
            if "directory_identity_path" in source:
                _require_relative_path(source["directory_identity_path"], f"sources.{source_name}.directory_identity_path")
        for field in ("launcher_path", "production_elf_path", "production_vk_path", "trace_schema_source"):
            _require_relative_path(value["execution"][field], f"execution.{field}")

        return cls(
            schema_version=value["schema_version"],
            purpose=value["purpose"],
            keys=tuple(value["keys"]),
            fit_counts=tuple(value["fit_counts"]),
            validation_counts=tuple(value["validation_counts"]),
            repeats=value["repeats"],
            sources=MappingProxyType({name: MappingProxyType(dict(source)) for name, source in value["sources"].items()}),
            execution=MappingProxyType(dict(value["execution"])),
            version_identity=MappingProxyType(dict(value["version_identity"])),
            fit_equations=MappingProxyType(dict(value["fit_equations"])),
            quality_gates=MappingProxyType(dict(value["quality_gates"])),
            repeat_contract=_deep_freeze(value["repeat_contract"]),
            family_promotion_contract=_deep_freeze(
                value["family_promotion_contract"]
            ),
            model_selection_order=tuple(value["model_selection_order"]),
            model_candidates=candidates,
            operations=operations,
            scenarios=scenarios,
            identity_sha256=sha256_bytes(canonical_json(value)),
        )


def _reject_duplicate_fields(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _reject_json_float(value: str) -> Any:
    raise ValueError(f"binary float is forbidden in production context manifest: {value}")


def load_production_context_manifest(path: pathlib.Path) -> ProductionContextManifest:
    try:
        payload = json.loads(
            path.read_bytes(),
            object_pairs_hook=_reject_duplicate_fields,
            parse_float=_reject_json_float,
            parse_constant=_reject_json_float,
        )
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError("production context manifest is not valid JSON") from error
    return ProductionContextManifest.from_mapping(payload)


def production_context_manifest_bytes() -> bytes:
    return canonical_json(canonical_production_context_manifest_payload()) + b"\n"


def write_production_context_manifest_create_only(path: pathlib.Path) -> ProductionContextManifest:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("xb") as output:
            output.write(production_context_manifest_bytes())
    except FileExistsError as error:
        raise ValueError(f"production context manifest output already exists: {path}") from error
    return load_production_context_manifest(path)


def _sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _load_pinned_json(repo_root: pathlib.Path, source: Mapping[str, Any], name: str) -> tuple[pathlib.Path, dict[str, Any]]:
    relative = _require_relative_path(source["path"], f"sources.{name}.path")
    path = repo_root / relative
    try:
        mode = path.stat(follow_symlinks=False).st_mode
        resolved = path.resolve(strict=True)
        root = repo_root.resolve(strict=True)
    except OSError as error:
        raise ValueError(f"pinned {name} source is missing") from error
    if path.is_symlink() or not stat.S_ISREG(mode) or not resolved.is_relative_to(root):
        raise ValueError(f"pinned {name} source is not a repository regular file")
    if _sha256_file(path) != source["file_sha256"]:
        raise ValueError(f"pinned {name} source bytes differ")
    try:
        payload = json.loads(path.read_bytes(), object_pairs_hook=_reject_duplicate_fields, parse_float=_reject_json_float, parse_constant=_reject_json_float)
    except (json.JSONDecodeError, ValueError) as error:
        raise ValueError(f"pinned {name} source JSON is invalid") from error
    if not isinstance(payload, dict):
        raise ValueError(f"pinned {name} source must be an object")
    return path, payload


def _validate_content_address(payload: Mapping[str, Any], expected: str, name: str) -> None:
    if payload.get("artifact_sha256") != expected or _SHA256_RE.fullmatch(expected) is None:
        raise ValueError(f"pinned {name} content identity differs")
    unhashed = dict(payload)
    unhashed.pop("artifact_sha256")
    if sha256_bytes(canonical_json(unhashed)) != expected:
        raise ValueError(f"pinned {name} content identity is invalid")


def _validate_operation_coverage_join(
    manifest: ProductionContextManifest, coverage: Mapping[str, Any]
) -> None:
    coverage_rows = coverage.get("execution_coverage")
    if not isinstance(coverage_rows, list):
        raise ValueError("pinned operation coverage rows differ")
    rows_by_key = {
        row.get("key"): row
        for row in coverage_rows
        if isinstance(row, Mapping) and row.get("key") in manifest.keys
    }
    if len(rows_by_key) != len(manifest.operations):
        raise ValueError("pinned operation coverage inventory differs")
    for operation in manifest.operations:
        row = rows_by_key.get(operation.key)
        if (
            row is None
            or row.get("component") != operation.component_kind
            or row.get("identifier") != f"0x{operation.opcode:02x}"
            or row.get("name") != operation.mnemonic.lower()
            or row.get("trace_selector_ref") != operation.trace_selector_ref
            or row.get("transaction_scope_selector_ref")
            != operation.transaction_scope_selector_ref
        ):
            raise ValueError("pinned operation coverage join differs")
        schedule_evidence = [
            evidence
            for evidence in row.get("source_evidence", ())
            if isinstance(evidence, Mapping)
            and evidence.get("kind") == "exported_unzen_schedule_entry"
        ]
        if (
            len(schedule_evidence) != 1
            or schedule_evidence[0].get("schedule_sha256")
            != operation.production_schedule_sha256
        ):
            raise ValueError("pinned operation schedule identity differs")


def validate_production_context_sources(manifest: ProductionContextManifest, repo_root: pathlib.Path) -> dict[str, str]:
    validated: dict[str, str] = {}
    coverage_source = manifest.sources["operation_coverage_v5"]
    _, coverage = _load_pinned_json(repo_root, coverage_source, "operation_coverage_v5")
    _validate_content_address(coverage, coverage_source["artifact_sha256"], "operation_coverage_v5")
    _validate_operation_coverage_join(manifest, coverage)
    validated["operation_coverage_v5"] = coverage_source["file_sha256"]

    higher_source = manifest.sources["higher_layer"]
    _, higher = _load_pinned_json(repo_root, higher_source, "higher_layer")
    if higher.get("identity_sha256") != higher_source["model_identity_sha256"]:
        raise ValueError("pinned higher_layer model identity differs")
    identity_source = {
        "path": higher_source["directory_identity_path"],
        "file_sha256": higher_source["directory_identity_file_sha256"],
    }
    _, identity = _load_pinned_json(repo_root, identity_source, "higher_layer_directory_identity")
    if (
        identity.get("identity_sha256") != higher_source["directory_identity_sha256"]
        or sha256_bytes(canonical_json(identity.get("identity"))) != identity.get("identity_sha256")
    ):
        raise ValueError("pinned higher_layer directory identity differs")
    expected_member_hash = identity.get("identity", {}).get("file_sha256s", {}).get(pathlib.PurePosixPath(higher_source["path"]).name)
    if expected_member_hash != higher_source["file_sha256"]:
        raise ValueError("pinned higher_layer member identity differs")
    validated["higher_layer"] = higher_source["file_sha256"]

    discovery_source = manifest.sources["discovery"]
    discovery_path, discovery = _load_pinned_json(repo_root, discovery_source, "discovery")
    if discovery_path.read_bytes() != canonical_json(discovery) + b"\n":
        raise ValueError("pinned discovery source is not canonical JSON")
    _validate_content_address(discovery, discovery_source["artifact_sha256"], "discovery")
    from context_opcode_campaign import verify_context_result

    verified_discovery = verify_context_result(discovery_path.parent)
    if canonical_json(verified_discovery) != canonical_json(discovery):
        raise ValueError("pinned discovery result differs from sealed directory replay")
    if (
        discovery.get("result_identity_sha256") != discovery_source["result_identity_sha256"]
        or discovery.get("candidate_eligible") is not False
        or discovery_source["numeric_parameter_authority"] is not False
    ):
        raise ValueError("pinned discovery authority differs")
    validated["discovery"] = discovery_source["file_sha256"]
    return validated


def _manifest_semantic_identity(manifest: ProductionContextManifest) -> str:
    return manifest.identity_sha256


def production_context_workload_id(manifest: ProductionContextManifest, scenario: ProductionContextScenario, count: int) -> str:
    if scenario not in manifest.scenarios or type(count) is not int or count not in scenario.counts(manifest):
        raise ValueError("workload scenario or count is outside the frozen manifest")
    return sha256_bytes(
        canonical_json(
            {
                "kind": "production_context_workload_v1",
                "manifest_identity_sha256": _manifest_semantic_identity(manifest),
                "scenario": {
                    "name": scenario.name,
                    "key": scenario.key,
                    "opcode": scenario.opcode,
                    "control_opcode": scenario.control_opcode,
                    "split": scenario.split,
                    "model_class": scenario.model_class,
                    "context": dict(scenario.context),
                    "reachability": scenario.reachability,
                },
                "count": count,
            }
        )
    )


def production_context_row_id(*, workload_id: str, lane: str, repeat_index: int) -> str:
    if _SHA256_RE.fullmatch(workload_id) is None:
        raise ValueError("workload_id must be lowercase SHA256")
    if lane not in {"target", "control"}:
        raise ValueError("production context lane must be target or control")
    if type(repeat_index) is not int or repeat_index not in range(REPEATS):
        raise ValueError("production context repeat index is outside the frozen range")
    return sha256_bytes(canonical_json({"kind": "production_context_row_v1", "workload_id": workload_id, "lane": lane, "repeat_index": repeat_index}))


@dataclass(frozen=True)
class ProductionContextRowSpec:
    scenario: str
    split: str
    count: int
    lane: str
    repeat_index: int
    workload_id: str
    row_id: str


def production_context_row_specs(manifest: ProductionContextManifest) -> tuple[ProductionContextRowSpec, ...]:
    rows = []
    for scenario in manifest.scenarios:
        for count in scenario.counts(manifest):
            workload_id = production_context_workload_id(manifest, scenario, count)
            for lane in ("control", "target"):
                for repeat_index in range(manifest.repeats):
                    rows.append(ProductionContextRowSpec(scenario.name, scenario.split, count, lane, repeat_index, workload_id, production_context_row_id(workload_id=workload_id, lane=lane, repeat_index=repeat_index)))
    return tuple(rows)


@dataclass(frozen=True)
class ProductionContextFixtureRequest:
    scenario: str
    split: str
    count: int
    lane: str
    repeat_index: int
    workload_id: str
    row_id: str
    builder_input: Mapping[str, Any]


_CONTEXT_OPCODE_NAMES = {
    0x30: "address",
    0x33: "caller",
    0x34: "callvalue",
    0x35: "calldataload",
    0x36: "calldatasize",
    0x42: "timestamp",
}


def _production_context_builder_profile(
    scenario: ProductionContextScenario,
) -> dict[str, Any]:
    context = dict(scenario.context)
    if scenario.opcode == 0x30:
        return {"kind": "address", "address_profile": context["address_profile"]}
    if scenario.opcode == 0x33:
        return {"kind": "caller", "caller_profile": context["caller_profile"]}
    if scenario.opcode == 0x34:
        return {
            "kind": "callvalue",
            "value": int(context["value"]),
            "value_class": context["value_class"],
        }
    if scenario.opcode == 0x35:
        return {
            "kind": "calldataload",
            "input_length": context["input_length"],
            "offset": context["offset"],
            "access_class": context["access_class"],
        }
    if scenario.opcode == 0x36:
        return {"kind": "calldatasize", "input_length": context["input_length"]}
    if scenario.opcode == 0x42:
        return {
            "kind": "timestamp",
            "timestamp_delta": context["timestamp_delta"],
            "value_class": context["value_class"],
        }
    raise ValueError(f"unsupported production context opcode: {scenario.opcode:#x}")


def production_context_fixture_requests(
    manifest: ProductionContextManifest,
) -> tuple[ProductionContextFixtureRequest, ...]:
    requests = []
    for row in production_context_row_specs(manifest):
        scenario = manifest.scenario(row.scenario)
        opcode_name = _CONTEXT_OPCODE_NAMES.get(scenario.opcode)
        if opcode_name is None:
            raise ValueError(f"unsupported production context opcode: {scenario.opcode:#x}")
        builder_input = {
            "row_id": row.row_id,
            "workload_family": "context_opcode",
            "split": "fit" if row.split == "fit" else "holdout",
            "block_count": 1,
            "transaction_count": 1,
            "program": {
                "kind": "context_opcode_loop",
                "workload_id": row.workload_id,
                "repeat_index": row.repeat_index,
                "opcode": opcode_name,
                "lane": row.lane,
                "count": row.count,
                "profile": _production_context_builder_profile(scenario),
            },
            "expected_final_state_root": "0x" + "00" * 32,
            "expected_raw_gas_by_key": {},
            "expected_context_features": {},
            "expected_features": {},
            "expected_diagnostics": {},
        }
        requests.append(
            ProductionContextFixtureRequest(
                scenario=row.scenario,
                split=row.split,
                count=row.count,
                lane=row.lane,
                repeat_index=row.repeat_index,
                workload_id=row.workload_id,
                row_id=row.row_id,
                builder_input=builder_input,
            )
        )
    return tuple(requests)


_FIXTURE_EVIDENCE_FIELDS = {
    "expected_final_state_root": "actual_final_state_root",
    "expected_raw_gas_by_key": "actual_raw_gas_by_key",
    "expected_context_features": "actual_context_features",
    "expected_features": "actual_features",
    "expected_diagnostics": "actual_diagnostics",
    "expected_backend_input_sha256": "backend_input_sha256",
    "expected_host_trace_sha256": "host_trace_sha256",
}


def validate_production_context_fixture_identity(
    request: ProductionContextFixtureRequest,
    bundle: Mapping[str, Any],
) -> Mapping[str, Any]:
    if not isinstance(bundle, Mapping) or set(bundle) != {
        "schema_version",
        "fixture_spec_sha256",
        "spec",
        "observation",
    }:
        raise ValueError("production context fixture identity fields differ")
    if bundle["schema_version"] != 1:
        raise ValueError("production context fixture identity schema differs")
    spec = bundle["spec"]
    observation = bundle["observation"]
    if not isinstance(spec, Mapping) or not isinstance(observation, Mapping):
        raise ValueError("production context fixture identity payload differs")
    source = {
        key: value
        for key, value in spec.items()
        if key not in _FIXTURE_EVIDENCE_FIELDS
    }
    request_source = {
        key: value
        for key, value in request.builder_input.items()
        if key not in _FIXTURE_EVIDENCE_FIELDS
    }
    if canonical_json(source) != canonical_json(request_source):
        raise ValueError("production context fixture source mismatch")
    if spec.get("row_id") != request.row_id or observation.get("row_id") != request.row_id:
        raise ValueError("production context fixture row identity mismatch")
    for declared, observed in _FIXTURE_EVIDENCE_FIELDS.items():
        if declared not in spec or observed not in observation:
            raise ValueError(f"production context fixture {declared} evidence is missing")
        if canonical_json(spec[declared]) != canonical_json(observation[observed]):
            raise ValueError(f"production context fixture {declared} mismatch")
    for field in (
        "fixture_spec_sha256",
        "expected_backend_input_sha256",
        "expected_host_trace_sha256",
    ):
        value = bundle[field] if field == "fixture_spec_sha256" else spec[field]
        if not isinstance(value, str) or _SHA256_RE.fullmatch(value) is None:
            raise ValueError(f"production context fixture {field} is not lowercase SHA256")
    expected_spec_sha256 = sha256_bytes(canonical_json(spec))
    if bundle["fixture_spec_sha256"] != expected_spec_sha256:
        raise ValueError("production context fixture spec identity mismatch")
    return bundle


def run_production_context_fixture_identity(
    request: ProductionContextFixtureRequest,
    *,
    launcher: pathlib.Path,
    repo_root: pathlib.Path,
) -> Mapping[str, Any]:
    launcher = launcher.resolve()
    repo_root = repo_root.resolve()
    if not launcher.is_file() or not repo_root.is_dir():
        raise ValueError("production context fixture launcher or repository root is missing")
    with tempfile.TemporaryDirectory(prefix="raiko2-context-fixture-") as directory:
        temporary = pathlib.Path(directory)
        input_path = temporary / "source-row.json"
        output_path = temporary / "identity.json"
        input_path.write_bytes(canonical_json(request.builder_input) + b"\n")
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
                str(input_path),
                "--json-out",
                str(output_path),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
            timeout=300,
        )
        try:
            bundle = json.loads(
                output_path.read_text(),
                object_pairs_hook=_reject_duplicate_fields,
                parse_float=_reject_json_float,
            )
        except (OSError, json.JSONDecodeError) as error:
            raise ValueError("production context fixture identity output is invalid") from error
    return validate_production_context_fixture_identity(request, bundle)


def production_context_parity_identity(
    *,
    row_id: str,
    standard: Mapping[str, Any],
    gas_estimator: Mapping[str, Any],
) -> dict[str, Any]:
    if _SHA256_RE.fullmatch(row_id) is None:
        raise ValueError("production context parity row ID must be lowercase SHA256")
    if standard.get("sp1_execution_engine") != "standard" or gas_estimator.get(
        "sp1_execution_engine"
    ) != "gas-estimator":
        raise ValueError("production context parity engine identity differs")

    def parity_values(report: Mapping[str, Any]) -> dict[str, Any]:
        try:
            controlled_block = report["controlled_block"]
            if (
                controlled_block["row_id"] != row_id
                or controlled_block["status"] != "accepted"
            ):
                raise ValueError(
                    "production context parity evidence is not an accepted row-bound result"
                )
            if type(report["exit_code"]) is not int or report["exit_code"] != 0:
                raise ValueError(
                    "production context parity evidence is not a successful execution"
                )
            return {
                "gas": report["gas"],
                "total_instruction_count": report["total_instruction_count"],
                "total_syscall_count": report["total_syscall_count"],
                "public_values": report["public_values"],
                "guest_input_sha256": report["guest_input_sha256"],
                "host_trace_sha256": controlled_block["observation"]["host_trace_sha256"],
                "exit_code": report["exit_code"],
            }
        except (KeyError, TypeError) as error:
            raise ValueError("production context parity evidence is incomplete") from error

    standard_values = parity_values(standard)
    estimator_values = parity_values(gas_estimator)
    if canonical_json(standard_values) != canonical_json(estimator_values):
        raise ValueError("production context standard/gas-estimator parity mismatch")
    identity = {
        "kind": "production_context_parity_v1",
        "row_id": row_id,
        "model_sample": False,
        "standard_execution_engine": "standard",
        "gas_estimator_execution_engine": "gas-estimator",
        "equal_values": standard_values,
    }
    identity["identity_sha256"] = sha256_bytes(canonical_json(identity))
    return identity


# Production-native fitting and bounded campaign execution.  These helpers are
# intentionally independent of the discovery-ELF model: only the frozen V5
# production registry contributes to a controlled row subtotal.


def _decimal(value: Any, *, label: str, nonnegative: bool = False) -> Decimal:
    if isinstance(value, bool) or isinstance(value, float):
        raise ValueError(f"{label} must use finite exact decimal arithmetic")
    if isinstance(value, Decimal):
        result = value
    elif isinstance(value, int):
        result = Decimal(value)
    elif isinstance(value, str):
        try:
            result = Decimal(value)
        except InvalidOperation as error:
            raise ValueError(f"{label} must be a finite Decimal") from error
    else:
        raise ValueError(f"{label} must be an integer or canonical decimal string")
    if not result.is_finite():
        raise ValueError(f"{label} must be finite")
    if nonnegative and result < 0:
        raise ValueError(f"{label} must be nonnegative")
    return result


def _decimal_text(value: Decimal) -> str:
    if not value.is_finite():
        raise ValueError("cannot serialize a nonfinite Decimal")
    if value == 0:
        return "0"
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _fraction(value: Any, *, label: str) -> Fraction:
    decimal = _decimal(value, label=label)
    numerator, denominator = decimal.as_integer_ratio()
    return Fraction(numerator, denominator)


def _exact_rank(matrix: Sequence[Sequence[Fraction]]) -> int:
    if not matrix:
        return 0
    width = len(matrix[0])
    if width == 0 or any(len(row) != width for row in matrix):
        raise ValueError("exact matrix dimensions differ")
    reduced = [list(row) for row in matrix]
    pivot_row = 0
    for column in range(width):
        pivot = next(
            (
                row
                for row in range(pivot_row, len(reduced))
                if reduced[row][column] != 0
            ),
            None,
        )
        if pivot is None:
            continue
        reduced[pivot_row], reduced[pivot] = reduced[pivot], reduced[pivot_row]
        divisor = reduced[pivot_row][column]
        reduced[pivot_row] = [value / divisor for value in reduced[pivot_row]]
        for row in range(len(reduced)):
            if row == pivot_row:
                continue
            factor = reduced[row][column]
            if factor:
                reduced[row] = [
                    value - factor * pivot_value
                    for value, pivot_value in zip(reduced[row], reduced[pivot_row])
                ]
        pivot_row += 1
        if pivot_row == len(reduced):
            break
    return pivot_row


def _solve_decimal_system(
    matrix: Sequence[Sequence[Decimal]], values: Sequence[Decimal]
) -> list[Decimal]:
    width = len(matrix)
    if width == 0 or len(values) != width or any(len(row) != width for row in matrix):
        raise ValueError("decimal system dimensions differ")
    augmented = [list(row) + [value] for row, value in zip(matrix, values)]
    for column in range(width):
        pivot = next(
            (row for row in range(column, width) if augmented[row][column] != 0),
            None,
        )
        if pivot is None:
            raise ValueError("exact model matrix is rank deficient")
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        augmented[column] = [value / divisor for value in augmented[column]]
        for row in range(width):
            if row == column:
                continue
            factor = augmented[row][column]
            if factor:
                augmented[row] = [
                    value - factor * pivot_value
                    for value, pivot_value in zip(
                        augmented[row], augmented[column]
                    )
                ]
    return [row[-1] for row in augmented]


def _solve_fraction_system(
    matrix: Sequence[Sequence[Fraction]], values: Sequence[Fraction]
) -> list[Fraction]:
    width = len(matrix)
    if width == 0 or len(values) != width or any(len(row) != width for row in matrix):
        raise ValueError("exact system dimensions differ")
    augmented = [list(row) + [value] for row, value in zip(matrix, values)]
    for column in range(width):
        pivot = next(
            (row for row in range(column, width) if augmented[row][column] != 0),
            None,
        )
        if pivot is None:
            raise ValueError("exact model matrix is rank deficient")
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        augmented[column] = [value / divisor for value in augmented[column]]
        for row in range(width):
            if row == column:
                continue
            factor = augmented[row][column]
            if factor:
                augmented[row] = [
                    value - factor * pivot_value
                    for value, pivot_value in zip(
                        augmented[row], augmented[column]
                    )
                ]
    return [row[-1] for row in augmented]


def _inverse_decimal_matrix(matrix: Sequence[Sequence[Decimal]]) -> list[list[Decimal]]:
    width = len(matrix)
    columns = []
    for column in range(width):
        unit = [Decimal(int(row == column)) for row in range(width)]
        columns.append(_solve_decimal_system(matrix, unit))
    return [[columns[column][row] for column in range(width)] for row in range(width)]


def fit_exact_decimal_model(
    *,
    matrix: Sequence[Sequence[Any]],
    observed: Sequence[Any],
    terms: Sequence[str],
) -> dict[str, Any]:
    """Fit a full-rank through-origin model with exact inputs and Decimal algebra."""
    if not matrix or len(matrix) != len(observed) or not terms:
        raise ValueError("exact model dimensions differ")
    if len(set(terms)) != len(terms):
        raise ValueError("exact model terms must be unique")
    width = len(terms)
    if any(len(row) != width for row in matrix):
        raise ValueError("exact model dimensions differ")
    exact_matrix = [
        [_fraction(value, label="design matrix value") for value in row]
        for row in matrix
    ]
    rank = _exact_rank(exact_matrix)
    if rank != width:
        raise ValueError("exact model matrix is rank deficient")
    targets = [_decimal(value, label="observed model value") for value in observed]
    exact_targets = [
        _fraction(value, label="observed model value") for value in observed
    ]
    decimal_matrix = [
        [_decimal(value, label="design matrix value") for value in row]
        for row in matrix
    ]
    with localcontext(_DECIMAL_CONTEXT):
        exact_gram = [
            [
                sum(
                    (row[left] * row[right] for row in decimal_matrix),
                    Decimal(0),
                )
                for right in range(width)
            ]
            for left in range(width)
        ]
        exact_gram_fraction = [
            [
                sum(
                    (row[left] * row[right] for row in exact_matrix),
                    Fraction(0),
                )
                for right in range(width)
            ]
            for left in range(width)
        ]
        exact_projection = [
            sum(
                (
                    row[column] * target
                    for row, target in zip(exact_matrix, exact_targets)
                ),
                Fraction(0),
            )
            for column in range(width)
        ]
        exact_coefficients = _solve_fraction_system(
            exact_gram_fraction, exact_projection
        )
        coefficients = [
            Decimal(value.numerator) / Decimal(value.denominator)
            for value in exact_coefficients
        ]
        if any(value < 0 for value in coefficients):
            raise ValueError("exact model coefficients must be nonnegative")
        predictions = [
            sum(
                (feature * coefficient for feature, coefficient in zip(row, coefficients)),
                Decimal(0),
            )
            for row in decimal_matrix
        ]
        residuals = [target - predicted for target, predicted in zip(targets, predictions)]
        sse = sum((value * value for value in residuals), Decimal(0))
        total_signal = sum((value * value for value in targets), Decimal(0))
        r2 = Decimal(1) if sse == 0 else (
            Decimal(1) - sse / total_signal if total_signal else Decimal(0)
        )
        degrees_of_freedom = len(targets) - width
        if degrees_of_freedom > 0:
            variance = sse / Decimal(degrees_of_freedom)
            inverse = _inverse_decimal_matrix(exact_gram)
            stderr = [
                (variance * inverse[index][index]).sqrt()
                for index in range(width)
            ]
        else:
            stderr = [Decimal(0) if sse == 0 else Decimal("Infinity")] * width
        relative_stderr = [
            (error / coefficient if coefficient else (Decimal(0) if error == 0 else Decimal("Infinity")))
            for error, coefficient in zip(stderr, coefficients)
        ]
    return {
        "terms": list(terms),
        "design_matrix": [
            [_decimal_text(value) for value in row] for row in decimal_matrix
        ],
        "observed": [_decimal_text(value) for value in targets],
        "rank": rank,
        "coefficients": {
            term: _decimal_text(value) for term, value in zip(terms, coefficients)
        },
        "predicted": [_decimal_text(value) for value in predictions],
        "residuals": [_decimal_text(value) for value in residuals],
        "r2": _decimal_text(r2),
        "coefficient_stderr": {
            term: _decimal_text(value) for term, value in zip(terms, stderr)
        },
        "relative_coefficient_stderr": {
            term: _decimal_text(value)
            for term, value in zip(terms, relative_stderr)
        },
    }


def production_context_design_row(
    candidate: ProductionContextModelCandidate,
    scenario: ProductionContextScenario,
) -> tuple[Decimal, ...]:
    context = scenario.context
    if candidate.name in {"address_constant", "caller_constant"}:
        return (Decimal(1),)
    if candidate.name == "callvalue_classes":
        value_class = context.get("value_class")
        if value_class not in {"zero", "nonzero"}:
            raise ValueError("CALLVALUE scenario has an unknown value class")
        return (Decimal(value_class == "zero"), Decimal(value_class == "nonzero"))
    if candidate.name == "calldataload_access_classes":
        access_class = context.get("access_class")
        if access_class not in {"zero", "partial", "full"}:
            raise ValueError("CALLDATALOAD scenario has an unknown access class")
        return tuple(
            Decimal(access_class == expected)
            for expected in ("zero", "partial", "full")
        )
    if candidate.name == "calldatasize_length":
        length = context.get("input_length")
        if type(length) is not int or length < 0:
            raise ValueError("CALLDATASIZE input length must be nonnegative")
        return (Decimal(1), Decimal(length))
    if candidate.name == "calldatasize_boundary":
        length = context.get("input_length")
        if type(length) is not int or length < 0:
            raise ValueError("CALLDATASIZE input length must be nonnegative")
        return (
            Decimal(1),
            Decimal((length + 31) // 32),
            Decimal(length % 32 != 0),
        )
    if candidate.name == "timestamp_nonzero":
        if context.get("value_class") != "nonzero":
            raise ValueError("TIMESTAMP fit admits only the nonzero class")
        return (Decimal(1),)
    raise ValueError(f"unknown production context candidate: {candidate.name}")


@dataclass(frozen=True)
class ProductionSubtotalModel:
    fixed_costs: Mapping[str, Decimal]
    opcode_prices: Mapping[str, Decimal]

    @classmethod
    def from_mappings(
        cls,
        *,
        fixed_costs: Mapping[str, Any],
        opcode_prices: Mapping[str, Any],
    ) -> "ProductionSubtotalModel":
        expected_fixed = {
            "proposal_startup",
            "block_base",
            "tx_base",
            "native_value_transfer",
        }
        if set(fixed_costs) != expected_fixed:
            raise ValueError("V5 subtotal fixed-cost inventory differs")
        if not opcode_prices:
            raise ValueError("V5 subtotal opcode price inventory is empty")
        parsed_fixed = {
            key: _decimal(value, label=f"fixed cost {key}", nonnegative=True)
            for key, value in fixed_costs.items()
        }
        parsed_prices = {
            key: _decimal(value, label=f"opcode price {key}", nonnegative=True)
            for key, value in opcode_prices.items()
        }
        if any(not isinstance(key, str) or not key.startswith("opcode:0x") for key in parsed_prices):
            raise ValueError("V5 subtotal opcode price key differs")
        return cls(MappingProxyType(parsed_fixed), MappingProxyType(parsed_prices))


def evaluate_v5_subtotal(
    row: Mapping[str, Any],
    model: ProductionSubtotalModel,
    *,
    excluded_target_key: str | None,
) -> Decimal:
    features = row.get("actual_features")
    raw_gas = row.get("actual_raw_gas_by_key")
    if not isinstance(features, Mapping) or set(features) != set(model.fixed_costs):
        raise ValueError("V5 subtotal higher-layer feature inventory differs")
    if not isinstance(raw_gas, Mapping):
        raise ValueError("V5 subtotal raw-gas ledger differs")
    subtotal_keys = row.get("subtotal_keys")
    if subtotal_keys is not None:
        if excluded_target_key is not None and excluded_target_key in subtotal_keys:
            raise ValueError("target context work was accidentally included in K")
        raise ValueError("V5 subtotal keys must be derived, not supplied")
    with localcontext(_DECIMAL_CONTEXT):
        subtotal = Decimal(0)
        for key, price in model.fixed_costs.items():
            count = features[key]
            if type(count) is not int or count < 0:
                raise ValueError("V5 subtotal feature counts must be nonnegative integers")
            subtotal += Decimal(count) * price
        for key, units in raw_gas.items():
            if type(units) is not int or units < 0:
                raise ValueError("V5 subtotal raw gas must be nonnegative integers")
            if key == excluded_target_key:
                continue
            price = model.opcode_prices.get(key)
            # A zero raw-gas subtotal term is exactly zero independently of a
            # coefficient.  This admits terminal STOP while still rejecting
            # any positive unpriced non-target work.
            if price is None and units == 0:
                continue
            if price is None:
                raise ValueError(f"unpriced non-target work in V5 subtotal: {key}")
            subtotal += Decimal(units) * price
    return subtotal


def _reject_forbidden_fit_fields(value: Any, path: str = "fit input") -> None:
    if isinstance(value, float):
        raise ValueError(f"forbidden binary float at {path}")
    if isinstance(value, Mapping):
        for key, child in value.items():
            normalized = str(key).lower().replace("-", "_")
            if "body_scale" in normalized or "context_elf" in normalized:
                raise ValueError(f"forbidden production fit field: {path}.{key}")
            _reject_forbidden_fit_fields(child, f"{path}.{key}")
    elif isinstance(value, (list, tuple)):
        for index, child in enumerate(value):
            _reject_forbidden_fit_fields(child, f"{path}[{index}]")


def exact_ape(predicted: Decimal, observed: Decimal) -> Decimal:
    predicted = _decimal(predicted, label="predicted increment")
    observed = _decimal(observed, label="observed increment")
    if observed == 0:
        raise ValueError("APE is undefined for zero observed signal")
    with localcontext(_DECIMAL_CONTEXT):
        return abs(predicted - observed) / abs(observed)


def _normalized_fit_row(row: Mapping[str, Any]) -> dict[str, Any]:
    required = {
        "row_id",
        "scenario",
        "split",
        "count",
        "lane",
        "repeat_index",
        "prover_gas",
        "public_output",
        "backend_input_sha256",
        "host_trace_sha256",
        "actual_raw_gas_by_key",
        "actual_context_features",
        "actual_features",
        "actual_diagnostics",
    }
    if not isinstance(row, Mapping) or not required.issubset(row):
        raise ValueError("production fit row fields differ")
    if _SHA256_RE.fullmatch(row["row_id"]) is None:
        raise ValueError("production fit row ID differs")
    if row["lane"] not in {"target", "control"}:
        raise ValueError("production fit lane differs")
    if type(row["count"]) is not int or row["count"] < 0:
        raise ValueError("production fit count differs")
    if type(row["repeat_index"]) is not int or row["repeat_index"] not in range(REPEATS):
        raise ValueError("production fit repeat index differs")
    for field in ("backend_input_sha256", "host_trace_sha256"):
        if _SHA256_RE.fullmatch(row[field]) is None:
            raise ValueError(f"production fit {field} differs")
    normalized = dict(row)
    normalized["prover_gas"] = _decimal(
        row["prover_gas"], label="row prover gas", nonnegative=True
    )
    for field in (
        "actual_raw_gas_by_key",
        "actual_context_features",
        "actual_features",
        "actual_diagnostics",
    ):
        value = row[field]
        if not isinstance(value, Mapping) or any(
            not isinstance(key, str)
            or type(count) is not int
            or count < 0
            for key, count in value.items()
        ):
            raise ValueError(f"production fit {field} differs")
        normalized[field] = dict(value)
    return normalized


def _candidate_gate_reasons(
    fit: Mapping[str, Any], observed: Sequence[Decimal]
) -> list[str]:
    reasons = []
    if _decimal(fit["r2"], label="fit R2") < Decimal(QUALITY_GATES["r2_min"]):
        reasons.append("fit_r2")
    relative = [
        _decimal(value, label="relative coefficient stderr")
        for value in fit["relative_coefficient_stderr"].values()
    ]
    if any(
        not value.is_finite()
        or value > Decimal(QUALITY_GATES["relative_coefficient_stderr_max"])
        for value in relative
    ):
        reasons.append("coefficient_stderr")
    maximum_signal = max((abs(value) for value in observed), default=Decimal(0))
    maximum_residual = max(
        (
            abs(_decimal(value, label="fit residual"))
            for value in fit["residuals"]
        ),
        default=Decimal(0),
    )
    if maximum_signal == 0 or maximum_residual / maximum_signal > Decimal(
        QUALITY_GATES["fit_residual_signal_max"]
    ):
        reasons.append("fit_residual")
    return reasons


def _expected_context_feature(
    operation: ProductionContextOperation,
    scenario: ProductionContextScenario,
) -> str:
    if operation.trace_input_kind == "context_fixed":
        return f"context_fixed:{operation.key}"
    if operation.trace_input_kind == "context_value":
        value_class = scenario.context.get("value_class")
        if value_class not in {"zero", "nonzero"}:
            raise ValueError("context-value scenario class differs")
        return f"context_value:{operation.key}:value_class:{value_class}"
    if operation.trace_input_kind == "calldata_load":
        access_class = scenario.context.get("access_class")
        if access_class not in {"zero", "partial", "full"}:
            raise ValueError("calldata-load scenario class differs")
        return f"calldata_load:{operation.key}:access_class:{access_class}"
    if operation.trace_input_kind == "calldata_size":
        input_length = scenario.context.get("input_length")
        if type(input_length) is not int or input_length < 0:
            raise ValueError("calldata-size scenario length differs")
        return f"calldata_size:{operation.key}:input_length:{input_length}"
    raise ValueError("production context trace input kind differs")


def fit_production_context_rows(
    manifest: ProductionContextManifest,
    rows: Iterable[Mapping[str, Any]],
    subtotal_model: ProductionSubtotalModel,
    *,
    family_keys: Sequence[str] = KEYS,
) -> dict[str, Any]:
    """Residualize and fit production rows without consulting a control coefficient."""
    raw_rows = list(rows)
    _reject_forbidden_fit_fields(raw_rows)
    if any(not isinstance(row, Mapping) for row in raw_rows):
        raise ValueError("production fit rows must be objects")
    if len({row.get("row_id") for row in raw_rows}) != len(raw_rows):
        raise ValueError("duplicate production fit row")
    normalized = [_normalized_fit_row(row) for row in raw_rows]
    unknown_families = set(family_keys) - set(manifest.keys)
    if unknown_families:
        raise ValueError(f"unknown production context families: {sorted(unknown_families)!r}")
    scenario_by_name = {scenario.name: scenario for scenario in manifest.scenarios}
    operation_by_key = {operation.key: operation for operation in manifest.operations}
    selected_scenarios = {
        scenario.name
        for scenario in manifest.scenarios
        if scenario.key in family_keys and scenario.reachability == "measured"
    }
    normalized = [row for row in normalized if row["scenario"] in selected_scenarios]
    grouped: dict[tuple[str, int, str], list[dict[str, Any]]] = {}
    for row in normalized:
        scenario = scenario_by_name[row["scenario"]]
        if row["split"] != scenario.split or row["count"] not in scenario.counts(manifest):
            raise ValueError("production fit row split or count differs from manifest")
        grouped.setdefault((row["scenario"], row["count"], row["lane"]), []).append(row)
    summaries: dict[tuple[str, int, str], dict[str, Any]] = {}
    for scenario_name in sorted(selected_scenarios):
        scenario = scenario_by_name[scenario_name]
        operation = operation_by_key[scenario.key]
        control_key = f"opcode:0x{operation.control_opcode:02x}"
        for count in scenario.counts(manifest):
            lane_rows = {}
            for lane in ("target", "control"):
                repeats = grouped.get((scenario_name, count, lane), [])
                if len(repeats) != manifest.repeats or {
                    row["repeat_index"] for row in repeats
                } != set(range(manifest.repeats)):
                    raise ValueError(
                        f"missing count-zero rows or duplicate repeats for {scenario_name} count {count} {lane}"
                        if count == 0
                        else f"missing rows or duplicate repeats for {scenario_name} count {count} {lane}"
                    )
                repeats.sort(key=lambda row: row["repeat_index"])
                lane_rows[lane] = repeats
            target = lane_rows["target"][0]
            control = lane_rows["control"][0]
            expected_context = (
                {_expected_context_feature(operation, scenario): count}
                if count
                else {}
            )
            if (
                target["actual_context_features"] != expected_context
                or control["actual_context_features"] != {}
            ):
                raise ValueError("exact context event matching differs")
            target_non_target = dict(target["actual_raw_gas_by_key"])
            target_non_target.pop(scenario.key, None)
            control_non_target = dict(control["actual_raw_gas_by_key"])
            control_non_target.pop(control_key, None)
            if (
                target_non_target != control_non_target
                or target["actual_features"] != control["actual_features"]
                or target["actual_diagnostics"] != control["actual_diagnostics"]
            ):
                raise ValueError("target/control non-target ledger differs")
            for lane, repeats in lane_rows.items():
                equality_fields = (
                    "prover_gas",
                    "public_output",
                    "backend_input_sha256",
                    "host_trace_sha256",
                    "actual_raw_gas_by_key",
                    "actual_context_features",
                    "actual_features",
                    "actual_diagnostics",
                )
                if any(
                    any(row[field] != repeats[0][field] for field in equality_fields)
                    for row in repeats[1:]
                ):
                    raise ValueError(
                        f"repeat drift for {scenario_name} count {count} {lane}"
                    )
                excluded = scenario.key if lane == "target" else None
                subtotal = evaluate_v5_subtotal(
                    repeats[0], subtotal_model, excluded_target_key=excluded
                )
                summaries[(scenario_name, count, lane)] = {
                    "prover_gas": repeats[0]["prover_gas"],
                    "subtotal": subtotal,
                }

    residual_rows: dict[tuple[str, int], dict[str, Decimal]] = {}
    for scenario_name in sorted(selected_scenarios):
        scenario = scenario_by_name[scenario_name]
        for lane in ("target", "control"):
            if (scenario_name, 0, lane) not in summaries:
                raise ValueError(f"missing count-zero row for {scenario_name} {lane}")
        for count in scenario.counts(manifest):
            target = summaries[(scenario_name, count, "target")]
            target_zero = summaries[(scenario_name, 0, "target")]
            control = summaries[(scenario_name, count, "control")]
            control_zero = summaries[(scenario_name, 0, "control")]
            residual_rows[(scenario_name, count)] = {
                "observed": (target["prover_gas"] - target_zero["prover_gas"])
                - (target["subtotal"] - target_zero["subtotal"]),
                "control": (control["prover_gas"] - control_zero["prover_gas"])
                - (control["subtotal"] - control_zero["subtotal"]),
            }

    family_results: dict[str, Any] = {}
    for key in family_keys:
        operation = operation_by_key[key]
        fit_scenarios = [
            scenario
            for scenario in manifest.scenarios
            if scenario.key == key and scenario.split == "fit"
        ]
        selection_scenarios = [
            scenario
            for scenario in manifest.scenarios
            if scenario.key == key and scenario.split == "model_selection"
        ]
        final_scenarios = [
            scenario
            for scenario in manifest.scenarios
            if scenario.key == key and scenario.split == "final_holdout"
        ]
        candidate_names = operation.candidate_ids
        candidate_reports = {}
        selected_candidate = None
        selected_fit = None
        selected_rows = None
        selection_decisions = []
        for candidate_name in candidate_names:
            candidate = manifest.model_candidate(candidate_name)
            design = []
            observed = []
            fit_row_labels = []
            for scenario in fit_scenarios:
                features = production_context_design_row(candidate, scenario)
                for count in manifest.fit_counts:
                    if count == 0:
                        continue
                    design.append(tuple(Decimal(count) * value for value in features))
                    observed.append(residual_rows[(scenario.name, count)]["observed"])
                    fit_row_labels.append({"scenario": scenario.name, "count": count})
            fit = fit_exact_decimal_model(
                matrix=design, observed=observed, terms=candidate.terms
            )
            reasons = _candidate_gate_reasons(fit, observed)
            coefficients = {
                term: _decimal(value, label=f"coefficient {term}", nonnegative=True)
                for term, value in fit["coefficients"].items()
            }
            selection_rows = []
            for scenario in selection_scenarios:
                features = production_context_design_row(candidate, scenario)
                for count in manifest.validation_counts:
                    if count == 0:
                        continue
                    prediction = Decimal(count) * sum(
                        (
                            feature * coefficients[term]
                            for feature, term in zip(features, candidate.terms)
                        ),
                        Decimal(0),
                    )
                    observed_value = residual_rows[(scenario.name, count)]["observed"]
                    try:
                        ape = exact_ape(prediction, observed_value)
                    except ValueError:
                        reasons.append("selection_zero_signal")
                        ape = None
                    if ape is not None and ape > Decimal(
                        QUALITY_GATES["count_holdout_ape_max"]
                        if count == 32
                        else QUALITY_GATES["extrapolation_ape_max"]
                    ):
                        reasons.append(
                            "count_holdout" if count == 32 else "extrapolation"
                        )
                    selection_rows.append(
                        {
                            "scenario": scenario.name,
                            "count": count,
                            "observed_increment": _decimal_text(observed_value),
                            "predicted_increment": _decimal_text(prediction),
                            "ape": _decimal_text(ape) if ape is not None else None,
                        }
                    )
            report = {
                **fit,
                "fit_rows": fit_row_labels,
                "selection_rows": selection_rows,
                "rejection_reasons": sorted(set(reasons)),
                "status": "accepted" if not reasons else "rejected",
            }
            candidate_reports[candidate_name] = report
            selection_decisions.append(
                {"candidate": candidate_name, "status": report["status"]}
            )
            if selected_candidate is None and not reasons:
                selected_candidate = candidate
                selected_fit = fit
                selected_rows = selection_rows
                if selection_scenarios:
                    break
        reasons = []
        if selected_candidate is None:
            reasons.append("no_candidate_passed")
            fallback_name = candidate_names[0]
            selected_candidate = manifest.model_candidate(fallback_name)
            selected_fit = candidate_reports[fallback_name]
            selected_rows = candidate_reports[fallback_name]["selection_rows"]
        reasons.extend(candidate_reports[selected_candidate.name]["rejection_reasons"])
        coefficients = {
            term: _decimal(value, label=f"coefficient {term}", nonnegative=True)
            for term, value in selected_fit["coefficients"].items()
        }
        for scenario in fit_scenarios:
            signal = abs(residual_rows[(scenario.name, 16)]["observed"])
            if signal < Decimal(QUALITY_GATES["signal_min_prover_gas"]):
                reasons.append("insufficient_signal")
        maximum_control = max(
            (
                abs(residual_rows[(scenario.name, count)]["control"])
                for scenario in (*fit_scenarios, *selection_scenarios, *final_scenarios)
                for count in scenario.counts(manifest)
            ),
            default=Decimal(0),
        )
        if maximum_control > Decimal(
            QUALITY_GATES["control_residual_abs_max_prover_gas"]
        ):
            reasons.append("control_contamination")

        class_slopes: dict[tuple[str, tuple[Decimal, ...]], list[Decimal]] = {}
        for scenario in fit_scenarios:
            numerator = sum(
                (
                    Decimal(count) * residual_rows[(scenario.name, count)]["observed"]
                    for count in manifest.fit_counts
                    if count
                ),
                Decimal(0),
            )
            denominator = sum(
                (Decimal(count * count) for count in manifest.fit_counts if count),
                Decimal(0),
            )
            sibling_key = (
                scenario.model_class,
                production_context_design_row(selected_candidate, scenario),
            )
            class_slopes.setdefault(sibling_key, []).append(numerator / denominator)
        sibling_decisions = []
        for (model_class, feature_vector), slopes in sorted(class_slopes.items()):
            passed = True
            relative_difference = Decimal(0)
            if len(slopes) > 1:
                maximum = max(abs(value) for value in slopes)
                relative_difference = (
                    (max(slopes) - min(slopes)) / maximum if maximum else Decimal(0)
                )
                passed = relative_difference <= Decimal(
                    QUALITY_GATES["sibling_slope_relative_difference_max"]
                )
                if not passed:
                    reasons.append("sibling_inconsistency")
            sibling_decisions.append(
                {
                    "model_class": model_class,
                    "feature_vector": [
                        _decimal_text(value) for value in feature_vector
                    ],
                    "diagnostic_slopes": [_decimal_text(value) for value in slopes],
                    "relative_difference": _decimal_text(relative_difference),
                    "status": "accepted" if passed else "rejected",
                }
            )

        final_rows = []
        final_apes = []
        for scenario in final_scenarios:
            features = production_context_design_row(selected_candidate, scenario)
            for count in manifest.validation_counts:
                if count == 0:
                    continue
                prediction = Decimal(count) * sum(
                    (
                        feature * coefficients[term]
                        for feature, term in zip(features, selected_candidate.terms)
                    ),
                    Decimal(0),
                )
                observed_value = residual_rows[(scenario.name, count)]["observed"]
                try:
                    ape = exact_ape(prediction, observed_value)
                except ValueError:
                    reasons.append("final_holdout_zero_signal")
                    ape = None
                if ape is not None:
                    final_apes.append(ape)
                    if ape > Decimal(QUALITY_GATES["final_scenario_row_ape_max"]):
                        reasons.append("final_holdout_row")
                    if count == 32 and ape > Decimal(
                        QUALITY_GATES["count_holdout_ape_max"]
                    ):
                        reasons.append("count_holdout")
                    if count == 64 and ape > Decimal(
                        QUALITY_GATES["extrapolation_ape_max"]
                    ):
                        reasons.append("extrapolation")
                final_rows.append(
                    {
                        "scenario": scenario.name,
                        "count": count,
                        "observed_increment": _decimal_text(observed_value),
                        "predicted_increment": _decimal_text(prediction),
                        "ape": _decimal_text(ape) if ape is not None else None,
                    }
                )
        family_mape = (
            sum(final_apes, Decimal(0)) / Decimal(len(final_apes))
            if final_apes
            else Decimal(0)
        )
        if family_mape > Decimal(QUALITY_GATES["final_scenario_family_mape_max"]):
            reasons.append("final_holdout_mape")
        row_decisions = []
        for scenario in (*fit_scenarios, *selection_scenarios, *final_scenarios):
            features = production_context_design_row(selected_candidate, scenario)
            event_cost = sum(
                (
                    feature * coefficients[term]
                    for feature, term in zip(features, selected_candidate.terms)
                ),
                Decimal(0),
            )
            if not event_cost.is_finite() or event_cost < 0:
                reasons.append("invalid_prediction")
            for count in scenario.counts(manifest):
                row_decisions.append(
                    {
                        "scenario": scenario.name,
                        "split": scenario.split,
                        "count": count,
                        "observed_increment": _decimal_text(
                            residual_rows[(scenario.name, count)]["observed"]
                        ),
                        "predicted_increment": _decimal_text(
                            Decimal(count) * event_cost
                        ),
                        "control_residual": _decimal_text(
                            residual_rows[(scenario.name, count)]["control"]
                        ),
                    }
                )
        reasons = sorted(set(reasons))
        family_results[key] = {
            "status": "accepted" if not reasons else "rejected",
            "selected_candidate": selected_candidate.name,
            "coefficients": {
                term: _decimal_text(value) for term, value in coefficients.items()
            },
            "candidate_reports": candidate_reports,
            "selection_decisions": selection_decisions,
            "sibling_decisions": sibling_decisions,
            "rows": row_decisions,
            "selection_rows": selected_rows,
            "final_holdout_rows": final_rows,
            "final_holdout_mape": _decimal_text(family_mape),
            "maximum_control_residual": _decimal_text(maximum_control),
            "rejection_reasons": reasons,
        }
    result = {
        "schema_version": 1,
        "purpose": "production_context_fit_decisions",
        "manifest_identity_sha256": manifest.identity_sha256,
        "families": family_results,
        "all_families_accepted": all(
            family["status"] == "accepted" for family in family_results.values()
        ),
    }
    result["decision_sha256"] = sha256_bytes(canonical_json(result))
    return result


def load_production_v5_subtotal_model(
    manifest: ProductionContextManifest, repo_root: pathlib.Path
) -> ProductionSubtotalModel:
    """Load production-scaled V5 prices while excluding unsupported context targets."""
    repo_root = repo_root.resolve(strict=True)
    _, coverage = _load_pinned_json(
        repo_root, manifest.sources["operation_coverage_v5"], "operation_coverage_v5"
    )
    _, higher = _load_pinned_json(
        repo_root, manifest.sources["higher_layer"], "higher_layer"
    )
    fixed_costs = higher.get("fixed_costs")
    if not isinstance(fixed_costs, Mapping):
        raise ValueError("pinned higher-layer fixed costs differ")
    target_keys = set(manifest.keys)
    opcode_prices: dict[str, Any] = {}
    artifact_cache: dict[str, Mapping[str, Any]] = {}
    for row in coverage.get("execution_coverage", ()):
        if not isinstance(row, Mapping) or row.get("component") != "opcode":
            continue
        key = row.get("key")
        if key in target_keys or row.get("model_status") != "measured":
            continue
        reference = row.get("artifact_ref")
        if not isinstance(reference, Mapping):
            continue
        relative = _require_relative_path(reference.get("path"), "V5 model path")
        artifact = artifact_cache.get(relative)
        if artifact is None:
            path = repo_root / relative
            try:
                artifact = json.loads(path.read_bytes(), object_pairs_hook=_reject_duplicate_fields)
            except (OSError, json.JSONDecodeError) as error:
                raise ValueError("V5 model artifact is unreadable") from error
            if artifact.get("artifact_sha256") != reference.get("artifact_sha256"):
                raise ValueError("V5 model artifact identity differs")
            _validate_content_address(
                artifact, reference["artifact_sha256"], "V5 model artifact"
            )
            artifact_cache[relative] = artifact
        model = artifact.get("registry", {}).get("models", {}).get(key)
        if (
            isinstance(model, Mapping)
            and model.get("kind") == "static_raw_gas"
            and isinstance(model.get("parameters"), Mapping)
        ):
            opcode_prices[key] = model["parameters"].get("body_per_raw_gas")
    return ProductionSubtotalModel.from_mappings(
        fixed_costs=fixed_costs, opcode_prices=opcode_prices
    )


def _write_json_create_only(path: pathlib.Path, payload: Mapping[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        with path.open("xb") as output:
            output.write(canonical_json(payload) + b"\n")
    except FileExistsError as error:
        raise ValueError(f"create-only output already exists: {path}") from error


def _load_canonical_json(path: pathlib.Path, *, label: str) -> dict[str, Any]:
    try:
        raw = path.read_bytes()
        payload = json.loads(
            raw,
            object_pairs_hook=_reject_duplicate_fields,
            parse_float=_reject_json_float,
            parse_constant=_reject_json_float,
        )
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is missing or invalid") from error
    if not isinstance(payload, dict) or raw != canonical_json(payload) + b"\n":
        raise ValueError(f"{label} is not canonical JSON")
    return payload


def load_production_context_parity_identity(path: pathlib.Path) -> dict[str, Any]:
    return _load_canonical_json(
        pathlib.Path(path), label="production context parity identity"
    )


def prepare_production_context_run(
    *,
    manifest: ProductionContextManifest,
    row_requests: Sequence[ProductionContextFixtureRequest],
    run: pathlib.Path,
    launcher: pathlib.Path,
    production_elf: pathlib.Path,
    production_vk: pathlib.Path,
    trace_source: pathlib.Path,
    implementation_revision: str,
    source_hashes: Mapping[str, str],
    parity_identity: Mapping[str, Any],
) -> pathlib.Path:
    if _GIT_REVISION_RE.fullmatch(implementation_revision) is None:
        raise ValueError("production context implementation revision differs")
    parity_unhashed = dict(parity_identity)
    parity_claimed = parity_unhashed.pop("identity_sha256", None)
    if (
        parity_identity.get("model_sample") is not False
        or _SHA256_RE.fullmatch(parity_claimed or "") is None
        or sha256_bytes(canonical_json(parity_unhashed)) != parity_claimed
    ):
        raise ValueError("production context parity identity differs")
    if not source_hashes or any(
        not isinstance(key, str) or _SHA256_RE.fullmatch(value) is None
        for key, value in source_hashes.items()
    ):
        raise ValueError("production context source hashes differ")
    assets = {
        "launcher": pathlib.Path(launcher),
        "production_elf": pathlib.Path(production_elf),
        "production_vk": pathlib.Path(production_vk),
        "trace_source": pathlib.Path(trace_source),
    }
    asset_identity = {}
    for key, path in assets.items():
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"production context {key} is not a regular file")
        asset_identity[key] = {"basename": path.name, "sha256": _sha256_file(path)}
    run = pathlib.Path(run)
    try:
        run.mkdir(parents=True, exist_ok=False)
    except FileExistsError as error:
        raise ValueError(f"production context run already exists: {run}") from error
    (run / "row-inputs").mkdir()
    (run / "rows").mkdir()
    row_entries = []
    seen = set()
    for request in row_requests:
        if request.row_id in seen:
            raise ValueError("duplicate production context prepared row")
        seen.add(request.row_id)
        payload = {
            "row_id": request.row_id,
            "workload_id": request.workload_id,
            "scenario": request.scenario,
            "split": request.split,
            "count": request.count,
            "lane": request.lane,
            "repeat_index": request.repeat_index,
            "builder_input": dict(request.builder_input),
        }
        input_sha256 = sha256_bytes(canonical_json(payload))
        wrapped = {**payload, "input_sha256": input_sha256}
        relative = pathlib.PurePosixPath("row-inputs") / f"{request.row_id}.json"
        _write_json_create_only(run / relative, wrapped)
        row_entries.append(
            {
                "row_id": request.row_id,
                "path": str(relative),
                "input_sha256": input_sha256,
            }
        )
    identity = {
        "schema_version": 1,
        "purpose": "production_context_run_identity",
        "manifest_identity_sha256": manifest.identity_sha256,
        "implementation_revision": implementation_revision,
        "source_hashes": dict(sorted(source_hashes.items())),
        "assets": asset_identity,
        "parity_identity": dict(parity_identity),
        "rows": row_entries,
    }
    identity["identity_sha256"] = sha256_bytes(canonical_json(identity))
    _write_json_create_only(run / "calibration-identity.json", identity)
    return run


def _resolve_run_asset(
    run: pathlib.Path,
    identity: Mapping[str, Any],
    role: str,
    supplied: pathlib.Path | None,
) -> pathlib.Path:
    asset = identity.get("assets", {}).get(role)
    if not isinstance(asset, Mapping) or set(asset) != {"basename", "sha256"}:
        raise ValueError(f"production context {role} identity differs")
    path = pathlib.Path(supplied) if supplied is not None else run.parent / asset["basename"]
    if not path.is_file() or path.is_symlink() or path.name != asset["basename"]:
        raise ValueError(f"production context {role} path differs")
    if _sha256_file(path) != asset["sha256"]:
        label = "launcher hash" if role == "launcher" else f"{role} hash"
        raise ValueError(f"production context {label} differs")
    return path


def _validate_run_identity(
    run: pathlib.Path,
    *,
    launcher: pathlib.Path | None = None,
    production_elf: pathlib.Path | None = None,
    production_vk: pathlib.Path | None = None,
    trace_source: pathlib.Path | None = None,
) -> tuple[dict[str, Any], dict[str, pathlib.Path]]:
    identity = _load_canonical_json(
        run / "calibration-identity.json", label="production context run identity"
    )
    claimed = identity.get("identity_sha256")
    unhashed = dict(identity)
    unhashed.pop("identity_sha256", None)
    if _SHA256_RE.fullmatch(claimed or "") is None or sha256_bytes(
        canonical_json(unhashed)
    ) != claimed:
        raise ValueError("production context run identity hash differs")
    resolved = {
        role: _resolve_run_asset(run, identity, role, supplied)
        for role, supplied in {
            "launcher": launcher,
            "production_elf": production_elf,
            "production_vk": production_vk,
            "trace_source": trace_source,
        }.items()
    }
    return identity, resolved


def _validate_prepared_row_input(
    run: pathlib.Path, entry: Mapping[str, Any]
) -> dict[str, Any]:
    if not isinstance(entry, Mapping) or set(entry) != {"row_id", "path", "input_sha256"}:
        raise ValueError("production context prepared row entry differs")
    expected_path = f"row-inputs/{entry['row_id']}.json"
    if entry["path"] != expected_path or _SHA256_RE.fullmatch(entry["row_id"]) is None:
        raise ValueError("production context prepared row path differs")
    payload = _load_canonical_json(run / expected_path, label="prepared row input")
    unhashed = dict(payload)
    claimed = unhashed.pop("input_sha256", None)
    if (
        claimed != entry["input_sha256"]
        or sha256_bytes(canonical_json(unhashed)) != claimed
        or payload.get("row_id") != entry["row_id"]
    ):
        raise ValueError("production context prepared row hash differs")
    return payload


def _normalize_execution_report(
    row_input: Mapping[str, Any], report: Mapping[str, Any]
) -> dict[str, Any]:
    _reject_forbidden_fit_fields(report, "execution report")
    try:
        controlled = report["controlled_block"]
        observation = controlled["observation"]
        guest_input_sha256 = report["guest_input_sha256"].removeprefix("0x")
        if (
            report["sp1_execution_engine"] != "gas-estimator"
            or report["exit_code"] != 0
            or controlled["status"] != "accepted"
            or controlled["row_id"] != row_input["row_id"]
            or observation["backend_input_sha256"] != guest_input_sha256
            or observation["public_output"].lower() != report["public_values"].lower()
        ):
            raise ValueError("production context execution report is not accepted")
        normalized = {
            "row_id": row_input["row_id"],
            "input_sha256": row_input["input_sha256"],
            "scenario": row_input["scenario"],
            "split": row_input["split"],
            "count": row_input["count"],
            "lane": row_input["lane"],
            "repeat_index": row_input["repeat_index"],
            "prover_gas": _decimal_text(
                _decimal(report["gas"], label="execution prover gas", nonnegative=True)
            ),
            "public_output": report["public_values"],
            "backend_input_sha256": observation["backend_input_sha256"],
            "host_trace_sha256": observation["host_trace_sha256"],
            "actual_raw_gas_by_key": observation["actual_raw_gas_by_key"],
            "actual_context_features": observation["actual_context_features"],
            "actual_features": observation["actual_features"],
            "actual_diagnostics": observation["actual_diagnostics"],
        }
    except (KeyError, AttributeError, TypeError) as error:
        raise ValueError("production context execution report is incomplete") from error
    _normalized_fit_row(normalized)
    evidence = dict(normalized)
    evidence["evidence_sha256"] = sha256_bytes(canonical_json(evidence))
    return evidence


def _default_row_executor(
    row_input: Mapping[str, Any], *, launcher: pathlib.Path, run: pathlib.Path
) -> Mapping[str, Any]:
    with tempfile.TemporaryDirectory(prefix="raiko2-production-context-") as directory:
        temporary = pathlib.Path(directory)
        source = temporary / "row.json"
        output = temporary / "report.jsonl"
        source.write_bytes(canonical_json(row_input["builder_input"]) + b"\n")
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
            cwd=run.parent,
            check=True,
            capture_output=True,
            text=True,
            timeout=3600,
        )
        lines = output.read_text().splitlines()
        if len(lines) != 1:
            raise ValueError("production context launcher emitted an unexpected row count")
        return json.loads(
            lines[0],
            object_pairs_hook=_reject_duplicate_fields,
            parse_float=_reject_json_float,
            parse_constant=_reject_json_float,
        )


def run_production_context_campaign(
    run: pathlib.Path,
    *,
    executor: Callable[[Mapping[str, Any]], Mapping[str, Any]] | None = None,
    launcher: pathlib.Path | None = None,
    production_elf: pathlib.Path | None = None,
    production_vk: pathlib.Path | None = None,
    trace_source: pathlib.Path | None = None,
) -> dict[str, Any]:
    """Run or resume rows, persisting every accepted row before continuing."""
    run = pathlib.Path(run)
    identity, assets = _validate_run_identity(
        run,
        launcher=launcher,
        production_elf=production_elf,
        production_vk=production_vk,
        trace_source=trace_source,
    )
    row_hashes = []
    for entry in identity.get("rows", ()):
        row_input = _validate_prepared_row_input(run, entry)
        output_path = run / "rows" / f"{entry['row_id']}.json"
        if output_path.exists():
            evidence = _load_canonical_json(output_path, label="production context row")
            claimed = evidence.get("evidence_sha256")
            unhashed = dict(evidence)
            unhashed.pop("evidence_sha256", None)
            if (
                claimed is None
                or sha256_bytes(canonical_json(unhashed)) != claimed
                or evidence.get("row_id") != entry["row_id"]
                or evidence.get("input_sha256") != entry["input_sha256"]
            ):
                raise ValueError("production context existing row hash differs")
        else:
            report = (
                executor(row_input)
                if executor is not None
                else _default_row_executor(
                    row_input, launcher=assets["launcher"], run=run
                )
            )
            evidence = _normalize_execution_report(row_input, report)
            _write_json_create_only(output_path, evidence)
        row_hashes.append(
            {"row_id": entry["row_id"], "evidence_sha256": evidence["evidence_sha256"]}
        )
    terminal = {
        "schema_version": 1,
        "status": "execution_complete",
        "identity_sha256": identity["identity_sha256"],
        "row_hashes": row_hashes,
    }
    terminal["terminal_sha256"] = sha256_bytes(canonical_json(terminal))
    path = run / "execution-complete.json"
    if path.exists():
        existing = _load_canonical_json(path, label="execution terminal")
        if canonical_json(existing) != canonical_json(terminal):
            raise ValueError("production context execution terminal differs")
    else:
        _write_json_create_only(path, terminal)
    return terminal


def fit_production_context_run(
    run: pathlib.Path,
    *,
    manifest: ProductionContextManifest,
    subtotal_model: ProductionSubtotalModel,
    launcher: pathlib.Path | None = None,
    production_elf: pathlib.Path | None = None,
    production_vk: pathlib.Path | None = None,
    trace_source: pathlib.Path | None = None,
) -> dict[str, Any]:
    run = pathlib.Path(run)
    identity, _assets = _validate_run_identity(
        run,
        launcher=launcher,
        production_elf=production_elf,
        production_vk=production_vk,
        trace_source=trace_source,
    )
    if identity.get("manifest_identity_sha256") != manifest.identity_sha256:
        raise ValueError("production context fit manifest identity differs")
    completion = _load_canonical_json(
        run / "execution-complete.json", label="production context execution terminal"
    )
    completion_unhashed = dict(completion)
    completion_claimed = completion_unhashed.pop("terminal_sha256", None)
    if (
        completion.get("identity_sha256") != identity["identity_sha256"]
        or _SHA256_RE.fullmatch(completion_claimed or "") is None
        or sha256_bytes(canonical_json(completion_unhashed)) != completion_claimed
    ):
        raise ValueError("production context execution identity differs")
    completion_hashes = completion.get("row_hashes")
    if not isinstance(completion_hashes, list) or len(completion_hashes) != len(
        identity["rows"]
    ):
        raise ValueError("production context execution row inventory differs")
    rows = []
    for entry, completed in zip(identity["rows"], completion_hashes):
        row = _load_canonical_json(
            run / "rows" / f"{entry['row_id']}.json",
            label="production context fit row",
        )
        row_unhashed = dict(row)
        row_claimed = row_unhashed.pop("evidence_sha256", None)
        if (
            completed
            != {"row_id": entry["row_id"], "evidence_sha256": row_claimed}
            or _SHA256_RE.fullmatch(row_claimed or "") is None
            or sha256_bytes(canonical_json(row_unhashed)) != row_claimed
            or row.get("input_sha256") != entry["input_sha256"]
        ):
            raise ValueError("production context completed row hash differs")
        rows.append(row)
    decisions = fit_production_context_rows(manifest, rows, subtotal_model)
    _write_json_create_only(run / "campaign-decisions.json", decisions)
    terminal = {
        "schema_version": 1,
        "status": (
            "accepted" if decisions["all_families_accepted"] else "rejected"
        ),
        "identity_sha256": identity["identity_sha256"],
        "decision_sha256": decisions["decision_sha256"],
    }
    terminal["terminal_sha256"] = sha256_bytes(canonical_json(terminal))
    _write_json_create_only(run / "terminal.json", terminal)
    return terminal
