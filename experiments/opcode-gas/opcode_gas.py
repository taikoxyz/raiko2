#!/usr/bin/env python3
"""SP1 opcode prover-gas experiment scaffold."""

from __future__ import annotations

import argparse
import fcntl
import functools
import hashlib
import json
import os
import pathlib
import statistics
import stat
import subprocess
import tarfile
import tempfile
import tomllib
from dataclasses import asdict, dataclass, field, replace
from decimal import Context, Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from types import MappingProxyType
from typing import Any, Iterable, Mapping

from calibration_model import (
    AffineOpcodeModel,
    BlockCalibrationRow,
    DynamicRelationObservation,
    RelationEquation,
    derive_affine_opcode_model,
    exact_rank,
    fit_block_calibration,
    validate_dynamic_holdouts,
)


_OPCODE_DECIMAL_CONTEXT = Context(prec=80, rounding=ROUND_HALF_EVEN, traps=[])


def _isolated_decimal_context(function):
    """Run Decimal-heavy library entrypoints without mutating caller state."""
    @functools.wraps(function)
    def wrapped(*args, **kwargs):
        with localcontext(_OPCODE_DECIMAL_CONTEXT):
            return function(*args, **kwargs)

    return wrapped


FORMAL_RELATION_PURPOSE = "formal_opcode_relation"
FORMAL_RELATION_SIGNAL_KIND = "signed_raw_gas_relation"
OPCODE_RELATION_ANCHORS = (
    "opcode:0x50",
    "opcode:0x5f",
    "opcode:0x80",
    "opcode:0x90",
)
DYNAMIC_RAW_GAS_KEYS = (
    "opcode:0x0a",
    "opcode:0x20",
    "opcode:0x51",
    "opcode:0x52",
    "opcode:0x53",
    "opcode:0x5e",
)
FORMAL_RELATION_QUALITY_GATES = {
    "repeats": 3,
    "r2_min": "0.995",
    "relative_slope_stderr_max": "0.05",
    "residual_signal_max": "0.02",
    "checkpoint_ape_max": "0.10",
    "signal_min_prover_gas": "1000",
    "signal_min_baseline_fraction": "0.01",
    "signal_min_repeat_noise_multiple": "20",
}
FORMAL_RELATION_PROVENANCE_FIELDS = (
    "calibration_id",
    "calibration_identity_sha256",
    "implementation_revision",
    "controlled_manifest_sha256",
    "controlled_manifest_rows_sha256",
)


@dataclass(frozen=True)
class CaseSpec:
    name: str
    scenario: str
    template: str
    target_raw_gas: int
    kind: str = "opcode"
    opcode: int | None = None
    address: int | None = None
    input_size: int | None = None
    execution_basis: str | None = None
    spawned: bool | None = None
    dispatch_status: str | None = None
    paired: bool = False
    expected_output_size: int | None = None


@dataclass(frozen=True)
class EventMatchSpec:
    component: str
    opcode: int | None = None
    address: int | None = None
    spawned: bool | None = None
    dispatch_status: str | None = None


@dataclass(frozen=True)
class MeasurementKeySpec:
    id: str
    production_schedule_key: str
    event_match: EventMatchSpec
    pricing_basis: str
    required_case_ids: tuple[str, ...]
    diagnostic_case_ids: tuple[str, ...]


@dataclass(frozen=True)
class OverheadCaseSpec:
    name: str
    overhead_key_id: str
    target_template: str
    control_template: str
    expected_changed_feature_keys: tuple[str, ...]


@dataclass(frozen=True)
class OverheadKeySpec:
    id: str
    unit: str
    formula_role: str
    subtract_keys: tuple[str, ...]
    bundled_keys: tuple[str, ...]
    bundled_ratios: Mapping[str, tuple[int, int]]
    required_case_ids: tuple[str, ...]
    diagnostic_case_ids: tuple[str, ...]


@dataclass(frozen=True)
class OpcodeRelationSpec:
    id: str
    case_id: str
    key_id: str
    split: str
    scenario_id: str
    scenario: Mapping[str, int | str]
    target_raw_gas_by_key: Mapping[str, int]
    control_raw_gas_by_key: Mapping[str, int]
    signed_raw_gas_by_key: Mapping[str, int]
    dynamic_key: str | None = None


@dataclass(frozen=True)
class ControlledBlockProgramSpec:
    kind: str
    value: int | None = None
    family: str | None = None
    count: int | None = None
    scenario: str | None = None


@dataclass(frozen=True)
class ControlledBlockRowSpec:
    row_id: str
    workload_family: str
    split: str
    block_count: int
    transaction_count: int
    program: ControlledBlockProgramSpec
    expected_final_state_root: str
    expected_raw_gas_by_key: Mapping[str, int]
    expected_features: Mapping[str, int]
    expected_diagnostics: Mapping[str, int]


@dataclass(frozen=True)
class Manifest:
    name: str
    backend: str
    variants: list[int]
    cases: list[CaseSpec]
    measurement_keys: tuple[MeasurementKeySpec, ...] = ()
    normalization_reference_key: str | None = None
    system_operation_ownership: str | None = None
    anchor_operation_ownership: str | None = None
    bridge_key_ids: tuple[str, ...] = ()
    bridge_model: str | None = None
    bridge_controlled_max_ape: Decimal | None = None
    bridge_proposal_max_ape: Decimal | None = None
    overhead_keys: tuple[OverheadKeySpec, ...] = ()
    overhead_cases: tuple[OverheadCaseSpec, ...] = ()
    q_formula: tuple[str, ...] = ()
    subtract_closure: Mapping[str, tuple[str, ...]] = field(default_factory=dict)
    opcode_relation_anchors: tuple[str, ...] = ()
    dynamic_raw_gas_keys: tuple[str, ...] = ()
    opcode_relations: tuple[OpcodeRelationSpec, ...] = ()
    block_calibration_rows: tuple[ControlledBlockRowSpec, ...] = ()


@dataclass(frozen=True)
class GeneratedBytecode:
    bytes_hex: str
    opcode_counts: dict[int, int]


@dataclass(frozen=True)
class FitResult:
    case: str
    metric: str
    sample_count: int
    slope_per_operation: float
    slope_per_raw_gas: float
    intercept: float
    r2: float


@dataclass(frozen=True)
class DamageResult:
    case: str
    kind: str
    workload_metric: str
    eth_gas_per_unit: int
    measured_workload_per_unit: float
    damage_ratio: float
    r2: float
    eth_only_units: int
    eth_only_damage: float
    zkgas_multiplier: int
    zkgas_per_unit: int
    candidate_units: int
    candidate_damage: float
    attack_reduction: float
    binding_resource: str


@dataclass(frozen=True)
class InventoryRow:
    kind: str
    identifier: str
    name: str
    multiplier: int
    status: str
    manifest_case: str | None = None


@dataclass(frozen=True)
class UnzenSchedule:
    opcode_multipliers: Mapping[int, int]
    precompile_multipliers: Mapping[int, int]
    opcode_explicit: Mapping[int, bool] = field(default_factory=dict)
    precompile_explicit: Mapping[int, bool] = field(default_factory=dict)
    precompile_fallback_multiplier: int | None = None
    failsafe_multiplier: int | None = None
    block_limit: int | None = None
    tx_intrinsic_zk_gas: int | None = None
    spawn_estimates: Mapping[str, int] = field(default_factory=dict)


REPO_ROOT = pathlib.Path(__file__).resolve().parents[2]

UZEN_OPCODE_NAMES = {
    0x00: "stop",
    0x01: "add",
    0x02: "mul",
    0x03: "sub",
    0x04: "div",
    0x05: "sdiv",
    0x06: "mod",
    0x07: "smod",
    0x08: "addmod",
    0x09: "mulmod",
    0x0A: "exp",
    0x0B: "signextend",
    0x10: "lt",
    0x11: "gt",
    0x12: "slt",
    0x13: "sgt",
    0x14: "eq",
    0x15: "iszero",
    0x16: "and",
    0x17: "or",
    0x18: "xor",
    0x19: "not",
    0x1A: "byte",
    0x1B: "shl",
    0x1C: "shr",
    0x1D: "sar",
    0x1E: "clz",
    0x20: "keccak256",
    0x30: "address",
    0x31: "balance",
    0x32: "origin",
    0x33: "caller",
    0x34: "callvalue",
    0x35: "calldataload",
    0x36: "calldatasize",
    0x37: "calldatacopy",
    0x38: "codesize",
    0x39: "codecopy",
    0x3A: "gasprice",
    0x3B: "extcodesize",
    0x3C: "extcodecopy",
    0x3D: "returndatasize",
    0x3E: "returndatacopy",
    0x3F: "extcodehash",
    0x40: "blockhash",
    0x41: "coinbase",
    0x42: "timestamp",
    0x43: "number",
    0x44: "prevrandao",
    0x45: "gaslimit",
    0x46: "chainid",
    0x47: "selfbalance",
    0x48: "basefee",
    0x49: "blobhash",
    0x4A: "blobbasefee",
    0x50: "pop",
    0x51: "mload",
    0x52: "mstore",
    0x53: "mstore8",
    0x54: "sload",
    0x55: "sstore",
    0x56: "jump",
    0x57: "jumpi",
    0x58: "pc",
    0x59: "msize",
    0x5A: "gas",
    0x5B: "jumpdest",
    0x5C: "tload",
    0x5D: "tstore",
    0x5E: "mcopy",
    0x5F: "push0",
    **{opcode: f"push{opcode - 0x5F}" for opcode in range(0x60, 0x80)},
    **{opcode: f"dup{opcode - 0x7F}" for opcode in range(0x80, 0x90)},
    **{opcode: f"swap{opcode - 0x8F}" for opcode in range(0x90, 0xA0)},
    0xA0: "log0",
    0xA1: "log1",
    0xA2: "log2",
    0xA3: "log3",
    0xA4: "log4",
    0xF0: "create",
    0xF1: "call",
    0xF2: "callcode",
    0xF3: "return",
    0xF4: "delegatecall",
    0xF5: "create2",
    0xFA: "staticcall",
    0xFD: "revert",
    0xFE: "invalid",
    0xFF: "selfdestruct",
}

UZEN_PRECOMPILE_NAMES = {
    0x01: "ecrecover",
    0x02: "sha256",
    0x03: "ripemd160",
    0x04: "identity",
    0x05: "modexp",
    0x06: "bn128_add",
    0x07: "bn128_mul",
    0x08: "bn128_pairing",
    0x09: "blake2f",
    0x0A: "point_evaluation",
    0x0B: "bls12_g1add",
    0x0C: "bls12_g1msm",
    0x0D: "bls12_g2add",
    0x0E: "bls12_g2msm",
    0x0F: "bls12_pairing",
    0x10: "bls12_map_fp_to_g1",
    0x11: "bls12_map_fp2_to_g2",
    0x100: "p256verify",
}

SPAWN_WRAPPER_OPCODES = {0xF0, 0xF1, 0xF2, 0xF4, 0xF5, 0xFA}
ZERO_OR_HALTING_OPCODES = {0x00, 0xF3, 0xFD, 0xFE, 0xFF}
STATE_OR_REVM_OPCODES = {
    0x30,
    0x31,
    0x32,
    0x33,
    0x34,
    0x35,
    0x36,
    0x37,
    0x38,
    0x39,
    0x3A,
    0x3B,
    0x3C,
    0x3D,
    0x3E,
    0x3F,
    0x40,
    0x41,
    0x42,
    0x43,
    0x44,
    0x45,
    0x46,
    0x47,
    0x48,
    0x49,
    0x4A,
    0x54,
    0x55,
    0x5C,
    0x5D,
    0xA0,
    0xA1,
    0xA2,
    0xA3,
    0xA4,
}
PURE_OPCODE_DEFAULTS = {
    0x01: ("arithmetic", "stack_binary", 3),
    0x02: ("arithmetic", "stack_binary", 5),
    0x03: ("arithmetic", "stack_binary", 3),
    0x04: ("arithmetic", "stack_binary", 5),
    0x05: ("arithmetic", "stack_binary", 5),
    0x06: ("arithmetic", "stack_binary", 5),
    0x07: ("arithmetic", "stack_binary", 5),
    0x08: ("arithmetic", "stack_ternary", 8),
    0x09: ("arithmetic", "stack_ternary", 8),
    0x0A: ("arithmetic", "stack_exp", 60),
    0x0B: ("arithmetic", "stack_binary", 5),
    0x10: ("comparison", "stack_binary", 3),
    0x11: ("comparison", "stack_binary", 3),
    0x12: ("comparison", "stack_binary", 3),
    0x13: ("comparison", "stack_binary", 3),
    0x14: ("comparison", "stack_binary", 3),
    0x15: ("bitwise", "stack_unary", 3),
    0x16: ("bitwise", "stack_binary", 3),
    0x17: ("bitwise", "stack_binary", 3),
    0x18: ("bitwise", "stack_binary", 3),
    0x19: ("bitwise", "stack_unary", 3),
    0x1A: ("bitwise", "stack_binary", 3),
    0x1B: ("bitwise", "stack_binary", 3),
    0x1C: ("bitwise", "stack_binary", 3),
    0x1D: ("bitwise", "stack_binary", 3),
    0x20: ("memory", "keccak_32", 36),
    0x50: ("stack", "stack_pop", 2),
    0x51: ("memory", "memory_load_32", 3),
    0x52: ("memory", "memory_store_32", 3),
    0x53: ("memory", "memory_store8", 3),
    0x56: ("control", "jump_chain", 8),
    0x57: ("control", "jumpi_chain", 10),
    0x58: ("control", "stack_unary_producer", 2),
    0x59: ("memory", "stack_unary_producer", 2),
    0x5A: ("control", "stack_unary_producer", 2),
    0x5B: ("control", "jumpdest_chain", 1),
    0x5E: ("memory", "memory_copy_32", 6),
    0x5F: ("stack", "stack_push", 2),
}

for _opcode in range(0x60, 0x80):
    PURE_OPCODE_DEFAULTS[_opcode] = ("stack", "stack_push", 3)
for _opcode in range(0x80, 0x90):
    PURE_OPCODE_DEFAULTS[_opcode] = ("stack", "stack_dup", 3)
for _opcode in range(0x90, 0xA0):
    PURE_OPCODE_DEFAULTS[_opcode] = ("stack", "stack_swap", 3)

PRECOMPILE_BODY_DEFAULTS = {
    0x01: ("precompile_ecrecover_valid", 128, 3_000),
    0x02: ("precompile_fixed_32", 32, 72),
    0x03: ("precompile_fixed_32", 32, 720),
    0x04: ("precompile_fixed_32", 32, 18),
    0x05: ("precompile_modexp_small", 99, 500),
    0x06: ("precompile_bn254_add", 128, 150),
    0x07: ("precompile_bn254_mul", 96, 6_000),
    0x08: ("precompile_bn254_pairing", 192, 79_000),
    0x09: ("precompile_blake2f_12_rounds", 213, 12),
    0x0A: ("precompile_kzg_point_evaluation", 192, 50_000),
    0x0B: ("precompile_bls12_g1add_zero", 256, 375),
    0x0C: ("precompile_bls12_g1msm_zero", 160, 12_000),
    0x0D: ("precompile_bls12_g2add_zero", 512, 600),
    0x0E: ("precompile_bls12_g2msm_zero", 288, 22_500),
    0x0F: ("precompile_bls12_pairing_zero", 384, 70_300),
    0x10: ("precompile_bls12_map_fp_to_g1_zero", 64, 5_500),
    0x11: ("precompile_bls12_map_fp2_to_g2_zero", 128, 23_800),
}

PLANNED_PURE_OPCODE_OPCODES = set(PURE_OPCODE_DEFAULTS)


def load_current_uzen_schedule() -> UnzenSchedule:
    command = [
        "cargo",
        "run",
        "--quiet",
        "--locked",
        "-p",
        "xtask",
        "--no-default-features",
        "--",
        "export-unzen-zk-gas-schedule",
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError as exc:
        detail = exc.stderr.strip() if exc.stderr and exc.stderr.strip() else str(exc)
        raise RuntimeError(f"Unzen schedule exporter failed:\n{detail}") from exc
    except OSError as exc:
        raise RuntimeError(f"failed to run Unzen schedule exporter: {exc}") from exc
    try:
        data = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        detail = (
            "empty output"
            if not completed.stdout.strip()
            else f"{exc}; stdout={completed.stdout[:200]!r}"
        )
        raise RuntimeError(
            f"Unzen schedule exporter did not return valid JSON: {detail}"
        ) from exc
    try:
        return parse_complete_uzen_schedule(data)
    except ValueError as exc:
        raise RuntimeError(f"invalid Unzen schedule export: {exc}") from exc


def canonical_json(value: Any) -> bytes:
    """Stable bytes for every content-addressed experiment artifact."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    return sha256_bytes(path.read_bytes())


def parse_complete_uzen_schedule(data: Any) -> UnzenSchedule:
    """Parse the full exporter contract, including failsafe identities.

    The old exporter omitted failsafe opcodes, which made a schedule hash unable to
    distinguish an explicitly priced entry from an inherited fail-safe value.
    """
    if not isinstance(data, dict):
        raise ValueError("Unzen schedule export must be an object")
    opcode_multipliers = _parse_schedule_rows(
        data, rows_key="opcodes", identifier_key="opcode", max_identifier=0xFF
    )
    if len(opcode_multipliers) != 256:
        raise ValueError("Unzen schedule export must contain all 256 opcode rows")
    precompile_multipliers = _parse_schedule_rows(
        data,
        rows_key="precompiles",
        identifier_key="address",
        max_identifier=(1 << 160) - 1,
    )
    failsafe = data.get("failsafe_multiplier", data.get("precompile_fallback_multiplier"))
    if isinstance(failsafe, bool) or not isinstance(failsafe, int) or failsafe < 0:
        raise ValueError("Unzen schedule export is missing failsafe_multiplier")
    fallback = data.get("precompile_fallback_multiplier", failsafe)
    if fallback != failsafe:
        raise ValueError("precompile fallback must equal failsafe multiplier")
    for key in ("block_limit", "tx_intrinsic_zk_gas"):
        if isinstance(data.get(key), bool) or not isinstance(data.get(key), int) or data[key] <= 0:
            raise ValueError(f"Unzen schedule export has invalid {key}")
    spawn = data.get("spawn_estimates")
    expected_spawn = {"call", "callcode", "delegatecall", "staticcall", "create", "create2"}
    if not isinstance(spawn, dict) or set(spawn) != expected_spawn or any(
        isinstance(value, bool) or not isinstance(value, int) or value <= 0
        for value in spawn.values()
    ):
        raise ValueError("Unzen schedule export has invalid spawn_estimates")
    opcode_explicit = {}
    for row in data["opcodes"]:
        opcode = parse_opcode(row["opcode"])
        explicit = row.get("explicit")
        if not isinstance(explicit, bool):
            raise ValueError("every opcode row must declare explicit identity")
        if not explicit and row["multiplier"] != failsafe:
            raise ValueError("non-explicit opcode must use failsafe multiplier")
        opcode_explicit[opcode] = explicit
    precompile_explicit = {}
    for row in data["precompiles"]:
        address = parse_opcode(row["address"])
        explicit = row.get("explicit", True)
        if not isinstance(explicit, bool) or not explicit:
            raise ValueError("precompile rows must be explicit schedule entries")
        precompile_explicit[address] = explicit
    schedule = UnzenSchedule(
        opcode_multipliers=MappingProxyType(opcode_multipliers),
        precompile_multipliers=MappingProxyType(precompile_multipliers),
        opcode_explicit=MappingProxyType(opcode_explicit),
        precompile_explicit=MappingProxyType(precompile_explicit),
        precompile_fallback_multiplier=fallback,
        failsafe_multiplier=failsafe,
        block_limit=data["block_limit"],
        tx_intrinsic_zk_gas=data["tx_intrinsic_zk_gas"],
        spawn_estimates=MappingProxyType(dict(spawn)),
    )
    exported_hash = data.get("schedule_sha256")
    if exported_hash is not None:
        if not isinstance(exported_hash, str) or exported_hash != schedule_sha256(schedule):
            raise ValueError("Unzen schedule export has an invalid complete schedule hash")
    return schedule


def complete_schedule_identity(schedule: UnzenSchedule) -> dict[str, Any]:
    if len(schedule.opcode_multipliers) != 256 or schedule.failsafe_multiplier is None:
        raise ValueError("schedule is not a complete exported Unzen schedule")
    return {
        "block_limit": schedule.block_limit,
        "failsafe_multiplier": schedule.failsafe_multiplier,
        "opcodes": [
            {
                "explicit": bool(schedule.opcode_explicit.get(opcode)),
                "opcode": f"0x{opcode:02x}",
                "multiplier": schedule.opcode_multipliers[opcode],
            }
            for opcode in range(256)
        ],
        "precompile_fallback_multiplier": schedule.precompile_fallback_multiplier,
        "precompiles": [
            {
                "explicit": bool(schedule.precompile_explicit.get(address)),
                "address": f"0x{address:040x}",
                "multiplier": multiplier,
            }
            for address, multiplier in sorted(schedule.precompile_multipliers.items())
        ],
        "spawn_estimates": dict(sorted(schedule.spawn_estimates.items())),
        "tx_intrinsic_zk_gas": schedule.tx_intrinsic_zk_gas,
    }


def schedule_sha256(schedule: UnzenSchedule) -> str:
    # The exporter uses this declared field order; vectors and maps above are
    # sorted, so these compact bytes are stable across invocations and languages.
    return sha256_bytes(
        json.dumps(complete_schedule_identity(schedule), separators=(",", ":"), ensure_ascii=True).encode()
    )


def _parse_schedule_rows(
    data: Any,
    *,
    rows_key: str,
    identifier_key: str,
    max_identifier: int,
) -> dict[int, int]:
    if not isinstance(data, dict) or not isinstance(data.get(rows_key), list):
        raise ValueError(f"Unzen schedule export is missing {rows_key}")

    result = {}
    for row in data[rows_key]:
        if not isinstance(row, dict) or identifier_key not in row or "multiplier" not in row:
            raise ValueError(f"invalid Unzen schedule {rows_key} row")
        raw_identifier = row[identifier_key]
        if isinstance(raw_identifier, bool) or not isinstance(raw_identifier, (int, str)):
            raise ValueError(
                f"invalid Unzen schedule {identifier_key}: {raw_identifier!r}"
            )
        identifier = parse_opcode(raw_identifier)
        multiplier = row["multiplier"]
        if not 0 <= identifier <= max_identifier:
            raise ValueError(f"invalid Unzen schedule {identifier_key}: {identifier}")
        if isinstance(multiplier, bool) or not isinstance(multiplier, int) or multiplier < 0:
            raise ValueError(f"invalid Unzen schedule multiplier for {row[identifier_key]}")
        if identifier in result:
            raise ValueError(f"duplicate Unzen schedule {identifier_key}: {row[identifier_key]}")
        result[identifier] = multiplier
    return result


@functools.cache
def current_uzen_schedule() -> UnzenSchedule:
    return load_current_uzen_schedule()


def _canonical_schedule_key(component: str, identifier: int) -> str:
    if component == "opcode":
        return f"opcode:0x{identifier:02x}"
    if component == "precompile":
        return f"precompile:0x{identifier:02x}"
    raise ValueError(f"unsupported component: {component}")


def _parse_schedule_key(value: Any) -> tuple[str, int]:
    if not isinstance(value, str) or ":" not in value:
        raise ValueError("production schedule key must be component:identifier")
    component, raw_identifier = value.split(":", 1)
    if component not in {"opcode", "precompile"}:
        raise ValueError("production schedule key has unsupported component")
    identifier = parse_opcode(raw_identifier)
    if value != _canonical_schedule_key(component, identifier):
        raise ValueError("production schedule key must be canonical lowercase hex")
    if component == "opcode" and not 0 <= identifier <= 0xFF:
        raise ValueError("production schedule key opcode is out of range")
    return component, identifier


def _unique_case_ids(
    required: Any,
    diagnostic: Any,
    *,
    label: str,
    required_nonempty: bool,
) -> tuple[tuple[str, ...], tuple[str, ...]]:
    if not isinstance(required, list) or not all(isinstance(value, str) for value in required):
        raise ValueError(f"{label} required_case_ids must be a list of strings")
    if required_nonempty and not required:
        raise ValueError(f"{label} required_case_ids must not be empty")
    if not isinstance(diagnostic, list) or not all(isinstance(value, str) for value in diagnostic):
        raise ValueError(f"{label} diagnostic_case_ids must be a list of strings")
    if len(set(required)) != len(required) or len(set(diagnostic)) != len(diagnostic):
        raise ValueError(f"{label} contains a duplicate case")
    overlap = set(required) & set(diagnostic)
    if overlap:
        raise ValueError(f"{label} case appears as both required and diagnostic")
    return tuple(required), tuple(diagnostic)


def _parse_event_match(item: Any) -> EventMatchSpec:
    if not isinstance(item, Mapping):
        raise ValueError("event_match must be an object")
    supported = {"component", "opcode", "address", "spawned", "dispatch_status"}
    unknown = set(item) - supported
    if unknown:
        raise ValueError(f"unsupported event-match field: {sorted(unknown)[0]}")
    component = item.get("component")
    if component not in {"opcode", "precompile"}:
        raise ValueError("event_match component must be opcode or precompile")
    opcode = parse_opcode(item["opcode"]) if "opcode" in item else None
    address = parse_opcode(item["address"]) if "address" in item else None
    spawned = item.get("spawned")
    dispatch_status = item.get("dispatch_status")
    if spawned is not None and not isinstance(spawned, bool):
        raise ValueError("event_match spawned must be boolean")
    if component == "opcode":
        if opcode is None or address is not None or not 0 <= opcode <= 0xFF:
            raise ValueError("opcode event_match requires only an opcode")
        if spawned is not None and opcode not in SPAWN_WRAPPER_OPCODES:
            raise ValueError("spawned context is supported only for CALL/CREATE opcodes")
        if opcode in SPAWN_WRAPPER_OPCODES and spawned is None:
            raise ValueError("CALL/CREATE event_match must declare spawned")
    else:
        if address is None or opcode is not None:
            raise ValueError("precompile event_match requires only an address")
        if spawned is not None or dispatch_status is not None:
            raise ValueError("precompile event_match cannot declare spawned context")
    if spawned is True:
        if dispatch_status != "confirmed":
            raise ValueError("spawned=true event_match requires dispatch_status=confirmed")
    elif dispatch_status is not None:
        raise ValueError("dispatch_status is valid only for spawned=true")
    return EventMatchSpec(
        component=component,
        opcode=opcode,
        address=address,
        spawned=spawned,
        dispatch_status=dispatch_status,
    )


def _event_matches_overlap(left: EventMatchSpec, right: EventMatchSpec) -> bool:
    if left.component != right.component:
        return False
    if left.opcode != right.opcode or left.address != right.address:
        return False
    if left.spawned is not None and right.spawned is not None and left.spawned != right.spawned:
        return False
    if (
        left.dispatch_status is not None
        and right.dispatch_status is not None
        and left.dispatch_status != right.dispatch_status
    ):
        return False
    return True


def _parse_positive_ratio(value: Any, *, label: str) -> tuple[int, int]:
    if isinstance(value, Mapping):
        if set(value) != {"numerator", "denominator"}:
            raise ValueError(
                f"{label} bundled ratio must contain canonical numerator and denominator"
            )
        numerator = value["numerator"]
        denominator = value["denominator"]
        if (
            isinstance(numerator, bool)
            or isinstance(denominator, bool)
            or not isinstance(numerator, int)
            or not isinstance(denominator, int)
        ):
            raise ValueError(f"{label} bundled ratio must use canonical integers")
        if numerator <= 0 or denominator <= 0:
            raise ValueError(f"{label} bundled ratio must be positive")
        return numerator, denominator
    if not isinstance(value, str) or "/" not in value:
        raise ValueError(f"{label} bundled ratio must be canonical numerator/denominator")
    numerator_text, denominator_text = value.split("/", 1)
    if (
        not numerator_text.isdecimal()
        or not denominator_text.isdecimal()
        or numerator_text.startswith("0")
        or denominator_text.startswith("0")
    ):
        raise ValueError(f"{label} bundled ratio must use positive canonical integers")
    numerator, denominator = int(numerator_text), int(denominator_text)
    if numerator <= 0 or denominator <= 0:
        raise ValueError(f"{label} bundled ratio must be positive")
    return numerator, denominator


def _subtract_closures(overheads: tuple[OverheadKeySpec, ...]) -> dict[str, tuple[str, ...]]:
    by_id = {item.id: item for item in overheads}
    for item in overheads:
        for child in (*item.subtract_keys, *item.bundled_keys):
            if child not in by_id:
                raise ValueError(f"missing dependency {child} for overhead {item.id}")
        if set(item.subtract_keys) & set(item.bundled_keys):
            raise ValueError(f"overhead {item.id} has overlapping subtract and bundled keys")
        if set(item.bundled_ratios) != set(item.bundled_keys):
            raise ValueError(f"overhead {item.id} bundled ratios do not match bundled_keys")

    visiting: set[str] = set()
    complete: dict[str, set[str]] = {}

    def visit(key: str) -> set[str]:
        if key in visiting:
            raise ValueError("overhead subtraction graph contains a cycle")
        if key in complete:
            return complete[key]
        visiting.add(key)
        closure: set[str] = set()
        for child in by_id[key].subtract_keys:
            closure.add(child)
            closure.update(visit(child))
        visiting.remove(key)
        complete[key] = closure
        return closure

    result = {key: tuple(sorted(visit(key))) for key in by_id}
    owners: dict[str, str] = {}
    for item in overheads:
        for child in item.bundled_keys:
            previous = owners.setdefault(child, item.id)
            if previous != item.id:
                raise ValueError(f"bundled key {child} has two parents")
            if child in result[item.id]:
                raise ValueError(f"bundled key {child} is reachable by subtraction")
    return result


BLOCK_CALIBRATION_FAMILIES = (
    "pop_family", "push_family", "dup_family", "swap_family",
    "proposal_startup", "block_base", "tx_base", "native_value_transfer",
)
BLOCK_CALIBRATION_DIAGNOSTICS = (
    "guest_input_bincode_length", "witness_node_count", "witness_byte_count",
    "blob_count", "kzg_invocation_count", "calldata_length", "bytecode_length",
    "touched_state_key_count",
)


def _strict_nonnegative_int_map(value: Any, *, label: str) -> dict[str, int]:
    if not isinstance(value, Mapping):
        raise ValueError(f"{label} must be an object")
    result = {}
    for key, number in value.items():
        if not isinstance(key, str) or type(number) is not int or number < 0:
            raise ValueError(f"{label} must contain non-negative integer values")
        result[key] = number
    return result


def _parse_block_calibration_rows(data: Mapping[str, Any]) -> tuple[ControlledBlockRowSpec, ...]:
    rows = []
    seen = set()
    for item in data.get("block_calibration_rows", []):
        if not isinstance(item, Mapping):
            raise ValueError("block calibration row must be an object")
        program_data = item.get("program")
        if not isinstance(program_data, Mapping):
            raise ValueError("block calibration program must be an object")
        kind = program_data.get("kind")
        if kind == "empty":
            if set(program_data) != {"kind"}:
                raise ValueError("empty block calibration program has extra fields")
            program = ControlledBlockProgramSpec(kind=kind)
        elif kind == "native_transfer":
            if (
                set(program_data) != {"kind", "value"}
                or type(program_data.get("value")) is not int
                or program_data["value"] < 0
            ):
                raise ValueError("native transfer block calibration program is invalid")
            program = ControlledBlockProgramSpec(kind=kind, value=program_data["value"])
        elif kind == "opcode_loop":
            if (
                set(program_data) != {"kind", "family", "count", "scenario"}
                or not isinstance(program_data.get("family"), str)
                or type(program_data.get("count")) is not int
                or program_data["count"] <= 0
                or not isinstance(program_data.get("scenario"), str)
                or not program_data["scenario"]
            ):
                raise ValueError("opcode-loop block calibration program is invalid")
            program = ControlledBlockProgramSpec(
                kind=kind,
                family=program_data["family"],
                count=program_data["count"],
                scenario=program_data["scenario"],
            )
        else:
            raise ValueError("unknown block calibration program kind")
        row_id = item.get("row_id")
        family = item.get("workload_family")
        split = item.get("split")
        block_count = item.get("block_count")
        transaction_count = item.get("transaction_count")
        final_state_root = item.get("expected_final_state_root")
        if not _is_sha256(row_id) or row_id in seen:
            raise ValueError("block calibration row ID is invalid or duplicate")
        if family not in BLOCK_CALIBRATION_FAMILIES or split not in {"fit", "holdout"}:
            raise ValueError("block calibration row family or split is invalid")
        if type(block_count) is not int or not 1 <= block_count <= 768:
            raise ValueError("block calibration block_count is invalid")
        if type(transaction_count) is not int or transaction_count < 0:
            raise ValueError("block calibration transaction_count is invalid")
        if (
            not isinstance(final_state_root, str)
            or len(final_state_root) != 66
            or not final_state_root.startswith("0x")
            or any(char not in "0123456789abcdef" for char in final_state_root[2:])
        ):
            raise ValueError("block calibration final state root is invalid")
        raw = _strict_nonnegative_int_map(
            item.get("expected_raw_gas_by_key"), label="block calibration raw-gas map"
        )
        if any(not key.startswith("opcode:0x") for key in raw):
            raise ValueError("block calibration raw-gas map contains precompile or spawned work")
        features = _strict_nonnegative_int_map(
            item.get("expected_features"), label="block calibration feature map"
        )
        if set(features) != set(Q_FORMULA) or features["proposal_startup"] != 1:
            raise ValueError("block calibration feature map differs from frozen Q_formula")
        diagnostics = _strict_nonnegative_int_map(
            item.get("expected_diagnostics"), label="block calibration diagnostic map"
        )
        if set(diagnostics) != set(BLOCK_CALIBRATION_DIAGNOSTICS):
            raise ValueError("block calibration diagnostic map is incomplete")
        semantics = {
            "workload_family": family,
            "split": split,
            "block_count": block_count,
            "transaction_count": transaction_count,
            "program": dict(program_data),
            "expected_final_state_root": final_state_root,
            "expected_raw_gas_by_key": raw,
            "expected_features": features,
            "expected_diagnostics": diagnostics,
        }
        if sha256_bytes(canonical_json(semantics)) != row_id:
            raise ValueError("block calibration row ID differs from semantic content")
        seen.add(row_id)
        rows.append(ControlledBlockRowSpec(
            row_id=row_id,
            workload_family=family,
            split=split,
            block_count=block_count,
            transaction_count=transaction_count,
            program=program,
            expected_final_state_root=final_state_root,
            expected_raw_gas_by_key=MappingProxyType(raw),
            expected_features=MappingProxyType(features),
            expected_diagnostics=MappingProxyType(diagnostics),
        ))
    return tuple(rows)


def parse_controlled_manifest(
    data: Mapping[str, Any], *, schedule_keys: set[str]
) -> Manifest:
    """Parse and validate the frozen V1 controlled-calibration contract."""
    if data.get("include_uzen_pure_opcodes") or data.get("include_uzen_precompile_bodies"):
        raise ValueError("controlled manifest must not use implicit include flags")
    cases: list[CaseSpec] = []
    for item in data.get("cases", []):
        if not isinstance(item, Mapping):
            raise ValueError("controlled case must be an object")
        cases.append(
            CaseSpec(
                name=item["name"],
                scenario=item.get("scenario", ""),
                template=item["template"],
                target_raw_gas=int(item["target_raw_gas"]) if "target_raw_gas" in item else 0,
                kind=item.get("kind", "opcode"),
                opcode=parse_opcode(item["opcode"]) if "opcode" in item else None,
                address=parse_opcode(item["address"]) if "address" in item else None,
                input_size=int(item["input_size"]) if "input_size" in item else None,
                execution_basis=item.get("execution_basis"),
                spawned=item.get("spawned"),
                dispatch_status=item.get("dispatch_status"),
                paired=bool(item.get("paired", False)),
                expected_output_size=(
                    int(item["expected_output_size"])
                    if "expected_output_size" in item
                    else None
                ),
            )
        )
    case_by_id = {case.name: case for case in cases}
    if len(case_by_id) != len(cases):
        raise ValueError("duplicate controlled case ID")

    measurement_keys: list[MeasurementKeySpec] = []
    for item in data.get("measurement_keys", []):
        if not isinstance(item, Mapping):
            raise ValueError("measurement key must be an object")
        required, diagnostic = _unique_case_ids(
            item.get("required_case_ids"),
            item.get("diagnostic_case_ids", []),
            label=f"measurement key {item.get('id')}",
            required_nonempty=True,
        )
        production_key = item.get("production_schedule_key")
        component, identifier = _parse_schedule_key(production_key)
        if production_key not in schedule_keys:
            raise ValueError(f"unknown production schedule key: {production_key}")
        event_match = _parse_event_match(item.get("event_match"))
        event_identifier = event_match.opcode if component == "opcode" else event_match.address
        if component != event_match.component or identifier != event_identifier:
            raise ValueError("production schedule key differs from event identity")
        basis = item.get("pricing_basis")
        if basis not in {"raw_gas_slope", "fixed_per_event"}:
            raise ValueError("missing or unknown pricing basis")
        if event_match.spawned is True and basis != "fixed_per_event":
            raise ValueError("raw_gas_slope is invalid on a spawned key")
        if event_match.spawned is not True and basis != "raw_gas_slope":
            raise ValueError("fixed_per_event is invalid for an ordinary opcode or precompile")
        measurement_keys.append(
            MeasurementKeySpec(
                id=item["id"],
                production_schedule_key=production_key,
                event_match=event_match,
                pricing_basis=basis,
                required_case_ids=required,
                diagnostic_case_ids=diagnostic,
            )
        )
    if len({item.id for item in measurement_keys}) != len(measurement_keys):
        raise ValueError("duplicate measurement key ID")
    for index, left in enumerate(measurement_keys):
        for right in measurement_keys[index + 1 :]:
            if _event_matches_overlap(left.event_match, right.event_match):
                raise ValueError(f"overlapping event matches: {left.id}, {right.id}")

    claimed_cases: set[str] = set()
    for key in measurement_keys:
        component, identifier = _parse_schedule_key(key.production_schedule_key)
        for case_id in (*key.required_case_ids, *key.diagnostic_case_ids):
            case = case_by_id.get(case_id)
            if case is None:
                raise ValueError(f"unknown case {case_id} in measurement key {key.id}")
            if case_id in claimed_cases:
                raise ValueError(f"duplicate case assignment: {case_id}")
            claimed_cases.add(case_id)
            if case.kind != component:
                raise ValueError(f"case component differs for {case_id}")
            if component == "opcode" and case.opcode != identifier:
                raise ValueError(f"case opcode differs for {case_id}")
            if component == "precompile" and case.address != identifier:
                raise ValueError(f"case precompile address differs for {case_id}")
            expected_execution_basis = (
                "fixed_per_event"
                if key.pricing_basis == "fixed_per_event"
                else "interpreter_raw_gas"
                if component == "opcode"
                else "native_gas"
            )
            if case.execution_basis != expected_execution_basis:
                raise ValueError(f"execution basis differs for {case_id}")
            if component == "opcode" and identifier in SPAWN_WRAPPER_OPCODES:
                if case.spawned is None or case.spawned != key.event_match.spawned:
                    raise ValueError(f"CALL/CREATE spawned context differs for {case_id}")
                if case.spawned is True:
                    if case.dispatch_status != "confirmed":
                        raise ValueError(f"spawned case {case_id} requires dispatch_status=confirmed")
                    if case.target_raw_gas:
                        raise ValueError("fixed-event case cannot contain a raw-gas field")
            elif case.spawned is not None or case.dispatch_status is not None:
                raise ValueError("spawned context is supported only for CALL/CREATE cases")
            if component == "precompile" and (
                not case.paired
                or case.expected_output_size is None
                or case.expected_output_size < 0
            ):
                raise ValueError(
                    f"precompile case {case_id} requires paired target/control output shape"
                )

    overheads: list[OverheadKeySpec] = []
    valid_units = {
        "proposal",
        "block",
        "started_non_anchor_transaction",
        "native_value_transfer",
        "witness_byte",
        "witness_node",
        "stdin_byte",
        "blob_byte",
        "kzg_invocation",
        "unique_state_access",
        "dirty_state_entry",
    }
    for item in data.get("overhead_keys", []):
        if item.get("unit") not in valid_units:
            raise ValueError(f"unknown overhead unit for {item.get('id')}")
        if item.get("formula_role") not in {"required", "diagnostic"}:
            raise ValueError(f"unknown formula_role for {item.get('id')}")
        required, diagnostic = _unique_case_ids(
            item.get("required_case_ids", []),
            item.get("diagnostic_case_ids", []),
            label=f"overhead key {item.get('id')}",
            required_nonempty=item.get("formula_role") == "required",
        )
        ratios = {
            key: _parse_positive_ratio(value, label=f"overhead {item.get('id')}")
            for key, value in item.get("bundled_ratios", {}).items()
        }
        overheads.append(
            OverheadKeySpec(
                id=item["id"],
                unit=item["unit"],
                formula_role=item["formula_role"],
                subtract_keys=tuple(item.get("subtract_keys", [])),
                bundled_keys=tuple(item.get("bundled_keys", [])),
                bundled_ratios=MappingProxyType(ratios),
                required_case_ids=required,
                diagnostic_case_ids=diagnostic,
            )
        )
    if len({item.id for item in overheads}) != len(overheads):
        raise ValueError("duplicate overhead key ID")
    overhead_tuple = tuple(overheads)
    subtract_closure = _subtract_closures(overhead_tuple)
    required_overheads = tuple(item.id for item in overheads if item.formula_role == "required")
    if set(required_overheads) != set(Q_FORMULA) or len(required_overheads) != len(Q_FORMULA):
        raise ValueError("V1 manifest requires exactly the four frozen overhead identities")
    q_formula = tuple(data.get("q_formula", []))
    if q_formula != tuple(Q_FORMULA):
        raise ValueError("Q_formula must equal the four frozen identities in canonical order")
    by_overhead = {item.id: item for item in overheads}
    for item in overheads:
        if item.formula_role == "required":
            for child in subtract_closure[item.id]:
                if by_overhead[child].formula_role != "required" or child not in q_formula:
                    raise ValueError("required overhead has a non-required transitive dependency")

    overhead_cases: list[OverheadCaseSpec] = []
    overhead_case_names: set[str] = set()
    for item in data.get("overhead_cases", []):
        name = item["name"]
        if name in overhead_case_names:
            raise ValueError(f"duplicate overhead case {name}")
        overhead_case_names.add(name)
        key_id = item["overhead_key_id"]
        if key_id not in by_overhead:
            raise ValueError(f"unknown overhead key {key_id} for case {name}")
        expected = tuple(item.get("expected_changed_feature_keys", []))
        allowed = {
            key_id,
            *subtract_closure[key_id],
            *by_overhead[key_id].bundled_keys,
        }
        if (
            key_id not in expected
            or not set(expected).issubset(allowed)
            or len(expected) != len(set(expected))
        ):
            raise ValueError(f"changed feature declaration differs for overhead case {name}")
        overhead_cases.append(
            OverheadCaseSpec(
                name=name,
                overhead_key_id=key_id,
                target_template=item["target_template"],
                control_template=item["control_template"],
                expected_changed_feature_keys=expected,
            )
        )
    case_owner: dict[str, str] = {}
    for key in overheads:
        for case_id in (*key.required_case_ids, *key.diagnostic_case_ids):
            if case_id not in overhead_case_names:
                raise ValueError(f"unknown overhead case {case_id}")
            if case_id in case_owner:
                raise ValueError(f"duplicate overhead case assignment {case_id}")
            case_owner[case_id] = key.id
    for case in overhead_cases:
        if case_owner.get(case.name) != case.overhead_key_id:
            raise ValueError(f"overhead case {case.name} is not owned by its overhead key")

    system_operation_ownership = data.get("system_operation_ownership")
    if system_operation_ownership != "block_base":
        raise ValueError("V1 system operation ownership must be block_base")
    anchor_operation_ownership = data.get("anchor_operation_ownership")
    if anchor_operation_ownership != "block_base":
        raise ValueError("V1 anchor operation ownership must be block_base")
    normalization = data.get("normalization_reference_key")
    if normalization != "opcode:0x01" or normalization not in {item.id for item in measurement_keys}:
        raise ValueError("normalization reference must be the ADD measurement key opcode:0x01")
    bridge_keys = data.get("bridge_key_ids")
    if not isinstance(bridge_keys, list) or not bridge_keys:
        raise ValueError("bridge_key_ids must be nonempty")
    if len(set(bridge_keys)) != len(bridge_keys):
        raise ValueError("duplicate bridge key ID")
    allowed_bridge = {item.id for item in measurement_keys} | set(Q_FORMULA)
    diagnostic_overheads = {item.id for item in overheads if item.formula_role == "diagnostic"}
    for key in bridge_keys:
        if key in diagnostic_overheads:
            raise ValueError("diagnostic-only overhead key cannot enter bridge_key_ids")
        if key not in allowed_bridge:
            raise ValueError(f"unknown bridge key: {key}")
    if normalization not in bridge_keys or any(key not in bridge_keys for key in Q_FORMULA):
        raise ValueError("bridge_key_ids must include normalization and every Q_formula key")
    other_opcodes = {
        item.id
        for item in measurement_keys
        if item.event_match.component == "opcode" and item.id != normalization
    }
    precompiles = {
        item.id for item in measurement_keys if item.event_match.component == "precompile"
    }
    if not set(bridge_keys) & other_opcodes:
        raise ValueError("bridge_key_ids must include an additional opcode")
    if not set(bridge_keys) & precompiles:
        raise ValueError("bridge_key_ids must include a precompile")
    if data.get("bridge_model") != "through_origin_equal_key_median":
        raise ValueError("bridge model must be through_origin_equal_key_median")
    try:
        controlled_threshold = Decimal(str(data.get("bridge_controlled_max_ape")))
        proposal_threshold = Decimal(str(data.get("bridge_proposal_max_ape")))
    except InvalidOperation as exc:
        raise ValueError("bridge thresholds must equal exactly 0.10") from exc
    if controlled_threshold != Decimal("0.10") or proposal_threshold != Decimal("0.10"):
        raise ValueError("bridge thresholds must equal exactly 0.10")

    relation_anchors, dynamic_raw_gas_keys, opcode_relations = (
        _parse_opcode_relations(data, cases)
    )
    block_calibration_rows = _parse_block_calibration_rows(data)

    return Manifest(
        name=str(data["name"]),
        backend=str(data.get("backend", "sp1")),
        variants=[int(value) for value in data.get("variants", [])],
        cases=cases,
        measurement_keys=tuple(measurement_keys),
        normalization_reference_key=normalization,
        system_operation_ownership=system_operation_ownership,
        anchor_operation_ownership=anchor_operation_ownership,
        bridge_key_ids=tuple(bridge_keys),
        bridge_model=data["bridge_model"],
        bridge_controlled_max_ape=controlled_threshold,
        bridge_proposal_max_ape=proposal_threshold,
        overhead_keys=overhead_tuple,
        overhead_cases=tuple(overhead_cases),
        q_formula=q_formula,
        subtract_closure=MappingProxyType(subtract_closure),
        opcode_relation_anchors=relation_anchors,
        dynamic_raw_gas_keys=dynamic_raw_gas_keys,
        opcode_relations=opcode_relations,
        block_calibration_rows=block_calibration_rows,
    )


def load_manifest(path: pathlib.Path, schedule: UnzenSchedule | None = None) -> Manifest:
    data = tomllib.loads(path.read_text())
    if "measurement_keys" in data or "overhead_keys" in data:
        if schedule is None:
            schedule = current_uzen_schedule()
        schedule_keys = {
            *(
                _canonical_schedule_key("opcode", opcode)
                for opcode in schedule.opcode_multipliers
            ),
            *(
                _canonical_schedule_key("precompile", address)
                for address in schedule.precompile_multipliers
            ),
        }
        return parse_controlled_manifest(data, schedule_keys=schedule_keys)
    cases = [
        CaseSpec(
            name=item["name"],
            scenario=item["scenario"],
            template=item["template"],
            target_raw_gas=int(item["target_raw_gas"]),
            kind=item.get("kind", "opcode"),
            opcode=parse_opcode(item["opcode"]) if "opcode" in item else None,
            address=parse_opcode(item["address"]) if "address" in item else None,
            input_size=int(item["input_size"]) if "input_size" in item else None,
        )
        for item in data.get("cases", [])
    ]
    include_uzen_pure_opcodes = bool(data.get("include_uzen_pure_opcodes", False))
    include_uzen_precompile_bodies = bool(data.get("include_uzen_precompile_bodies", False))
    if schedule is None and (include_uzen_pure_opcodes or include_uzen_precompile_bodies):
        schedule = current_uzen_schedule()
    cases = expand_manifest_cases(
        cases,
        schedule=schedule,
        include_uzen_pure_opcodes=include_uzen_pure_opcodes,
        include_uzen_precompile_bodies=include_uzen_precompile_bodies,
    )
    return Manifest(
        name=data["name"],
        backend=data.get("backend", "sp1"),
        variants=[int(value) for value in data.get("variants", [])],
        cases=cases,
    )


def expand_manifest_cases(
    cases: list[CaseSpec],
    *,
    schedule: UnzenSchedule | None,
    include_uzen_pure_opcodes: bool,
    include_uzen_precompile_bodies: bool,
) -> list[CaseSpec]:
    expanded = list(cases)
    measured_opcodes = {
        case.opcode for case in expanded if case.kind == "opcode" and case.opcode is not None
    }
    measured_precompiles = {
        case.address for case in expanded if case.kind == "precompile" and case.address is not None
    }

    if include_uzen_pure_opcodes:
        if schedule is None:
            raise ValueError("Unzen schedule is required to expand opcode cases")
        active_pure_opcodes = set(schedule.opcode_multipliers) & PLANNED_PURE_OPCODE_OPCODES
        for opcode in sorted(active_pure_opcodes - measured_opcodes):
            expanded.append(default_opcode_case(opcode))
    if include_uzen_precompile_bodies:
        if schedule is None:
            raise ValueError("Unzen schedule is required to expand precompile cases")
        active_precompiles = set(schedule.precompile_multipliers) & set(PRECOMPILE_BODY_DEFAULTS)
        for address in sorted(active_precompiles - measured_precompiles):
            expanded.append(default_precompile_case(address))
    return expanded


def default_opcode_case(opcode: int) -> CaseSpec:
    name = UZEN_OPCODE_NAMES.get(opcode, f"opcode_0x{opcode:02x}")
    try:
        scenario, template, target_raw_gas = PURE_OPCODE_DEFAULTS[opcode]
    except KeyError as exc:
        raise ValueError(f"no pure opcode default for 0x{opcode:02x} {name}") from exc
    return CaseSpec(
        name=name,
        opcode=opcode,
        scenario=scenario,
        template=template,
        target_raw_gas=target_raw_gas,
    )


def default_precompile_case(address: int) -> CaseSpec:
    name = UZEN_PRECOMPILE_NAMES.get(address, f"precompile_0x{address:x}")
    try:
        template, input_size, target_raw_gas = PRECOMPILE_BODY_DEFAULTS[address]
    except KeyError as exc:
        raise ValueError(f"no precompile body default for 0x{address:02x} {name}") from exc
    return CaseSpec(
        name=name,
        kind="precompile",
        address=address,
        scenario="precompile",
        template=template,
        input_size=input_size,
        target_raw_gas=target_raw_gas,
    )


def parse_opcode(value: int | str) -> int:
    if isinstance(value, int):
        return value
    return int(value, 16 if value.lower().startswith("0x") else 10)


def build_bytecode(case: CaseSpec, target_count: int) -> GeneratedBytecode:
    if target_count < 0:
        raise ValueError("target_count must be non-negative")
    if case.opcode is None:
        raise ValueError(f"opcode case {case.name} is missing opcode")
    if case.template == "stack_binary":
        bytecode = build_stack_binary_bytecode(case.opcode, target_count)
    elif case.template == "stack_unary":
        bytecode = build_stack_unary_bytecode(case.opcode, target_count)
    elif case.template == "stack_ternary":
        bytecode = build_stack_ternary_bytecode(case.opcode, target_count)
    elif case.template == "stack_exp":
        bytecode = build_stack_exp_bytecode(target_count)
    elif case.template == "keccak_32":
        bytecode = build_keccak_32_bytecode(target_count)
    elif case.template == "memory_load_32":
        bytecode = build_memory_load_32_bytecode(target_count)
    elif case.template == "memory_store_32":
        bytecode = build_memory_store_32_bytecode(target_count)
    elif case.template == "memory_store8":
        bytecode = build_memory_store8_bytecode(target_count)
    elif case.template == "stack_pop":
        bytecode = build_stack_pop_bytecode(target_count)
    elif case.template == "stack_swap1":
        bytecode = build_stack_swap1_bytecode(target_count)
    elif case.template == "stack_push":
        bytecode = build_stack_push_bytecode(case.opcode, target_count)
    elif case.template == "stack_dup":
        bytecode = build_stack_dup_bytecode(case.opcode, target_count)
    elif case.template == "stack_swap":
        bytecode = build_stack_swap_bytecode(case.opcode, target_count)
    elif case.template == "jump_chain":
        bytecode = build_jump_chain_bytecode(target_count)
    elif case.template == "jumpi_chain":
        bytecode = build_jumpi_chain_bytecode(target_count)
    elif case.template == "stack_unary_producer":
        bytecode = build_stack_unary_producer_bytecode(case.opcode, target_count)
    elif case.template == "jumpdest_chain":
        bytecode = build_jumpdest_chain_bytecode(target_count)
    elif case.template == "memory_copy_32":
        bytecode = build_memory_copy_32_bytecode(target_count)
    else:
        raise ValueError(f"unknown template: {case.template}")
    return GeneratedBytecode(bytes_hex=bytecode.hex(), opcode_counts=count_opcodes(bytecode))


def _fixed_push(value: int, *, target_opcode: int) -> bytes:
    opcode = 0x7E if target_opcode == 0x7F else 0x7F
    size = opcode - 0x5F
    return bytes([opcode]) + value.to_bytes(size, "big")


def _fixed_target_instruction(case: CaseSpec) -> bytes:
    if case.opcode is None:
        raise ValueError(f"opcode case {case.name} is missing opcode")
    if 0x60 <= case.opcode <= 0x7F:
        return bytes([case.opcode]) + bytes(case.opcode - 0x5F)
    return bytes([case.opcode])


def _fixed_memory_warmup(case: CaseSpec) -> bytes:
    assert case.opcode is not None
    return b"".join(
        _fixed_push(value, target_opcode=case.opcode) for value in (32, 0, 0)
    ) + bytes([0x37])


FIXED_MICROPROGRAM_MAGIC = bytes([0xEF, 0x4D, 0x50, 0x01])
MATCHED_CONTROL_PURPOSE = "matched_control_diagnostic"
MATCHED_CONTROL_OPERAND_PROFILES = {
    "zero": {
        "stack_binary": (0, 0),
        "stack_exp": (2, 2),
        "stack_unary": (1,),
    },
    "small_nonzero": {
        "stack_binary": (7, 3),
        "stack_exp": (3, 5),
        "stack_unary": (7,),
    },
}
POP_OPCODE = 0x50
POP_RAW_GAS = 2
NOT_OPCODE = 0x19
NOT_RAW_GAS = 3
DUP1_OPCODE = 0x80
DUP1_RAW_GAS = 3
SWAP1_OPCODE = 0x90
SWAP1_RAW_GAS = 3
PUSH0_OPCODE = 0x5F
PUSH0_RAW_GAS = 2


@dataclass(frozen=True)
class MatchedControlSpec:
    reference_opcode: int
    reference_raw_gas: int
    relation: str
    final_stack_height: int
    operands: tuple[int, ...]
    setup: bytes
    target_program: bytes = b""
    reference_program: bytes = b""
    reference_opcode_counts: tuple[tuple[int, int], ...] = ()
    reference_raw_gas_total: int = 0
    target_pre_suffix_padding: bytes = b""
    control_pre_suffix_padding: bytes = b""
    common_suffix: bytes = b""
    target_post_suffix_padding: bytes = b""
    control_post_suffix_padding: bytes = b""

    @property
    def compound(self) -> bool:
        return bool(self.target_program)


MATCHED_CONTROL_COMPOUND_FIELDS = (
    "target_program",
    "reference_program",
    "reference_opcode_counts",
    "reference_raw_gas_total",
    "target_pre_suffix_padding",
    "control_pre_suffix_padding",
    "common_suffix",
    "target_post_suffix_padding",
    "control_post_suffix_padding",
)


def encode_fixed_microprograms(programs: Iterable[bytes]) -> bytes:
    rows = list(programs)
    out = bytearray(FIXED_MICROPROGRAM_MAGIC)
    out.extend(len(rows).to_bytes(4, "big"))
    for program in rows:
        out.extend(len(program).to_bytes(4, "big"))
        out.extend(program)
    return bytes(out)


def decode_fixed_microprograms(encoded: bytes) -> list[bytes]:
    if not encoded.startswith(FIXED_MICROPROGRAM_MAGIC):
        return [encoded]
    if len(encoded) < 8:
        raise ValueError("truncated fixed-microprogram header")
    count = int.from_bytes(encoded[4:8], "big")
    cursor = 8
    programs = []
    for _ in range(count):
        if cursor + 4 > len(encoded):
            raise ValueError("truncated fixed-microprogram length")
        length = int.from_bytes(encoded[cursor : cursor + 4], "big")
        cursor += 4
        if cursor + length > len(encoded):
            raise ValueError("truncated fixed microprogram")
        programs.append(encoded[cursor : cursor + length])
        cursor += length
    if cursor != len(encoded) or not programs:
        raise ValueError("invalid fixed-microprogram framing")
    return programs


def _fixed_slot(case: CaseSpec, *, active: bool) -> bytes:
    assert case.opcode is not None
    setup = bytearray()

    def push(value: int = 0) -> None:
        setup.extend(_fixed_push(value, target_opcode=case.opcode or 0))

    if case.template == "stack_binary":
        push()
        push()
    elif case.template == "stack_exp":
        push(2)
        push(2)
    elif case.template == "stack_ternary":
        push()
        push()
        push()
    elif case.template == "stack_unary":
        push(1)
    elif case.template == "keccak_32":
        setup.extend(_fixed_memory_warmup(case))
        push(32)
        push()
    elif case.template == "memory_load_32":
        setup.extend(_fixed_memory_warmup(case))
        push()
    elif case.template in {"memory_store_32", "memory_store8"}:
        setup.extend(_fixed_memory_warmup(case))
        push(1)
        push()
    elif case.template == "stack_pop":
        push(1)
    elif case.template == "stack_push":
        pass
    elif case.template == "stack_dup":
        for value in range(1, case.opcode - 0x7F + 1):
            push(value)
    elif case.template == "stack_swap":
        for value in range(1, case.opcode - 0x8F + 2):
            push(value)
    elif case.template in {"stack_unary_producer", "jumpdest_chain"}:
        pass
    elif case.template == "memory_copy_32":
        setup.extend(_fixed_memory_warmup(case))
        for value in (32, 0, 0):
            push(value)
    elif case.template in {"jump_chain", "jumpi_chain"}:
        pushes = 2 if case.template == "jumpi_chain" else 1
        setup_len = pushes * 33
        destination = setup_len + 1
        if case.template == "jumpi_chain":
            push(1)
        push(destination)
        if active:
            return bytes(setup) + bytes([case.opcode, 0x5B, 0x00])
        return bytes(setup) + bytes([0x5B, 0x00, case.opcode])
    else:
        raise ValueError(f"no fixed-footprint construction for template: {case.template}")

    instruction = _fixed_target_instruction(case)
    return (
        bytes(setup) + instruction + bytes([0x00])
        if active
        else bytes(setup) + bytes([0x00]) + instruction
    )


def build_fixed_footprint_bytecode(
    case: CaseSpec, target_count: int, generator_max_count: int
) -> GeneratedBytecode:
    """Build one fixed-size sweep member whose executed helper work is count-invariant."""
    if target_count < 0 or generator_max_count < 0 or target_count > generator_max_count:
        raise ValueError("target_count exceeds generator_max_count")
    if case.opcode is None:
        raise ValueError(f"opcode case {case.name} is missing opcode")
    programs = [
        _fixed_slot(case, active=index < target_count)
        for index in range(generator_max_count)
    ]
    if len({len(program) for program in programs}) > 1:
        raise AssertionError("fixed microprogram slots must have one byte length")
    encoded = encode_fixed_microprograms(programs)
    counts: dict[int, int] = {}
    for program in programs:
        for opcode, count in count_opcodes(program).items():
            counts[opcode] = counts.get(opcode, 0) + count
    return GeneratedBytecode(bytes_hex=encoded.hex(), opcode_counts=counts)


def _program_raw_gas(program: bytes) -> int:
    total = 0
    for opcode, count in count_opcodes(program).items():
        if opcode == 0x00:
            raw_gas = 0
        else:
            try:
                raw_gas = PURE_OPCODE_DEFAULTS[opcode][2]
            except KeyError as exc:
                raise ValueError(
                    f"matched control reference opcode 0x{opcode:02x} has no raw gas"
                ) from exc
        total += raw_gas * count
    return total


def _compound_matched_control_spec(case: CaseSpec) -> MatchedControlSpec | None:
    assert case.opcode is not None
    target_program = _fixed_target_instruction(case)
    target_pre_suffix_padding = b""
    control_pre_suffix_padding = b""
    common_suffix = b"\x00"

    if case.template == "stack_ternary":
        operands = (0, 0, 0)
        reference_program = b"\x50\x01"
        reference_opcode = POP_OPCODE
        relation = "OP-(POP+ADD)"
        final_stack_height = 1
    elif case.template == "stack_pop":
        operands = (1,)
        reference_program = b"\x19\x50"
        reference_opcode = NOT_OPCODE
        relation = "POP-(NOT+POP)"
        final_stack_height = 0
    elif case.template in {"memory_store_32", "memory_store8"}:
        operands = (1, 0)
        reference_program = b"\x01\x50"
        reference_opcode = 0x01
        relation = "OP-(ADD+POP)"
        final_stack_height = 0
    elif case.template == "memory_copy_32":
        operands = (32, 0, 0)
        reference_program = b"\x01\x02\x50"
        reference_opcode = 0x01
        relation = "MCOPY-(ADD+MUL+POP)"
        final_stack_height = 0
    elif case.template == "stack_push" and case.opcode != PUSH0_OPCODE:
        operands = ()
        reference_program = bytes([PUSH0_OPCODE])
        reference_opcode = PUSH0_OPCODE
        relation = "OP-PUSH0"
        final_stack_height = 1
    elif case.template == "jump_chain":
        reference_program = bytes([POP_OPCODE])
        reference_opcode = POP_OPCODE
        relation = "JUMP-POP"
        final_stack_height = 0
        destination = len(_fixed_push(0, target_opcode=case.opcode)) + len(target_program)
        operands = (destination,)
        setup = _fixed_push(destination, target_opcode=case.opcode)
        common_suffix = b"\x5b\x00"
    elif case.template == "jumpi_chain":
        reference_program = b"\x01\x50"
        reference_opcode = 0x01
        relation = "JUMPI-(ADD+POP)"
        final_stack_height = 0
        empty_push = _fixed_push(0, target_opcode=case.opcode)
        destination = 2 * len(empty_push) + 2
        operands = (1, destination)
        setup = _fixed_push(1, target_opcode=case.opcode) + _fixed_push(
            destination, target_opcode=case.opcode
        )
        target_pre_suffix_padding = b"\x00"
        common_suffix = b"\x5b\x00"
    elif case.template == "jumpdest_chain":
        operands = ()
        reference_program = b"\x5f\x50"
        reference_opcode = PUSH0_OPCODE
        relation = "JUMPDEST-(PUSH0+POP)"
        final_stack_height = 0
    else:
        return None

    if case.template not in {"jump_chain", "jumpi_chain"}:
        active = _fixed_slot(case, active=True)
        if not active.endswith(target_program + b"\x00"):
            raise AssertionError("matched-control target slot has an unexpected suffix")
        setup = active[: -len(target_program) - 1]

    reference_counts = tuple(sorted(count_opcodes(reference_program).items()))
    reference_raw_gas_total = _program_raw_gas(reference_program)
    reference_raw_gas = PURE_OPCODE_DEFAULTS[reference_opcode][2]
    target_post_suffix_padding = reference_program
    control_post_suffix_padding = target_program + target_pre_suffix_padding
    return MatchedControlSpec(
        reference_opcode=reference_opcode,
        reference_raw_gas=reference_raw_gas,
        relation=relation,
        final_stack_height=final_stack_height,
        operands=operands,
        setup=setup,
        target_program=target_program,
        reference_program=reference_program,
        reference_opcode_counts=reference_counts,
        reference_raw_gas_total=reference_raw_gas_total,
        target_pre_suffix_padding=target_pre_suffix_padding,
        control_pre_suffix_padding=control_pre_suffix_padding,
        common_suffix=common_suffix,
        target_post_suffix_padding=target_post_suffix_padding,
        control_post_suffix_padding=control_post_suffix_padding,
    )


def matched_control_spec(
    case: CaseSpec, operand_profile: str = "zero"
) -> MatchedControlSpec:
    """Derive the complete contextual matched-control contract for one case."""
    if case.opcode is None:
        raise ValueError(f"opcode case {case.name} is missing opcode")
    canonical = PURE_OPCODE_DEFAULTS.get(case.opcode)
    if canonical is None or canonical[1] != case.template:
        canonical_template = canonical[1] if canonical is not None else "unsupported"
        raise ValueError(
            "matched control canonical opcode/template mismatch: "
            f"0x{case.opcode:02x} maps to {canonical_template}, not {case.template}"
        )
    if operand_profile not in MATCHED_CONTROL_OPERAND_PROFILES:
        raise ValueError(
            f"matched control operand profile {operand_profile!r} is unsupported"
        )

    compound = _compound_matched_control_spec(case)
    if compound is not None:
        if operand_profile != "zero":
            raise ValueError(
                f"matched control template {case.template} only supports operand profile zero"
            )
        return compound

    if case.template in {"stack_binary", "stack_exp", "stack_unary"}:
        operands = MATCHED_CONTROL_OPERAND_PROFILES[operand_profile][case.template]
        if case.template == "stack_unary":
            reference_opcode = NOT_OPCODE
            reference_raw_gas = NOT_RAW_GAS
            relation = "OP-NOT"
        else:
            reference_opcode = POP_OPCODE
            reference_raw_gas = POP_RAW_GAS
            relation = "OP-POP"
        final_stack_height = 1
        setup = b"".join(
            _fixed_push(value, target_opcode=case.opcode) for value in operands
        )
    else:
        if case.template == "stack_push" and case.opcode != PUSH0_OPCODE:
            raise ValueError(
                f"matched control template {case.template} is unsupported for "
                f"opcode 0x{case.opcode:02x}"
            )
        if case.template not in {
            "memory_load_32",
            "keccak_32",
            "stack_dup",
            "stack_swap",
            "stack_unary_producer",
            "stack_push",
        }:
            raise ValueError(f"matched control template {case.template} is unsupported")
        if operand_profile != "zero":
            raise ValueError(
                f"matched control template {case.template} only supports operand profile zero"
            )
        if case.template == "memory_load_32":
            operands = (0,)
            reference_opcode = NOT_OPCODE
            reference_raw_gas = NOT_RAW_GAS
            relation = "OP-NOT"
            final_stack_height = 1
        elif case.template == "keccak_32":
            operands = (32, 0)
            reference_opcode = POP_OPCODE
            reference_raw_gas = POP_RAW_GAS
            relation = "OP-POP"
            final_stack_height = 1
        elif case.template == "stack_dup":
            depth = case.opcode - 0x7F
            if not 1 <= depth <= 16:
                raise ValueError(f"matched control DUP opcode 0x{case.opcode:02x} is invalid")
            operands = tuple(range(1, depth + 1))
            reference_opcode = DUP1_OPCODE
            reference_raw_gas = DUP1_RAW_GAS
            relation = "OP-DUP1"
            final_stack_height = depth + 1
        elif case.template == "stack_swap":
            depth = case.opcode - 0x8F
            if not 1 <= depth <= 16:
                raise ValueError(f"matched control SWAP opcode 0x{case.opcode:02x} is invalid")
            operands = tuple(range(1, depth + 2))
            reference_opcode = SWAP1_OPCODE
            reference_raw_gas = SWAP1_RAW_GAS
            relation = "OP-SWAP1"
            final_stack_height = depth + 1
        elif case.template == "stack_unary_producer":
            if case.opcode not in {0x58, 0x59, 0x5A}:
                raise ValueError(
                    f"matched control unary producer opcode 0x{case.opcode:02x} is unsupported"
                )
            operands = ()
            reference_opcode = PUSH0_OPCODE
            reference_raw_gas = PUSH0_RAW_GAS
            relation = "OP-PUSH0"
            final_stack_height = 1
        else:
            operands = ()
            reference_opcode = PUSH0_OPCODE
            reference_raw_gas = PUSH0_RAW_GAS
            relation = "OP-PUSH0"
            final_stack_height = 1

        active = _fixed_slot(case, active=True)
        instruction = _fixed_target_instruction(case)
        if not active.endswith(instruction + b"\x00"):
            raise AssertionError("matched-control target slot has an unexpected suffix")
        setup = active[: -len(instruction) - 1]
    return MatchedControlSpec(
        reference_opcode=reference_opcode,
        reference_raw_gas=reference_raw_gas,
        relation=relation,
        final_stack_height=final_stack_height,
        operands=operands,
        setup=setup,
    )


def _memory_cost(words: int) -> int:
    if words < 0:
        raise ValueError("memory word count must be nonnegative")
    return 3 * words + words * words // 512


def _dynamic_relation_target_raw_gas(
    case: CaseSpec, scenario: Mapping[str, Any]
) -> int:
    """Return the frozen actual target-op raw gas for a formal scenario."""
    initial_words = scenario.get("initial_memory_words", 0)
    if isinstance(initial_words, bool) or not isinstance(initial_words, int) or initial_words < 0:
        raise ValueError("dynamic relation initial_memory_words must be nonnegative")
    if case.template == "stack_exp":
        byte_length = scenario.get("exponent_byte_length")
        if byte_length not in {1, 8, 32}:
            raise ValueError("EXP relation requires exponent byte length 1, 8, or 32")
        return 10 + 50 * int(byte_length)
    if case.template == "keccak_32":
        length = scenario.get("input_length")
        if length not in {32, 256, 1024}:
            raise ValueError("KECCAK256 relation requires input length 32, 256, or 1024")
        words = (int(length) + 31) // 32
        return 30 + 6 * words + max(0, _memory_cost(words) - _memory_cost(initial_words))
    if case.template in {"memory_load_32", "memory_store_32", "memory_store8"}:
        offset = scenario.get("highest_touched_offset")
        if offset not in {0, 0x0100, 0x1000}:
            raise ValueError("memory relation requires offset 0x00, 0x0100, or 0x1000")
        touched = int(offset) + (1 if case.template == "memory_store8" else 32)
        words = (touched + 31) // 32
        return 3 + max(0, _memory_cost(words) - _memory_cost(initial_words))
    if case.template == "memory_copy_32":
        length = scenario.get("copy_length")
        if length not in {32, 256, 1024}:
            raise ValueError("MCOPY relation requires copy length 32, 256, or 1024")
        words = (int(length) + 31) // 32
        return 3 + 3 * words + max(
            0, _memory_cost(words) - _memory_cost(initial_words)
        )
    raise ValueError(f"unmarked dynamic raw-gas template: {case.template}")


def relation_matched_control_spec(
    case: CaseSpec, scenario: Mapping[str, Any] | None = None
) -> MatchedControlSpec:
    """Reuse the matched-control contract with frozen formal dynamic operands."""
    if case.opcode == POP_OPCODE:
        return MatchedControlSpec(
            reference_opcode=POP_OPCODE,
            reference_raw_gas=POP_RAW_GAS,
            relation="POP-POP",
            final_stack_height=0,
            operands=(1,),
            setup=_fixed_push(1, target_opcode=case.opcode),
        )
    if case.opcode == NOT_OPCODE:
        reference_program = bytes([POP_OPCODE, PUSH0_OPCODE])
        return MatchedControlSpec(
            reference_opcode=POP_OPCODE,
            reference_raw_gas=POP_RAW_GAS,
            relation="NOT-(POP+PUSH0)",
            final_stack_height=1,
            operands=(1,),
            setup=_fixed_push(1, target_opcode=case.opcode),
            target_program=bytes([NOT_OPCODE]),
            reference_program=reference_program,
            reference_opcode_counts=tuple(sorted(count_opcodes(reference_program).items())),
            reference_raw_gas_total=POP_RAW_GAS + PUSH0_RAW_GAS,
            common_suffix=b"\x00",
            target_post_suffix_padding=reference_program,
            control_post_suffix_padding=bytes([NOT_OPCODE]),
        )
    base = matched_control_spec(case)
    if not scenario:
        return base
    if case.template == "stack_exp":
        byte_length = int(scenario["exponent_byte_length"])
        exponent = 1 << (8 * (byte_length - 1))
        operands = (2, exponent)
        setup = b"".join(_fixed_push(value, target_opcode=case.opcode or 0) for value in operands)
    elif case.template == "keccak_32":
        operands = (int(scenario["input_length"]), 0)
        setup = _fixed_memory_warmup(case) + b"".join(
            _fixed_push(value, target_opcode=case.opcode or 0) for value in operands
        )
    elif case.template == "memory_load_32":
        operands = (int(scenario["highest_touched_offset"]),)
        setup = _fixed_memory_warmup(case) + b"".join(
            _fixed_push(value, target_opcode=case.opcode or 0) for value in operands
        )
    elif case.template in {"memory_store_32", "memory_store8"}:
        operands = (1, int(scenario["highest_touched_offset"]))
        setup = _fixed_memory_warmup(case) + b"".join(
            _fixed_push(value, target_opcode=case.opcode or 0) for value in operands
        )
    elif case.template == "memory_copy_32":
        operands = (int(scenario["copy_length"]), 0, 0)
        setup = _fixed_memory_warmup(case) + b"".join(
            _fixed_push(value, target_opcode=case.opcode or 0) for value in operands
        )
    else:
        raise ValueError(f"unmarked dynamic raw-gas template: {case.template}")
    return replace(base, operands=operands, setup=setup)


def _opcode_relation_maps(
    case: CaseSpec, spec: MatchedControlSpec, target_raw_gas: int
) -> tuple[dict[str, int], dict[str, int], dict[str, int]]:
    assert case.opcode is not None
    target = {f"opcode:0x{case.opcode:02x}": target_raw_gas}
    if spec.compound:
        control = {
            f"opcode:0x{opcode:02x}": PURE_OPCODE_DEFAULTS[opcode][2] * count
            for opcode, count in spec.reference_opcode_counts
        }
    else:
        control = {
            f"opcode:0x{spec.reference_opcode:02x}": spec.reference_raw_gas
        }
    signed = {
        key: target.get(key, 0) - control.get(key, 0)
        for key in target.keys() | control.keys()
        if target.get(key, 0) != control.get(key, 0)
    }
    return target, control, signed


def _parse_opcode_relations(
    data: Mapping[str, Any], cases: list[CaseSpec]
) -> tuple[tuple[str, ...], tuple[str, ...], tuple[OpcodeRelationSpec, ...]]:
    anchors = tuple(data.get("opcode_relation_anchors", ()))
    if anchors != OPCODE_RELATION_ANCHORS:
        raise ValueError("opcode relation anchors must match the canonical ordered anchors")
    dynamic_keys = tuple(data.get("dynamic_raw_gas_keys", ()))
    if dynamic_keys != DYNAMIC_RAW_GAS_KEYS:
        raise ValueError("dynamic raw-gas keys must match the canonical ordered keys")

    all_opcode_cases = {
        f"opcode:0x{case.opcode:02x}": case
        for case in cases
        if case.kind == "opcode" and case.opcode is not None
    }
    dynamic_templates = {
        "stack_exp",
        "keccak_32",
        "memory_load_32",
        "memory_store_32",
        "memory_store8",
        "memory_copy_32",
    }
    for key, case in all_opcode_cases.items():
        if case.template in dynamic_templates and key not in dynamic_keys:
            raise ValueError(f"unmarked dynamic raw-gas template: {case.template}")
    opcode_cases = {
        key: case
        for key, case in all_opcode_cases.items()
        if case.opcode in PURE_OPCODE_DEFAULTS
        and PURE_OPCODE_DEFAULTS[case.opcode][1] == case.template
    }

    raw_scenarios = data.get("opcode_relation_scenarios", [])
    if not isinstance(raw_scenarios, list):
        raise ValueError("opcode relation scenarios must be a list")
    explicit_by_key: dict[str, list[OpcodeRelationSpec]] = {}
    relation_ids: set[str] = set()
    for item in raw_scenarios:
        if not isinstance(item, Mapping):
            raise ValueError("opcode relation scenario must be an object")
        relation_id = item.get("id")
        if not isinstance(relation_id, str) or not relation_id:
            raise ValueError("opcode relation ID must be nonempty")
        if relation_id in relation_ids:
            raise ValueError(f"duplicate relation ID: {relation_id}")
        relation_ids.add(relation_id)
        key_id = item.get("key_id")
        case_id = item.get("case_id")
        case = opcode_cases.get(str(key_id))
        if case is None or case.name != case_id or key_id not in dynamic_keys:
            raise ValueError("dynamic relation key/case identity is invalid")
        split = item.get("split")
        if split not in {"canonical", "dynamic_holdout"}:
            raise ValueError("dynamic relation split is invalid")
        scenario = item.get("scenario")
        if not isinstance(scenario, Mapping):
            raise ValueError("dynamic relation scenario parameters are missing")
        scenario = MappingProxyType(dict(scenario))
        target_raw_gas = _dynamic_relation_target_raw_gas(case, scenario)
        if item.get("target_raw_gas") != target_raw_gas:
            raise ValueError("relation target raw-gas total differs from fixture contract")
        matched = relation_matched_control_spec(case, scenario)
        expected_reference = (
            matched.reference_raw_gas_total
            if matched.compound
            else matched.reference_raw_gas
        )
        if item.get("reference_raw_gas_total") != expected_reference:
            raise ValueError("relation reference raw-gas total differs from fixture contract")
        target, control, signed = _opcode_relation_maps(case, matched, target_raw_gas)
        relation = OpcodeRelationSpec(
            id=relation_id,
            case_id=case.name,
            key_id=str(key_id),
            split=str(split),
            scenario_id=str(item.get("scenario_id")),
            scenario=scenario,
            target_raw_gas_by_key=MappingProxyType(target),
            control_raw_gas_by_key=MappingProxyType(control),
            signed_raw_gas_by_key=MappingProxyType(signed),
            dynamic_key=str(key_id),
        )
        explicit_by_key.setdefault(str(key_id), []).append(relation)

    expected_dynamic = set(dynamic_keys) & set(opcode_cases)
    if set(explicit_by_key) != expected_dynamic:
        missing = sorted(expected_dynamic - set(explicit_by_key))
        raise ValueError(f"dynamic relation scenarios are missing: {missing!r}")
    for key, relations in explicit_by_key.items():
        if [relation.split for relation in relations] != [
            "canonical",
            "dynamic_holdout",
            "dynamic_holdout",
        ]:
            raise ValueError(f"dynamic relation {key} must have one canonical and two holdouts")
        if len({relation.scenario_id for relation in relations}) != 3:
            raise ValueError(f"dynamic relation {key} has duplicate scenario IDs")
        totals = [relation.target_raw_gas_by_key[key] for relation in relations]
        if not (0 < totals[0] < totals[1] < totals[2]):
            raise ValueError(f"dynamic relation {key} raw-gas totals are not distinct positive")
        if totals[1] < 2 * totals[0] or totals[2] < 4 * totals[0]:
            raise ValueError(f"dynamic relation {key} fails the 1x/2x/4x raw-gas range")

    relations: list[OpcodeRelationSpec] = []
    for key, case in opcode_cases.items():
        if key in explicit_by_key:
            relations.extend(explicit_by_key[key])
            continue
        matched = relation_matched_control_spec(case)
        target, control, signed = _opcode_relation_maps(
            case, matched, case.target_raw_gas
        )
        relation_id = f"{key}:canonical"
        if relation_id in relation_ids:
            raise ValueError(f"duplicate relation ID: {relation_id}")
        relation_ids.add(relation_id)
        relations.append(
            OpcodeRelationSpec(
                id=relation_id,
                case_id=case.name,
                key_id=key,
                split="canonical",
                scenario_id="canonical",
                scenario=MappingProxyType({}),
                target_raw_gas_by_key=MappingProxyType(target),
                control_raw_gas_by_key=MappingProxyType(control),
                signed_raw_gas_by_key=MappingProxyType(signed),
            )
        )
    return anchors, dynamic_keys, tuple(relations)


def _matched_control_slot(
    case: CaseSpec, *, execute_target: bool, operand_profile: str = "zero"
) -> bytes:
    """Build one diagnostic slot with the same setup and final stack height."""
    spec = matched_control_spec(case, operand_profile)
    if spec.compound:
        if execute_target:
            return (
                spec.setup
                + spec.target_program
                + spec.target_pre_suffix_padding
                + spec.common_suffix
                + spec.target_post_suffix_padding
            )
        return (
            spec.setup
            + spec.reference_program
            + spec.control_pre_suffix_padding
            + spec.common_suffix
            + spec.control_post_suffix_padding
        )
    opcode = case.opcode if execute_target else spec.reference_opcode
    assert opcode is not None
    return spec.setup + bytes([opcode, 0x00])


def _matched_control_executed_counts(
    spec: MatchedControlSpec, *, execute_target: bool
) -> dict[int, int]:
    if not spec.compound:
        raise AssertionError("one-op matched control has no compound count")
    program = spec.target_program if execute_target else spec.reference_program
    counts: dict[int, int] = {}
    for fragment in (spec.setup, program, spec.common_suffix):
        for opcode, count in count_opcodes(fragment).items():
            counts[opcode] = counts.get(opcode, 0) + count
    return counts


def _matched_control_declared_target_count(
    case: CaseSpec,
    spec: MatchedControlSpec,
    diagnostic_count: int,
    generator_max_count: int,
) -> int:
    assert case.opcode is not None
    if not spec.compound:
        return (
            generator_max_count
            if case.opcode == spec.reference_opcode
            else diagnostic_count
        )
    target_count = _matched_control_executed_counts(
        spec, execute_target=True
    ).get(case.opcode, 0)
    reference_count = _matched_control_executed_counts(
        spec, execute_target=False
    ).get(case.opcode, 0)
    return diagnostic_count * target_count + (
        generator_max_count - diagnostic_count
    ) * reference_count


def _matched_control_declared_control_count(
    spec: MatchedControlSpec, generator_max_count: int
) -> int:
    if not spec.compound:
        return generator_max_count
    per_slot = _matched_control_executed_counts(
        spec, execute_target=False
    ).get(spec.reference_opcode, 0)
    return generator_max_count * per_slot


def _matched_control_compound_metadata(spec: MatchedControlSpec) -> dict[str, Any]:
    if not spec.compound:
        return {}
    return {
        "target_program": "0x" + spec.target_program.hex(),
        "reference_program": "0x" + spec.reference_program.hex(),
        "reference_opcode_counts": {
            f"0x{opcode:02x}": count
            for opcode, count in spec.reference_opcode_counts
        },
        "reference_raw_gas_total": spec.reference_raw_gas_total,
        "target_pre_suffix_padding": "0x" + spec.target_pre_suffix_padding.hex(),
        "control_pre_suffix_padding": "0x" + spec.control_pre_suffix_padding.hex(),
        "common_suffix": "0x" + spec.common_suffix.hex(),
        "target_post_suffix_padding": "0x"
        + spec.target_post_suffix_padding.hex(),
        "control_post_suffix_padding": "0x"
        + spec.control_post_suffix_padding.hex(),
    }


def _validate_matched_control_compound_metadata(
    target: Mapping[str, Any],
    control: Mapping[str, Any],
    spec: MatchedControlSpec,
) -> dict[str, Any]:
    target_fields = {field for field in MATCHED_CONTROL_COMPOUND_FIELDS if field in target}
    control_fields = {
        field for field in MATCHED_CONTROL_COMPOUND_FIELDS if field in control
    }
    expected_fields = set(MATCHED_CONTROL_COMPOUND_FIELDS)
    if target_fields != control_fields or target_fields not in (set(), expected_fields):
        raise ValueError("matched-control compound metadata must be complete")
    expected = _matched_control_compound_metadata(spec)
    if spec.compound and target_fields != expected_fields:
        raise ValueError("matched-control compound metadata must be complete")
    if not spec.compound and target_fields:
        raise ValueError("matched-control compound metadata is unexpected")
    for field_name, expected_value in expected.items():
        if target.get(field_name) != control.get(field_name):
            raise ValueError(
                f"matched-control {field_name.replace('_', ' ')} mismatch"
            )
        if target.get(field_name) != expected_value:
            raise ValueError(
                f"matched-control {field_name.replace('_', ' ')} differs from canonical spec"
            )
    return expected


def build_matched_control_bytecode(
    case: CaseSpec,
    diagnostic_count: int,
    generator_max_count: int,
    *,
    lane: str,
    operand_profile: str = "zero",
) -> GeneratedBytecode:
    """Build one lane of a diagnostic OP-minus-control fixed-footprint pair."""
    matched_control_spec(case, operand_profile)
    if diagnostic_count < 0 or diagnostic_count > generator_max_count:
        raise ValueError("diagnostic_count exceeds generator_max_count")
    if lane not in {"target", "control"}:
        raise ValueError("matched control lane must be target or control")
    programs = [
        _matched_control_slot(
            case,
            execute_target=lane == "target" and index < diagnostic_count,
            operand_profile=operand_profile,
        )
        for index in range(generator_max_count)
    ]
    if len({len(program) for program in programs}) > 1:
        raise AssertionError("matched-control microprogram slots must have one byte length")
    encoded = encode_fixed_microprograms(programs)
    counts: dict[int, int] = {}
    for program in programs:
        for opcode, count in count_opcodes(program).items():
            counts[opcode] = counts.get(opcode, 0) + count
    return GeneratedBytecode(bytes_hex=encoded.hex(), opcode_counts=counts)


def build_relation_bytecode(
    case: CaseSpec,
    relation: OpcodeRelationSpec,
    relation_count: int,
    generator_max_count: int,
    *,
    lane: str,
) -> GeneratedBytecode:
    spec = relation_matched_control_spec(case, relation.scenario)
    if relation_count < 0 or relation_count > generator_max_count:
        raise ValueError("relation count exceeds generator_max_count")
    if lane not in {"target", "control"}:
        raise ValueError("formal relation lane must be target or control")

    def slot(execute_target: bool) -> bytes:
        if spec.compound:
            if execute_target:
                return (
                    spec.setup
                    + spec.target_program
                    + spec.target_pre_suffix_padding
                    + spec.common_suffix
                    + spec.target_post_suffix_padding
                )
            return (
                spec.setup
                + spec.reference_program
                + spec.control_pre_suffix_padding
                + spec.common_suffix
                + spec.control_post_suffix_padding
            )
        opcode = case.opcode if execute_target else spec.reference_opcode
        assert opcode is not None
        return spec.setup + bytes([opcode, 0x00])

    programs = [
        slot(lane == "target" and index < relation_count)
        for index in range(generator_max_count)
    ]
    if len({len(program) for program in programs}) > 1:
        raise AssertionError("formal relation microprogram slots must have one byte length")
    encoded = encode_fixed_microprograms(programs)
    counts: dict[int, int] = {}
    for program in programs:
        for opcode, count in count_opcodes(program).items():
            counts[opcode] = counts.get(opcode, 0) + count
    return GeneratedBytecode(bytes_hex=encoded.hex(), opcode_counts=counts)


def matched_control_operands(
    case: CaseSpec, operand_profile: str = "zero"
) -> list[int]:
    return list(matched_control_spec(case, operand_profile).operands)


MATCHED_CONTROL_WORKLOAD_FIELDS = (
    "suite",
    "backend",
    "purpose",
    "diagnostic_only",
    "original_case",
    "original_opcode",
    "scenario",
    "operand_profile",
    "operands",
    "template",
    "relation",
    "diagnostic_count",
    "final_stack_height",
    "generator_max_count",
    "fixed_bytecode_len",
    "tx_gas_limit",
    "signal_kind",
)
MATCHED_CONTROL_COMMON_FIELDS = MATCHED_CONTROL_WORKLOAD_FIELDS + (
    "calibration_id",
    "calibration_identity_sha256",
    "implementation_revision",
    "controlled_manifest_sha256",
    "controlled_manifest_rows_sha256",
)

FORMAL_RELATION_FIELDS = (
    "relation_id",
    "relation_split",
    "scenario_id",
    "relation_scenario",
    "dynamic_key",
    "target_raw_gas_by_key",
    "control_raw_gas_by_key",
    "signed_raw_gas_by_key",
)


def _matched_control_pair_spec(
    target: Mapping[str, Any], control: Mapping[str, Any]
) -> dict[str, Any]:
    for field_name in MATCHED_CONTROL_COMMON_FIELDS:
        if target.get(field_name) != control.get(field_name):
            raise ValueError(f"matched-control {field_name} mismatch")
    purpose = target.get("purpose")
    if purpose not in {MATCHED_CONTROL_PURPOSE, FORMAL_RELATION_PURPOSE}:
        raise ValueError("matched-control purpose is invalid")
    expected_diagnostic = purpose == MATCHED_CONTROL_PURPOSE
    expected_signal = (
        "contextual_relative"
        if expected_diagnostic
        else FORMAL_RELATION_SIGNAL_KIND
    )
    if target.get("diagnostic_only") is not expected_diagnostic:
        raise ValueError("matched-control fixture purpose flag is invalid")
    if target.get("signal_kind") != expected_signal:
        raise ValueError("matched-control signal kind is invalid")
    for field_name in FORMAL_RELATION_FIELDS:
        if target.get(field_name) != control.get(field_name):
            raise ValueError(f"matched-control {field_name} mismatch")
    if expected_diagnostic and any(field in target or field in control for field in FORMAL_RELATION_FIELDS):
        raise ValueError("diagnostic matched-control fixture contains formal relation fields")
    template = target.get("template")
    diagnostic_count = target.get("diagnostic_count")
    generator_max_count = target.get("generator_max_count")
    if (
        isinstance(diagnostic_count, bool)
        or not isinstance(diagnostic_count, int)
        or isinstance(generator_max_count, bool)
        or not isinstance(generator_max_count, int)
        or diagnostic_count < 0
        or diagnostic_count > generator_max_count
    ):
        raise ValueError("matched-control diagnostic count is invalid")
    original_opcode = parse_opcode(target.get("original_opcode"))
    target_raw_gas = target.get("target_raw_gas")
    if (
        isinstance(target_raw_gas, bool)
        or not isinstance(target_raw_gas, int)
        or target_raw_gas <= 0
    ):
        raise ValueError("matched-control target raw gas is invalid")
    if parse_opcode(target.get("opcode")) != original_opcode:
        raise ValueError("matched-control target opcode differs from original opcode")
    fixed_len = target.get("fixed_bytecode_len")
    tx_gas_limit = target.get("tx_gas_limit")
    if (
        isinstance(fixed_len, bool)
        or not isinstance(fixed_len, int)
        or fixed_len <= 0
        or isinstance(tx_gas_limit, bool)
        or not isinstance(tx_gas_limit, int)
        or tx_gas_limit <= 0
    ):
        raise ValueError("matched-control footprint is invalid")
    for lane, row in (("target", target), ("control", control)):
        if row.get("lane") != lane:
            raise ValueError("matched-control lane metadata is invalid")
        if not _is_sha256(row.get("fixture_sha256")):
            raise ValueError("matched-control fixture SHA256 is invalid")
        bytecode = _normalized_hex(row.get("bytecode"), field_name="matched bytecode")
        if len(bytes.fromhex(bytecode[2:])) != fixed_len:
            raise ValueError("matched-control fixed_bytecode_len mismatch")
    reconstructed_case = CaseSpec(
        name=str(target.get("original_case")),
        scenario=str(target.get("scenario")),
        template=str(template),
        target_raw_gas=target_raw_gas,
        opcode=original_opcode,
    )
    operand_profile = target.get("operand_profile")
    if not isinstance(operand_profile, str):
        raise ValueError("matched-control operand profile is invalid")
    relation_scenario = target.get("relation_scenario", {})
    if not isinstance(relation_scenario, Mapping):
        raise ValueError("formal relation scenario is invalid")
    spec = (
        relation_matched_control_spec(reconstructed_case, relation_scenario)
        if purpose == FORMAL_RELATION_PURPOSE
        else matched_control_spec(reconstructed_case, operand_profile)
    )
    if target.get("relation") != spec.relation:
        raise ValueError("matched-control relation/template mismatch")
    if target.get("final_stack_height") != spec.final_stack_height:
        raise ValueError("matched-control final stack height mismatch")
    if target.get("operands") != list(spec.operands):
        raise ValueError("matched-control operands do not match operand profile")
    compound_metadata = _validate_matched_control_compound_metadata(
        target, control, spec
    )
    expected_target_count = _matched_control_declared_target_count(
        reconstructed_case, spec, diagnostic_count, generator_max_count
    )
    if target.get("target_count") != expected_target_count:
        raise ValueError("matched-control target count differs from executed opcode count")
    if (
        parse_opcode(control.get("opcode")) != spec.reference_opcode
        or control.get("target_count")
        != _matched_control_declared_control_count(spec, generator_max_count)
        or control.get("target_raw_gas") != spec.reference_raw_gas
    ):
        raise ValueError("matched-control control declaration is invalid")
    if purpose == FORMAL_RELATION_PURPOSE:
        target_map = _parse_canonical_int_map(
            target.get("target_raw_gas_by_key"), label="target raw-gas map"
        )
        control_map = _parse_canonical_int_map(
            target.get("control_raw_gas_by_key"), label="control raw-gas map"
        )
        signed_map = _parse_canonical_int_map(
            target.get("signed_raw_gas_by_key"),
            label="signed raw-gas map",
            allow_negative=True,
        )
        expected_target_map, expected_control_map, expected_signed_map = (
            _opcode_relation_maps(reconstructed_case, spec, target_raw_gas)
        )
        if target_map != expected_target_map:
            raise ValueError("formal relation target raw-gas map differs from executed program")
        if control_map != expected_control_map:
            raise ValueError("formal relation control raw-gas map differs from executed program")
        if signed_map != expected_signed_map:
            raise ValueError("formal relation signed raw-gas map differs from executed programs")
        relation_spec = OpcodeRelationSpec(
            id=str(target.get("relation_id")),
            case_id=reconstructed_case.name,
            key_id=f"opcode:0x{original_opcode:02x}",
            split=str(target.get("relation_split")),
            scenario_id=str(target.get("scenario_id")),
            scenario=MappingProxyType(dict(relation_scenario)),
            target_raw_gas_by_key=MappingProxyType(target_map),
            control_raw_gas_by_key=MappingProxyType(control_map),
            signed_raw_gas_by_key=MappingProxyType(signed_map),
            dynamic_key=target.get("dynamic_key"),
        )
        expected_target = build_relation_bytecode(
            reconstructed_case, relation_spec, diagnostic_count, generator_max_count, lane="target"
        )
        expected_control = build_relation_bytecode(
            reconstructed_case, relation_spec, diagnostic_count, generator_max_count, lane="control"
        )
    else:
        expected_target = build_matched_control_bytecode(
            reconstructed_case,
            diagnostic_count,
            generator_max_count,
            lane="target",
            operand_profile=operand_profile,
        )
        expected_control = build_matched_control_bytecode(
            reconstructed_case,
            diagnostic_count,
            generator_max_count,
            lane="control",
            operand_profile=operand_profile,
        )
    if target["bytecode"] != "0x" + expected_target.bytes_hex or control[
        "bytecode"
    ] != "0x" + expected_control.bytes_hex:
        raise ValueError("matched-control bytecode differs from matched layout")
    for digest_field in (
        "calibration_identity_sha256",
        "controlled_manifest_sha256",
        "controlled_manifest_rows_sha256",
    ):
        if not _is_sha256(target.get(digest_field)):
            raise ValueError(f"matched-control {digest_field} is invalid")
    if not _is_git_revision(target.get("implementation_revision")):
        raise ValueError("matched-control implementation revision is invalid")
    calibration_id = target.get("calibration_id")
    if (
        not isinstance(calibration_id, str)
        or len(calibration_id) != 24
        or any(char not in "0123456789abcdef" for char in calibration_id)
    ):
        raise ValueError("matched-control calibration ID is invalid")
    workload = {
        field_name: target[field_name]
        for field_name in MATCHED_CONTROL_WORKLOAD_FIELDS
    }
    if purpose == FORMAL_RELATION_PURPOSE:
        workload.update({field_name: target[field_name] for field_name in FORMAL_RELATION_FIELDS})
    workload.update(compound_metadata)
    pair_spec = {
        "schema_version": 1,
        "purpose": purpose,
        "calibration_execution_identity": {
            field_name: target[field_name]
            for field_name in (
                "calibration_id",
                "calibration_identity_sha256",
                "implementation_revision",
                "controlled_manifest_sha256",
                "controlled_manifest_rows_sha256",
            )
        },
        "workload": workload,
        "lanes": {
            lane: {
                "case": row["case"],
                "opcode": row["opcode"],
                "target_count": row["target_count"],
                "target_raw_gas": row["target_raw_gas"],
                "fixture_sha256": row["fixture_sha256"],
            }
            for lane, row in (("target", target), ("control", control))
        },
    }
    return pair_spec


def matched_control_pair_id(
    target: Mapping[str, Any], control: Mapping[str, Any]
) -> str:
    return sha256_bytes(
        canonical_json(
            {"kind": "matched_control_pair", "pair_spec": _matched_control_pair_spec(target, control)}
        )
    )


def validate_matched_control_fixture_pairs(
    rows: Iterable[Mapping[str, Any]],
    *,
    expected_purpose: str | None = None,
    calibration_execution_identity: Mapping[str, Any] | None = None,
    guest_inputs: Mapping[str, Mapping[str, Any]] | None = None,
) -> dict[str, tuple[Mapping[str, Any], Mapping[str, Any]]]:
    grouped: dict[tuple[Any, ...], list[Mapping[str, Any]]] = {}
    for row in rows:
        key = (
            row.get("suite"),
            row.get("original_case"),
            row.get("relation_id"),
            row.get("diagnostic_count"),
            row.get("generator_max_count"),
        )
        grouped.setdefault(key, []).append(row)
    if not grouped:
        raise ValueError("matched-control fixtures are empty")
    pairs = {}
    for grouped_rows in grouped.values():
        if len(grouped_rows) != 2:
            raise ValueError("matched-control pair requires exactly one target and one control")
        lanes = [row.get("lane") for row in grouped_rows]
        if len(set(lanes)) != len(lanes):
            raise ValueError("matched-control pair has a duplicate lane")
        if set(lanes) != {"target", "control"}:
            raise ValueError("matched-control pair requires exactly one target and one control")
        target = next(row for row in grouped_rows if row["lane"] == "target")
        control = next(row for row in grouped_rows if row["lane"] == "control")
        if expected_purpose is not None and target.get("purpose") != expected_purpose:
            raise ValueError(f"matched-control fixture purpose must be {expected_purpose}")
        pair_ids = {target.get("pair_id"), control.get("pair_id")}
        if len(pair_ids) != 1 or not _is_sha256(next(iter(pair_ids))):
            raise ValueError("matched-control pair_id mismatch")
        pair_id = next(iter(pair_ids))
        if matched_control_pair_id(target, control) != pair_id:
            raise ValueError("matched-control pair_id does not match fixture identity")
        if guest_inputs is not None:
            for row in (target, control):
                fixture_sha256 = row["fixture_sha256"]
                guest_input = guest_inputs.get(fixture_sha256)
                if not isinstance(guest_input, Mapping):
                    raise ValueError("matched-control guest input is missing")
                for field_name in (
                    "case",
                    "scenario",
                    "opcode",
                    "target_count",
                    "target_raw_gas",
                    "generator_max_count",
                    "fixed_bytecode_len",
                    "tx_gas_limit",
                ):
                    expected = (
                        parse_opcode(row[field_name])
                        if field_name == "opcode"
                        else row[field_name]
                    )
                    if guest_input.get(field_name) != expected:
                        raise ValueError(
                            f"matched-control guest input {field_name} mismatch"
                        )
                if _normalized_hex(
                    guest_input.get("bytecode"),
                    field_name="matched guest input bytecode",
                ) != _normalized_hex(
                    row.get("bytecode"), field_name="matched case bytecode"
                ):
                    raise ValueError("matched-control guest input bytecode mismatch")
        if calibration_execution_identity is not None and any(
            target.get(field_name) != calibration_execution_identity.get(field_name)
            for field_name in (
                "calibration_id",
                "calibration_identity_sha256",
                "implementation_revision",
                "controlled_manifest_sha256",
                "controlled_manifest_rows_sha256",
            )
        ):
            raise ValueError("matched-control calibration execution identity mismatch")
        if pair_id in pairs:
            raise ValueError("matched-control pair_id collision")
        pairs[pair_id] = (target, control)
    return pairs


def build_matched_control_report(
    rows: Iterable[Mapping[str, Any]],
    *,
    expected_pair_ids: set[str] | None = None,
    expected_repeats: int | None = None,
) -> dict[str, Any]:
    grouped: dict[tuple[str, int], list[Mapping[str, Any]]] = {}
    for row in rows:
        if row.get("purpose") != MATCHED_CONTROL_PURPOSE:
            raise ValueError("matched-control report received a non-diagnostic row")
        pair_id = row.get("pair_id")
        repeat_index = row.get("repeat_index")
        if not _is_sha256(pair_id) or isinstance(repeat_index, bool) or not isinstance(
            repeat_index, int
        ):
            raise ValueError("matched-control result identity is invalid")
        grouped.setdefault((pair_id, repeat_index), []).append(row)
    if not grouped:
        raise ValueError("matched-control report has no rows")
    if expected_pair_ids is not None or expected_repeats is not None:
        if (
            expected_pair_ids is None
            or not expected_pair_ids
            or not all(_is_sha256(pair_id) for pair_id in expected_pair_ids)
            or isinstance(expected_repeats, bool)
            or not isinstance(expected_repeats, int)
            or expected_repeats <= 0
        ):
            raise ValueError("matched-control expected pair/repeat declaration is invalid")
        expected_keys = {
            (pair_id, repeat_index)
            for pair_id in expected_pair_ids
            for repeat_index in range(expected_repeats)
        }
        if set(grouped) != expected_keys:
            raise ValueError("matched-control result is missing expected pair/repeat")
    results = []
    for (pair_id, repeat_index), grouped_rows in sorted(grouped.items()):
        if len(grouped_rows) != 2:
            raise ValueError("matched-control result requires one target and one control")
        lanes = [row.get("lane") for row in grouped_rows]
        if len(set(lanes)) != len(lanes):
            raise ValueError("matched-control result has a duplicate lane")
        if set(lanes) != {"target", "control"}:
            raise ValueError("matched-control result requires one target and one control")
        target = next(row for row in grouped_rows if row["lane"] == "target")
        control = next(row for row in grouped_rows if row["lane"] == "control")
        for field_name in (
            "original_case",
            "original_opcode",
            "template",
            "scenario",
            "operand_profile",
            "operands",
            "relation",
            "signal_kind",
            "diagnostic_count",
            "final_stack_height",
            "generator_max_count",
            "fixed_bytecode_len",
            "tx_gas_limit",
        ):
            if target.get(field_name) != control.get(field_name):
                raise ValueError(f"matched-control result {field_name} mismatch")
        report_case = CaseSpec(
            name=str(target.get("original_case")),
            scenario=str(target.get("scenario")),
            template=str(target.get("template")),
            target_raw_gas=1,
            opcode=parse_opcode(target.get("original_opcode")),
        )
        operand_profile = target.get("operand_profile")
        if not isinstance(operand_profile, str):
            raise ValueError("matched-control result operand profile is invalid")
        spec = matched_control_spec(report_case, operand_profile)
        if target.get("operands") != list(spec.operands):
            raise ValueError(
                "matched-control result operands do not match operand profile"
            )
        if target.get("relation") != spec.relation:
            raise ValueError("matched-control result relation/template mismatch")
        if target.get("final_stack_height") != spec.final_stack_height:
            raise ValueError("matched-control result final stack height mismatch")
        compound_metadata = _validate_matched_control_compound_metadata(
            target, control, spec
        )
        diagnostic_count = target.get("diagnostic_count")
        generator_max_count = target.get("generator_max_count")
        if (
            isinstance(diagnostic_count, bool)
            or not isinstance(diagnostic_count, int)
            or isinstance(generator_max_count, bool)
            or not isinstance(generator_max_count, int)
            or diagnostic_count < 0
            or diagnostic_count > generator_max_count
        ):
            raise ValueError("matched-control result diagnostic count is invalid")
        expected_target_count = _matched_control_declared_target_count(
            report_case, spec, diagnostic_count, generator_max_count
        )
        if (
            parse_opcode(target.get("opcode")) != report_case.opcode
            or target.get("target_count") != expected_target_count
        ):
            raise ValueError("matched-control result target declaration is invalid")
        if (
            parse_opcode(control.get("opcode")) != spec.reference_opcode
            or control.get("target_count")
            != _matched_control_declared_control_count(spec, generator_max_count)
            or control.get("target_raw_gas") != spec.reference_raw_gas
        ):
            raise ValueError("matched-control result control declaration is invalid")
        for row in (target, control):
            isolation = row.get("isolation")
            if (
                not isinstance(isolation, Mapping)
                or isolation.get("status") != "passed"
                or not _is_sha256(row.get("workload_id"))
                or not _is_sha256(row.get("backend_input_sha256"))
            ):
                raise ValueError("matched-control result is missing an actual trace")
        target_isolation = target["isolation"]
        control_isolation = control["isolation"]
        backend_input_len = target_isolation.get("input_size")
        if (
            isinstance(backend_input_len, bool)
            or not isinstance(backend_input_len, int)
            or backend_input_len <= 0
            or backend_input_len != control_isolation.get("input_size")
        ):
            raise ValueError("matched-control backend input length mismatch")
        if target_isolation.get("bytecode_size") != control_isolation.get(
            "bytecode_size"
        ) or target_isolation.get("bytecode_size") != target.get("fixed_bytecode_len"):
            raise ValueError("matched-control actual bytecode footprint mismatch")
        if (
            target_isolation.get("tx_gas_limit")
            != control_isolation.get("tx_gas_limit")
            or target_isolation.get("tx_gas_limit") != target.get("tx_gas_limit")
        ):
            raise ValueError("matched-control actual tx gas limit mismatch")
        execution_provenance = (
            target.get("sp1_execution_engine"),
            target.get("sp1_gas_trace_chunk_threshold"),
            target.get("sp1_gas_trace_chunk_slots"),
        )
        if execution_provenance != (
            control.get("sp1_execution_engine"),
            control.get("sp1_gas_trace_chunk_threshold"),
            control.get("sp1_gas_trace_chunk_slots"),
        ):
            raise ValueError("matched-control execution engine/cadence mismatch")
        if any(
            isinstance(row.get("exit_code"), bool)
            or not isinstance(row.get("exit_code"), int)
            or row.get("exit_code") != 0
            for row in (target, control)
        ):
            raise ValueError("matched-control result requires exit_code == 0")
        target_gas = target.get("prover_gas", target.get("gas"))
        control_gas = control.get("prover_gas", control.get("gas"))
        target_instructions = target.get("total_instruction_count")
        control_instructions = control.get("total_instruction_count")
        if any(
            isinstance(value, bool) or not isinstance(value, int)
            for value in (
                target_gas,
                control_gas,
                target_instructions,
                control_instructions,
            )
        ):
            raise ValueError("matched-control result metrics must be integers")
        diagnostic_count = target["diagnostic_count"]
        prover_gas_delta = target_gas - control_gas
        result = {
                "pair_id": pair_id,
                "repeat_index": repeat_index,
                "original_case": target["original_case"],
                "original_opcode": target["original_opcode"],
                "template": target["template"],
                "scenario": target["scenario"],
                "operand_profile": target["operand_profile"],
                "operands": target["operands"],
                "relation": target["relation"],
                "signal_kind": target["signal_kind"],
                "diagnostic_count": diagnostic_count,
                "prover_gas_delta": prover_gas_delta,
                "prover_gas_per_relation": (
                    _decimal_text(Decimal(prover_gas_delta) / Decimal(diagnostic_count))
                    if diagnostic_count > 0
                    else None
                ),
                "instruction_count_delta": target_instructions - control_instructions,
                "backend_input_len": backend_input_len,
                "bytecode_len": target_isolation["bytecode_size"],
                "tx_gas_limit": target_isolation["tx_gas_limit"],
                "sp1_execution_engine": execution_provenance[0],
                "sp1_gas_trace_chunk_threshold": execution_provenance[1],
                "sp1_gas_trace_chunk_slots": execution_provenance[2],
            }
        result.update(compound_metadata)
        results.append(result)
    return {
        "schema_version": 1,
        "purpose": MATCHED_CONTROL_PURPOSE,
        "results": results,
    }


def build_stack_binary_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    if target_count == 0:
        out.extend([0x60, 0x01, 0x50, 0x00])  # PUSH1 1; POP; STOP
        return bytes(out)

    out.extend([0x60, 0x01, 0x60, 0x02, opcode])
    for _ in range(target_count - 1):
        out.extend([0x60, 0x01, opcode])
    out.extend([0x50, 0x00])  # POP; STOP
    return bytes(out)


def build_stack_unary_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    if target_count == 0:
        out.extend([0x60, 0x01, 0x50, 0x00])  # PUSH1 1; POP; STOP
        return bytes(out)

    out.extend([0x60, 0x01])
    for _ in range(target_count):
        out.append(opcode)
    out.extend([0x50, 0x00])  # POP; STOP
    return bytes(out)


def build_stack_ternary_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    if target_count == 0:
        out.extend([0x60, 0x01, 0x50, 0x00])  # PUSH1 1; POP; STOP
        return bytes(out)

    out.extend([0x60, 0x01, 0x60, 0x02, 0x60, 0x05, opcode])
    for _ in range(target_count - 1):
        out.extend([0x60, 0x01, 0x60, 0x05, opcode])
    out.extend([0x50, 0x00])  # POP; STOP
    return bytes(out)


def build_stack_exp_bytecode(target_count: int) -> bytes:
    out = bytearray()
    if target_count == 0:
        out.extend([0x60, 0x01, 0x50, 0x00])  # PUSH1 1; POP; STOP
        return bytes(out)

    out.extend([0x60, 0x02])
    for _ in range(target_count):
        out.extend([0x60, 0x02, 0x0A])
    out.extend([0x50, 0x00])  # POP; STOP
    return bytes(out)


def build_keccak_32_bytecode(target_count: int) -> bytes:
    out = bytearray([0x60, 0x00, 0x60, 0x00, 0x52])  # zero one memory word
    for _ in range(target_count):
        out.extend([0x60, 0x20, 0x60, 0x00, 0x20, 0x50])
    out.append(0x00)
    return bytes(out)


def build_memory_load_32_bytecode(target_count: int) -> bytes:
    out = bytearray([0x60, 0x00, 0x60, 0x00, 0x52])  # zero one memory word
    for _ in range(target_count):
        out.extend([0x60, 0x00, 0x51, 0x50])  # MLOAD offset 0; POP
    out.append(0x00)
    return bytes(out)


def build_memory_store_32_bytecode(target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        out.extend([0x60, 0x01, 0x60, 0x00, 0x52])
    out.append(0x00)
    return bytes(out)


def build_memory_store8_bytecode(target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        out.extend([0x60, 0x01, 0x60, 0x00, 0x53])
    out.append(0x00)
    return bytes(out)


def build_stack_pop_bytecode(target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        out.extend([0x60, 0x01, 0x50])
    out.append(0x00)
    return bytes(out)


def build_stack_swap1_bytecode(target_count: int) -> bytes:
    out = bytearray([0x60, 0x01, 0x60, 0x02])
    for _ in range(target_count):
        out.append(0x90)
    out.extend([0x50, 0x50, 0x00])
    return bytes(out)


def build_stack_push_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        append_push_opcode(out, opcode)
        out.append(0x50)
    out.append(0x00)
    return bytes(out)


def build_stack_dup_bytecode(opcode: int, target_count: int) -> bytes:
    depth = opcode - 0x7F
    if not 1 <= depth <= 16:
        raise ValueError(f"not a DUP opcode: 0x{opcode:02x}")
    out = bytearray()
    for value in range(1, depth + 1):
        out.extend([0x60, value])
    for _ in range(target_count):
        out.extend([opcode, 0x50])
    out.extend([0x50] * depth)
    out.append(0x00)
    return bytes(out)


def build_stack_swap_bytecode(opcode: int, target_count: int) -> bytes:
    depth = opcode - 0x8F
    if not 1 <= depth <= 16:
        raise ValueError(f"not a SWAP opcode: 0x{opcode:02x}")
    out = bytearray()
    for value in range(1, depth + 2):
        out.extend([0x60, value])
    out.extend([opcode] * target_count)
    out.extend([0x50] * (depth + 1))
    out.append(0x00)
    return bytes(out)


def build_jump_chain_bytecode(target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        dest = len(out) + 3
        if dest > 0xFF:
            raise ValueError("jump_chain template currently supports only PUSH1 destinations")
        out.extend([0x60, dest, 0x56, 0x5B])
    out.append(0x00)
    return bytes(out)


def build_jumpi_chain_bytecode(target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        dest = len(out) + 5
        if dest > 0xFF:
            raise ValueError("jumpi_chain template currently supports only PUSH1 destinations")
        out.extend([0x60, 0x01, 0x60, dest, 0x57, 0x5B])
    out.append(0x00)
    return bytes(out)


def build_stack_unary_producer_bytecode(opcode: int, target_count: int) -> bytes:
    out = bytearray()
    for _ in range(target_count):
        out.extend([opcode, 0x50])
    out.append(0x00)
    return bytes(out)


def build_jumpdest_chain_bytecode(target_count: int) -> bytes:
    out = bytearray([0x5B] * target_count)
    out.append(0x00)
    return bytes(out)


def build_memory_copy_32_bytecode(target_count: int) -> bytes:
    out = bytearray([0x60, 0x01, 0x60, 0x00, 0x52])  # initialize source word
    for _ in range(target_count):
        out.extend([0x60, 0x20, 0x60, 0x00, 0x60, 0x20, 0x5E])
    out.append(0x00)
    return bytes(out)


def append_push_opcode(out: bytearray, opcode: int) -> None:
    if opcode == 0x5F:
        out.append(opcode)
        return
    if not 0x60 <= opcode <= 0x7F:
        raise ValueError(f"not a PUSH opcode: 0x{opcode:02x}")
    size = opcode - 0x5F
    out.append(opcode)
    out.extend((index + 1) & 0xFF for index in range(size))


def count_opcodes(bytecode: bytes) -> dict[int, int]:
    counts: dict[int, int] = {}
    i = 0
    while i < len(bytecode):
        opcode = bytecode[i]
        counts[opcode] = counts.get(opcode, 0) + 1
        i += 1
        if 0x60 <= opcode <= 0x7F:
            i += opcode - 0x5F
    return counts


def build_precompile_input(case: CaseSpec) -> str:
    if case.input_size is None:
        raise ValueError(f"precompile case {case.name} is missing input_size")
    payload = build_precompile_payload(case)
    if len(payload) != case.input_size:
        raise ValueError(
            f"precompile case {case.name} template emitted {len(payload)} bytes, "
            f"expected {case.input_size}"
        )
    return "0x" + payload.hex()


def build_precompile_payload(case: CaseSpec) -> bytes:
    if case.template in {"precompile_fixed", "precompile_fixed_32"}:
        if case.input_size is None:
            raise ValueError(f"precompile case {case.name} is missing input_size")
        return bytes((index % 251 for index in range(case.input_size)))
    if case.template == "precompile_ecrecover_valid":
        return ecrecover_payload()
    if case.template == "precompile_modexp_small":
        return modexp_small_payload()
    if case.template == "precompile_bn254_add":
        return bytes(128)
    if case.template == "precompile_bn254_mul":
        return bytes(96)
    if case.template == "precompile_bn254_pairing":
        return bytes(192)
    if case.template == "precompile_blake2f_12_rounds":
        payload = bytearray(213)
        payload[:4] = (12).to_bytes(4, "big")
        payload[212] = 1
        return bytes(payload)
    if case.template == "precompile_kzg_point_evaluation":
        return bytes.fromhex(
            "01e798154708fe7789429634053cbf9f99b619f9f084048927333fce637f549b"
            "73eda753299d7d483339d80809a1d80553bda402fffe5bfeffffffff00000000"
            "1522a4a7f34e1ea350ae07c29c96c7e79655aa926122e95fe69fcbd932ca49e9"
            "8f59a8d2a1a625a17f3fea0fe5eb8c896db3764f3185481bc22f91b4aaffcca25f26936857bc3a7c2539ea8ec3a952b7"
            "a62ad71d14c5719385c0686f1871430475bf3a00f0aa3f7b8dd99a9abc2160744faf0070725e00b60ad9a026a15b1a8c"
        )
    zero_sizes = {
        "precompile_bls12_g1add_zero": 256,
        "precompile_bls12_g1msm_zero": 160,
        "precompile_bls12_g2add_zero": 512,
        "precompile_bls12_g2msm_zero": 288,
        "precompile_bls12_pairing_zero": 384,
        "precompile_bls12_map_fp_to_g1_zero": 64,
        "precompile_bls12_map_fp2_to_g2_zero": 128,
    }
    if case.template in zero_sizes:
        return bytes(zero_sizes[case.template])
    raise ValueError(f"unknown precompile template: {case.template}")


def ecrecover_payload() -> bytes:
    return bytes.fromhex(
        "6b6f6f7468656e65766572676f6e6e6167697665796f75726d696e646f6e6e61"
        "000000000000000000000000000000000000000000000000000000000000001b"
        "b50bb6795f31748a4d37c3a97ebd06a22ea33771040f5c05d6e2bb2d38c6227c"
        "343b6659db969959d9fddb44bd0dd9b9dd47666ab52871901d1761eb82ec8722"
    )


def modexp_small_payload() -> bytes:
    out = bytearray()
    for size in (1, 1, 1):
        out.extend(bytes(31))
        out.append(size)
    out.extend([2, 3, 5])
    return bytes(out)


def generate_cases(
    manifest: Manifest,
    out_dir: pathlib.Path,
    provenance: Mapping[str, Any] | None = None,
    generator_max_count: int | None = None,
    matched_control_diagnostic: bool = False,
    operand_profile: str = "zero",
) -> list[pathlib.Path]:
    written = []
    controlled_max = (
        8 if manifest.measurement_keys and generator_max_count is None else generator_max_count
    )
    if matched_control_diagnostic:
        if controlled_max is None:
            raise ValueError("matched control diagnostic requires generator_max_count")
        for case in manifest.cases:
            if case.kind != "opcode":
                raise ValueError(
                    f"matched control template {case.template} is unsupported"
                )
            matched_control_spec(case, operand_profile)
    for case in manifest.cases:
        for variant in manifest.variants:
            if controlled_max is not None and variant > controlled_max:
                continue
            case_dir = out_dir / manifest.name / case.name / f"count-{variant}"
            if case.kind == "opcode":
                if matched_control_diagnostic:
                    assert controlled_max is not None
                    if case.opcode is None:
                        raise ValueError(f"opcode case {case.name} is missing opcode")
                    matched_spec = matched_control_spec(case, operand_profile)
                    target = build_matched_control_bytecode(
                        case,
                        variant,
                        controlled_max,
                        lane="target",
                        operand_profile=operand_profile,
                    )
                    control = build_matched_control_bytecode(
                        case,
                        variant,
                        controlled_max,
                        lane="control",
                        operand_profile=operand_profile,
                    )
                    target_len = len(bytes.fromhex(target.bytes_hex))
                    control_len = len(bytes.fromhex(control.bytes_hex))
                    if target_len != control_len:
                        raise AssertionError("matched-control bytecode footprints differ")
                    tx_gas_limit = 1_000_000 + controlled_max * max(
                        case.target_raw_gas,
                        matched_spec.reference_raw_gas_total
                        if matched_spec.compound
                        else matched_spec.reference_raw_gas,
                    )
                    lane_artifacts = []
                    for lane, generated in (("target", target), ("control", control)):
                        lane_dir = case_dir / lane
                        if lane == "target":
                            declared_opcode = case.opcode
                            declared_count = _matched_control_declared_target_count(
                                case, matched_spec, variant, controlled_max
                            )
                            declared_raw_gas = case.target_raw_gas
                            lane_case = f"{case.name}__matched_t"
                        else:
                            declared_opcode = matched_spec.reference_opcode
                            declared_count = _matched_control_declared_control_count(
                                matched_spec, controlled_max
                            )
                            declared_raw_gas = matched_spec.reference_raw_gas
                            lane_case = f"{case.name}__matched_c"
                        payload = {
                            "suite": manifest.name,
                            "backend": manifest.backend,
                            "kind": "opcode",
                            "purpose": MATCHED_CONTROL_PURPOSE,
                            "diagnostic_only": True,
                            "case": lane_case,
                            "original_case": case.name,
                            "original_opcode": f"0x{case.opcode:02x}",
                            "opcode": f"0x{declared_opcode:02x}",
                            "scenario": case.scenario,
                            "operand_profile": operand_profile,
                            "operands": list(matched_spec.operands),
                            "template": case.template,
                            "lane": lane,
                            "relation": matched_spec.relation,
                            "signal_kind": "contextual_relative",
                            "diagnostic_count": variant,
                            "final_stack_height": matched_spec.final_stack_height,
                            "target_count": declared_count,
                            "target_raw_gas": declared_raw_gas,
                            "bytecode": "0x" + generated.bytes_hex,
                            "opcode_counts": {
                                f"0x{k:02x}": v
                                for k, v in sorted(generated.opcode_counts.items())
                            },
                            "generator_max_count": controlled_max,
                            "fixed_bytecode_len": target_len,
                            "tx_gas_limit": tx_gas_limit,
                            "guest_input_status": "opcode_lab_guest_input",
                        }
                        payload.update(
                            _matched_control_compound_metadata(matched_spec)
                        )
                        guest_input = {
                            "case": lane_case,
                            "scenario": case.scenario,
                            "opcode": declared_opcode,
                            "target_count": declared_count,
                            "target_raw_gas": declared_raw_gas,
                            "bytecode": "0x" + generated.bytes_hex,
                            "generator_max_count": controlled_max,
                            "fixed_bytecode_len": target_len,
                            "tx_gas_limit": tx_gas_limit,
                        }
                        guest_input_bytes = (
                            json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
                        ).encode()
                        if provenance:
                            payload.update(provenance)
                        payload["fixture_sha256"] = sha256_bytes(guest_input_bytes)
                        lane_artifacts.append((lane_dir, payload, guest_input_bytes))
                    target_payload = lane_artifacts[0][1]
                    control_payload = lane_artifacts[1][1]
                    pair_id = matched_control_pair_id(target_payload, control_payload)
                    for lane_dir, payload, guest_input_bytes in lane_artifacts:
                        lane_dir.mkdir(parents=True, exist_ok=True)
                        payload["pair_id"] = pair_id
                        path = lane_dir / "case.json"
                        path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                        (lane_dir / "guest-input.json").write_bytes(guest_input_bytes)
                        written.append(path)
                    continue
                case_dir.mkdir(parents=True, exist_ok=True)
                if case.opcode is None:
                    raise ValueError(f"opcode case {case.name} is missing opcode")
                generated = (
                    build_fixed_footprint_bytecode(case, variant, controlled_max)
                    if controlled_max is not None
                    else build_bytecode(case, variant)
                )
                payload = {
                    "suite": manifest.name,
                    "backend": manifest.backend,
                    "kind": case.kind,
                    "case": case.name,
                    "opcode": f"0x{case.opcode:02x}",
                    "scenario": case.scenario,
                    "template": case.template,
                    "target_count": variant,
                    "target_raw_gas": case.target_raw_gas,
                    "target_feature": variant * case.target_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                    "opcode_counts": {
                        f"0x{k:02x}": v for k, v in sorted(generated.opcode_counts.items())
                    },
                    "guest_input_status": "opcode_lab_guest_input",
                }
                guest_input = {
                    "case": case.name,
                    "scenario": case.scenario,
                    "opcode": case.opcode,
                    "target_count": variant,
                    "target_raw_gas": case.target_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                }
                if controlled_max is not None:
                    fixed_len = len(bytes.fromhex(generated.bytes_hex))
                    tx_gas_limit = 1_000_000 + controlled_max * case.target_raw_gas
                    payload["generator_max_count"] = controlled_max
                    payload["fixed_bytecode_len"] = fixed_len
                    payload["tx_gas_limit"] = tx_gas_limit
                    guest_input["generator_max_count"] = controlled_max
                    guest_input["fixed_bytecode_len"] = fixed_len
                    guest_input["tx_gas_limit"] = tx_gas_limit
            elif case.kind == "precompile":
                if case.address is None:
                    raise ValueError(f"precompile case {case.name} is missing address")
                precompile_input = build_precompile_input(case)
                lanes = ("target", "control") if case.paired else ("target",)
                pair_spec = {
                    "schema_version": 1,
                    "key_id": f"precompile:0x{case.address:02x}",
                    "case_id": case.name,
                    "target_count": variant,
                    "input": {
                        "address": case.address,
                        "calldata": precompile_input,
                        "target_raw_gas": case.target_raw_gas,
                        "expected_output_size": case.expected_output_size,
                    },
                }
                pair_id = sha256_bytes(
                    canonical_json({"kind": "controlled_precompile_pair", "pair_spec": pair_spec})
                )
                for lane in lanes:
                    lane_dir = case_dir / lane if case.paired else case_dir
                    lane_dir.mkdir(parents=True, exist_ok=True)
                    payload = {
                        "suite": manifest.name,
                        "backend": manifest.backend,
                        "kind": case.kind,
                        "case": case.name,
                        "address": f"0x{case.address:02x}",
                        "scenario": case.scenario,
                        "template": case.template,
                        "lane": lane,
                        "pair_id": pair_id,
                        "target_count": variant,
                        "input_size": case.input_size,
                        "expected_output_size": case.expected_output_size,
                        "target_raw_gas": case.target_raw_gas,
                        "target_feature": variant * case.target_raw_gas,
                        "input": precompile_input,
                        "guest_input_status": "precompile_lab_guest_input",
                    }
                    guest_input = {
                        "case": case.name,
                        "scenario": case.scenario,
                        "lane": lane,
                        "address": case.address,
                        "target_count": variant,
                        "input_size": case.input_size,
                        "target_raw_gas": case.target_raw_gas,
                        "expected_output_size": case.expected_output_size,
                        "input": precompile_input,
                    }
                    if controlled_max is not None:
                        payload["generator_max_count"] = controlled_max
                        guest_input["generator_max_count"] = controlled_max
                    guest_input_bytes = (
                        json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
                    ).encode()
                    if provenance:
                        payload.update(provenance)
                    payload["fixture_sha256"] = sha256_bytes(guest_input_bytes)
                    path = lane_dir / "case.json"
                    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                    (lane_dir / "guest-input.json").write_bytes(guest_input_bytes)
                    written.append(path)
                continue
            else:
                raise ValueError(f"unknown case kind: {case.kind}")
            guest_input_bytes = (
                json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
            ).encode()
            if provenance:
                payload.update(provenance)
            payload["fixture_sha256"] = sha256_bytes(guest_input_bytes)
            path = case_dir / "case.json"
            path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            (case_dir / "guest-input.json").write_bytes(guest_input_bytes)
            written.append(path)
    return written


def generate_relation_cases(
    manifest: Manifest,
    out_dir: pathlib.Path,
    *,
    provenance: Mapping[str, Any],
    generator_max_count: int = 8,
) -> list[pathlib.Path]:
    """Generate the formal matched-control relation campaign without changing diagnostics."""
    if generator_max_count not in OUT_OF_FIT_CHECKPOINTS.values():
        raise ValueError("formal relation generator bound must be a frozen checkpoint")
    required_provenance = {
        "calibration_id",
        "calibration_identity_sha256",
        "implementation_revision",
        "controlled_manifest_sha256",
        "controlled_manifest_rows_sha256",
    }
    if not required_provenance.issubset(provenance):
        raise ValueError("formal relation generation requires calibration identity")
    cases = {case.name: case for case in manifest.cases}
    written: list[pathlib.Path] = []
    for relation in manifest.opcode_relations:
        case = cases[relation.case_id]
        if case.opcode is None:
            raise ValueError("formal opcode relation requires an opcode case")
        spec = relation_matched_control_spec(case, relation.scenario)
        target_raw_gas = relation.target_raw_gas_by_key[relation.key_id]
        relation_case = replace(case, target_raw_gas=target_raw_gas)
        for count in manifest.variants:
            if count > generator_max_count:
                continue
            case_dir = (
                out_dir
                / manifest.name
                / relation.id.replace(":", "-")
                / f"count-{count}"
            )
            target = build_relation_bytecode(
                relation_case, relation, count, generator_max_count, lane="target"
            )
            control = build_relation_bytecode(
                relation_case, relation, count, generator_max_count, lane="control"
            )
            target_len = len(bytes.fromhex(target.bytes_hex))
            if len(bytes.fromhex(control.bytes_hex)) != target_len:
                raise AssertionError("formal relation bytecode footprints differ")
            control_total = sum(relation.control_raw_gas_by_key.values())
            tx_gas_limit = 1_000_000 + generator_max_count * max(
                target_raw_gas, control_total
            )
            lane_artifacts = []
            for lane, generated in (("target", target), ("control", control)):
                if lane == "target":
                    declared_opcode = case.opcode
                    declared_count = _matched_control_declared_target_count(
                        relation_case, spec, count, generator_max_count
                    )
                    declared_raw_gas = target_raw_gas
                else:
                    declared_opcode = spec.reference_opcode
                    declared_count = _matched_control_declared_control_count(
                        spec, generator_max_count
                    )
                    declared_raw_gas = spec.reference_raw_gas
                lane_case = f"{case.name}__relation_{lane}"
                payload = {
                    "suite": manifest.name,
                    "backend": manifest.backend,
                    "kind": "opcode",
                    "purpose": FORMAL_RELATION_PURPOSE,
                    "diagnostic_only": False,
                    "case": lane_case,
                    "original_case": case.name,
                    "original_opcode": f"0x{case.opcode:02x}",
                    "opcode": f"0x{declared_opcode:02x}",
                    "scenario": case.scenario,
                    "operand_profile": "zero",
                    "operands": list(spec.operands),
                    "template": case.template,
                    "lane": lane,
                    "relation": spec.relation,
                    "signal_kind": FORMAL_RELATION_SIGNAL_KIND,
                    "diagnostic_count": count,
                    "final_stack_height": spec.final_stack_height,
                    "target_count": declared_count,
                    "target_raw_gas": declared_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                    "opcode_counts": {
                        f"0x{opcode:02x}": opcode_count
                        for opcode, opcode_count in sorted(generated.opcode_counts.items())
                    },
                    "generator_max_count": generator_max_count,
                    "fixed_bytecode_len": target_len,
                    "tx_gas_limit": tx_gas_limit,
                    "guest_input_status": "opcode_lab_guest_input",
                    "relation_id": relation.id,
                    "relation_split": relation.split,
                    "scenario_id": relation.scenario_id,
                    "relation_scenario": dict(relation.scenario),
                    "dynamic_key": relation.dynamic_key,
                    "target_raw_gas_by_key": {
                        key: str(value)
                        for key, value in sorted(relation.target_raw_gas_by_key.items())
                    },
                    "control_raw_gas_by_key": {
                        key: str(value)
                        for key, value in sorted(relation.control_raw_gas_by_key.items())
                    },
                    "signed_raw_gas_by_key": {
                        key: str(value)
                        for key, value in sorted(relation.signed_raw_gas_by_key.items())
                    },
                    **provenance,
                }
                payload.update(_matched_control_compound_metadata(spec))
                guest_input = {
                    "case": lane_case,
                    "scenario": case.scenario,
                    "opcode": declared_opcode,
                    "target_count": declared_count,
                    "target_raw_gas": declared_raw_gas,
                    "bytecode": "0x" + generated.bytes_hex,
                    "generator_max_count": generator_max_count,
                    "fixed_bytecode_len": target_len,
                    "tx_gas_limit": tx_gas_limit,
                }
                guest_input_bytes = (
                    json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
                ).encode()
                payload["fixture_sha256"] = sha256_bytes(guest_input_bytes)
                lane_artifacts.append(
                    (case_dir / lane, payload, guest_input_bytes)
                )
            pair_id = matched_control_pair_id(
                lane_artifacts[0][1], lane_artifacts[1][1]
            )
            for lane_dir, payload, guest_input_bytes in lane_artifacts:
                lane_dir.mkdir(parents=True, exist_ok=True)
                payload["pair_id"] = pair_id
                path = lane_dir / "case.json"
                path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
                (lane_dir / "guest-input.json").write_bytes(guest_input_bytes)
                written.append(path)
    return written


def select_matched_control_cases(
    manifest: Manifest, case_ids: Iterable[str]
) -> Manifest:
    requested = list(case_ids)
    if not requested:
        raise ValueError("matched-control generation requires at least one --case")
    if len(set(requested)) != len(requested):
        raise ValueError("matched-control case selection contains a duplicate")
    available = {case.name for case in manifest.cases}
    unknown = set(requested) - available
    if unknown:
        raise ValueError(f"unknown matched-control case: {sorted(unknown)[0]}")
    selected = [case for case in manifest.cases if case.name in set(requested)]
    return replace(manifest, cases=selected)


def run_guest_input(
    guest_launcher: pathlib.Path,
    elf_path: pathlib.Path,
    input_path: pathlib.Path,
    json_out: pathlib.Path,
    stage: str = "opcode-lab",
) -> None:
    cmd = [
        str(guest_launcher),
        "--stage",
        stage,
        "--proof-type",
        "sp1",
        "--mode",
        "execute",
        "--sp1-prover",
        "local",
        "--elf",
        str(elf_path),
        "--input",
        str(input_path),
        "--json-out",
        str(json_out),
    ]
    if stage in {"opcode-lab", "revm-opcode-lab"}:
        cmd.extend(["--sp1-execution-engine", "gas-estimator"])
    subprocess.run(cmd, check=True)


def run_guest_inputs(
    guest_launcher: pathlib.Path,
    elf_path: pathlib.Path,
    input_paths: list[pathlib.Path],
    reports_jsonl: pathlib.Path,
    stage: str = "opcode-lab",
) -> pathlib.Path:
    reports_jsonl.parent.mkdir(parents=True, exist_ok=True)
    input_list_path = reports_jsonl.with_name("opcode-lab-inputs.json")
    input_list_path.write_text(
        json.dumps([str(path) for path in input_paths], indent=2, sort_keys=True) + "\n"
    )
    cmd = [
        str(guest_launcher),
        "--stage",
        stage,
        "--proof-type",
        "sp1",
        "--mode",
        "execute",
        "--sp1-prover",
        "local",
        "--elf",
        str(elf_path),
        "--input-list",
        str(input_list_path),
        "--jsonl-out",
        str(reports_jsonl),
    ]
    if stage in {"opcode-lab", "revm-opcode-lab"}:
        cmd.extend(["--sp1-execution-engine", "gas-estimator"])
    subprocess.run(cmd, check=True)
    return input_list_path


def run_proposal_guest_input(
    *,
    guest_launcher: pathlib.Path,
    guest_input: pathlib.Path,
    proof_type: str,
    case_name: str,
    target_raw_gas: int,
    target_count: int,
    out: pathlib.Path,
    risc0_execution_po2: int = 20,
) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    report_path = out.with_name(f"{out.stem}.guest-launcher.json")
    trace_path = out.with_name(f"{out.stem}.proposal-trace.json.gz")
    trace_summary_path = out.with_name(f"{out.stem}.proposal-trace.summary.json")
    trace = None
    if proof_type == "sp1":
        subprocess.run(
            [
                str(guest_launcher),
                "--stage",
                "proposal-trace",
                "--proof-type",
                "native",
                "--mode",
                "execute",
                "--input",
                str(guest_input),
                "--json-out",
                str(trace_path),
            ],
            check=True,
        )
        trace = json.loads(trace_summary_path.read_text())
    cmd = [
        str(guest_launcher),
        "--stage",
        "proposal",
        "--proof-type",
        proof_type,
        "--mode",
        "execute",
        "--input",
        str(guest_input),
        "--json-out",
        str(report_path),
    ]
    if proof_type == "sp1":
        cmd.extend(["--sp1-prover", "local"])
    elif proof_type == "risc0":
        cmd.extend(["--risc0-execution-po2", str(risc0_execution_po2)])
    else:
        raise ValueError("run-proposal supports proof_type sp1 or risc0")
    subprocess.run(cmd, check=True)

    report = json.loads(report_path.read_text())
    case = {
        "case": case_name,
        "kind": "proposal",
        "proof_type": proof_type,
        "guest_input": str(guest_input),
        "target_count": target_count,
        "target_raw_gas": target_raw_gas,
    }
    if trace is not None:
        case.update(join_proposal_trace_and_sp1(trace, report))
        case["proposal_trace"] = str(trace_path)
        case["proposal_trace_summary"] = str(trace_summary_path)
    out.write_text(json.dumps(raw_run_from_report(case, report), sort_keys=True) + "\n")


def _normalized_hex(value: Any, *, field_name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be a hex string")
    normalized = value.lower()
    if not normalized.startswith("0x"):
        normalized = "0x" + normalized
    try:
        bytes.fromhex(normalized[2:])
    except ValueError as exc:
        raise ValueError(f"{field_name} must be a hex string") from exc
    return normalized


def join_proposal_trace_and_sp1(
    trace: Mapping[str, Any], report: Mapping[str, Any]
) -> dict[str, Any]:
    """Validate the only supported trace/SP1 join keys and return a compact summary."""
    trace_hash = trace.get("guest_input_sha256")
    report_hash = report.get("guest_input_sha256")
    if (
        not isinstance(trace_hash, str)
        or len(trace_hash) != 66
        or not isinstance(report_hash, str)
        or trace_hash != report_hash
    ):
        raise ValueError("trace/SP1 GuestInput hash mismatch or missing hash")
    if trace.get("status") != "complete" or trace.get("parity_passed") is not True:
        raise ValueError("proposal trace did not pass the ordinary/traced A/B gate")
    if trace.get("partial_block_count") != 0:
        raise ValueError("proposal trace contains rejected partial block diagnostics")
    trace_output = _normalized_hex(trace.get("public_output"), field_name="trace public output")
    sp1_output = _normalized_hex(report.get("public_values"), field_name="SP1 public output")
    if trace_output != sp1_output:
        raise ValueError("trace/SP1 public output mismatch")
    block_count = trace.get("block_count")
    if not isinstance(block_count, int) or block_count <= 0:
        raise ValueError("complete proposal trace must contain block rows")
    return {
        "guest_input_sha256": trace_hash,
        "public_output": trace_output,
        "trace_ab_passed": True,
        "trace_block_count": block_count,
    }


def _formal_actual_raw_gas_map(
    case: Mapping[str, Any], trace: Mapping[str, Any]
) -> dict[str, str]:
    executed_count = trace.get("executed_target_count")
    executed_raw_gas = trace.get("executed_target_raw_gas")
    target_count = case.get("target_count")
    target_raw_gas = case.get("target_raw_gas")
    if any(
        isinstance(value, bool) or not isinstance(value, int) or value < 0
        for value in (executed_count, executed_raw_gas, target_count, target_raw_gas)
    ):
        raise ValueError("formal relation trace has invalid executed target raw gas")
    if executed_count != target_count or executed_raw_gas != target_count * target_raw_gas:
        raise ValueError("formal relation executed target raw gas differs from fixture units")
    non_target_counts = trace.get("non_target_counts")
    non_target_raw_gas = trace.get("non_target_raw_gas")
    total_raw_gas = trace.get("total_raw_gas")
    if (
        not isinstance(non_target_counts, Mapping)
        or isinstance(non_target_raw_gas, bool)
        or not isinstance(non_target_raw_gas, int)
        or non_target_raw_gas < 0
        or isinstance(total_raw_gas, bool)
        or not isinstance(total_raw_gas, int)
        or total_raw_gas != executed_raw_gas + non_target_raw_gas
    ):
        raise ValueError("formal relation total raw gas differs from host trace components")
    target_opcode = parse_opcode(case.get("opcode"))
    actual: dict[str, int] = {f"opcode:0x{target_opcode:02x}": executed_raw_gas}
    unresolved_non_target_keys: list[str] = []
    for raw_key, count in non_target_counts.items():
        if isinstance(count, bool) or not isinstance(count, int) or count < 0:
            raise ValueError("formal relation trace has invalid non-target opcode counts")
        try:
            component, opcode = _parse_schedule_key(raw_key)
        except (TypeError, ValueError):
            continue
        if component != "opcode" or opcode == 0x00:
            continue
        key = f"opcode:0x{opcode:02x}"
        if opcode not in PURE_OPCODE_DEFAULTS:
            if count:
                unresolved_non_target_keys.append(key)
            continue
        actual[key] = actual.get(key, 0) + PURE_OPCODE_DEFAULTS[opcode][2] * count
    unresolved_raw_gas = total_raw_gas - sum(actual.values())
    if unresolved_raw_gas < 0:
        raise ValueError("formal relation non-target raw gas differs from opcode counts")
    if unresolved_raw_gas:
        if len(unresolved_non_target_keys) != 1:
            raise ValueError("formal relation non-target raw gas is ambiguous")
        key = unresolved_non_target_keys[0]
        actual[key] = actual.get(key, 0) + unresolved_raw_gas
    if sum(actual.values()) != total_raw_gas:
        raise ValueError("formal relation non-target raw gas differs from opcode counts")
    return {key: str(value) for key, value in sorted(actual.items()) if value}


def validate_formal_dynamic_raw_gas_preflight(
    rows: Iterable[Mapping[str, Any]],
) -> None:
    """Require executed dynamic scenario totals to span the frozen range before publish."""
    grouped: dict[str, dict[str, list[Mapping[str, Any]]]] = {}
    for row in rows:
        dynamic_key = row.get("dynamic_key")
        if (
            row.get("purpose") != FORMAL_RELATION_PURPOSE
            or dynamic_key is None
            or row.get("lane") != "target"
            or row.get("diagnostic_count") != 1
        ):
            continue
        grouped.setdefault(str(dynamic_key), {}).setdefault(
            str(row.get("relation_id")), []
        ).append(row)
    if set(grouped) != set(DYNAMIC_RAW_GAS_KEYS):
        raise ValueError("formal dynamic raw-gas preflight is incomplete")
    for key, relations in grouped.items():
        observations: list[tuple[str, int]] = []
        if len(relations) != 3:
            raise ValueError(f"formal dynamic relation {key} must have three scenarios")
        for relation_rows in relations.values():
            ordered = sorted(
                relation_rows, key=lambda row: int(row.get("repeat_index", -1))
            )
            if [row.get("repeat_index") for row in ordered] != [0, 1, 2]:
                raise ValueError("formal dynamic preflight requires exactly three repeats")
            values = []
            for row in ordered:
                trace = row.get("controlled_trace")
                if not isinstance(trace, Mapping):
                    raise ValueError("formal dynamic preflight is missing the executed trace")
                count = trace.get("executed_target_count")
                total = trace.get("executed_target_raw_gas")
                if count != 1 or isinstance(total, bool) or not isinstance(total, int) or total <= 0:
                    raise ValueError("formal dynamic preflight has invalid executed target raw gas")
                values.append(total)
            if len(set(values)) != 1:
                raise ValueError("formal dynamic preflight executed totals are nondeterministic")
            observations.append((str(ordered[0].get("relation_split")), values[0]))
        canonical = [total for split, total in observations if split == "canonical"]
        holdouts = sorted(
            total for split, total in observations if split == "dynamic_holdout"
        )
        if (
            len(canonical) != 1
            or len(holdouts) != 2
            or len({canonical[0], *holdouts}) != 3
            or holdouts[0] < 2 * canonical[0]
            or holdouts[1] < 4 * canonical[0]
        ):
            raise ValueError(f"formal dynamic relation {key} fails actual 1x/2x/4x preflight")


def raw_run_from_report(case: dict[str, Any], report: dict[str, Any]) -> dict[str, Any]:
    workload_kind = case.get("kind")
    if workload_kind in {"opcode", "precompile"}:
        validate_sp1_execution_provenance(report, workload_kind=workload_kind)
    raw_run = {**case, **report}
    controlled_trace = raw_run.get("controlled_trace")
    if isinstance(controlled_trace, Mapping):
        trace_kind = controlled_trace.get("kind", "revm_opcode")
        if trace_kind == "precompile":
            identity_fields = (
                (controlled_trace.get("address"), parse_opcode(case.get("address"))),
                (controlled_trace.get("target_count"), case.get("target_count")),
                (controlled_trace.get("target_raw_gas"), case.get("target_raw_gas")),
                (controlled_trace.get("lane"), case.get("lane")),
                (controlled_trace.get("pair_id"), case.get("pair_id")),
            )
        elif trace_kind == "revm_opcode":
            identity_fields = (
                (controlled_trace.get("target_opcode"), parse_opcode(case.get("opcode"))),
                (controlled_trace.get("declared_target_count"), case.get("target_count")),
                (
                    controlled_trace.get("declared_target_raw_gas"),
                    case.get("target_raw_gas"),
                ),
                (controlled_trace.get("tx_gas_limit"), case.get("tx_gas_limit")),
            )
        else:
            raise ValueError("unknown controlled trace kind")
        if any(actual != expected for actual, expected in identity_fields):
            raise ValueError("controlled trace identity does not match case")
        workload_id = controlled_trace.get("workload_id")
        backend_input_sha256 = controlled_trace.get("backend_input_sha256")
        if not _is_sha256(workload_id) or not _is_sha256(backend_input_sha256):
            raise ValueError("controlled trace identity is missing a canonical SHA256")
        raw_run["workload_id"] = workload_id
        raw_run["backend_input_sha256"] = backend_input_sha256
        if case.get("purpose") == FORMAL_RELATION_PURPOSE:
            raw_run["actual_raw_gas_by_key"] = _formal_actual_raw_gas_map(
                case, controlled_trace
            )
        if trace_kind == "precompile":
            raw_run["pair_id"] = controlled_trace["pair_id"]
            raw_run["isolation"] = {
                "status": "passed",
                "input_size": controlled_trace.get("input_len"),
                "output_size": controlled_trace.get("output_len"),
                "loop_iterations": controlled_trace.get("loop_iterations"),
                "folded_bytes_per_iteration": controlled_trace.get(
                    "folded_bytes_per_iteration"
                ),
            }
        else:
            raw_run["isolation"] = {
                "status": "passed",
                "bytecode_size": controlled_trace.get("bytecode_len"),
                "input_size": controlled_trace.get("backend_input_len"),
                "non_target_counts": controlled_trace.get("non_target_counts"),
                "non_target_raw_gas": controlled_trace.get("non_target_raw_gas"),
                "tx_gas_limit": controlled_trace.get("tx_gas_limit"),
            }
    if "prover_gas" not in raw_run and "gas" in raw_run:
        raw_run["prover_gas"] = raw_run["gas"]
    primary_metric = raw_run.get("primary_workload_metric")
    if isinstance(primary_metric, dict):
        label = primary_metric.get("label")
        count = primary_metric.get("count")
        if label is not None and count is not None:
            raw_run.setdefault("workload_metric", label)
            raw_run.setdefault("workload_value", count)
    if "workload_metric" not in raw_run and "prover_gas" in raw_run:
        raw_run["workload_metric"] = "prover_gas"
    if "workload_value" not in raw_run and "prover_gas" in raw_run:
        raw_run["workload_value"] = raw_run["prover_gas"]
    return raw_run


def reject_matched_control_diagnostics(
    rows: Iterable[Mapping[str, Any]], *, context: str
) -> None:
    if any(row.get("purpose") == MATCHED_CONTROL_PURPOSE for row in rows):
        raise ValueError(f"{context} rejects matched-control diagnostic rows")


def validate_fixture_purpose(
    case: Mapping[str, Any], *, expected_purpose: str | None
) -> None:
    purpose = case.get("purpose")
    if expected_purpose is None:
        if purpose is not None:
            raise ValueError("formal run rejects diagnostic fixture purpose")
        return
    if purpose != expected_purpose:
        raise ValueError(
            f"diagnostic run requires fixture purpose {expected_purpose}"
        )


def iter_jsonl(path: pathlib.Path) -> Iterable[dict[str, Any]]:
    with path.open() as fh:
        for line in fh:
            line = line.strip()
            if line:
                yield json.loads(line)


def run_metric_value(run: dict[str, Any], metric: str) -> float:
    if metric in run:
        return float(run[metric])
    if run.get("workload_metric") == metric and "workload_value" in run:
        return float(run["workload_value"])
    raise KeyError(f"run is missing workload metric {metric}")


def fit_case(runs: list[dict[str, Any]], metric: str = "prover_gas") -> FitResult:
    if len(runs) < 2:
        raise ValueError("at least two runs are required")
    case_name = str(runs[0]["case"])
    raw_gas = float(runs[0]["target_raw_gas"])
    xs = [float(run["target_count"]) for run in runs]
    ys = [run_metric_value(run, metric) for run in runs]
    mean_x = statistics.fmean(xs)
    mean_y = statistics.fmean(ys)
    denom = sum((x - mean_x) ** 2 for x in xs)
    if denom == 0:
        raise ValueError("target_count must vary")
    slope = sum((x - mean_x) * (y - mean_y) for x, y in zip(xs, ys)) / denom
    intercept = mean_y - slope * mean_x
    predicted = [intercept + slope * x for x in xs]
    ss_res = sum((y - y_hat) ** 2 for y, y_hat in zip(ys, predicted))
    ss_tot = sum((y - mean_y) ** 2 for y in ys)
    r2 = 1.0 if ss_tot == 0 else 1.0 - (ss_res / ss_tot)
    return FitResult(
        case=case_name,
        metric=metric,
        sample_count=len(runs),
        slope_per_operation=slope,
        slope_per_raw_gas=slope / raw_gas,
        intercept=intercept,
        r2=r2,
    )


def fit_report(
    runs_path: pathlib.Path,
    out_dir: pathlib.Path,
    metric: str = "prover_gas",
) -> list[FitResult]:
    grouped: dict[str, list[dict[str, Any]]] = {}
    for run in iter_jsonl(runs_path):
        grouped.setdefault(str(run["case"]), []).append(run)
    results = [
        fit_case(sorted(items, key=lambda item: item["target_count"]), metric=metric)
        for items in grouped.values()
    ]
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "fit.json").write_text(
        json.dumps([asdict(result) for result in results], indent=2, sort_keys=True) + "\n"
    )
    (out_dir / "coefficients.json").write_text(
        json.dumps(
            {result.case: result.slope_per_raw_gas for result in results},
            indent=2,
            sort_keys=True,
        )
        + "\n"
    )
    write_markdown_report(out_dir / "uzen-vs-fit.md", results, metric=metric)
    return results


def compute_damage_result(
    *,
    case: str,
    kind: str,
    workload_metric: str = "prover_gas",
    eth_gas_per_unit: int,
    measured_workload_per_unit: float,
    zkgas_multiplier: int,
    eth_gas_limit: int,
    zk_gas_limit: int,
    r2: float = 1.0,
) -> DamageResult:
    if eth_gas_per_unit <= 0:
        raise ValueError("eth_gas_per_unit must be positive")
    if zkgas_multiplier < 0:
        raise ValueError("zkgas_multiplier must be non-negative")
    if eth_gas_limit < 0:
        raise ValueError("eth_gas_limit must be non-negative")
    if zk_gas_limit < 0:
        raise ValueError("zk_gas_limit must be non-negative")

    eth_only_units = eth_gas_limit // eth_gas_per_unit
    eth_only_damage = eth_only_units * measured_workload_per_unit
    zkgas_per_unit = eth_gas_per_unit * zkgas_multiplier
    zkgas_units = eth_only_units if zkgas_per_unit == 0 else zk_gas_limit // zkgas_per_unit
    candidate_units = min(eth_only_units, zkgas_units)
    candidate_damage = candidate_units * measured_workload_per_unit
    attack_reduction = (
        0.0 if eth_only_damage == 0 else 1.0 - (candidate_damage / eth_only_damage)
    )
    binding_resource = "zkgas" if candidate_units < eth_only_units else "eth"
    return DamageResult(
        case=case,
        kind=kind,
        workload_metric=workload_metric,
        eth_gas_per_unit=eth_gas_per_unit,
        measured_workload_per_unit=measured_workload_per_unit,
        damage_ratio=measured_workload_per_unit / eth_gas_per_unit,
        r2=r2,
        eth_only_units=eth_only_units,
        eth_only_damage=eth_only_damage,
        zkgas_multiplier=zkgas_multiplier,
        zkgas_per_unit=zkgas_per_unit,
        candidate_units=candidate_units,
        candidate_damage=candidate_damage,
        attack_reduction=attack_reduction,
        binding_resource=binding_resource,
    )


def current_uzen_multiplier(case: CaseSpec, schedule: UnzenSchedule | None = None) -> int:
    schedule = schedule or current_uzen_schedule()
    if case.kind == "opcode":
        if case.opcode is None:
            raise ValueError(f"opcode case {case.name} is missing opcode")
        try:
            return schedule.opcode_multipliers[case.opcode]
        except KeyError as exc:
            raise ValueError(f"no current-Unzen opcode multiplier for {case.name}") from exc
    if case.kind == "precompile":
        if case.address is None:
            raise ValueError(f"precompile case {case.name} is missing address")
        try:
            return schedule.precompile_multipliers[case.address]
        except KeyError as exc:
            raise ValueError(f"no current-Unzen precompile multiplier for {case.name}") from exc
    raise ValueError(f"unknown case kind: {case.kind}")


def build_inventory(
    manifest: Manifest, schedule: UnzenSchedule | None = None
) -> list[InventoryRow]:
    schedule = schedule or current_uzen_schedule()
    measured_opcodes = {
        case.opcode: case.name
        for case in manifest.cases
        if case.kind == "opcode" and case.opcode is not None
    }
    measured_precompiles = {
        case.address: case.name
        for case in manifest.cases
        if case.kind == "precompile" and case.address is not None
    }

    rows = []
    for opcode, multiplier in sorted(schedule.opcode_multipliers.items()):
        manifest_case = measured_opcodes.get(opcode)
        rows.append(
            InventoryRow(
                kind="opcode",
                identifier=f"0x{opcode:02x}",
                name=UZEN_OPCODE_NAMES.get(opcode, f"opcode_0x{opcode:02x}"),
                multiplier=multiplier,
                status=manifest_case and "measured" or classify_opcode_status(opcode),
                manifest_case=manifest_case,
            )
        )
    for address, multiplier in sorted(schedule.precompile_multipliers.items()):
        manifest_case = measured_precompiles.get(address)
        rows.append(
            InventoryRow(
                kind="precompile",
                identifier=f"0x{address:02x}",
                name=UZEN_PRECOMPILE_NAMES.get(address, f"precompile_0x{address:x}"),
                multiplier=multiplier,
                status=(
                    "measured"
                    if manifest_case
                    else "needs_precompile_body"
                    if address in PRECOMPILE_BODY_DEFAULTS
                    else "unsupported_by_experiment"
                ),
                manifest_case=manifest_case,
            )
        )
    return rows


def classify_opcode_status(opcode: int) -> str:
    if opcode in SPAWN_WRAPPER_OPCODES:
        return "needs_spawn_wrapper"
    if opcode in ZERO_OR_HALTING_OPCODES:
        return "not_measured_zero_or_halting"
    if opcode in STATE_OR_REVM_OPCODES:
        return "needs_state_or_revm"
    if opcode in PLANNED_PURE_OPCODE_OPCODES:
        return "planned_pure_opcode"
    return "unsupported_by_experiment"


def inventory_report(manifest_path: pathlib.Path, out_dir: pathlib.Path) -> list[InventoryRow]:
    schedule = current_uzen_schedule()
    manifest = load_manifest(manifest_path, schedule=schedule)
    rows = build_inventory(manifest, schedule=schedule)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "inventory.json").write_text(
        json.dumps([asdict(row) for row in rows], indent=2, sort_keys=True) + "\n"
    )
    write_inventory_markdown_report(out_dir / "inventory.md", rows)
    return rows


def damage_report(
    *,
    fit_path: pathlib.Path,
    manifest_path: pathlib.Path,
    eth_gas_limit: int,
    zk_gas_limit: int,
    out_dir: pathlib.Path,
) -> list[DamageResult]:
    schedule = current_uzen_schedule()
    manifest = load_manifest(manifest_path, schedule=schedule)
    case_by_name = {case.name: case for case in manifest.cases}
    fit_rows = json.loads(fit_path.read_text())
    workload_metric = str(fit_rows[0].get("metric", "prover_gas")) if fit_rows else "prover_gas"
    results = []
    for row in fit_rows:
        row_metric = str(row.get("metric", workload_metric))
        if row_metric != workload_metric:
            raise ValueError("damage report requires one workload metric per fit file")
        case = case_by_name[str(row["case"])]
        results.append(
            compute_damage_result(
                case=case.name,
                kind=case.kind,
                workload_metric=workload_metric,
                eth_gas_per_unit=case.target_raw_gas,
                measured_workload_per_unit=float(row["slope_per_operation"]),
                zkgas_multiplier=current_uzen_multiplier(case, schedule),
                eth_gas_limit=eth_gas_limit,
                zk_gas_limit=zk_gas_limit,
                r2=float(row["r2"]),
            )
        )

    results = sorted(results, key=lambda item: item.eth_only_damage, reverse=True)
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / "damage.json").write_text(
        json.dumps([asdict(result) for result in results], indent=2, sort_keys=True) + "\n"
    )
    write_damage_markdown_report(
        out_dir / "damage.md",
        results=results,
        eth_gas_limit=eth_gas_limit,
        zk_gas_limit=zk_gas_limit,
        workload_metric=workload_metric,
    )
    return results


def write_inventory_markdown_report(path: pathlib.Path, rows: list[InventoryRow]) -> None:
    lines = [
        "# ZKGas Coverage Inventory",
        "",
        "| Kind | ID | Name | Multiplier | Status | Manifest case |",
        "| --- | --- | --- | ---: | --- | --- |",
    ]
    for row in rows:
        lines.append(
            f"| {row.kind} | {row.identifier} | {row.name} | {row.multiplier} | "
            f"{row.status} | {row.manifest_case or ''} |"
        )
    path.write_text("\n".join(lines) + "\n")


def write_markdown_report(
    path: pathlib.Path,
    results: list[FitResult],
    metric: str = "prover_gas",
) -> None:
    lines = [
        "# Unzen Vs Fitted Workload Metric",
        "",
        f"- Workload metric: `{metric}`",
        "",
        "| Case | Samples | Slope/op | Slope/raw-gas | R2 |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for result in sorted(results, key=lambda item: item.case):
        lines.append(
            f"| {result.case} | {result.sample_count} | "
            f"{result.slope_per_operation:.6g} | {result.slope_per_raw_gas:.6g} | {result.r2:.6g} |"
        )
    path.write_text("\n".join(lines) + "\n")


def write_damage_markdown_report(
    path: pathlib.Path,
    *,
    results: list[DamageResult],
    eth_gas_limit: int,
    zk_gas_limit: int,
    workload_metric: str = "prover_gas",
) -> None:
    lines = [
        "# ZKGas Workload Damage Report",
        "",
        f"- Eth gas limit: `{eth_gas_limit}`",
        f"- ZK gas limit: `{zk_gas_limit}`",
        f"- Workload metric: `{workload_metric}`",
        "- Candidate table: current Unzen multipliers",
        "- Realistic workload impact: pending real block/app contribution accounting",
        "",
        "## Metric Meaning",
        "",
        f"- `Workload/unit`: fitted `{workload_metric}` increase per target opcode or precompile body execution.",
        "- `Damage ratio`: `Workload/unit / Eth gas/unit`, the measured zk workload reachable per ETH gas.",
        "- `R2`: linear-fit quality for the smoke variants. Low R2 means template noise or too few counts; do not use that slope as a coefficient without a better sweep.",
        "- `Eth-only units`: max target executions under the ETH gas limit only.",
        "- `Eth-only damage`: `Eth-only units * Workload/unit`, before any zkgas accounting.",
        "- `ZK gas/unit`: `Eth gas/unit * current-Unzen multiplier`.",
        "- `Candidate units`: max target executions after applying both ETH gas and zkgas limits.",
        "- `Candidate damage`: `Candidate units * Workload/unit`, the workload still reachable under current zkgas accounting.",
        "- `Attack reduction`: reduction from eth-only damage after applying the current zkgas limit.",
        "- `binding_resource = zkgas`: the current zkgas limit caps this homogeneous workload before the ETH gas limit. A 30M gas block filled with that case would be filtered by zkgas first.",
        "- `binding_resource = eth`: the ETH gas limit caps this workload first; current zkgas has headroom for that homogeneous case.",
        "",
        "## Eth-Only Damage Frontier",
        "",
        "| Case | Kind | Eth gas/unit | Workload/unit | Damage ratio | R2 | Eth-only units | Eth-only damage |",
        "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    for result in sorted(results, key=lambda item: item.damage_ratio, reverse=True):
        lines.append(
            f"| {result.case} | {result.kind} | {result.eth_gas_per_unit} | "
            f"{result.measured_workload_per_unit:.6g} | {result.damage_ratio:.6g} | "
            f"{result.r2:.6g} | {result.eth_only_units} | {result.eth_only_damage:.6g} |"
        )

    lines.extend(
        [
            "",
            "## Current-Unzen Containment",
            "",
            "| Case | Multiplier | ZK gas/unit | Candidate units | Candidate damage | "
            "Attack reduction | Binding resource | R2 |",
            "| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: |",
        ]
    )
    for result in sorted(results, key=lambda item: item.candidate_damage, reverse=True):
        lines.append(
            f"| {result.case} | {result.zkgas_multiplier} | {result.zkgas_per_unit} | "
            f"{result.candidate_units} | {result.candidate_damage:.6g} | "
            f"{result.attack_reduction:.2%} | {result.binding_resource} | {result.r2:.6g} |"
        )

    path.write_text("\n".join(lines) + "\n")


FINAL_CORPUS_FIXTURE_DIR = REPO_ROOT / "tests/fixtures/risc0-zkgas/2026-09-02-m2-aggregation-direct-v3"
GENERATED_EXPERIMENT_PREFIXES = (
    "experiments/opcode-gas/corpora/", "experiments/opcode-gas/manifests/proposals/",
    "experiments/opcode-gas/runs/", "experiments/opcode-gas/validations/",
)
Q_FORMULA = ["proposal_startup", "block_base", "tx_base", "native_value_transfer"]
OUT_OF_FIT_CHECKPOINTS = {"4": 8, "16": 32, "64": 128, "256": 512, "1024": 2048}
CONTROLLED_PREFIXES = (
    (0, 1, 2, 4),
    (0, 1, 2, 4, 8, 16),
    (0, 1, 2, 4, 8, 16, 32, 64),
    (0, 1, 2, 4, 8, 16, 32, 64, 128, 256),
    (0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024),
)
SP1_GAS_TRACE_CHUNK_THRESHOLD = 134_217_728
SP1_GAS_TRACE_CHUNK_SLOTS = 2


def sp1_execution_parameters() -> dict[str, Any]:
    return {
        "mode": "execute",
        "prover": "local",
        "primary_api": "ExecutionReport::gas",
        "engines": {
            "opcode": "gas-estimator",
            "precompile": "standard",
            "overhead": "standard",
        },
        "gas_estimator": {
            "gas_trace_chunk_threshold": SP1_GAS_TRACE_CHUNK_THRESHOLD,
            "gas_trace_chunk_slots": SP1_GAS_TRACE_CHUNK_SLOTS,
        },
    }


def validate_sp1_execution_provenance(
    row: Mapping[str, Any], *, workload_kind: str
) -> None:
    expected_engine = sp1_execution_parameters()["engines"].get(workload_kind)
    if expected_engine is None:
        raise ValueError(f"unknown SP1 workload kind {workload_kind}")
    if row.get("sp1_execution_engine") != expected_engine:
        raise ValueError(
            f"SP1 execution provenance requires {workload_kind} engine {expected_engine}"
        )
    threshold = row.get("sp1_gas_trace_chunk_threshold")
    slots = row.get("sp1_gas_trace_chunk_slots")
    if workload_kind == "opcode":
        if (
            type(threshold) is not int
            or threshold != SP1_GAS_TRACE_CHUNK_THRESHOLD
            or type(slots) is not int
            or slots != SP1_GAS_TRACE_CHUNK_SLOTS
        ):
            raise ValueError("SP1 execution provenance has noncanonical gas chunk settings")
    elif threshold is not None or slots is not None:
        raise ValueError("SP1 execution provenance has unexpected gas chunk settings")


def _decimal(value: Any, *, label: str) -> Decimal:
    if isinstance(value, bool):
        raise ValueError(f"{label} must be a finite decimal")
    try:
        result = Decimal(str(value))
    except (InvalidOperation, ValueError) as exc:
        raise ValueError(f"{label} must be a finite decimal") from exc
    if not result.is_finite():
        raise ValueError(f"{label} must be a finite decimal")
    return result


def _decimal_text(value: Decimal) -> str:
    if value == 0:
        return "0"
    text = format(value, "f")
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return text


def _parse_canonical_int_map(
    value: Any, *, label: str, allow_negative: bool = False
) -> dict[str, int]:
    if not isinstance(value, Mapping):
        raise ValueError(f"{label} must be an object")
    parsed: dict[str, int] = {}
    for key, raw in value.items():
        if not isinstance(key, str) or not key.startswith("opcode:0x"):
            raise ValueError(f"{label} has an invalid opcode key")
        if not isinstance(raw, str) or raw != str(int(raw)):
            raise ValueError(f"{label} must use canonical integer strings")
        number = int(raw)
        if (allow_negative and number == 0) or (not allow_negative and number <= 0):
            raise ValueError(f"{label} contains an invalid raw-gas total")
        parsed[key] = number
    if not parsed:
        if allow_negative:
            return {}
        raise ValueError(f"{label} must not be empty")
    return parsed


def controlled_workload_id(workload_spec: Mapping[str, Any], **_ignored: Any) -> str:
    return sha256_bytes(
        canonical_json({"kind": "controlled", "workload_spec": workload_spec})
    )


def controlled_execution_row_id(
    workload_id: str,
    *,
    backend: str,
    execution_engine: str,
    run_id: str,
    repeat_index: int,
    backend_input_sha256: str,
) -> str:
    return sha256_bytes(
        canonical_json(
            {
                "backend": backend,
                "backend_input_sha256": backend_input_sha256,
                "execution_engine": execution_engine,
                "kind": "controlled_execution",
                "repeat_index": repeat_index,
                "run_id": run_id,
                "workload_id": workload_id,
            }
        )
    )


def event_matches(spec: EventMatchSpec, operation: Mapping[str, Any]) -> bool:
    component = operation.get("kind", operation.get("component"))
    if component != spec.component:
        return False
    if spec.component == "opcode":
        try:
            identifier = parse_opcode(operation.get("opcode"))
        except (TypeError, ValueError):
            return False
        if identifier != spec.opcode:
            return False
        if spec.spawned is not None and operation.get("spawned") is not spec.spawned:
            return False
        if (
            spec.dispatch_status is not None
            and operation.get("dispatch_status") != spec.dispatch_status
        ):
            return False
    else:
        try:
            identifier = parse_opcode(operation.get("address"))
        except (TypeError, ValueError):
            return False
        if identifier != spec.address:
            return False
    return True


def resolve_measurement_key(
    manifest: Manifest, operation: Mapping[str, Any]
) -> tuple[str | None, str]:
    if operation.get("dispatch_status") == "selected_not_dispatched":
        return None, "selected_not_dispatched"
    matches = [
        key
        for key in manifest.measurement_keys
        if event_matches(key.event_match, operation)
        and operation.get("pricing_basis") == key.pricing_basis
    ]
    if not matches:
        return None, "no_measurement_key"
    if len(matches) != 1:
        return None, "ambiguous_measurement_key"
    return matches[0].id, "measured"


def _validate_repeat_point(point: Mapping[str, Any]) -> tuple[Decimal, Decimal | None, list[str]]:
    reasons: list[str] = []
    identity_fields = (
        "case_input_sha256_repeats",
        "exit_code_repeats",
        "public_values_repeats",
    )
    for field_name in identity_fields:
        values = point.get(field_name)
        if not isinstance(values, list) or len(values) != 3 or len(set(map(str, values))) != 1:
            reasons.append("repeat_identity")
    exit_codes = point.get("exit_code_repeats")
    if isinstance(exit_codes, list) and any(value != 0 for value in exit_codes):
        reasons.append("repeat_identity")
    gas_repeats = point.get("prover_gas_repeats")
    if not isinstance(gas_repeats, list) or len(gas_repeats) != 3:
        reasons.append("repeat_identity")
        gas = Decimal(0)
    else:
        gas_values = [_decimal(value, label="proverGas repeat") for value in gas_repeats]
        if max(gas_values) - min(gas_values) != 0:
            reasons.append("repeat_noise_p")
        gas = gas_values[0]
    secondary_repeats = point.get("instruction_count_repeats")
    secondary: Decimal | None = None
    if isinstance(secondary_repeats, list) and len(secondary_repeats) == 3:
        secondary_values = [
            _decimal(value, label="instruction-count repeat") for value in secondary_repeats
        ]
        if max(secondary_values) - min(secondary_values) == 0:
            secondary = secondary_values[0]
    return gas, secondary, list(dict.fromkeys(reasons))


def _fit_decimal(xs: list[Decimal], ys: list[Decimal]) -> dict[str, Decimal | list[str]]:
    count = Decimal(len(xs))
    mean_x = sum(xs) / count
    mean_y = sum(ys) / count
    denominator = sum((value - mean_x) ** 2 for value in xs)
    if denominator == 0:
        return {"reasons": ["primary_slope"]}
    slope = sum(
        (x_value - mean_x) * (y_value - mean_y)
        for x_value, y_value in zip(xs, ys)
    ) / denominator
    intercept = mean_y - slope * mean_x
    predicted = [intercept + slope * value for value in xs]
    residuals = [actual - estimate for actual, estimate in zip(ys, predicted)]
    ss_res = sum(value * value for value in residuals)
    ss_total = sum((value - mean_y) ** 2 for value in ys)
    signal = max(ys) - min(ys)
    reasons = []
    if slope <= 0:
        reasons.append("primary_slope")
    if signal < max(Decimal(1000), abs(ys[0]) * Decimal("0.01")):
        reasons.append("primary_signal")
    if ss_total == 0:
        r2 = Decimal(0)
        reasons.append("primary_r2")
    else:
        r2 = Decimal(1) - ss_res / ss_total
        if r2 < Decimal("0.995"):
            reasons.append("primary_r2")
    if len(xs) <= 2:
        stderr = Decimal("Infinity")
        reasons.append("primary_slope_stderr")
    else:
        stderr = (ss_res / Decimal(len(xs) - 2) / denominator).sqrt()
        if slope <= 0 or stderr / slope > Decimal("0.05"):
            reasons.append("primary_slope_stderr")
    max_residual = max(abs(value) for value in residuals)
    if signal <= 0 or max_residual > signal * Decimal("0.02"):
        reasons.append("primary_residual")
    return {
        "slope": slope,
        "intercept": intercept,
        "r2": r2,
        "stderr": stderr,
        "signal": signal,
        "max_residual": max_residual,
        "reasons": list(dict.fromkeys(reasons)),
    }


def _isolation_reasons(
    points: list[Mapping[str, Any]], *, require_tx_gas_limit: bool
) -> list[str]:
    identities = []
    for point in points:
        isolation = point.get("isolation")
        if not isinstance(isolation, Mapping) or isolation.get("status") != "passed":
            return ["confounded_template"]
        tx_gas_limit = isolation.get("tx_gas_limit")
        if require_tx_gas_limit and (
            type(tx_gas_limit) is not int or tx_gas_limit <= 0
        ):
            return ["confounded_template"]
        identities.append(
            canonical_json(
                {
                    "bytecode_size": isolation.get("bytecode_size"),
                    "input_size": isolation.get("input_size"),
                    "non_target_counts": isolation.get("non_target_counts"),
                    "non_target_raw_gas": isolation.get("non_target_raw_gas"),
                    "tx_gas_limit": isolation.get("tx_gas_limit"),
                }
            )
        )
    return [] if len(set(identities)) == 1 else ["confounded_template"]


def evaluate_controlled_sweep(
    observations: Iterable[Mapping[str, Any]],
    *,
    pricing_basis: str,
    target_raw_gas: int | None,
    generator_max_count: int,
    require_tx_gas_limit: bool | None = None,
) -> dict[str, Any]:
    if require_tx_gas_limit is None:
        require_tx_gas_limit = pricing_basis == "raw_gas_slope"
    observation_list = list(observations)
    points = {int(point["count"]): point for point in observation_list}
    if len(points) != len(observation_list):
        return {"status": "rejected", "reasons": ["duplicate_count"]}
    parsed: dict[int, tuple[Decimal, Decimal | None, list[str]]] = {
        count: _validate_repeat_point(point) for count, point in points.items()
    }
    repeat_reasons = [reason for _, _, reasons in parsed.values() for reason in reasons]
    if repeat_reasons:
        return {
            "status": "rejected",
            "reasons": list(dict.fromkeys(repeat_reasons)),
        }
    selected: tuple[int, ...] | None = None
    fit: dict[str, Any] | None = None
    last_reasons: list[str] = []
    for prefix in CONTROLLED_PREFIXES:
        if max(prefix) > generator_max_count or any(count not in points for count in prefix):
            continue
        isolation_reasons = _isolation_reasons(
            [points[count] for count in prefix],
            require_tx_gas_limit=require_tx_gas_limit,
        )
        if isolation_reasons:
            return {"status": "rejected", "reasons": isolation_reasons}
        fit = _fit_decimal(
            [Decimal(count) for count in prefix],
            [parsed[count][0] for count in prefix],
        )
        last_reasons = list(fit["reasons"])
        if not last_reasons:
            selected = prefix
            break
    if selected is None or fit is None:
        return {
            "status": "rejected",
            "reasons": list(dict.fromkeys(last_reasons + ["exhausted_sweep"])),
        }
    checkpoint_count = OUT_OF_FIT_CHECKPOINTS[str(max(selected))]
    if checkpoint_count > generator_max_count:
        return {
            "status": "rejected",
            "selected_counts": list(selected),
            "reasons": ["checkpoint_generator_bound"],
        }
    checkpoint = points.get(checkpoint_count)
    if checkpoint is None:
        return {
            "status": "rejected",
            "selected_counts": list(selected),
            "reasons": ["checkpoint_missing"],
        }
    isolation_reasons = _isolation_reasons(
        [points[count] for count in selected] + [checkpoint],
        require_tx_gas_limit=require_tx_gas_limit,
    )
    if isolation_reasons:
        return {
            "status": "rejected",
            "selected_counts": list(selected),
            "reasons": isolation_reasons,
        }
    base = parsed[0][0]
    observed_delta = parsed[checkpoint_count][0] - base
    predicted_delta = fit["slope"] * Decimal(checkpoint_count)
    if observed_delta <= 0 or predicted_delta <= 0:
        return {
            "status": "rejected",
            "selected_counts": list(selected),
            "reasons": ["checkpoint_nonpositive"],
        }
    ape = abs(predicted_delta - observed_delta) / observed_delta
    checkpoint_record = {
        "count": checkpoint_count,
        "observed_delta_p": _decimal_text(observed_delta),
        "predicted_delta_p": _decimal_text(predicted_delta),
        "ape_p": _decimal_text(ape),
        "status": "passed" if ape <= Decimal("0.10") else "failed",
    }
    if ape > Decimal("0.10"):
        return {
            "status": "rejected",
            "selected_counts": list(selected),
            "checkpoint": checkpoint_record,
            "reasons": ["extrapolation_check_failed"],
        }
    slope = fit["slope"]
    result = {
        "status": "accepted",
        "selected_counts": list(selected),
        "ols_counts": list(selected),
        "g_p": _decimal_text(slope),
        "intercept_p": _decimal_text(fit["intercept"]),
        "r2_p": _decimal_text(fit["r2"]),
        "slope_stderr_p": _decimal_text(fit["stderr"]),
        "signal_p": _decimal_text(fit["signal"]),
        "max_residual_p": _decimal_text(fit["max_residual"]),
        "checkpoint": checkpoint_record,
    }
    if pricing_basis == "raw_gas_slope":
        if target_raw_gas is None or target_raw_gas <= 0:
            return {"status": "rejected", "reasons": ["invalid_raw_gas"]}
        result["c_p"] = _decimal_text(slope / Decimal(target_raw_gas))
    elif pricing_basis == "fixed_per_event":
        if target_raw_gas not in (None, 0):
            return {"status": "rejected", "reasons": ["fixed_event_raw_gas"]}
        result["f_p"] = _decimal_text(slope)
    else:
        return {"status": "rejected", "reasons": ["pricing_basis"]}
    secondary_values = [parsed[count][1] for count in selected]
    if any(value is None for value in secondary_values):
        result["secondary"] = {"status": "failed", "reason": "repeat_noise_s"}
    else:
        secondary_fit = _fit_decimal(
            [Decimal(count) for count in selected],
            [value for value in secondary_values if value is not None],
        )
        result["secondary"] = {
            "status": "available" if not secondary_fit["reasons"] else "failed",
            "g_s": _decimal_text(secondary_fit["slope"]),
            "reasons": secondary_fit["reasons"],
        }
    return result


def evaluate_paired_precompile_sweep(
    observations: Iterable[Mapping[str, Any]],
    *,
    target_raw_gas: int,
    generator_max_count: int,
) -> dict[str, Any]:
    synthesized = []
    expected_shape: bytes | None = None
    for row in observations:
        shape = canonical_json(row.get("shape"))
        if expected_shape is None:
            expected_shape = shape
        if shape != expected_shape:
            return {"status": "rejected", "reasons": ["confounded_template"]}
        target = row["target"]
        control = row["control"]
        target_gas, target_secondary, target_reasons = _validate_repeat_point(target)
        control_gas, control_secondary, control_reasons = _validate_repeat_point(control)
        if target_reasons or control_reasons:
            return {
                "status": "rejected",
                "reasons": list(dict.fromkeys(target_reasons + control_reasons)),
            }
        point = {
            "count": row["count"],
            "prover_gas_repeats": [_decimal_text(target_gas - control_gas)] * 3,
            "instruction_count_repeats": (
                [_decimal_text(target_secondary - control_secondary)] * 3
                if target_secondary is not None and control_secondary is not None
                else []
            ),
            "case_input_sha256_repeats": ["0" * 64] * 3,
            "exit_code_repeats": [0, 0, 0],
            "public_values_repeats": ["paired"] * 3,
            "isolation": {
                "status": "passed",
                "bytecode_size": 0,
                "input_size": row.get("shape", {}).get("input_size"),
                "non_target_counts": row.get("shape"),
                "non_target_raw_gas": "0",
            },
        }
        synthesized.append(point)
    return evaluate_controlled_sweep(
        synthesized,
        pricing_basis="raw_gas_slope",
        target_raw_gas=target_raw_gas,
        generator_max_count=generator_max_count,
        require_tx_gas_limit=False,
    )


def _signed_relation_fit(
    counts: Mapping[int, tuple[Decimal, Decimal, Decimal]],
    *,
    generator_max_count: int,
) -> dict[str, Any]:
    """Fit target-minus-control responses while allowing either slope sign."""
    last_reasons: list[str] = []
    for prefix in CONTROLLED_PREFIXES:
        if max(prefix) > generator_max_count or any(count not in counts for count in prefix):
            continue
        xs = [Decimal(count) for count in prefix]
        ys = [counts[count][2] for count in prefix]
        n = Decimal(len(xs))
        mean_x = sum(xs) / n
        mean_y = sum(ys) / n
        denominator = sum((value - mean_x) ** 2 for value in xs)
        slope = sum(
            (x_value - mean_x) * (y_value - mean_y)
            for x_value, y_value in zip(xs, ys)
        ) / denominator
        intercept = mean_y - slope * mean_x
        predicted = [intercept + slope * value for value in xs]
        residuals = [actual - estimate for actual, estimate in zip(ys, predicted)]
        ss_res = sum(value * value for value in residuals)
        ss_total = sum((value - mean_y) ** 2 for value in ys)
        signal = abs(max(ys) - min(ys))
        baseline = abs(counts[0][2])
        reasons: list[str] = []
        if slope == 0:
            reasons.append("signed slope is zero")
        if signal < max(Decimal(1000), baseline * Decimal("0.01")):
            reasons.append("signed signal is too small")
        if ss_total == 0:
            r2 = Decimal(0)
            reasons.append("signed R2 gate failed")
        else:
            r2 = Decimal(1) - ss_res / ss_total
            if r2 < Decimal("0.995"):
                reasons.append("signed R2 gate failed")
        stderr = (ss_res / Decimal(len(xs) - 2) / denominator).sqrt()
        if slope == 0 or stderr / abs(slope) > Decimal("0.05"):
            reasons.append("signed slope stderr gate failed")
        max_residual = max(abs(value) for value in residuals)
        if signal == 0 or max_residual / signal > Decimal("0.02"):
            reasons.append("signed residual gate failed")
        last_reasons = reasons
        if reasons:
            continue

        checkpoint_count = OUT_OF_FIT_CHECKPOINTS[str(max(prefix))]
        if checkpoint_count > generator_max_count or checkpoint_count not in counts:
            last_reasons = ["signed checkpoint is missing"]
            continue
        observed = counts[checkpoint_count][2] - counts[0][2]
        predicted_checkpoint = slope * Decimal(checkpoint_count)
        if (
            observed == 0
            or predicted_checkpoint == 0
            or (observed > 0) != (predicted_checkpoint > 0)
        ):
            raise ValueError("formal relation checkpoint sign differs from fitted slope")
        ape = abs(predicted_checkpoint - observed) / abs(observed)
        if ape > Decimal("0.10"):
            raise ValueError("formal relation signed checkpoint APE gate failed")
        return {
            "selected_counts": list(prefix),
            "slope": slope,
            "intercept": intercept,
            "r2": r2,
            "stderr": stderr,
            "signal": signal,
            "max_residual": max_residual,
            "checkpoint": {
                "count": checkpoint_count,
                "observed_delta_p": _decimal_text(observed),
                "predicted_delta_p": _decimal_text(predicted_checkpoint),
                "ape_p": _decimal_text(ape),
                "status": "passed",
            },
        }
    raise ValueError(
        "formal relation quality gate failed: "
        + ", ".join(last_reasons or ["exhausted signed prefix search"])
    )


def _relation_row_map(row: Mapping[str, Any], field: str) -> dict[str, int]:
    return _parse_canonical_int_map(
        row.get(field),
        label=field.replace("_", " "),
        allow_negative=field == "signed_raw_gas_by_key",
    )


def _subtract_int_maps(left: Mapping[str, int], right: Mapping[str, int]) -> dict[str, int]:
    return {
        key: left.get(key, 0) - right.get(key, 0)
        for key in left.keys() | right.keys()
        if left.get(key, 0) != right.get(key, 0)
    }


def _fit_one_opcode_relation(
    relation: OpcodeRelationSpec,
    rows: list[Mapping[str, Any]],
) -> dict[str, Any]:
    expected_target = dict(relation.target_raw_gas_by_key)
    expected_control = dict(relation.control_raw_gas_by_key)
    expected_signed = dict(relation.signed_raw_gas_by_key)
    grouped: dict[tuple[int, str], list[Mapping[str, Any]]] = {}
    generator_bounds: set[int] = set()
    for row in rows:
        if row.get("purpose") != FORMAL_RELATION_PURPOSE:
            raise ValueError("formal relation fit received a non-formal row")
        if row.get("signal_kind") != FORMAL_RELATION_SIGNAL_KIND:
            raise ValueError("formal relation row has the wrong signal kind")
        validate_sp1_execution_provenance(row, workload_kind="opcode")
        if row.get("relation_split") != relation.split or row.get(
            "scenario_id"
        ) != relation.scenario_id or row.get("dynamic_key") != relation.dynamic_key:
            raise ValueError("formal relation row differs from manifest scenario identity")
        if _relation_row_map(row, "target_raw_gas_by_key") != expected_target:
            raise ValueError("formal relation target raw-gas units differ from manifest")
        if _relation_row_map(row, "control_raw_gas_by_key") != expected_control:
            raise ValueError("formal relation control raw-gas units differ from manifest")
        if _relation_row_map(row, "signed_raw_gas_by_key") != expected_signed:
            raise ValueError("formal relation signed raw-gas units differ from manifest")
        count = row.get("diagnostic_count")
        repeat_index = row.get("repeat_index")
        lane = row.get("lane")
        generator_max = row.get("generator_max_count")
        if (
            isinstance(count, bool)
            or not isinstance(count, int)
            or count < 0
            or lane not in {"target", "control"}
            or isinstance(repeat_index, bool)
            or not isinstance(repeat_index, int)
            or isinstance(generator_max, bool)
            or not isinstance(generator_max, int)
            or generator_max <= 0
        ):
            raise ValueError("formal relation row has invalid count/lane identity")
        generator_bounds.add(generator_max)
        grouped.setdefault((count, str(lane)), []).append(row)
    if len(generator_bounds) != 1:
        raise ValueError("formal relation rows do not share one generator bound")
    generator_max_count = next(iter(generator_bounds))
    counts: dict[int, tuple[Decimal, Decimal, Decimal]] = {}
    for count in sorted({key[0] for key in grouped}):
        lanes: dict[str, tuple[Decimal, dict[str, int]]] = {}
        for lane in ("target", "control"):
            repeats = sorted(
                grouped.get((count, lane), []),
                key=lambda row: int(row.get("repeat_index", -1)),
            )
            if [row.get("repeat_index") for row in repeats] != [0, 1, 2]:
                raise ValueError("formal relation requires exactly three repeats")
            gas_values = [
                _decimal(row.get("prover_gas", row.get("gas")), label="formal proverGas")
                for row in repeats
            ]
            if max(gas_values) - min(gas_values) != 0:
                raise ValueError("formal relation repeat noise is nonzero")
            for field in ("backend_input_sha256", "public_values", "exit_code"):
                if len({str(row.get(field)) for row in repeats}) != 1:
                    raise ValueError("formal relation repeat identity differs")
            if any(row.get("exit_code") != 0 for row in repeats):
                raise ValueError("formal relation repeat exit code is nonzero")
            actual_maps = [
                _parse_canonical_int_map(
                    row.get("actual_raw_gas_by_key"), label="actual raw-gas map"
                )
                for row in repeats
            ]
            if any(actual != actual_maps[0] for actual in actual_maps[1:]):
                raise ValueError("formal relation repeat actual raw-gas map differs")
            lanes[lane] = gas_values[0], actual_maps[0]
        actual_delta = _subtract_int_maps(lanes["target"][1], lanes["control"][1])
        expected_delta = {
            key: value * count for key, value in expected_signed.items() if value * count
        }
        if actual_delta != expected_delta:
            raise ValueError("formal relation actual trace has wrong raw-gas units")
        counts[count] = (
            lanes["target"][0],
            lanes["control"][0],
            lanes["target"][0] - lanes["control"][0],
        )

    if not expected_signed:
        expected_counts = controlled_round_counts(generator_max_count)
        if tuple(sorted(counts)) != expected_counts:
            raise ValueError("formal self-control counts omit a frozen prefix or checkpoint")
        if relation.key_id not in OPCODE_RELATION_ANCHORS or any(
            target != control or delta != 0
            for target, control, delta in counts.values()
        ):
            raise ValueError("formal self-control must have exact zero delta")
        return {
            "relation_id": relation.id,
            "key_id": relation.key_id,
            "scenario_id": relation.scenario_id,
            "status": "passed",
            "exact_zero": True,
            "checked_counts": list(expected_counts),
            "checkpoint": {
                "count": generator_max_count,
                "observed_delta_p": "0",
                "status": "passed_exact_zero",
            },
        }

    fit = _signed_relation_fit(counts, generator_max_count=generator_max_count)
    return {
        "relation_id": relation.id,
        "key_id": relation.key_id,
        "split": relation.split,
        "scenario_id": relation.scenario_id,
        "dynamic_key": relation.dynamic_key,
        "target_raw_gas_by_key": {
            key: str(value) for key, value in sorted(expected_target.items())
        },
        "control_raw_gas_by_key": {
            key: str(value) for key, value in sorted(expected_control.items())
        },
        "signed_raw_gas_by_key": {
            key: str(value) for key, value in sorted(expected_signed.items())
        },
        "slope_p": _decimal_text(fit["slope"]),
        "intercept_p": _decimal_text(fit["intercept"]),
        "r2_p": _decimal_text(fit["r2"]),
        "slope_stderr_p": _decimal_text(fit["stderr"]),
        "relative_slope_stderr": _decimal_text(fit["stderr"] / abs(fit["slope"])),
        "signal_p": _decimal_text(fit["signal"]),
        "zero_delta_p": _decimal_text(counts[0][2]),
        "max_residual_p": _decimal_text(fit["max_residual"]),
        "selected_counts": fit["selected_counts"],
        "checkpoint": fit["checkpoint"],
        "status": "accepted",
    }


def _fraction_text(value: Fraction) -> str:
    return (
        str(value.numerator)
        if value.denominator == 1
        else f"{value.numerator}/{value.denominator}"
    )


def _serialize_affine_model(model: Any) -> dict[str, Any]:
    payload = {
        "opcode_keys": list(model.opcode_keys),
        "anchor_keys": list(model.anchor_keys),
        "rank": model.rank,
        "nullity": model.nullity,
        "mu_zero": {
            key: _decimal_text(model.mu_zero[key]) for key in model.opcode_keys
        },
        "B": {
            key: {
                anchor: _fraction_text(model.anchor_basis[key][anchor])
                for anchor in model.anchor_keys
            }
            for key in model.opcode_keys
        },
    }
    payload["model_sha256"] = sha256_bytes(canonical_json(payload))
    return payload


def _validate_formal_relation_provenance(value: Any) -> dict[str, str]:
    if not isinstance(value, Mapping) or set(value) != set(
        FORMAL_RELATION_PROVENANCE_FIELDS
    ):
        raise ValueError("formal relation provenance schema is invalid")
    provenance = {field: value.get(field) for field in FORMAL_RELATION_PROVENANCE_FIELDS}
    identity_sha256 = provenance["calibration_identity_sha256"]
    if (
        not _is_sha256(identity_sha256)
        or provenance["calibration_id"] != identity_sha256[:24]
        or not _is_git_revision(provenance["implementation_revision"])
        or not _is_sha256(provenance["controlled_manifest_sha256"])
        or not _is_sha256(provenance["controlled_manifest_rows_sha256"])
    ):
        raise ValueError("formal relation provenance is not content-addressed")
    return provenance


@_isolated_decimal_context
def fit_opcode_relations(
    manifest: Manifest,
    rows: Iterable[Mapping[str, Any]],
) -> dict[str, Any]:
    rows = list(rows)
    expected = {relation.id: relation for relation in manifest.opcode_relations}
    actual_ids = {str(row.get("relation_id")) for row in rows}
    missing = set(expected) - actual_ids
    extra = actual_ids - set(expected)
    if missing:
        raise ValueError(f"missing canonical relation rows: {sorted(missing)!r}")
    if extra:
        raise ValueError(f"unknown formal relation rows: {sorted(extra)!r}")
    provenance = {
        field: rows[0].get(field) for field in FORMAL_RELATION_PROVENANCE_FIELDS
    }
    if any(row.get(field) != value for row in rows for field, value in provenance.items()):
        raise ValueError("formal relation rows do not share durable provenance")
    provenance = _validate_formal_relation_provenance(provenance)

    results = []
    for relation in manifest.opcode_relations:
        relation_rows = [row for row in rows if row.get("relation_id") == relation.id]
        results.append(_fit_one_opcode_relation(relation, relation_rows))

    self_controls = [row for row in results if row.get("exact_zero") is True]
    equations = [
        row
        for row in results
        if row.get("split") == "canonical" and row.get("status") == "accepted"
    ]
    holdouts = [
        row
        for row in results
        if row.get("split") == "dynamic_holdout" and row.get("status") == "accepted"
    ]
    if len(self_controls) != 4 or {row["key_id"] for row in self_controls} != set(
        manifest.opcode_relation_anchors
    ):
        raise ValueError("formal relation self-control set is incomplete")
    if len(equations) != 98:
        raise ValueError(f"formal relation matrix requires 98 equations, got {len(equations)}")
    if len(holdouts) != 12:
        raise ValueError("formal dynamic holdout set is incomplete")
    for key in manifest.dynamic_raw_gas_keys:
        observed = [
            relation.target_raw_gas_by_key[key]
            for relation in manifest.opcode_relations
            if relation.dynamic_key == key
        ]
        if len(observed) != 3 or observed[1] < 2 * observed[0] or observed[2] < 4 * observed[0]:
            raise ValueError(f"formal dynamic relation {key} fails actual 1x/2x/4x preflight")

    opcode_keys = tuple(
        f"opcode:0x{case.opcode:02x}"
        for case in manifest.cases
        if case.kind == "opcode"
        and case.opcode is not None
        and case.opcode in PURE_OPCODE_DEFAULTS
        and PURE_OPCODE_DEFAULTS[case.opcode][1] == case.template
    )
    algebra_equations = tuple(
        RelationEquation(
            relation_id=row["relation_id"],
            coefficients={
                key: Fraction(value)
                for key, value in row["signed_raw_gas_by_key"].items()
            },
            slope=Decimal(row["slope_p"]),
        )
        for row in equations
    )
    model = derive_affine_opcode_model(
        opcode_keys,
        algebra_equations,
        manifest.opcode_relation_anchors,
    )
    affine = _serialize_affine_model(model)
    matrix_sha256 = sha256_bytes(
        canonical_json(
            [
                {
                    "relation_id": equation.relation_id,
                    "coefficients": {
                        key: _fraction_text(value)
                        for key, value in sorted(equation.coefficients.items())
                    },
                    "slope": _decimal_text(equation.slope),
                }
                for equation in algebra_equations
            ]
        )
    )
    artifact = {
        "schema_version": 1,
        "purpose": FORMAL_RELATION_PURPOSE,
        "signal_kind": FORMAL_RELATION_SIGNAL_KIND,
        "status": "accepted",
        "provenance": provenance,
        "quality_gates": dict(FORMAL_RELATION_QUALITY_GATES),
        "equations": equations,
        "self_controls": self_controls,
        "dynamic_holdouts": holdouts,
        "relation_matrix_sha256": matrix_sha256,
        "raw_rows_sha256": sha256_bytes(canonical_json(rows)),
        "affine_model": affine,
    }
    artifact["artifact_sha256"] = sha256_bytes(canonical_json(artifact))
    validate_opcode_relations_artifact(manifest, artifact, rows, provenance)
    return artifact


def _parse_fraction_text(value: Any) -> Fraction:
    if not isinstance(value, str):
        raise ValueError("basis coefficient must be a canonical Fraction string")
    try:
        parsed = Fraction(value)
    except (ValueError, ZeroDivisionError) as exc:
        raise ValueError("basis coefficient must be a canonical Fraction string") from exc
    if _fraction_text(parsed) != value:
        raise ValueError("basis coefficient is rounded or noncanonical")
    return parsed


def _artifact_decimal(row: Mapping[str, Any], field: str, *, label: str) -> Decimal:
    raw = row.get(field)
    value = _decimal(raw, label=f"{label} {field}")
    if not isinstance(raw, str) or _decimal_text(value) != raw:
        raise ValueError(f"{label} quality evidence has noncanonical {field}")
    return value


def _validate_artifact_relation_rows(
    rows: Any,
    relations: list[OpcodeRelationSpec],
    *,
    label: str,
) -> list[RelationEquation]:
    if not isinstance(rows, list) or [row.get("relation_id") for row in rows] != [
        relation.id for relation in relations
    ]:
        raise ValueError(f"opcode relation artifact {label} set is incomplete")
    expected_fields = {
        "relation_id",
        "key_id",
        "split",
        "scenario_id",
        "dynamic_key",
        "target_raw_gas_by_key",
        "control_raw_gas_by_key",
        "signed_raw_gas_by_key",
        "slope_p",
        "intercept_p",
        "r2_p",
        "slope_stderr_p",
        "relative_slope_stderr",
        "signal_p",
        "zero_delta_p",
        "max_residual_p",
        "selected_counts",
        "checkpoint",
        "status",
    }
    algebra: list[RelationEquation] = []
    for row, relation in zip(rows, relations):
        if not isinstance(row, Mapping) or set(row) != expected_fields:
            raise ValueError(f"opcode relation artifact {label} quality evidence schema is invalid")
        if (
            row.get("key_id") != relation.key_id
            or row.get("split") != relation.split
            or row.get("scenario_id") != relation.scenario_id
            or row.get("dynamic_key") != relation.dynamic_key
            or row.get("status") != "accepted"
        ):
            raise ValueError(f"opcode relation artifact {label} identity differs from manifest")
        target = _parse_canonical_int_map(
            row.get("target_raw_gas_by_key"), label=f"artifact {label} target map"
        )
        control = _parse_canonical_int_map(
            row.get("control_raw_gas_by_key"), label=f"artifact {label} control map"
        )
        signed = _parse_canonical_int_map(
            row.get("signed_raw_gas_by_key"),
            label=f"artifact {label} signed map",
            allow_negative=True,
        )
        if (
            target != dict(relation.target_raw_gas_by_key)
            or control != dict(relation.control_raw_gas_by_key)
            or signed != dict(relation.signed_raw_gas_by_key)
        ):
            raise ValueError(f"opcode relation artifact {label} coefficient maps differ")
        slope = _artifact_decimal(row, "slope_p", label=label)
        _artifact_decimal(row, "intercept_p", label=label)
        r2 = _artifact_decimal(row, "r2_p", label=label)
        stderr = _artifact_decimal(row, "slope_stderr_p", label=label)
        relative_stderr = _artifact_decimal(
            row, "relative_slope_stderr", label=label
        )
        signal = _artifact_decimal(row, "signal_p", label=label)
        zero_delta = _artifact_decimal(row, "zero_delta_p", label=label)
        residual = _artifact_decimal(row, "max_residual_p", label=label)
        if (
            slope == 0
            or r2 < Decimal("0.995")
            or r2 > 1
            or stderr < 0
            or relative_stderr < 0
            or relative_stderr != stderr / abs(slope)
            or relative_stderr > Decimal("0.05")
            or signal < max(Decimal(1000), abs(zero_delta) * Decimal("0.01"))
            or residual < 0
            or residual / signal > Decimal("0.02")
        ):
            raise ValueError(f"opcode relation artifact {label} quality evidence fails gates")
        selected = row.get("selected_counts")
        if (
            not isinstance(selected, list)
            or any(type(count) is not int for count in selected)
            or tuple(selected) not in CONTROLLED_PREFIXES
        ):
            raise ValueError(f"opcode relation artifact {label} quality evidence has bad prefix")
        checkpoint = row.get("checkpoint")
        if not isinstance(checkpoint, Mapping) or set(checkpoint) != {
            "count",
            "observed_delta_p",
            "predicted_delta_p",
            "ape_p",
            "status",
        }:
            raise ValueError(f"opcode relation artifact {label} quality evidence has bad checkpoint")
        expected_checkpoint = OUT_OF_FIT_CHECKPOINTS[str(max(selected))]
        observed = _artifact_decimal(checkpoint, "observed_delta_p", label=label)
        predicted = _artifact_decimal(checkpoint, "predicted_delta_p", label=label)
        ape = _artifact_decimal(checkpoint, "ape_p", label=label)
        if (
            checkpoint.get("count") != expected_checkpoint
            or type(checkpoint.get("count")) is not int
            or checkpoint.get("status") != "passed"
            or observed == 0
            or predicted != slope * Decimal(expected_checkpoint)
            or (observed > 0) != (predicted > 0)
            or ape != abs(predicted - observed) / abs(observed)
            or ape < 0
            or ape > Decimal("0.10")
        ):
            raise ValueError(f"opcode relation artifact {label} quality evidence fails checkpoint")
        algebra.append(
            RelationEquation(
                relation.id,
                {key: Fraction(value) for key, value in signed.items()},
                slope,
            )
        )
    return algebra


@_isolated_decimal_context
def validate_opcode_relations_artifact(
    manifest: Manifest,
    artifact: Mapping[str, Any],
    rows: Iterable[Mapping[str, Any]],
    expected_provenance: Mapping[str, Any],
) -> None:
    rows = list(rows)
    expected_provenance = _validate_formal_relation_provenance(expected_provenance)
    if (
        artifact.get("purpose") != FORMAL_RELATION_PURPOSE
        or artifact.get("signal_kind") != FORMAL_RELATION_SIGNAL_KIND
        or artifact.get("status") != "accepted"
    ):
        raise ValueError("opcode relation artifact header is invalid")
    recorded_artifact_hash = artifact.get("artifact_sha256")
    unhashed = {key: value for key, value in artifact.items() if key != "artifact_sha256"}
    if not _is_sha256(recorded_artifact_hash) or recorded_artifact_hash != sha256_bytes(
        canonical_json(unhashed)
    ):
        raise ValueError("opcode relation artifact content hash differs")
    if set(artifact) != {
        "schema_version",
        "purpose",
        "signal_kind",
        "status",
        "provenance",
        "quality_gates",
        "equations",
        "self_controls",
        "dynamic_holdouts",
        "relation_matrix_sha256",
        "raw_rows_sha256",
        "affine_model",
        "artifact_sha256",
    } or artifact.get("schema_version") != 1:
        raise ValueError("opcode relation artifact schema is invalid")
    artifact_provenance = _validate_formal_relation_provenance(
        artifact.get("provenance")
    )
    if artifact_provenance != expected_provenance:
        raise ValueError("opcode relation artifact provenance differs from calibration run")
    if artifact.get("quality_gates") != FORMAL_RELATION_QUALITY_GATES:
        raise ValueError("opcode relation artifact quality gates differ from frozen gates")
    raw_rows_sha256 = sha256_bytes(canonical_json(rows))
    if artifact.get("raw_rows_sha256") != raw_rows_sha256:
        raise ValueError("opcode relation artifact raw rows hash differs")
    if not rows:
        raise ValueError("opcode relation artifact raw rows are empty")
    row_provenance = {
        field: rows[0].get(field) for field in FORMAL_RELATION_PROVENANCE_FIELDS
    }
    if any(
        row.get(field) != value
        for row in rows
        for field, value in expected_provenance.items()
    ) or _validate_formal_relation_provenance(row_provenance) != expected_provenance:
        raise ValueError("opcode relation artifact raw rows differ from calibration run")
    expected_relation_ids = {relation.id for relation in manifest.opcode_relations}
    actual_relation_ids = {str(row.get("relation_id")) for row in rows}
    if actual_relation_ids != expected_relation_ids:
        raise ValueError("opcode relation artifact raw relation set differs from manifest")
    recomputed_results = [
        _fit_one_opcode_relation(
            relation,
            [row for row in rows if row.get("relation_id") == relation.id],
        )
        for relation in manifest.opcode_relations
    ]
    recomputed_self_controls = [
        row for row in recomputed_results if row.get("exact_zero") is True
    ]
    recomputed_equations = [
        row
        for row in recomputed_results
        if row.get("split") == "canonical" and row.get("status") == "accepted"
    ]
    recomputed_holdouts = [
        row
        for row in recomputed_results
        if row.get("split") == "dynamic_holdout" and row.get("status") == "accepted"
    ]
    if artifact.get("self_controls") != recomputed_self_controls:
        raise ValueError("opcode relation artifact self controls differ from raw rows")
    if artifact.get("equations") != recomputed_equations:
        raise ValueError("opcode relation artifact equations quality evidence differs from raw rows")
    if artifact.get("dynamic_holdouts") != recomputed_holdouts:
        raise ValueError(
            "opcode relation artifact dynamic holdouts quality evidence differs from raw rows"
        )
    self_relations = [
        relation for relation in manifest.opcode_relations if not relation.signed_raw_gas_by_key
    ]
    self_controls = artifact.get("self_controls")
    if not isinstance(self_controls, list) or [
        row.get("relation_id") for row in self_controls
    ] != [relation.id for relation in self_relations]:
        raise ValueError("opcode relation artifact self controls are incomplete")
    for row, relation in zip(self_controls, self_relations):
        if not isinstance(row, Mapping) or set(row) != {
            "relation_id",
            "key_id",
            "scenario_id",
            "status",
            "exact_zero",
            "checked_counts",
            "checkpoint",
        }:
            raise ValueError("opcode relation artifact self controls schema is invalid")
        checkpoint = row.get("checkpoint")
        if not isinstance(checkpoint, Mapping):
            raise ValueError("opcode relation artifact self controls checkpoint is invalid")
        bound = checkpoint.get("count")
        if type(bound) is not int:
            raise ValueError("opcode relation artifact self controls checkpoint is invalid")
        try:
            expected_counts = list(controlled_round_counts(bound))
        except ValueError as exc:
            raise ValueError(
                "opcode relation artifact self controls checkpoint is invalid"
            ) from exc
        if (
            row.get("key_id") != relation.key_id
            or row.get("scenario_id") != relation.scenario_id
            or row.get("status") != "passed"
            or row.get("exact_zero") is not True
            or row.get("checked_counts") != expected_counts
            or dict(checkpoint)
            != {
                "count": bound,
                "observed_delta_p": "0",
                "status": "passed_exact_zero",
            }
        ):
            raise ValueError("opcode relation artifact self controls evidence is invalid")
    equation_relations = [
        relation
        for relation in manifest.opcode_relations
        if relation.split == "canonical" and relation.signed_raw_gas_by_key
    ]
    if len(equation_relations) != 98:
        raise ValueError("opcode relation manifest must define 98 equations")
    algebra = _validate_artifact_relation_rows(
        artifact.get("equations"), equation_relations, label="equations"
    )
    holdout_relations = [
        relation
        for relation in manifest.opcode_relations
        if relation.split == "dynamic_holdout"
    ]
    if len(holdout_relations) != 12:
        raise ValueError("opcode relation manifest must define 12 dynamic holdouts")
    _validate_artifact_relation_rows(
        artifact.get("dynamic_holdouts"),
        holdout_relations,
        label="dynamic holdouts",
    )
    opcode_keys = tuple(
        f"opcode:0x{case.opcode:02x}"
        for case in manifest.cases
        if case.kind == "opcode"
        and case.opcode is not None
        and case.opcode in PURE_OPCODE_DEFAULTS
        and PURE_OPCODE_DEFAULTS[case.opcode][1] == case.template
    )
    expected_model = _serialize_affine_model(
        derive_affine_opcode_model(
            opcode_keys, tuple(algebra), manifest.opcode_relation_anchors
        )
    )
    expected_matrix_hash = sha256_bytes(
        canonical_json(
            [
                {
                    "relation_id": equation.relation_id,
                    "coefficients": {
                        key: _fraction_text(value)
                        for key, value in sorted(equation.coefficients.items())
                    },
                    "slope": _decimal_text(equation.slope),
                }
                for equation in algebra
            ]
        )
    )
    if artifact.get("relation_matrix_sha256") != expected_matrix_hash:
        raise ValueError("opcode relation artifact matrix hash differs")
    actual_model = artifact.get("affine_model")
    if not isinstance(actual_model, Mapping):
        raise ValueError("opcode relation artifact basis is missing")
    basis = actual_model.get("B")
    if not isinstance(basis, Mapping):
        raise ValueError("opcode relation artifact basis is missing")
    try:
        for row in basis.values():
            if not isinstance(row, Mapping):
                raise ValueError("basis row is invalid")
            for value in row.values():
                _parse_fraction_text(value)
    except ValueError as exc:
        raise ValueError(f"opcode relation artifact basis is invalid: {exc}") from exc
    if dict(actual_model) != expected_model:
        raise ValueError("opcode relation artifact basis differs from exact derivation")


def preflight_block_calibration_rows(
    manifest: Manifest,
    affine_model: AffineOpcodeModel,
) -> dict[str, Any]:
    rows = manifest.block_calibration_rows
    fit_rows = [row for row in rows if row.split == "fit"]
    holdout_rows = [row for row in rows if row.split == "holdout"]
    if len(fit_rows) != 40:
        raise ValueError(f"block calibration requires exactly 40 fit rows, got {len(fit_rows)}")
    if len(holdout_rows) != 8:
        raise ValueError(
            f"block calibration requires exactly 8 holdout rows, got {len(holdout_rows)}"
        )
    expected_families = set(BLOCK_CALIBRATION_FAMILIES)
    for family in BLOCK_CALIBRATION_FAMILIES:
        if sum(row.workload_family == family for row in fit_rows) != 5:
            raise ValueError(f"block calibration family {family} does not have five fit rows")
        if sum(row.workload_family == family for row in holdout_rows) != 1:
            raise ValueError(f"block calibration family {family} does not have one holdout row")
    if tuple(affine_model.anchor_keys) != OPCODE_RELATION_ANCHORS:
        raise ValueError("block calibration affine model has wrong natural anchors")
    opcode_keys = set(affine_model.opcode_keys)
    matrix = []
    for row in rows:
        raw = row.expected_raw_gas_by_key
        if any(key.startswith("precompile:") or ":spawned" in key for key in raw):
            raise ValueError("block calibration contains precompile or spawned work")
        if any(raw.get(key, 0) != 0 for key in manifest.dynamic_raw_gas_keys):
            raise ValueError("block calibration dynamic raw-gas totals must be zero")
        unknown = set(raw) - opcode_keys
        if unknown:
            raise ValueError(f"block calibration contains unknown opcode keys: {sorted(unknown)!r}")
        projected = []
        for anchor in affine_model.anchor_keys:
            value = Fraction(0)
            for key, units in raw.items():
                coefficient = affine_model.anchor_basis[key][anchor]
                if not isinstance(coefficient, Fraction):
                    raise ValueError("block calibration rejects rounded basis coefficients")
                value += Fraction(units) * coefficient
            projected.append(value)
        q = []
        for key in Q_FORMULA:
            units = row.expected_features[key]
            if type(units) is not int:
                raise ValueError("block calibration fixed/base feature is not exact")
            q.append(Fraction(units))
        if row.split == "fit":
            matrix.append([*projected, *q])
    rank = exact_rank(matrix)
    if rank != 8:
        raise ValueError(f"block calibration exact [xB,q] fit matrix rank must be eight, got {rank}")
    holdout_families = sorted(row.workload_family for row in holdout_rows)
    if set(holdout_families) != expected_families:
        raise ValueError("block calibration holdouts do not cover all parameter families")
    return {
        "fit_row_count": len(fit_rows),
        "holdout_row_count": len(holdout_rows),
        "fit_rank": rank,
        "holdout_families": holdout_families,
        "fit_matrix": [[_fraction_text(value) for value in row] for row in matrix],
    }


def _atomic_write_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    """Atomically create one JSON artifact without replacing any prior run."""
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = pathlib.Path(name)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(json.dumps(value, indent=2, sort_keys=True) + "\n")
            output.flush()
            os.fsync(output.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError as exc:
            raise ValueError(f"artifact already exists: {path}") from exc
    finally:
        if temporary.exists():
            temporary.unlink()
        directory_fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)


def case_primary_value(case_result: Mapping[str, Any]) -> tuple[str, Decimal]:
    if case_result.get("status") != "accepted":
        raise ValueError("case is not accepted")
    slope = _decimal(case_result.get("g_p"), label="g_p")
    if slope <= 0:
        raise ValueError("primary slope must be positive")
    basis = case_result.get("pricing_basis")
    if basis == "raw_gas_slope":
        raw_gas = _decimal(case_result.get("target_raw_gas"), label="target raw gas")
        if raw_gas <= 0:
            raise ValueError("target raw gas must be positive")
        return "c_p", slope / raw_gas
    if basis == "fixed_per_event":
        if case_result.get("target_raw_gas") not in (None, 0, "0"):
            raise ValueError("fixed-event primary value cannot consume raw gas")
        return "f_p", slope
    raise ValueError("unknown pricing basis")


def residualize_overhead_delta(
    manifest: Manifest,
    overhead_key_id: str,
    *,
    target_minus_control: Any,
    feature_deltas: Mapping[str, int],
    accepted_costs: Mapping[str, Any],
) -> Decimal:
    overheads = {item.id: item for item in manifest.overhead_keys}
    key = overheads.get(overhead_key_id)
    if key is None:
        raise ValueError(f"unknown overhead key: {overhead_key_id}")
    closure = set(manifest.subtract_closure.get(overhead_key_id, ()))
    declared = {overhead_key_id, *closure, *key.bundled_keys}
    if set(feature_deltas) != declared:
        raise ValueError("controlled overhead has an undeclared changed feature")
    own_units = feature_deltas[overhead_key_id]
    if own_units <= 0:
        raise ValueError("controlled overhead target units must be positive")
    residual = _decimal(target_minus_control, label="overhead target/control delta")
    for child in sorted(closure):
        if child not in accepted_costs:
            raise ValueError(f"failed required dependency: {child}")
        residual -= Decimal(feature_deltas[child]) * _decimal(
            accepted_costs[child], label=f"accepted overhead {child}"
        )
    for bundled in key.bundled_keys:
        numerator, denominator = key.bundled_ratios[bundled]
        if feature_deltas[bundled] * denominator != own_units * numerator:
            raise ValueError(f"controlled-mismatched bundled ratio for {bundled}")
    return residual / Decimal(own_units)


def fixed_startup_residual(panel_repeat_residuals: Iterable[Iterable[Any]]) -> Decimal:
    case_values = []
    for repeats in panel_repeat_residuals:
        values = [
            _decimal(value, label="startup repeat residual") for value in repeats
        ]
        if len(values) != 3 or max(values) != min(values):
            raise ValueError("startup requires a repeat-stable residual for every panel case")
        case_values.append(values[0])
    if not case_values:
        raise ValueError("startup residual panel is empty")
    return sum(case_values) / Decimal(len(case_values))


@_isolated_decimal_context
def construct_measurement_values(
    manifest: Manifest, case_results: Iterable[Mapping[str, Any]]
) -> dict[str, dict[str, Any]]:
    by_case = {str(result.get("case_id")): result for result in case_results}
    values: dict[str, dict[str, Any]] = {}
    for key in manifest.measurement_keys:
        required = [by_case.get(case_id) for case_id in key.required_case_ids]
        diagnostic = [
            {
                field_name: field_value
                for field_name, field_value in by_case[case_id].items()
                if field_name != "secondary"
                and "instruction" not in field_name
                and field_name not in {"g_s", "c_s", "f_s", "o_s"}
            }
            for case_id in key.diagnostic_case_ids
            if case_id in by_case
        ]
        if any(result is None or result.get("status") != "accepted" for result in required):
            values[key.id] = {
                "status": "required_case_incomplete",
                "pricing_basis": key.pricing_basis,
                "required_case_ids": list(key.required_case_ids),
                "diagnostic_results": diagnostic,
            }
            continue
        typed_values = [case_primary_value(result) for result in required if result is not None]
        value_names = {name for name, _ in typed_values}
        if value_names != {"c_p" if key.pricing_basis == "raw_gas_slope" else "f_p"}:
            raise ValueError(f"case basis differs from measurement key {key.id}")
        primary_values = [value for _, value in typed_values]
        minimum, maximum = min(primary_values), max(primary_values)
        if minimum <= 0 or maximum / minimum - Decimal(1) > Decimal("0.05"):
            values[key.id] = {
                "status": "scenario_dependent",
                "pricing_basis": key.pricing_basis,
                "required_case_ids": list(key.required_case_ids),
                "diagnostic_results": diagnostic,
            }
            continue
        value = sum(primary_values) / Decimal(len(primary_values))
        field_name = typed_values[0][0]
        values[key.id] = {
            "status": "accepted",
            "pricing_basis": key.pricing_basis,
            field_name: _decimal_text(value),
            "required_case_ids": list(key.required_case_ids),
            "diagnostic_results": diagnostic,
            "checkpoint_evidence": {
                result["case_id"]: result.get("checkpoint") for result in required
            },
        }
    reference = values.get(manifest.normalization_reference_key or "")
    if reference is not None and reference.get("status") == "accepted":
        anchor = _decimal(reference["c_p"], label="ADD c_p")
        for value in values.values():
            if value.get("status") == "accepted" and "c_p" in value:
                value["m_p"] = _decimal_text(
                    _decimal(value["c_p"], label="c_p") / anchor
                )
    return values


def build_component_cost_ledger(
    manifest: Manifest,
    measurements: Mapping[str, Mapping[str, Any]],
    overhead_values: Mapping[str, Any],
    schedule: UnzenSchedule,
) -> dict[str, Any]:
    """Build a complete review ledger without inventing units for host-only work."""
    by_schedule_key: dict[str, list[MeasurementKeySpec]] = {}
    for key in manifest.measurement_keys:
        by_schedule_key.setdefault(key.production_schedule_key, []).append(key)
    primary_rows: list[dict[str, Any]] = []
    for component, rows in (
        ("opcode", schedule.opcode_multipliers),
        ("precompile", schedule.precompile_multipliers),
    ):
        for identifier, multiplier in sorted(rows.items()):
            schedule_key = _canonical_schedule_key(component, identifier)
            keys = by_schedule_key.get(schedule_key) or [None]
            for key in keys:
                value = measurements.get(key.id, {}) if key is not None else {}
                row = {
                    "component_id": key.id if key is not None else schedule_key,
                    "production_schedule_key": schedule_key,
                    "component": component,
                    "raw_gas_unit": (
                        "interpreter_raw_gas"
                        if component == "opcode"
                        else "native_gas"
                    ),
                    "current_zkgas_multiplier": multiplier,
                    "measurement_key_id": key.id if key is not None else None,
                    "status": value.get("status", "unmeasured"),
                    "marginal_prover_gas": value.get("c_p"),
                    "cost_index": value.get("m_p"),
                    "index_kind": "add_normalized_prover_gas_per_raw_gas",
                }
                if key is not None and key.pricing_basis == "fixed_per_event":
                    opcode_name = UZEN_OPCODE_NAMES.get(identifier)
                    row.update(
                        {
                            "raw_gas_unit": None,
                            "current_zkgas_multiplier": None,
                            "current_zkgas_fixed_charge": (
                                schedule.spawn_estimates.get(opcode_name)
                                if opcode_name is not None
                                else None
                            ),
                            "marginal_prover_gas": value.get("f_p"),
                            "cost_index": value.get("f_p"),
                            "index_kind": "prover_gas_per_event_not_add_normalized",
                        }
                    )
                primary_rows.append(row)
    overhead_units = {key.id: key.unit for key in manifest.overhead_keys}
    for key in Q_FORMULA:
        current_charge = (
            schedule.tx_intrinsic_zk_gas if key == "tx_base" else None
        )
        value = overhead_values.get(key)
        primary_rows.append(
            {
                "component_id": key,
                "component": "controlled_overhead",
                "unit": overhead_units.get(key),
                "raw_gas_unit": None,
                "current_zkgas_multiplier": None,
                "current_zkgas_fixed_charge": current_charge,
                "marginal_prover_gas": (
                    _decimal_text(_decimal(value, label=f"overhead {key}"))
                    if value is not None
                    else None
                ),
                "cost_index": (
                    _decimal_text(_decimal(value, label=f"overhead {key}"))
                    if value is not None
                    else None
                ),
                "index_kind": "prover_gas_per_declared_unit",
                "status": "accepted" if value is not None else "unmeasured",
            }
        )
    diagnostic_rows = [
        {
            "component_id": key.id,
            "component": "diagnostic_overhead",
            "unit": key.unit,
            "raw_gas_unit": None,
            "current_zkgas_multiplier": None,
            "marginal_prover_gas": None,
            "cost_index": None,
            "status": "unmeasured",
            "reason": "diagnostic_controlled_result_not_supplied",
        }
        for key in manifest.overhead_keys
        if key.formula_role == "diagnostic"
    ]
    diagnostic_rows.extend(
        {
            "component_id": component_id,
            "component": "host_guest_internal_action",
            "unit": None,
            "raw_gas_unit": None,
            "current_zkgas_multiplier": None,
            "marginal_prover_gas": None,
            "cost_index": None,
            "status": "unmeasured",
            "reason": "no_independent_controlled_observable_unit",
        }
        for component_id in (
            "host_action:block_header_hash",
            "host_action:trie_merkle_hash",
            "host_action:witness_processing",
        )
    )
    payload = {
        "schema_version": 1,
        "primary_rows": primary_rows,
        "diagnostic_rows": diagnostic_rows,
        "unit_policy": (
            "never divide host-only or fixed/base work by EVM raw gas; "
            "unisolated actions remain explicit unmeasured coverage"
        ),
    }
    return {**payload, "sha256": sha256_bytes(canonical_json(payload))}


def _primary_case_projection(case_results: Iterable[Mapping[str, Any]]) -> list[dict[str, Any]]:
    projection = []
    for row in case_results:
        projection.append(
            {
                key: value
                for key, value in row.items()
                if key != "secondary"
                and "instruction" not in key
                and key not in {"g_s", "c_s", "f_s", "o_s"}
            }
        )
    return sorted(projection, key=lambda row: str(row.get("case_id")))


def build_candidate_components(
    manifest: Manifest,
    case_results: Iterable[Mapping[str, Any]],
    overhead_values: Mapping[str, Any],
    provenance: Mapping[str, Any],
    schedule: UnzenSchedule | None = None,
) -> dict[str, Any]:
    rows = list(case_results)
    reject_matched_control_diagnostics(rows, context="formal candidate")
    measurements = construct_measurement_values(manifest, rows)
    add = measurements.get(manifest.normalization_reference_key or "")
    if add is None or add.get("status") != "accepted":
        raise ValueError("ADD measurement is required before candidate sealing")
    missing_q = [key for key in Q_FORMULA if key not in overhead_values]
    if missing_q:
        raise ValueError(f"Q_formula value is missing: {missing_q[0]}")
    primary_overheads = {}
    for key in Q_FORMULA:
        value = _decimal(overhead_values[key], label=f"overhead {key}")
        if value <= 0:
            raise ValueError(f"Q_formula value must be positive: {key}")
        primary_overheads[key] = _decimal_text(value)
    for key_id, value in measurements.items():
        if value.get("status") != "accepted":
            continue
        evidence = value.get("checkpoint_evidence")
        if not isinstance(evidence, Mapping) or any(
            not isinstance(item, Mapping) or item.get("status") != "passed"
            for item in evidence.values()
        ):
            raise ValueError(f"checkpoint evidence is missing or failed for {key_id}")
    primary_rows = _primary_case_projection(rows)
    normalized = {
        "schema_version": 1,
        "measurements": measurements,
        "overheads": primary_overheads,
        "operation_phase_ownership": "transaction_non_anchor_only",
        "system_operation_ownership": manifest.system_operation_ownership,
        "anchor_operation_ownership": manifest.anchor_operation_ownership,
    }
    observations = {"schema_version": 1, "rows": primary_rows}
    normalized_sha = sha256_bytes(canonical_json(normalized))
    observations_sha = sha256_bytes(canonical_json(observations))
    component_hashes = {
        "normalized-primary.json": normalized_sha,
        "primary-observations.json": observations_sha,
    }
    component_ledger = None
    primary_ledger = None
    diagnostic_ledger = None
    if schedule is not None:
        component_ledger = build_component_cost_ledger(
            manifest, measurements, primary_overheads, schedule
        )
        primary_ledger = {
            "schema_version": component_ledger["schema_version"],
            "rows": component_ledger["primary_rows"],
            "unit_policy": component_ledger["unit_policy"],
        }
        diagnostic_ledger = {
            "schema_version": component_ledger["schema_version"],
            "rows": component_ledger["diagnostic_rows"],
            "unit_policy": component_ledger["unit_policy"],
        }
        component_hashes["primary-component-ledger.json"] = sha256_bytes(
            canonical_json(primary_ledger)
        )
    root = {
        "schema_version": 1,
        "implementation_revision": provenance.get(
            "implementation_revision", provenance.get("revision")
        ),
        "status": "sealed_controlled_candidate",
        "review_only": True,
        "production_write": False,
        "integer_schedule_emitted": False,
        "components": component_hashes,
        "q_formula": list(Q_FORMULA),
        "operation_phase_ownership": "transaction_non_anchor_only",
        "system_operation_ownership": manifest.system_operation_ownership,
        "anchor_operation_ownership": manifest.anchor_operation_ownership,
        "residualization_dag": {
            key.id: list(key.subtract_keys) for key in manifest.overhead_keys
        },
        "formula": (
            "p_hat=sum(raw_gas*c_p)+sum(events*f_p)+proposal_startup+"
            "blocks*block_base+started_non_anchor_transactions*tx_base+"
            "native_value_transfers*native_value_transfer"
        ),
        "thresholds": {
            "required_case_consistency_max": "0.05",
            "checkpoint_ape_max": "0.10",
            "proposal_ape_max": "0.10",
        },
        "normalization_reference_key": manifest.normalization_reference_key,
        "out_of_fit_checkpoint_mapping": OUT_OF_FIT_CHECKPOINTS,
        "provenance": dict(provenance),
    }
    candidate_sha = sha256_bytes(canonical_json(root))
    cycle_samples = {
        "schema_version": 1,
        "rows": [
            {
                "case_id": row.get("case_id"),
                "secondary": row.get("secondary", {"status": "missing"}),
            }
            for row in sorted(rows, key=lambda item: str(item.get("case_id")))
        ],
    }
    return {
        "candidate_manifest": root,
        "candidate_sha256": candidate_sha,
        "normalized_primary": normalized,
        "primary_observations": observations,
        "primary_component_ledger": primary_ledger,
        "diagnostic_component_ledger": diagnostic_ledger,
        "diagnostic_component_ledger_sha256": (
            sha256_bytes(canonical_json(diagnostic_ledger))
            if diagnostic_ledger is not None
            else None
        ),
        "cycle_samples": cycle_samples,
        "cycle_sample_sha256": sha256_bytes(canonical_json(cycle_samples)),
        "c_p": {
            key: value["c_p"]
            for key, value in measurements.items()
            if value.get("status") == "accepted" and "c_p" in value
        },
        "f_p": {
            key: value["f_p"]
            for key, value in measurements.items()
            if value.get("status") == "accepted" and "f_p" in value
        },
        "o_p": primary_overheads,
    }


def build_controlled_bridge(
    manifest: Manifest, samples: Mapping[str, Mapping[str, Any]]
) -> dict[str, Any]:
    rows: dict[str, dict[str, Any]] = {}
    ratios: list[Decimal] = []
    missing: list[str] = []
    for key in manifest.bridge_key_ids:
        sample = samples.get(key)
        if sample is None or sample.get("status") == "unavailable":
            missing.append(key)
            rows[key] = {
                "status": "missing",
                "reason": (
                    sample.get("reason", "missing")
                    if isinstance(sample, Mapping)
                    else "missing"
                ),
            }
            continue
        prover_gas = _decimal(sample.get("prover_gas"), label=f"{key} proverGas")
        instruction_count = _decimal(
            sample.get("instruction_count"), label=f"{key} instruction count"
        )
        if prover_gas <= 0 or instruction_count <= 0:
            missing.append(key)
            rows[key] = {"status": "nonpositive"}
            continue
        ratio = prover_gas / instruction_count
        ratios.append(ratio)
        rows[key] = {
            "status": "available",
            "prover_gas": _decimal_text(prover_gas),
            "instruction_count": _decimal_text(instruction_count),
            "rho": _decimal_text(ratio),
        }
    kappa: Decimal | None = None
    status = "insufficient_data"
    if not missing:
        ordered = sorted(ratios)
        middle = len(ordered) // 2
        kappa = (
            ordered[middle]
            if len(ordered) % 2
            else (ordered[middle - 1] + ordered[middle]) / Decimal(2)
        )
        stable = True
        for key, row in rows.items():
            actual = _decimal(row["prover_gas"], label=f"{key} proverGas")
            prediction = _decimal(
                row["instruction_count"], label=f"{key} instruction count"
            ) * kappa
            ape = abs(prediction - actual) / actual
            row["predicted_prover_gas"] = _decimal_text(prediction)
            row["ape"] = _decimal_text(ape)
            stable = stable and ape <= Decimal("0.10")
        status = "stable_controlled" if stable else "not_stable_controlled"
    result = {
        "schema_version": 1,
        "model": manifest.bridge_model,
        "threshold": _decimal_text(manifest.bridge_controlled_max_ape or Decimal("0.10")),
        "bridge_key_ids": list(manifest.bridge_key_ids),
        "keys": rows,
        "missing_keys": missing,
        "kappa_sp1": _decimal_text(kappa) if kappa is not None else None,
        "status": status,
    }
    result["bridge_sha256"] = sha256_bytes(canonical_json(result))
    return result


def build_controlled_cycle_cost_samples(
    manifest: Manifest,
    case_results: Iterable[Mapping[str, Any]],
    overhead_artifact: Mapping[str, Any],
) -> dict[str, Any]:
    """Project controlled fits into same-unit marginal proverGas/instruction pairs."""
    rows = list(case_results)
    operation_costs = _operation_costs(manifest, rows)
    overhead_primary = overhead_artifact.get("o_p", {})
    overhead_secondary = overhead_artifact.get("o_s", {})
    if not isinstance(overhead_primary, Mapping) or not isinstance(
        overhead_secondary, Mapping
    ):
        raise ValueError("controlled overhead artifact has invalid marginal mappings")
    samples: dict[str, dict[str, Any]] = {}
    for key_id in manifest.bridge_key_ids:
        if key_id in operation_costs:
            cost = operation_costs[key_id]
            if "secondary_cost" in cost:
                samples[key_id] = {
                    "status": "available",
                    "unit": cost["pricing_basis"],
                    "prover_gas": _decimal_text(cost["cost"]),
                    "instruction_count": _decimal_text(cost["secondary_cost"]),
                }
            else:
                samples[key_id] = {
                    "status": "unavailable",
                    "reason": "secondary_operation_cost_unavailable",
                }
        elif key_id in Q_FORMULA and key_id in overhead_primary:
            if key_id in overhead_secondary:
                samples[key_id] = {
                    "status": "available",
                    "unit": next(
                        key.unit for key in manifest.overhead_keys if key.id == key_id
                    ),
                    "prover_gas": _decimal_text(
                        _decimal(overhead_primary[key_id], label=f"overhead {key_id}")
                    ),
                    "instruction_count": _decimal_text(
                        _decimal(
                            overhead_secondary[key_id],
                            label=f"secondary overhead {key_id}",
                        )
                    ),
                }
            else:
                samples[key_id] = {
                    "status": "unavailable",
                    "reason": "secondary_overhead_cost_unavailable",
                }
        else:
            samples[key_id] = {
                "status": "unavailable",
                "reason": "primary_controlled_cost_unavailable",
            }
    return {
        "schema_version": 1,
        "bridge_key_ids": list(manifest.bridge_key_ids),
        "samples": samples,
        "operation_case_results": rows,
        "overhead_case_results": list(overhead_artifact.get("case_results", [])),
        "secondary_policy": (
            "marginal c_p/c_s or f_p/f_s for operations and o_p/o_s for overheads; "
            "unresidualized secondary dependencies remain unavailable"
        ),
    }


def build_diagnostic_overheads(
    manifest: Manifest, rows: Iterable[Mapping[str, Any]]
) -> dict[str, Any]:
    diagnostic_keys = {
        key.id for key in manifest.overhead_keys if key.formula_role == "diagnostic"
    }
    output_rows = []
    for row in rows:
        key_id = row.get("overhead_key_id")
        if key_id not in diagnostic_keys:
            raise ValueError(f"non-diagnostic overhead in diagnostic artifact: {key_id}")
        output_rows.append(dict(row))
    payload = {
        "schema_version": 1,
        "q_formula": list(Q_FORMULA),
        "diagnostic_key_ids": sorted(diagnostic_keys),
        "rows": sorted(
            output_rows,
            key=lambda row: (
                str(row.get("overhead_key_id")),
                str(row.get("case_id")),
            ),
        ),
    }
    return {**payload, "sha256": sha256_bytes(canonical_json(payload))}


def bridge_validation_outcome(controlled_status: str, proposal_status: str) -> str:
    if controlled_status == "insufficient_data":
        return "inconclusive"
    if controlled_status == "not_stable_controlled":
        return "not_supported"
    if controlled_status != "stable_controlled":
        raise ValueError("unknown controlled bridge status")
    if proposal_status == "not_evaluable_on_proposals":
        return "inconclusive"
    return "supported" if proposal_status == "validated_on_proposals" else "not_supported"


def _is_block_owned_operation(operation: Mapping[str, Any]) -> bool:
    return (
        operation.get("phase") == "system"
        or operation.get("disposition") == "system"
        or operation.get("is_anchor") is True
        or operation.get("operation_ownership") == "block_base"
    )


def predict_operations_from_trace(
    manifest: Manifest,
    candidate: Mapping[str, Any],
    operations: Iterable[Mapping[str, Any]],
) -> dict[str, Any]:
    predicted = Decimal(0)
    measured_count = 0
    unmeasured: list[dict[str, Any]] = []
    measured_raw_gas = Decimal(0)
    unmeasured_raw_gas = Decimal(0)
    block_owned_system_operation_count = 0
    for operation in operations:
        if _is_block_owned_operation(operation):
            block_owned_system_operation_count += 1
            continue
        if operation.get("disposition") not in {"attempted", "committed", "system"}:
            continue
        key_id, resolution = resolve_measurement_key(manifest, operation)
        if key_id is None:
            raw_gas = operation.get(
                "interpreter_raw_gas", operation.get("native_gas", 0)
            )
            if raw_gas is not None:
                unmeasured_raw_gas += _decimal(
                    raw_gas, label="unmeasured operation raw gas"
                )
            unmeasured.append(
                {
                    "operation_id": operation.get("operation_id"),
                    "reason": resolution,
                }
            )
            continue
        key = next(item for item in manifest.measurement_keys if item.id == key_id)
        if key.pricing_basis == "raw_gas_slope":
            coefficient = candidate.get("c_p", {}).get(key_id)
            raw_gas = (
                operation.get("interpreter_raw_gas")
                if key.event_match.component == "opcode"
                else operation.get("native_gas")
            )
            if coefficient is None or raw_gas is None:
                unmeasured.append(
                    {
                        "operation_id": operation.get("operation_id"),
                        "reason": "accepted_measurement_missing_candidate_value",
                    }
                )
                continue
            raw_value = _decimal(raw_gas, label=f"operation raw gas for {key_id}")
            if raw_value < 0:
                raise ValueError("operation raw gas cannot be negative")
            predicted += raw_value * _decimal(
                coefficient, label=f"candidate c_p {key_id}"
            )
            measured_raw_gas += raw_value
        else:
            coefficient = candidate.get("f_p", {}).get(key_id)
            if coefficient is None:
                unmeasured.append(
                    {
                        "operation_id": operation.get("operation_id"),
                        "reason": "accepted_measurement_missing_candidate_value",
                    }
                )
                continue
            if operation.get("interpreter_raw_gas") is not None or operation.get(
                "forwarded_gas"
            ) is not None:
                raise ValueError(
                    "spawned forwarded/interpreter gas cannot enter proposal prediction"
                )
            predicted += _decimal(coefficient, label=f"candidate f_p {key_id}")
        measured_count += 1
    return {
        "predicted_prover_gas": _decimal_text(predicted),
        "measured_operation_count": measured_count,
        "unmeasured_operation_count": len(unmeasured),
        "measured_raw_gas": _decimal_text(measured_raw_gas),
        "unmeasured_raw_gas": _decimal_text(unmeasured_raw_gas),
        "block_owned_system_operation_count": block_owned_system_operation_count,
        "unmeasured": unmeasured,
    }


def validate_sealed_candidate(
    candidate: Mapping[str, Any], proposal_rows: Iterable[Mapping[str, Any]]
) -> dict[str, Any]:
    threshold = _decimal(
        candidate.get("thresholds", {}).get("proposal_ape_max"),
        label="proposal APE threshold",
    )
    result_rows = []
    for proposal in proposal_rows:
        actual = _decimal(proposal.get("actual_prover_gas"), label="actual proverGas")
        if actual <= 0:
            raise ValueError("proposal actual proverGas must be positive")
        predicted = Decimal(0)
        measured_operations = 0
        unmeasured_operations = 0
        measured_raw_gas = Decimal(0)
        block_owned_system_operations = 0
        validation_errors = []
        for field_name in (
            "trace_ab_passed",
            "guest_input_join",
            "public_output_join",
            "difficulty_reconciled",
        ):
            if proposal.get(field_name) is not True:
                validation_errors.append(field_name)
        for operation in proposal.get("operations", []):
            if _is_block_owned_operation(operation):
                block_owned_system_operations += 1
                continue
            if operation.get("disposition") not in {"committed", "attempted", "system"}:
                continue
            key = operation.get("measurement_key_id")
            basis = operation.get("basis")
            if basis == "raw_gas_slope" and key in candidate.get("c_p", {}):
                raw_gas = _decimal(operation.get("raw_gas"), label="operation raw gas")
                predicted += raw_gas * _decimal(
                    candidate["c_p"][key], label=f"candidate c_p {key}"
                )
                measured_raw_gas += raw_gas
                measured_operations += 1
            elif basis == "fixed_per_event" and key in candidate.get("f_p", {}):
                if operation.get("forwarded_gas") is not None:
                    raise ValueError("spawned forwarded gas cannot be a proving-gas coefficient")
                predicted += _decimal(candidate["f_p"][key], label=f"candidate f_p {key}")
                measured_operations += 1
            else:
                unmeasured_operations += 1
        features = proposal.get("features", {})
        for key in Q_FORMULA:
            if key not in features:
                validation_errors.append(f"missing_feature:{key}")
            predicted += Decimal(int(features.get(key, 0))) * _decimal(
                candidate.get("o_p", {}).get(key), label=f"candidate overhead {key}"
            )
        denominators = proposal.get("coverage_denominators", {})
        operation_denominator = denominators.get("operation_count")
        raw_gas_denominator = denominators.get("raw_gas")
        spawned_denominator = denominators.get("spawned_event")
        if (
            isinstance(operation_denominator, bool)
            or not isinstance(operation_denominator, int)
            or operation_denominator <= 0
        ):
            validation_errors.append("operation_count_coverage_denominator")
        if (
            isinstance(raw_gas_denominator, bool)
            or not isinstance(raw_gas_denominator, int)
            or raw_gas_denominator <= 0
        ):
            validation_errors.append("raw_gas_coverage_denominator")
        if (
            isinstance(spawned_denominator, bool)
            or not isinstance(spawned_denominator, int)
            or spawned_denominator < 0
        ):
            validation_errors.append("spawned_event_coverage_denominator")
        if predicted <= 0:
            validation_errors.append("nonpositive_prediction")
        ape = abs(predicted - actual) / actual
        result_rows.append(
            {
                "network": proposal.get("network"),
                "proposal_id": proposal.get("proposal_id"),
                "actual_prover_gas": _decimal_text(actual),
                "predicted_prover_gas": _decimal_text(predicted),
                "ape": _decimal_text(ape),
                "measured_operation_count": measured_operations,
                "unmeasured_operation_count": unmeasured_operations,
                "measured_raw_gas": _decimal_text(measured_raw_gas),
                "block_owned_system_operation_count": block_owned_system_operations,
                "coverage_denominators": denominators,
                "validation_errors": validation_errors,
            }
        )
    if not result_rows:
        raise ValueError("proposal validation requires rows")
    network_mapes = {}
    for network in sorted({row["network"] for row in result_rows}):
        apes = [_decimal(row["ape"], label="proposal APE") for row in result_rows if row["network"] == network]
        network_mapes[network] = _decimal_text(sum(apes) / Decimal(len(apes)))
    combined = sum(_decimal(row["ape"], label="proposal APE") for row in result_rows) / Decimal(len(result_rows))
    classification = (
        "candidate_table_validated_at_reported_coverage"
        if all(
            _decimal(row["ape"], label="proposal APE") <= threshold
            and not row["validation_errors"]
            for row in result_rows
        )
        else "candidate_table_not_validated"
    )
    result_rows.sort(key=lambda row: (-_decimal(row["ape"], label="proposal APE"), str(row["network"]), int(row["proposal_id"])))
    return {
        "schema_version": 1,
        "candidate_sha256": candidate.get("candidate_sha256"),
        "classification": classification,
        "combined_mape": _decimal_text(combined),
        "network_mape": network_mapes,
        "underprediction_count": sum(
            _decimal(row["predicted_prover_gas"], label="prediction")
            < _decimal(row["actual_prover_gas"], label="actual")
            for row in result_rows
        ),
        "rows": result_rows,
    }


def seal_candidate_directory(
    run: pathlib.Path,
    manifest: Manifest,
    case_results: Iterable[Mapping[str, Any]],
    overhead_values: Mapping[str, Any],
    provenance: Mapping[str, Any],
    schedule: UnzenSchedule | None = None,
    controlled_cycle_samples: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    if any(run.glob("**/proposal*.json*")):
        raise ValueError("proposal result already exists in calibration directory")
    components = build_candidate_components(
        manifest, case_results, overhead_values, provenance, schedule
    )
    if controlled_cycle_samples is not None:
        components["cycle_samples"] = dict(controlled_cycle_samples)
        components["cycle_sample_sha256"] = sha256_bytes(
            canonical_json(controlled_cycle_samples)
        )
    candidate_dir = run / "candidate"
    samples_dir = run / "samples"
    candidate_dir.mkdir(parents=True, exist_ok=True)
    samples_dir.mkdir(parents=True, exist_ok=True)
    artifacts = {
        candidate_dir / "normalized-primary.json": components["normalized_primary"],
        candidate_dir / "primary-observations.json": components["primary_observations"],
        candidate_dir / "candidate-manifest.json": components["candidate_manifest"],
        samples_dir / "controlled-cycle-cost-samples.json": components["cycle_samples"],
    }
    if components["primary_component_ledger"] is not None:
        artifacts[candidate_dir / "primary-component-ledger.json"] = components[
            "primary_component_ledger"
        ]
    if components["diagnostic_component_ledger"] is not None:
        diagnostic_dir = run / "diagnostics"
        diagnostic_dir.mkdir(parents=True, exist_ok=True)
        artifacts[diagnostic_dir / "component-ledger.json"] = components[
            "diagnostic_component_ledger"
        ]
    for path, payload in artifacts.items():
        path.write_bytes(canonical_json(payload))
    (candidate_dir / "candidate.sha256").write_text(
        components["candidate_sha256"] + "\n"
    )
    return components


def seal_bridge_directory(
    run: pathlib.Path,
    manifest: Manifest,
    controlled_samples: Mapping[str, Mapping[str, Any]],
    controlled_sample_artifact: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    if any(run.glob("**/proposal*.json*")):
        raise ValueError("proposal result already exists in calibration directory")
    experiment = json.loads((run / "experiment.json").read_text())
    revision = experiment.get("implementation_revision")
    bridge_dir = run / "bridge"
    frozen_manifest_path = bridge_dir / "bridge-manifest.json"
    frozen_manifest = json.loads(frozen_manifest_path.read_text())
    expected_contract = {
        "schema_version": 1,
        "implementation_revision": revision,
        "bridge_key_ids": list(manifest.bridge_key_ids),
        "model": manifest.bridge_model,
        "controlled_ape_max": "0.10",
        "proposal_ape_max": "0.10",
        "missing_data": "insufficient_data_is_sealable_and_non_gating",
    }
    if frozen_manifest != expected_contract:
        raise ValueError("premeasurement bridge manifest does not match experiment contract")
    controlled_bridge = build_controlled_bridge(manifest, controlled_samples)
    samples_payload = dict(controlled_sample_artifact) if controlled_sample_artifact else {
        "schema_version": 1,
        "bridge_key_ids": list(manifest.bridge_key_ids),
        "samples": dict(controlled_samples),
    }
    if samples_payload.get("samples") != dict(controlled_samples):
        raise ValueError("controlled sample artifact differs from bridge samples")
    samples_dir = run / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)
    samples_path = samples_dir / "controlled-cycle-cost-samples.json"
    samples_path.write_bytes(canonical_json(samples_payload))
    controlled_path = bridge_dir / "controlled-bridge.json"
    controlled_path.write_bytes(canonical_json(controlled_bridge))
    bridge_root = {
        "schema_version": 1,
        "implementation_revision": revision,
        "components": {
            "bridge-manifest.json": sha256_bytes(canonical_json(frozen_manifest)),
            "controlled-cycle-cost-samples.json": sha256_bytes(
                canonical_json(samples_payload)
            ),
            "controlled-bridge.json": sha256_bytes(
                canonical_json(controlled_bridge)
            ),
        },
        "component_schemas": {
            "bridge-manifest.json": 1,
            "controlled-cycle-cost-samples.json": 1,
            "controlled-bridge.json": 1,
        },
        "status": controlled_bridge["status"],
    }
    if controlled_sample_artifact is not None:
        calibration_id = samples_payload.get("calibration_id")
        candidate_sha256 = samples_payload.get("candidate_sha256")
        if not isinstance(calibration_id, str) or not _is_sha256(candidate_sha256):
            raise ValueError("controlled sample artifact has no candidate/run identity")
        bridge_root["calibration_id"] = calibration_id
        bridge_root["candidate_sha256"] = candidate_sha256
    (bridge_dir / "bridge-root.json").write_bytes(canonical_json(bridge_root))
    bridge_sha = sha256_bytes(canonical_json(bridge_root))
    (bridge_dir / "bridge.sha256").write_text(bridge_sha + "\n")
    return {
        "bridge_root": bridge_root,
        "bridge_sha256": bridge_sha,
        "controlled_bridge": controlled_bridge,
    }


def verify_candidate_directory(run: pathlib.Path) -> dict[str, Any]:
    candidate_dir = run / "candidate"
    manifest_path = candidate_dir / "candidate-manifest.json"
    manifest = json.loads(manifest_path.read_text())
    recorded = _read_digest(candidate_dir / "candidate.sha256")
    actual = sha256_bytes(canonical_json(manifest))
    if recorded != actual:
        raise ValueError("candidate root digest does not match canonical bytes")
    if (
        manifest.get("q_formula") != Q_FORMULA
        or manifest.get("status") != "sealed_controlled_candidate"
        or manifest.get("review_only") is not True
        or manifest.get("production_write") is not False
        or manifest.get("integer_schedule_emitted") is not False
        or manifest.get("thresholds", {}).get("proposal_ape_max") != "0.10"
    ):
        raise ValueError("candidate root contract differs from frozen V1 contract")
    components = manifest.get("components")
    if not isinstance(components, Mapping) or not components:
        raise ValueError("candidate root has no components")
    for relative, expected in components.items():
        if relative not in {
            "normalized-primary.json",
            "primary-observations.json",
            "primary-component-ledger.json",
        }:
            raise ValueError(f"candidate root references unknown component: {relative}")
        path = candidate_dir / relative
        try:
            payload = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"candidate component is unreadable: {relative}") from exc
        if sha256_bytes(canonical_json(payload)) != expected:
            raise ValueError(f"candidate component digest mismatch: {relative}")
    return {
        "candidate_sha256": recorded,
        "candidate_manifest": manifest,
    }


def verify_bridge_directory(run: pathlib.Path) -> dict[str, Any]:
    bridge_dir = run / "bridge"
    root = json.loads((bridge_dir / "bridge-root.json").read_text())
    recorded = _read_digest(bridge_dir / "bridge.sha256")
    if recorded != sha256_bytes(canonical_json(root)):
        raise ValueError("bridge root digest does not match canonical bytes")
    if root.get("status") not in {
        "stable_controlled",
        "not_stable_controlled",
        "insufficient_data",
    }:
        raise ValueError("bridge root has an invalid controlled status")
    expected_paths = {
        "bridge-manifest.json": bridge_dir / "bridge-manifest.json",
        "controlled-cycle-cost-samples.json": run
        / "samples"
        / "controlled-cycle-cost-samples.json",
        "controlled-bridge.json": bridge_dir / "controlled-bridge.json",
    }
    components = root.get("components")
    if not isinstance(components, Mapping) or set(components) != set(expected_paths):
        raise ValueError("bridge root component set differs from frozen V1 contract")
    for name, path in expected_paths.items():
        try:
            payload = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            raise ValueError(f"bridge component is unreadable: {name}") from exc
        if sha256_bytes(canonical_json(payload)) != components[name]:
            raise ValueError(f"bridge component digest mismatch: {name}")
    return {"bridge_sha256": recorded, "bridge_root": root}


def _read_candidate_pool(paths: Iterable[pathlib.Path]) -> list[dict[str, Any]]:
    rows, seen = [], set()
    for path in paths:
        for raw in iter_jsonl(path):
            try:
                row = {"network": str(raw["network"]), "proposal_id": int(raw["proposal_id"]), "block_count": int(raw["block_count"]), "total_zkgas": int(raw["total_zkgas"])}
            except (KeyError, TypeError, ValueError) as exc:
                raise ValueError(f"invalid corpus candidate in {path}") from exc
            key = row["network"], row["proposal_id"]
            if key in seen or row["block_count"] <= 0 or row["total_zkgas"] <= 0:
                raise ValueError(f"invalid or duplicate corpus candidate: {key}")
            rows.append(row); seen.add(key)
    counts = {network: sum(row["network"] == network for row in rows) for network in ("taiko_hoodi", "taiko_mainnet")}
    if len(rows) != 140 or counts != {"taiko_hoodi": 120, "taiko_mainnet": 20}:
        raise ValueError("final corpus candidate pool must contain exactly 120 Hoodi and 20 Mainnet rows")
    return rows


def select_final_validation_corpus(paths: Iterable[pathlib.Path] | None = None) -> list[dict[str, Any]]:
    """Freeze V1's selection before controlled/proposal execution data exists."""
    paths = paths or [FINAL_CORPUS_FIXTURE_DIR / "hoodi-fit.jsonl", FINAL_CORPUS_FIXTURE_DIR / "validation.jsonl"]
    rows = _read_candidate_pool(paths)
    key = lambda row: (row["block_count"], row["total_zkgas"], row["proposal_id"])
    hoodi = sorted((row for row in rows if row["network"] == "taiko_hoodi"), key=key)
    mainnet = sorted((row for row in rows if row["network"] == "taiko_mainnet"), key=key)
    selected = [hoodi[k * 119 // 39] for k in range(40)] + mainnet
    if len({(row["network"], row["proposal_id"]) for row in selected}) != 60:
        raise ValueError("fixed final corpus selection produced duplicate rows")
    return [{**row, "purpose": "final_validation"} for row in selected]


def final_validation_membership(rows: Iterable[Mapping[str, Any]]) -> set[tuple[str, int]]:
    return {(str(row["network"]), int(row["proposal_id"])) for row in rows if row.get("purpose") == "final_validation"}


def assert_integration_smoke_is_disjoint(final_rows: Iterable[Mapping[str, Any]], network: str, proposal_id: int) -> None:
    if (network, proposal_id) in final_validation_membership(final_rows):
        raise ValueError("integration_smoke proposal is in the final_validation corpus")


def freeze_smoke_row(row: Mapping[str, Any], *, purpose: str) -> dict[str, Any]:
    if row.get("purpose") != "integration_smoke" or purpose != "integration_smoke":
        raise ValueError("integration_smoke rows cannot be relabeled after execution")
    return dict(row)


def _proposal_guest_input_identity(guest_input: pathlib.Path) -> tuple[str, int, str]:
    if not guest_input.is_file():
        raise ValueError(f"GuestInput does not exist: {guest_input}")
    try:
        raw = json.loads(guest_input.read_text())
        taiko = raw["taiko"]
        network = taiko["chain_spec"]["name"]
        proposal_id = taiko["proposal_id"]
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        raise ValueError("GuestInput is missing its Taiko network/proposal identity") from error
    if not isinstance(network, str) or not network:
        raise ValueError("GuestInput has an invalid Taiko network")
    if not isinstance(proposal_id, int) or isinstance(proposal_id, bool):
        raise ValueError("GuestInput has an invalid Taiko proposal ID")
    return network, proposal_id, sha256_file(guest_input)


def prepare_integration_smoke(
    network: str,
    proposal_id: int,
    *,
    guest_input: pathlib.Path | None = None,
    purpose: str = "integration_smoke",
) -> dict[str, Any]:
    """Freeze a smoke row before execution; it never joins final validation."""
    final_rows = select_final_validation_corpus()
    assert_integration_smoke_is_disjoint(final_rows, network, proposal_id)
    if purpose != "integration_smoke":
        raise ValueError("integration_smoke rows cannot be relabeled after execution")
    if guest_input is None:
        raise ValueError("integration_smoke requires a GuestInput")
    input_network, input_proposal_id, fixture_sha256 = _proposal_guest_input_identity(
        guest_input
    )
    if (input_network, input_proposal_id) != (network, proposal_id):
        raise ValueError("GuestInput does not match integration_smoke identity")
    return freeze_smoke_row(
        {
            "network": network,
            "proposal_id": proposal_id,
            "purpose": "integration_smoke",
            "fixture_sha256": fixture_sha256,
        },
        purpose=purpose,
    )


def verify_prepared_integration_smoke(
    record_path: pathlib.Path,
    *,
    network: str,
    proposal_id: int,
    guest_input: pathlib.Path,
) -> dict[str, Any]:
    if not record_path.is_file():
        raise ValueError("run-proposal requires a prepared integration_smoke record")
    record = json.loads(record_path.read_text())
    if record.get("purpose") != "integration_smoke":
        raise ValueError("integration_smoke rows cannot be relabeled after execution")
    if record.get("network") != network or record.get("proposal_id") != proposal_id:
        raise ValueError("prepared integration_smoke record does not match run-proposal identity")
    input_network, input_proposal_id, fixture_sha256 = _proposal_guest_input_identity(
        guest_input
    )
    if (input_network, input_proposal_id) != (network, proposal_id):
        raise ValueError("GuestInput does not match run-proposal identity")
    if record.get("fixture_sha256") != fixture_sha256:
        raise ValueError("prepared integration_smoke record does not match GuestInput bytes")
    assert_integration_smoke_is_disjoint(
        select_final_validation_corpus(), network, proposal_id
    )
    return freeze_smoke_row(record, purpose="integration_smoke")


def proposal_workload_id(guest_input_sha256: str) -> str:
    if len(guest_input_sha256) != 64:
        raise ValueError("guest_input_sha256 must be a SHA256 digest")
    return sha256_bytes(canonical_json({"guest_input_sha256": guest_input_sha256, "kind": "proposal"}))


def _is_sha256(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 64 and all(
        char in "0123456789abcdef" for char in value
    )


def _is_git_revision(value: Any) -> bool:
    return isinstance(value, str) and len(value) == 40 and all(
        char in "0123456789abcdef" for char in value
    )


def _repo_relative_path(value: Any, *, field_name: str) -> pathlib.Path:
    if not isinstance(value, str):
        raise ValueError(f"{field_name} must be a repository-relative path")
    path = pathlib.Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError(f"{field_name} must be a repository-relative path")
    return REPO_ROOT / path


def _resolve_repo_path(value: pathlib.Path | str, *, field_name: str) -> pathlib.Path:
    path = pathlib.Path(value)
    root = REPO_ROOT.resolve()
    resolved = (path if path.is_absolute() else root / path).resolve()
    if not resolved.is_relative_to(root):
        raise ValueError(f"{field_name} must stay within the repository")
    return resolved


def _unzen_activation_from_chain_spec(
    chain_spec_path: pathlib.Path, network: str
) -> tuple[str, int]:
    data = json.loads(chain_spec_path.read_text())
    if not isinstance(data, list):
        raise ValueError("chain-spec file must be a JSON array")
    matches = [item for item in data if isinstance(item, Mapping) and item.get("name") == network]
    if len(matches) != 1:
        raise ValueError(f"chain-spec file must contain exactly one {network} entry")
    hard_forks = matches[0].get("hard_forks")
    unzen = hard_forks.get("UNZEN") if isinstance(hard_forks, Mapping) else None
    if not isinstance(unzen, Mapping) or len(unzen) != 1:
        raise ValueError(f"chain-spec for {network} must declare hard_forks.UNZEN")
    kind, activation = next(iter(unzen.items()))
    if kind not in ("Timestamp", "Block") or isinstance(activation, bool) or not isinstance(activation, int):
        raise ValueError(f"chain-spec for {network} has an invalid UNZEN activation")
    return kind, activation


def _discovery_rows(path: pathlib.Path) -> dict[int, Mapping[str, Any]]:
    data = json.loads(path.read_text())
    values = data if isinstance(data, list) else data.get("proposals", [])
    if not isinstance(values, list):
        raise ValueError("proposal discovery output must contain a list")
    result = {int(row["proposal_id"]): row for row in values}
    if len(result) != len(values):
        raise ValueError("proposal discovery output has duplicate proposal IDs")
    return result


def _discovery_value(row: Mapping[str, Any], *names: str) -> int:
    for name in names:
        if name in row:
            return int(row[name])
    raise ValueError(f"discovery output missing one of {names}")


def _guest_input_block_summary(path: pathlib.Path) -> tuple[int, int]:
    guest_input = json.loads(path.read_text())
    blocks = guest_input.get("blocks")
    if blocks is None:
        witnesses = guest_input.get("witnesses")
        if isinstance(witnesses, list):
            blocks = [item.get("block", {}).get("header", {}) for item in witnesses]
    if not isinstance(blocks, list) or not blocks:
        raise ValueError("GuestInput must contain blocks")
    values = []
    for block in blocks:
        value = block.get("block_difficulty", block.get("difficulty"))
        value = int(value, 0) if isinstance(value, str) else value
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise ValueError("GuestInput has non-positive block difficulty")
        values.append(value)
    return len(values), sum(values)


def _assert_guest_input_post_unzen(path: pathlib.Path, activation: tuple[str, int]) -> None:
    guest_input = json.loads(path.read_text())
    blocks = guest_input.get("blocks")
    if blocks is None:
        witnesses = guest_input.get("witnesses")
        if isinstance(witnesses, list):
            blocks = [item.get("block", {}).get("header", {}) for item in witnesses]
    if not isinstance(blocks, list) or not blocks:
        raise ValueError("GuestInput must contain blocks for post-Unzen validation")
    kind, threshold = activation
    for block in blocks:
        value = block.get("timestamp") if kind == "Timestamp" else block.get("number")
        if isinstance(value, str):
            value = int(value, 0)
        if isinstance(value, bool) or not isinstance(value, int) or value < threshold:
            raise ValueError("GuestInput is not post-Unzen under the acquisition chain spec")


def write_deterministic_tar(corpus_root: pathlib.Path, archive: pathlib.Path) -> str:
    archive.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "w") as tar:
        for path in sorted(item for item in corpus_root.rglob("*") if item.is_file()):
            info = tar.gettarinfo(str(path), arcname=str(path.relative_to(corpus_root)))
            info.uid = info.gid = info.mtime = 0; info.uname = info.gname = ""; info.mode = 0o644
            with path.open("rb") as fh: tar.addfile(info, fh)
    return sha256_file(archive)


def prepare_corpus(*, corpus_root: pathlib.Path, l1_rpc_by_network: Mapping[str, str], l2_rpc_by_network: Mapping[str, str], chain_spec_hash_by_network: Mapping[str, str], chain_spec_path_by_network: Mapping[str, pathlib.Path], implementation_revision: str | None = None, manifest_path: pathlib.Path | None = None) -> dict[str, Any]:
    rows = select_final_validation_corpus()
    if any(row.get("purpose") != "final_validation" for row in rows):
        raise ValueError("prepare-corpus accepts only final_validation rows")
    implementation_revision = implementation_revision or git_head()
    if not _is_git_revision(implementation_revision):
        raise ValueError("implementation_revision must be a git revision")
    if git_head() != implementation_revision:
        raise ValueError("current HEAD does not match implementation_revision")
    assert_generated_paths_only(git_worktree_status())
    corpus_root = _resolve_repo_path(corpus_root, field_name="corpus_root")
    if manifest_path is not None:
        manifest_path = _resolve_repo_path(manifest_path, field_name="manifest_path")
    corpus_root.mkdir(parents=True, exist_ok=True); final_rows = []
    for network in sorted({str(row["network"]) for row in rows}):
        selected = sorted((row for row in rows if row["network"] == network), key=lambda row: row["proposal_id"])
        if not all(network in values for values in (l1_rpc_by_network, l2_rpc_by_network, chain_spec_hash_by_network, chain_spec_path_by_network)):
            raise ValueError(f"missing RPC/chain-spec input for {network}")
        expected_chain_spec_hash = chain_spec_hash_by_network[network]
        chain_spec_path = _resolve_repo_path(
            chain_spec_path_by_network[network], field_name="chain_spec_file"
        )
        if not _is_sha256(expected_chain_spec_hash) or sha256_file(chain_spec_path) != expected_chain_spec_hash:
            raise ValueError(f"chain-spec hash does not match {network} chain-spec file")
        unzen_activation = _unzen_activation_from_chain_spec(chain_spec_path, network)
        with tempfile.TemporaryDirectory(prefix="opcode-gas-discovery-") as tmp:
            discovery = pathlib.Path(tmp) / f"{network}.json"
            subprocess.run([os.environ.get("PYTHON_BIN", str(pathlib.Path.home() / ".venv/bin/python")), "scripts/regression/stress_shasta_proposal.py", "--network", network, "--l1-rpc", l1_rpc_by_network[network], "--l2-rpc", l2_rpc_by_network[network], "--proposal-ids", ",".join(str(row["proposal_id"]) for row in selected), "--discover-only", "--proposal-out", str(discovery)], cwd=REPO_ROOT, check=True)
            discovered = _discovery_rows(discovery)
            for row in selected:
                proposal_id = int(row["proposal_id"])
                if proposal_id not in discovered: raise ValueError(f"acquisition did not discover fixed proposal {network}/{proposal_id}")
                item, output = discovered[proposal_id], corpus_root / network / f"proposal_{proposal_id}.json"
                output.parent.mkdir(parents=True, exist_ok=True)
                temporary_output = output.with_name(f".{output.name}.tmp")
                subprocess.run(["target/release/preflight", "--chain-spec-file", str(chain_spec_path), "--network", network, "--rpc-url", l2_rpc_by_network[network], "--l1-rpc-url", l1_rpc_by_network[network], "--proposal-id", str(proposal_id), "--l1-inclusion-block-number", str(_discovery_value(item, "l1_inclusion_block_number", "inclusion_block")), "--last-anchor-block-number", str(_discovery_value(item, "last_anchor_block_number", "previous_anchor_block")), "--l2-start", str(_discovery_value(item, "l2_start", "l2_block_start")), "--l2-end", str(_discovery_value(item, "l2_end", "l2_block_end")), "--proof-type", "sp1", "--validate", "--output", str(temporary_output)], cwd=REPO_ROOT, check=True)
                block_count, difficulty_sum = _guest_input_block_summary(temporary_output)
                _assert_guest_input_post_unzen(temporary_output, unzen_activation)
                if block_count != row["block_count"] or difficulty_sum != row["total_zkgas"]:
                    raise ValueError(f"GuestInput does not match fixed count/total for {network}/{proposal_id}")
                os.replace(temporary_output, output)
                guest_sha = sha256_file(output)
                final_rows.append({**row, "fixture_path": str(output.relative_to(REPO_ROOT)) if output.is_relative_to(REPO_ROOT) else str(output), "guest_input_sha256": guest_sha, "workload_id": proposal_workload_id(guest_sha), "l1_inclusion_block_number": _discovery_value(item, "l1_inclusion_block_number", "inclusion_block"), "last_anchor_block_number": _discovery_value(item, "last_anchor_block_number", "previous_anchor_block"), "l2_start": _discovery_value(item, "l2_start", "l2_block_start"), "l2_end": _discovery_value(item, "l2_block_end", "l2_end"), "block_difficulty_sum": difficulty_sum, "acquisition_chain_spec_sha256": chain_spec_hash_by_network[network]})
    final_rows.sort(key=lambda row: (row["network"], row["proposal_id"]))
    archive = corpus_root.parent / "sp1-mainnet-hoodi-v1.tar"
    manifest = {"schema_version": 1, "implementation_revision": implementation_revision, "corpus_root": str(corpus_root.relative_to(REPO_ROOT)), "archive_path": str(archive.relative_to(REPO_ROOT)), "archive_uri": "local_unpublished", "archive_generation": None, "archive_size_bytes": None, "archive_sha256": write_deterministic_tar(corpus_root, archive), "rows": final_rows}
    if manifest_path:
        manifest_path.parent.mkdir(parents=True, exist_ok=True); manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return manifest


def publish_corpus(archive: pathlib.Path, object_uri: str) -> dict[str, Any]:
    digest = sha256_file(archive)
    if not object_uri.startswith("gs://") or not object_uri.endswith(f"{digest}.tar"):
        raise ValueError("GCS object name must end in <archive_sha256>.tar")
    try:
        subprocess.run(["gcloud", "storage", "cp", "--if-generation-match=0", "--print-created-message", str(archive), object_uri], check=True, capture_output=True, text=True)
    except subprocess.CalledProcessError:
        pass
    metadata = json.loads(subprocess.run(["gcloud", "storage", "objects", "describe", object_uri, "--format=json"], check=True, capture_output=True, text=True).stdout)
    generation, size = int(metadata["generation"]), int(metadata["size"])
    if generation <= 0 or size <= 0: raise ValueError("published corpus must have positive generation and size")
    if size != archive.stat().st_size:
        raise ValueError("published corpus size does not match local archive")
    versioned = f"{object_uri}#{generation}"
    with tempfile.TemporaryDirectory(prefix="opcode-gas-corpus-readback-") as tmp:
        readback = pathlib.Path(tmp) / "corpus.tar"
        subprocess.run(["gcloud", "storage", "cp", versioned, str(readback)], check=True)
        if sha256_file(readback) != digest: raise ValueError("exact-generation GCS readback SHA256 mismatch")
    return {"archive_uri": versioned, "archive_generation": generation, "archive_size_bytes": size, "archive_sha256": digest}


def git_head(source_root: pathlib.Path = REPO_ROOT) -> str:
    return subprocess.run(["git", "rev-parse", "HEAD"], cwd=source_root, check=True, capture_output=True, text=True).stdout.strip()


def git_worktree_status(source_root: pathlib.Path = REPO_ROOT) -> str:
    return subprocess.run(["git", "status", "--porcelain"], cwd=source_root, check=True, capture_output=True, text=True).stdout


def assert_generated_paths_only(status: str) -> None:
    for line in status.splitlines():
        if line and not any(line[3:].split(" -> ")[-1].startswith(prefix) for prefix in GENERATED_EXPERIMENT_PREFIXES):
            raise ValueError(f"dirty implementation path is not an allowed generated output: {line[3:]}")


def _toml_scalar_list(path: pathlib.Path, key: str) -> list[str]:
    value = tomllib.loads(path.read_text()).get(key)
    if not isinstance(value, list) or not value or not all(isinstance(item, str) for item in value): raise ValueError(f"controlled manifest requires nonempty {key}")
    return value


def _locked_package_version(package_name: str) -> str:
    lock = tomllib.loads((REPO_ROOT / "Cargo.lock").read_text())
    versions = [
        package.get("version")
        for package in lock.get("package", [])
        if package.get("name") == package_name and isinstance(package.get("version"), str)
    ]
    if len(set(versions)) != 1:
        raise ValueError(f"Cargo.lock must contain one {package_name} version")
    return versions[0]


def _rust_version() -> str:
    return subprocess.run(["rustc", "--version"], check=True, capture_output=True, text=True).stdout.strip()


def controlled_manifest_rows_sha256(path: pathlib.Path) -> str:
    data = tomllib.loads(path.read_text())
    return sha256_bytes(canonical_json(data))


def verify_frozen_controlled_manifest(
    run: pathlib.Path, supplied_manifest: pathlib.Path
) -> tuple[Manifest, Mapping[str, Any]]:
    experiment = json.loads((run / "experiment.json").read_text())
    identity = experiment.get("calibration_identity")
    if not isinstance(identity, Mapping):
        raise ValueError("calibration run has no calibration_identity")
    expected_hash = identity.get("controlled_manifest_sha256")
    expected_rows = identity.get("controlled_manifest_rows_sha256")
    if not _is_sha256(expected_hash) or not _is_sha256(expected_rows):
        raise ValueError("calibration identity has no sealed controlled manifest")
    frozen = run / "controlled-manifest.toml"
    seal = run / "controlled-manifest.sha256"
    if not frozen.is_file() or not seal.is_file() or _read_digest(seal) != expected_hash:
        raise ValueError("calibration controlled manifest seal is missing or invalid")
    if sha256_file(frozen) != expected_hash:
        raise ValueError("calibration controlled manifest bytes do not match calibration identity")
    if sha256_file(supplied_manifest) != expected_hash:
        raise ValueError("controlled manifest does not match calibration identity")
    if controlled_manifest_rows_sha256(supplied_manifest) != expected_rows:
        raise ValueError("controlled manifest rows do not match calibration identity")
    return load_manifest(frozen), identity


EXPERIMENT_IDENTITY_DUPLICATE_FIELDS = (
    "implementation_revision",
    "alethia_reth_revision",
    "complete_schedule_sha256",
    "controlled_manifest_sha256",
    "guest_artifacts",
    "guest_artifacts_sha256",
    "rust_version",
    "sp1_sdk_version",
    "normalization_reference_key",
    "primary_metric",
    "sp1_execution_parameters",
    "sp1_instruction_count",
    "workload_identity_schema_version",
    "workload_canonicalization",
    "primary_formulas",
    "q_formula",
    "out_of_fit_checkpoint",
    "quality_gates",
)


def experiment_provenance_declaration(
    experiment: Mapping[str, Any],
) -> dict[str, Any]:
    """Return the caller-visible provenance declaration frozen by experiment.json."""
    identity = experiment.get("calibration_identity")
    if not isinstance(identity, Mapping):
        raise ValueError("calibration run has no calibration_identity")
    calibration_id = experiment.get("calibration_id")
    expected_calibration_id = sha256_bytes(canonical_json(identity))[:24]
    if calibration_id != expected_calibration_id:
        raise ValueError("experiment has a stale content-addressed calibration_id")
    for field in EXPERIMENT_IDENTITY_DUPLICATE_FIELDS:
        if field not in identity or experiment.get(field) != identity[field]:
            raise ValueError(f"experiment duplicate field {field} differs from identity")
    revision = experiment.get("implementation_revision")
    manifest_sha256 = identity.get("controlled_manifest_sha256")
    rows_sha256 = identity.get("controlled_manifest_rows_sha256")
    if (
        not isinstance(calibration_id, str)
        or not _is_git_revision(revision)
        or not _is_sha256(manifest_sha256)
        or not _is_sha256(rows_sha256)
    ):
        raise ValueError("experiment has invalid controlled calibration provenance")
    if experiment.get("controlled_manifest_sha256") != manifest_sha256:
        raise ValueError("experiment controlled manifest identity is inconsistent")
    if identity.get("sp1_execution_parameters") != sp1_execution_parameters():
        raise ValueError("experiment has unexpected SP1 execution parameters")
    return {
        "schema_version": 1,
        "calibration_id": calibration_id,
        "implementation_revision": revision,
        "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
        "controlled_manifest_sha256": manifest_sha256,
        "controlled_manifest_rows_sha256": rows_sha256,
    }


def validate_calibration_execution_identity(
    calibration_run: pathlib.Path,
) -> Mapping[str, Any]:
    """Reject execution when the frozen calibration no longer describes this checkout."""
    experiment_path = calibration_run / "experiment.json"
    provenance_path = calibration_run / "provenance.json"
    if not experiment_path.is_file() or not provenance_path.is_file():
        raise ValueError("calibration run is missing experiment provenance")
    experiment = json.loads(experiment_path.read_text())
    declaration = experiment_provenance_declaration(experiment)
    if declaration["calibration_id"] != calibration_run.name:
        raise ValueError("experiment calibration_id does not match calibration directory")
    if json.loads(provenance_path.read_text()) != declaration:
        raise ValueError("persisted calibration provenance differs from experiment identity")
    if git_head() != declaration["implementation_revision"]:
        raise ValueError("current HEAD does not match frozen implementation_revision")
    assert_generated_paths_only(git_worktree_status())

    identity = experiment["calibration_identity"]
    guest_artifacts = identity.get("guest_artifacts")
    if not isinstance(guest_artifacts, Mapping) or not guest_artifacts:
        raise ValueError("calibration identity has no frozen guest artifacts")
    if sha256_bytes(canonical_json(guest_artifacts)) != identity.get(
        "guest_artifacts_sha256"
    ):
        raise ValueError("frozen guest artifact map digest does not match calibration identity")
    for relative, expected_sha256 in guest_artifacts.items():
        if (
            not isinstance(relative, str)
            or pathlib.Path(relative).is_absolute()
            or not _is_sha256(expected_sha256)
        ):
            raise ValueError("calibration identity has an invalid guest artifact entry")
        artifact = (REPO_ROOT / relative).resolve()
        if not artifact.is_relative_to(REPO_ROOT.resolve()):
            raise ValueError("frozen guest artifact path escapes the repository")
        if not artifact.is_file() or sha256_file(artifact) != expected_sha256:
            raise ValueError(f"frozen guest artifact changed: {relative}")
    return identity


def prepare_calibration(
    output_root: pathlib.Path,
    controlled_manifest: pathlib.Path,
    *,
    implementation_revision: str | None = None,
    complete_schedule_hash: str | None = None,
) -> dict[str, Any]:
    assert_generated_paths_only(git_worktree_status())
    revision = implementation_revision or git_head()
    if implementation_revision is not None and git_head() != revision:
        raise ValueError("current HEAD does not match implementation_revision")
    controlled_hash = sha256_file(controlled_manifest)
    controlled_rows_hash = controlled_manifest_rows_sha256(controlled_manifest)
    data = tomllib.loads(controlled_manifest.read_text())
    if data.get("include_uzen_pure_opcodes") or data.get("include_uzen_precompile_bodies"):
        raise ValueError("controlled manifest must not use implicit include flags")
    bridge_key_ids = _toml_scalar_list(controlled_manifest, "bridge_key_ids")
    q_formula = _toml_scalar_list(controlled_manifest, "q_formula")
    normalization = data.get("normalization_reference_key")
    if normalization != "opcode:0x01" or normalization not in bridge_key_ids:
        raise ValueError("bridge_key_ids must include normalization_reference_key opcode:0x01")
    if q_formula != Q_FORMULA or any(key not in bridge_key_ids for key in Q_FORMULA):
        raise ValueError("controlled manifest must freeze the exact Q_formula in bridge_key_ids")
    if complete_schedule_hash is None:
        complete_schedule_hash = schedule_sha256(current_uzen_schedule())
    if len(complete_schedule_hash) != 64:
        raise ValueError("complete_schedule_hash must be a SHA256 digest")
    workspace = tomllib.loads((REPO_ROOT / "Cargo.toml").read_text())
    alethia_revision = workspace["workspace"]["dependencies"]["alethia-reth-chainspec"]["rev"]
    guest_artifacts = {
        str(path.relative_to(REPO_ROOT)): sha256_file(path)
        for path in sorted((REPO_ROOT / "crates/guests/elf").glob("sp1*"))
        if path.is_file() and (path.name.endswith(".elf") or path.name.endswith(".vk.bin"))
    }
    guest_artifacts_sha256 = sha256_bytes(canonical_json(guest_artifacts))
    rust_version = _rust_version()
    sp1_sdk_version = _locked_package_version("sp1-sdk")
    execution_parameters = sp1_execution_parameters()
    out_of_fit_checkpoint = {
        "mapping": OUT_OF_FIT_CHECKPOINTS,
        "ape_max": 0.10,
    }
    quality_gates = {"checkpoint_ape_max": 0.10}
    bridge_contract = {
        "bridge_key_ids": bridge_key_ids,
        "model": "through_origin_equal_key_median",
        "controlled_ape_max": "0.10",
        "proposal_ape_max": "0.10",
        "missing_data": "insufficient_data_is_sealable_and_non_gating",
    }
    calibration_identity = {
        "implementation_revision": revision,
        "alethia_reth_revision": alethia_revision,
        "rust_version": rust_version,
        "sp1_sdk_version": sp1_sdk_version,
        "controlled_manifest_sha256": controlled_hash,
        "controlled_manifest_rows_sha256": controlled_rows_hash,
        "complete_schedule_sha256": complete_schedule_hash,
        "guest_artifacts": guest_artifacts,
        "guest_artifacts_sha256": guest_artifacts_sha256,
        "normalization_reference_key": normalization,
        "sp1_execution_parameters": execution_parameters,
        "primary_metric": "proverGas",
        "sp1_instruction_count": "secondary_non_gating",
        "workload_identity_schema_version": 1,
        "workload_canonicalization": "sha256(canonical_json(workload_spec))",
        "primary_formulas": {
            "candidate_cost": "g_p(k) / r(k)",
            "candidate_multiplier": "c_p(k) / c_p(opcode:0x01)",
        },
        "q_formula": q_formula,
        "out_of_fit_checkpoint": out_of_fit_checkpoint,
        "quality_gates": quality_gates,
        "bridge": bridge_contract,
    }
    calibration_id = sha256_bytes(
        canonical_json(calibration_identity)
    )[:24]
    run = output_root / "runs" / calibration_id
    if run.exists():
        raise ValueError(f"calibration directory already exists: {run}")
    experiment = {
        "schema_version": 1,
        "calibration_id": calibration_id,
        "implementation_revision": revision,
        "dirty_state": False,
        "calibration_identity": calibration_identity,
        "alethia_reth_revision": alethia_revision,
        "complete_schedule_sha256": complete_schedule_hash,
        "controlled_manifest_sha256": controlled_hash,
        "guest_artifacts": guest_artifacts,
        "guest_artifacts_sha256": guest_artifacts_sha256,
        "rust_version": rust_version,
        "sp1_sdk_version": sp1_sdk_version,
        "normalization_reference_key": normalization,
        "primary_metric": "proverGas",
        "sp1_execution_parameters": execution_parameters,
        "sp1_instruction_count": "secondary_non_gating",
        "workload_identity_schema_version": 1,
        "workload_canonicalization": "sha256(canonical_json(workload_spec))",
        "primary_formulas": {
            "candidate_cost": "g_p(k) / r(k)",
            "candidate_multiplier": "c_p(k) / c_p(opcode:0x01)",
        },
        "q_formula": q_formula,
        "out_of_fit_checkpoint": out_of_fit_checkpoint,
        "quality_gates": quality_gates,
    }
    bridge = {
        "schema_version": 1,
        "implementation_revision": revision,
        **bridge_contract,
    }
    (run / "bridge").mkdir(parents=True)
    (run / "controlled-manifest.toml").write_bytes(controlled_manifest.read_bytes())
    (run / "controlled-manifest.sha256").write_text(controlled_hash + "\n")
    (run / "experiment.json").write_text(json.dumps(experiment, indent=2, sort_keys=True) + "\n")
    (run / "provenance.json").write_text(
        json.dumps(
            experiment_provenance_declaration(experiment), indent=2, sort_keys=True
        )
        + "\n"
    )
    (run / "bridge" / "bridge-manifest.json").write_text(
        json.dumps(bridge, indent=2, sort_keys=True) + "\n"
    )
    return experiment


def _read_digest(path: pathlib.Path) -> str:
    value = path.read_text().strip()
    if len(value) != 64 or any(char not in "0123456789abcdef" for char in value): raise ValueError(f"invalid SHA256 digest in {path}")
    return value


def _validate_frozen_corpus(corpus: Mapping[str, Any], revision: str) -> list[Mapping[str, Any]]:
    if corpus.get("implementation_revision") != revision:
        raise ValueError("corpus implementation_revision does not match calibration")
    archive_sha = corpus.get("archive_sha256")
    if not _is_sha256(archive_sha):
        raise ValueError("corpus must record archive_sha256")
    uri = corpus.get("archive_uri")
    if not isinstance(uri, str) or not uri.startswith("gs://") or uri.count("#") != 1:
        raise ValueError("corpus is local_unpublished or has no exact archive identity")
    object_uri, generation_text = uri.rsplit("#", 1)
    if not generation_text.isdecimal() or int(generation_text) <= 0 or not object_uri.endswith(f"{archive_sha}.tar"):
        raise ValueError("corpus archive URI is not content-addressed at an exact generation")
    if corpus.get("archive_generation") != int(generation_text) or not isinstance(corpus.get("archive_size_bytes"), int) or corpus["archive_size_bytes"] <= 0:
        raise ValueError("corpus is local_unpublished or has no exact archive identity")
    corpus_root = _repo_relative_path(corpus.get("corpus_root"), field_name="corpus_root")
    archive_path = _repo_relative_path(corpus.get("archive_path"), field_name="archive_path")
    if not archive_path.is_file() or sha256_file(archive_path) != archive_sha:
        raise ValueError("local corpus archive does not match archive_sha256")
    if corpus["archive_size_bytes"] != archive_path.stat().st_size:
        raise ValueError("local corpus archive size does not match archive_size_bytes")
    rows = corpus.get("rows")
    expected = {
        (row["network"], row["proposal_id"]): row
        for row in select_final_validation_corpus()
    }
    if not isinstance(rows, list) or len(rows) != len(expected):
        raise ValueError("validation requires exactly 60 completed GuestInputs")
    seen = set()
    for row in rows:
        key = row.get("network"), row.get("proposal_id")
        if key in seen or key not in expected or any(
            row.get(field) != expected[key][field] for field in ("block_count", "total_zkgas")
        ):
            raise ValueError("corpus rows must be the exact selected final_validation corpus")
        seen.add(key)
        if (
            row.get("purpose") != "final_validation"
            or not _is_sha256(row.get("guest_input_sha256"))
            or not _is_sha256(row.get("acquisition_chain_spec_sha256"))
        ):
            raise ValueError("invalid final_validation corpus row")
        fixture = _repo_relative_path(row.get("fixture_path"), field_name="fixture_path")
        if not fixture.is_relative_to(corpus_root) or not fixture.is_file():
            raise ValueError("final_validation fixture is missing from the local corpus")
        if sha256_file(fixture) != row["guest_input_sha256"]:
            raise ValueError("final_validation fixture hash does not match manifest")
        block_count, difficulty_sum = _guest_input_block_summary(fixture)
        if (
            block_count != row["block_count"]
            or difficulty_sum != row["total_zkgas"]
            or row.get("block_difficulty_sum") != difficulty_sum
        ):
            raise ValueError("final_validation fixture does not reconcile per block")
        if row.get("workload_id") != proposal_workload_id(row["guest_input_sha256"]):
            raise ValueError("proposal workload_id does not match guest_input_sha256")
    if seen != set(expected):
        raise ValueError("corpus rows must be the exact selected final_validation corpus")
    return rows


def validate_manifest_for_publication(manifest: Mapping[str, Any], archive: pathlib.Path) -> None:
    archive_sha = sha256_file(archive)
    if manifest.get("archive_sha256") != archive_sha:
        raise ValueError("manifest archive_sha256 does not match the archive to publish")
    corpus_root = _repo_relative_path(manifest.get("corpus_root"), field_name="corpus_root")
    if not archive.is_relative_to(REPO_ROOT) or not archive.is_file():
        raise ValueError("archive to publish must be a repository-local corpus archive")
    if manifest.get("archive_path") != str(archive.relative_to(REPO_ROOT)):
        raise ValueError("manifest archive_path does not match the archive to publish")
    rows = manifest.get("rows")
    expected = {
        (row["network"], row["proposal_id"]): row
        for row in select_final_validation_corpus()
    }
    if not isinstance(rows, list) or len(rows) != len(expected):
        raise ValueError("manifest must contain the exact 60 final_validation rows")
    seen = set()
    for row in rows:
        key = row.get("network"), row.get("proposal_id")
        if key in seen or key not in expected or any(
            row.get(field) != expected[key][field] for field in ("block_count", "total_zkgas")
        ):
            raise ValueError("manifest rows are not the exact final_validation corpus")
        seen.add(key)
        if row.get("purpose") != "final_validation" or not _is_sha256(row.get("guest_input_sha256")):
            raise ValueError("manifest rows have invalid fixture identities")
        fixture = _repo_relative_path(row.get("fixture_path"), field_name="fixture_path")
        if not fixture.is_relative_to(corpus_root) or not fixture.is_file() or sha256_file(fixture) != row.get("guest_input_sha256"):
            raise ValueError("manifest rows do not match local corpus fixtures")
        block_count, difficulty_sum = _guest_input_block_summary(fixture)
        if (
            block_count != row["block_count"]
            or difficulty_sum != row["total_zkgas"]
            or row.get("block_difficulty_sum") != difficulty_sum
            or not _is_sha256(row.get("acquisition_chain_spec_sha256"))
        ):
            raise ValueError("manifest rows do not reconcile completed local fixtures")
    if seen != set(expected):
        raise ValueError("manifest rows are not the exact final_validation corpus")
    with tarfile.open(archive, "r") as tar:
        files = [member for member in tar.getmembers() if member.isfile()]
        members = {member.name: member for member in files}
        expected_names = {
            _repo_relative_path(row["fixture_path"], field_name="fixture_path")
            .relative_to(corpus_root)
            .as_posix()
            for row in rows
        }
        if len(members) != len(files) or set(members) != expected_names:
            raise ValueError("archive contents do not exactly match manifest fixtures")
        for row in rows:
            fixture = _repo_relative_path(row["fixture_path"], field_name="fixture_path")
            member = members.get(fixture.relative_to(corpus_root).as_posix())
            if member is None:
                raise ValueError("archive does not contain every manifest fixture")
            source = tar.extractfile(member)
            if source is None or sha256_bytes(source.read()) != row["guest_input_sha256"]:
                raise ValueError("archive fixture bytes do not match manifest")


def _verify_seal(path: pathlib.Path, payload: Mapping[str, Any], *, label: str) -> str:
    recorded = _read_digest(path)
    actual = sha256_bytes(canonical_json(payload))
    if recorded != actual:
        raise ValueError(f"{label} seal does not match canonical manifest bytes")
    return recorded


def prepare_validation(output_root: pathlib.Path, run: pathlib.Path, corpus_path: pathlib.Path) -> dict[str, Any]:
    revision = str(json.loads((run / "experiment.json").read_text())["implementation_revision"])
    if git_head() != revision:
        raise ValueError("current HEAD does not match implementation_revision")
    assert_generated_paths_only(git_worktree_status())
    candidate_manifest = json.loads((run / "candidate" / "candidate-manifest.json").read_text())
    bridge_root = json.loads((run / "bridge" / "bridge-root.json").read_text())
    if candidate_manifest.get("implementation_revision") != revision:
        raise ValueError("candidate provenance does not match implementation_revision")
    if bridge_root.get("implementation_revision") != revision:
        raise ValueError("bridge provenance does not match implementation_revision")
    candidate_sha = _verify_seal(
        run / "candidate" / "candidate.sha256", candidate_manifest, label="candidate"
    )
    bridge_sha = _verify_seal(run / "bridge" / "bridge.sha256", bridge_root, label="bridge")
    corpus = json.loads(corpus_path.read_text())
    _validate_frozen_corpus(corpus, revision)
    validate_manifest_for_publication(
        corpus, _repo_relative_path(corpus["archive_path"], field_name="archive_path")
    )
    identity = {
        "candidate_sha256": candidate_sha,
        "bridge_sha256": bridge_sha,
        "corpus_sha256": sha256_file(corpus_path),
        "implementation_revision": revision,
    }
    validation_id = sha256_bytes(canonical_json(identity))[:24]
    output = output_root / "validations" / validation_id
    if output.exists():
        raise ValueError(f"validation directory already exists: {output}")
    output.mkdir(parents=True)
    (output / "candidate-ref.json").write_text(
        json.dumps({"candidate_sha256": candidate_sha, "implementation_revision": revision}, indent=2, sort_keys=True) + "\n"
    )
    (output / "bridge-ref.json").write_text(
        json.dumps({"bridge_sha256": bridge_sha, "implementation_revision": revision}, indent=2, sort_keys=True) + "\n"
    )
    (output / "proposal-manifest.json").write_text(json.dumps(corpus, indent=2, sort_keys=True) + "\n")
    return {"validation_id": validation_id, **identity}


def _network_values(values: list[str]) -> dict[str, str]:
    result = {}
    for value in values:
        network, separator, item = value.partition("=")
        if not separator or not network or not item or network in result:
            raise ValueError("network values must be unique NETWORK=VALUE pairs")
        result[network] = item
    return result


def cmd_prepare_corpus(args: argparse.Namespace) -> None:
    manifest = prepare_corpus(
        corpus_root=_resolve_repo_path(args.corpus_root, field_name="corpus_root"),
        l1_rpc_by_network=_network_values(args.l1_rpc),
        l2_rpc_by_network=_network_values(args.l2_rpc),
        chain_spec_hash_by_network=_network_values(args.chain_spec_hash),
        chain_spec_path_by_network={
            network: _resolve_repo_path(path, field_name="chain_spec_file")
            for network, path in _network_values(args.chain_spec_file).items()
        },
        manifest_path=(
            _resolve_repo_path(args.manifest, field_name="manifest_path")
            if args.manifest else None
        ),
    )
    print(f"prepared {len(manifest['rows'])} final-validation GuestInputs")


def cmd_publish_corpus(args: argparse.Namespace) -> None:
    manifest_path = _resolve_repo_path(args.manifest, field_name="manifest_path")
    archive = _resolve_repo_path(args.archive, field_name="archive")
    manifest = json.loads(manifest_path.read_text())
    validate_manifest_for_publication(manifest, archive)
    published = publish_corpus(archive, args.object_uri)
    manifest.update(published)
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"published corpus at {published['archive_uri']}")


def cmd_prepare_integration_smoke(args: argparse.Namespace) -> None:
    smoke = prepare_integration_smoke(
        args.network,
        args.proposal_id,
        guest_input=args.guest_input,
        purpose=args.purpose,
    )
    if args.out:
        out = _resolve_repo_path(args.out, field_name="smoke_record")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(smoke, indent=2, sort_keys=True) + "\n")
    print(f"prepared {smoke['purpose']} {smoke['network']}/{smoke['proposal_id']}")


def cmd_prepare_calibration(args: argparse.Namespace) -> None:
    output_root = _resolve_repo_path(args.out, field_name="calibration_output")
    experiment = prepare_calibration(
        output_root,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
        implementation_revision=args.implementation_revision,
    )
    if args.run_path_file is not None:
        write_run_path_file(
            args.run_path_file,
            output_root / "runs" / experiment["calibration_id"],
        )
    print(f"prepared calibration {experiment['calibration_id']}")


def write_run_path_file(path: pathlib.Path, run: pathlib.Path) -> None:
    """Durably publish one machine-readable run path without partial contents."""
    provenance = run / "provenance.json"
    if not provenance.is_file():
        raise ValueError("calibration provenance must be durable before run-path handoff")
    for item in sorted(run.rglob("*")):
        if item.is_file():
            with item.open("rb") as input_file:
                os.fsync(input_file.fileno())
    directories = [item for item in run.rglob("*") if item.is_dir()]
    for directory in sorted(directories, key=lambda item: len(item.parts), reverse=True) + [run]:
        directory_fd = os.open(directory, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    run_parent_fd = os.open(run.parent, os.O_RDONLY)
    try:
        os.fsync(run_parent_fd)
    finally:
        os.close(run_parent_fd)
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = pathlib.Path(temporary_name)
    try:
        with os.fdopen(descriptor, "w") as output:
            output.write(str(run) + "\n")
            output.flush()
            os.fsync(output.fileno())
        try:
            os.link(temporary, path)
        except FileExistsError as exc:
            try:
                existing_fd = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
            except OSError as open_exc:
                raise ValueError(f"run-path target cannot be claimed: {path}") from open_exc
            with os.fdopen(existing_fd, "r+b") as existing:
                try:
                    fcntl.flock(existing.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError as lock_exc:
                    raise ValueError("run-path file has a concurrent writer") from lock_exc
                existing_stat = os.fstat(existing.fileno())
                try:
                    path_stat = path.stat(follow_symlinks=False)
                except FileNotFoundError as stat_exc:
                    raise ValueError("run-path file changed during publication") from stat_exc
                if (
                    not stat.S_ISREG(existing_stat.st_mode)
                    or existing_stat.st_size != 0
                    or (existing_stat.st_dev, existing_stat.st_ino)
                    != (path_stat.st_dev, path_stat.st_ino)
                ):
                    raise ValueError("run-path file already exists and is non-empty") from exc
                os.replace(temporary, path)
    finally:
        if temporary.exists():
            temporary.unlink()
        directory_fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)


def cmd_prepare_validation(args: argparse.Namespace) -> None:
    validation = prepare_validation(
        _resolve_repo_path(args.out, field_name="validation_output"),
        _resolve_repo_path(args.run, field_name="calibration_run"),
        _resolve_repo_path(args.corpus, field_name="corpus_manifest"),
    )
    print(f"prepared validation {validation['validation_id']}")


def cmd_generate(args: argparse.Namespace) -> None:
    calibration_run = _resolve_repo_path(args.calibration_run, field_name="calibration_run")
    validate_calibration_execution_identity(calibration_run)
    manifest_path = _resolve_repo_path(args.manifest, field_name="controlled_manifest")
    manifest, identity = verify_frozen_controlled_manifest(calibration_run, manifest_path)
    matched_control_diagnostic = getattr(args, "matched_control_diagnostic", False)
    if matched_control_diagnostic:
        manifest = select_matched_control_cases(manifest, args.matched_control_cases)
    provenance = {
        "calibration_id": calibration_run.name,
        "controlled_manifest_sha256": identity["controlled_manifest_sha256"],
        "controlled_manifest_rows_sha256": identity["controlled_manifest_rows_sha256"],
    }
    if matched_control_diagnostic:
        provenance.update(
            {
                "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
                "implementation_revision": identity["implementation_revision"],
            }
        )
    written = generate_cases(
        manifest,
        _resolve_repo_path(args.out, field_name="generated_fixtures"),
        provenance=provenance,
        generator_max_count=args.generator_max_count,
        matched_control_diagnostic=matched_control_diagnostic,
        operand_profile=getattr(args, "operand_profile", "zero"),
    )
    print(f"wrote {len(written)} case metadata files")


def cmd_generate_relations(args: argparse.Namespace) -> None:
    calibration_run = _resolve_repo_path(args.calibration_run, field_name="calibration_run")
    validate_calibration_execution_identity(calibration_run)
    manifest, identity = verify_frozen_controlled_manifest(
        calibration_run,
        _resolve_repo_path(args.manifest, field_name="controlled_manifest"),
    )
    provenance = {
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
        "implementation_revision": identity["implementation_revision"],
        "controlled_manifest_sha256": identity["controlled_manifest_sha256"],
        "controlled_manifest_rows_sha256": identity[
            "controlled_manifest_rows_sha256"
        ],
    }
    written = generate_relation_cases(
        manifest,
        _resolve_repo_path(args.out, field_name="generated_relation_fixtures"),
        provenance=provenance,
        generator_max_count=args.generator_max_count,
    )
    print(f"wrote {len(written)} formal relation metadata files")


def cmd_fit(args: argparse.Namespace) -> None:
    results = fit_report(args.runs, args.out, metric=args.metric)
    print(f"fit {len(results)} case(s)")


def cmd_damage(args: argparse.Namespace) -> None:
    results = damage_report(
        fit_path=args.fit,
        manifest_path=args.manifest,
        eth_gas_limit=args.eth_gas_limit,
        zk_gas_limit=args.zk_gas_limit,
        out_dir=args.out,
    )
    print(f"wrote damage report for {len(results)} case(s)")


def cmd_inventory(args: argparse.Namespace) -> None:
    rows = inventory_report(manifest_path=args.manifest, out_dir=args.out)
    print(f"wrote inventory report for {len(rows)} row(s)")


def _add_controlled_run_arguments(
    parser: argparse.ArgumentParser,
    *,
    opcode_stage_choices: tuple[str, ...] = ("opcode-lab", "revm-opcode-lab"),
    opcode_stage_default: str = "opcode-lab",
    opcode_elf_default: pathlib.Path = pathlib.Path(
        "crates/guests/elf/sp1_opcode_lab.elf"
    ),
    opcode_elf_choices: tuple[pathlib.Path, ...] | None = None,
) -> None:
    parser.add_argument("--fixtures", type=pathlib.Path, required=True)
    parser.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    parser.add_argument(
        "--elf",
        type=pathlib.Path,
        choices=opcode_elf_choices,
        default=opcode_elf_default,
        help="SP1 opcode-lab guest ELF",
    )
    parser.add_argument(
        "--precompile-elf",
        type=pathlib.Path,
        default=pathlib.Path("crates/guests/elf/sp1_precompile_lab.elf"),
        help="SP1 precompile-lab guest ELF",
    )
    parser.add_argument(
        "--opcode-stage",
        choices=opcode_stage_choices,
        default=opcode_stage_default,
        help="SP1 opcode lab stage to run for opcode fixtures",
    )
    parser.add_argument("--calibration-run", type=pathlib.Path, required=True)
    parser.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    parser.add_argument("--repeats", type=int, default=1)


def _controlled_repeat_point(rows: Iterable[Mapping[str, Any]]) -> dict[str, Any]:
    repeats = sorted(rows, key=lambda row: int(row.get("repeat_index", -1)))
    if [row.get("repeat_index") for row in repeats] != [0, 1, 2]:
        raise ValueError("controlled point requires repeat_index 0, 1, and 2")
    workload_ids = {row.get("workload_id") for row in repeats}
    if len(workload_ids) != 1 or not _is_sha256(next(iter(workload_ids))):
        raise ValueError("controlled repeats do not share one workload_id")
    execution_row_ids = [row.get("execution_row_id") for row in repeats]
    if len(set(execution_row_ids)) != 3 or not all(
        _is_sha256(value) for value in execution_row_ids
    ):
        raise ValueError("controlled repeats require distinct execution_row_id values")
    workload_kind = "overhead" if repeats[0].get("overhead_key_id") else repeats[0].get("kind")
    if workload_kind not in {"opcode", "precompile", "overhead"}:
        raise ValueError("controlled repeats have unknown SP1 workload kind")
    for row in repeats:
        validate_sp1_execution_provenance(row, workload_kind=str(workload_kind))
    first = repeats[0]
    point = {
        "count": int(first["target_count"]),
        "prover_gas_repeats": [str(row.get("prover_gas", row.get("gas"))) for row in repeats],
        "instruction_count_repeats": [
            str(row.get("total_instruction_count")) for row in repeats
        ],
        "case_input_sha256_repeats": [row.get("backend_input_sha256") for row in repeats],
        "exit_code_repeats": [row.get("exit_code") for row in repeats],
        "public_values_repeats": [row.get("public_values") for row in repeats],
        "isolation": first.get("isolation"),
    }
    pair_ids = {row.get("pair_id") for row in repeats}
    if pair_ids != {None}:
        if len(pair_ids) != 1 or not _is_sha256(next(iter(pair_ids))):
            raise ValueError("controlled repeats do not share one pair_id")
        point["pair_id"] = next(iter(pair_ids))
    return point


def fit_controlled_costs(
    manifest: Manifest, rows: Iterable[Mapping[str, Any]]
) -> list[dict[str, Any]]:
    """Fit primary controlled cases after enforcing exact three-repeat identities."""
    rows = list(rows)
    reject_matched_control_diagnostics(rows, context="formal controlled fit")
    rows_by_case_lane_count: dict[tuple[str, str, int], list[Mapping[str, Any]]] = {}
    for row in rows:
        case_id = str(row.get("case"))
        lane = str(row.get("lane", "target"))
        count = int(row.get("target_count", -1))
        rows_by_case_lane_count.setdefault((case_id, lane, count), []).append(row)

    key_by_case = {
        case_id: key
        for key in manifest.measurement_keys
        for case_id in (*key.required_case_ids, *key.diagnostic_case_ids)
    }
    results: list[dict[str, Any]] = []
    for case in manifest.cases:
        key = key_by_case[case.name]
        generator_bounds = {
            int(row.get("generator_max_count", -1))
            for (case_id, _lane, _count), grouped in rows_by_case_lane_count.items()
            if case_id == case.name
            for row in grouped
        }
        if len(generator_bounds) != 1 or next(iter(generator_bounds)) <= 0:
            results.append(
                {
                    "case_id": case.name,
                    "measurement_key_id": key.id,
                    "pricing_basis": key.pricing_basis,
                    "status": "rejected",
                    "reasons": ["generator_footprint"],
                }
            )
            continue
        generator_max_count = next(iter(generator_bounds))
        if case.kind == "precompile":
            observations = []
            counts = sorted(
                count
                for case_id, lane, count in rows_by_case_lane_count
                if case_id == case.name and lane == "target"
            )
            for count in counts:
                target = _controlled_repeat_point(
                    rows_by_case_lane_count.get((case.name, "target", count), [])
                )
                control = _controlled_repeat_point(
                    rows_by_case_lane_count.get((case.name, "control", count), [])
                )
                if target.get("pair_id") != control.get("pair_id") or target.get(
                    "pair_id"
                ) is None:
                    raise ValueError("paired precompile target/control pair_id mismatch")
                isolation = target.get("isolation") or {}
                control_isolation = control.get("isolation") or {}
                shape = {
                    "input_size": isolation.get("input_size"),
                    "output_size": isolation.get("output_size"),
                    "folded_bytes_per_iteration": isolation.get(
                        "folded_bytes_per_iteration"
                    ),
                }
                if shape != {
                    "input_size": control_isolation.get("input_size"),
                    "output_size": control_isolation.get("output_size"),
                    "folded_bytes_per_iteration": control_isolation.get(
                        "folded_bytes_per_iteration"
                    ),
                }:
                    shape["control_shape_mismatch"] = True
                observations.append(
                    {"count": count, "target": target, "control": control, "shape": shape}
                )
            result = evaluate_paired_precompile_sweep(
                observations,
                target_raw_gas=case.target_raw_gas,
                generator_max_count=generator_max_count,
            )
        else:
            observations = [
                _controlled_repeat_point(grouped)
                for (case_id, lane, _count), grouped in sorted(
                    rows_by_case_lane_count.items(), key=lambda item: item[0][2]
                )
                if case_id == case.name and lane == "target"
            ]
            result = evaluate_controlled_sweep(
                observations,
                pricing_basis=key.pricing_basis,
                target_raw_gas=case.target_raw_gas or None,
                generator_max_count=generator_max_count,
            )
        results.append(
            {
                "case_id": case.name,
                "measurement_key_id": key.id,
                "pricing_basis": key.pricing_basis,
                "target_raw_gas": case.target_raw_gas or None,
                "generator_max_count": generator_max_count,
                **result,
            }
        )
    return results


def cmd_fit_controlled_costs(args: argparse.Namespace) -> None:
    manifest = load_manifest(
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest")
    )
    results = fit_controlled_costs(
        manifest, iter_jsonl(_resolve_repo_path(args.runs, field_name="controlled_runs"))
    )
    output = _resolve_repo_path(args.out, field_name="controlled_fit")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps({"schema_version": 1, "case_results": results}, indent=2, sort_keys=True)
        + "\n"
    )
    print(f"fit {len(results)} controlled case(s)")


def cmd_fit_relations(args: argparse.Namespace) -> None:
    runs_path = _resolve_repo_path(args.runs, field_name="formal_relation_runs")
    if runs_path.name != "formal-relations.jsonl" or runs_path.parent.name != "raw":
        raise ValueError(
            "formal relation runs must use $CALIBRATION_RUN/raw/formal-relations.jsonl"
        )
    calibration_run = runs_path.parent.parent
    if args.calibration_run is not None:
        supplied_run = _resolve_repo_path(
            args.calibration_run, field_name="calibration_run"
        )
        if supplied_run != calibration_run:
            raise ValueError(
                "supplied calibration run differs from canonical formal relation runs path"
            )
    execution_identity = validate_calibration_execution_identity(calibration_run)
    controlled_manifest = _resolve_repo_path(
        args.controlled_manifest, field_name="controlled_manifest"
    )
    manifest, frozen_identity = verify_frozen_controlled_manifest(
        calibration_run, controlled_manifest
    )
    if execution_identity != frozen_identity:
        raise ValueError("formal relation calibration identity changed during validation")
    expected_provenance = {
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(
            canonical_json(execution_identity)
        ),
        "implementation_revision": execution_identity["implementation_revision"],
        "controlled_manifest_sha256": execution_identity[
            "controlled_manifest_sha256"
        ],
        "controlled_manifest_rows_sha256": execution_identity[
            "controlled_manifest_rows_sha256"
        ],
    }
    rows = list(iter_jsonl(runs_path))
    if any(
        row.get(field) != value
        for row in rows
        for field, value in expected_provenance.items()
    ):
        raise ValueError("formal relation rows differ from durable calibration provenance")
    artifact = fit_opcode_relations(
        manifest,
        rows,
    )
    output = _resolve_repo_path(args.out, field_name="opcode_relations")
    output.parent.mkdir(parents=True, exist_ok=True)
    _atomic_write_json(output, artifact)
    print(f"fit {len(artifact['equations'])} formal opcode relation(s)")


def _affine_model_from_validated_artifact(
    manifest: Manifest, artifact: Mapping[str, Any]
) -> AffineOpcodeModel:
    opcode_keys = tuple(
        f"opcode:0x{case.opcode:02x}"
        for case in manifest.cases
        if case.kind == "opcode"
        and case.opcode is not None
        and case.opcode in PURE_OPCODE_DEFAULTS
        and PURE_OPCODE_DEFAULTS[case.opcode][1] == case.template
    )
    equations = tuple(
        RelationEquation(
            relation_id=str(row["relation_id"]),
            coefficients={
                key: Fraction(value)
                for key, value in row["signed_raw_gas_by_key"].items()
            },
            slope=Decimal(row["slope_p"]),
        )
        for row in artifact["equations"]
    )
    return derive_affine_opcode_model(
        opcode_keys, equations, manifest.opcode_relation_anchors
    )


def _controlled_block_row_payload(row: ControlledBlockRowSpec) -> dict[str, Any]:
    program = {"kind": row.program.kind}
    if row.program.kind == "native_transfer":
        program["value"] = row.program.value
    elif row.program.kind == "opcode_loop":
        program.update(
            family=row.program.family,
            count=row.program.count,
            scenario=row.program.scenario,
        )
    return {
        "row_id": row.row_id,
        "workload_family": row.workload_family,
        "split": row.split,
        "block_count": row.block_count,
        "transaction_count": row.transaction_count,
        "program": program,
        "expected_final_state_root": row.expected_final_state_root,
        "expected_raw_gas_by_key": dict(row.expected_raw_gas_by_key),
        "expected_features": dict(row.expected_features),
        "expected_diagnostics": dict(row.expected_diagnostics),
    }


def run_block_calibration_rows(
    *,
    manifest: Manifest,
    affine_model: AffineOpcodeModel,
    guest_launcher: pathlib.Path,
    calibration_run_id: str,
    relation_artifact_sha256: str,
    relation_raw_rows_sha256: str,
    out: pathlib.Path,
    repeats: int = 3,
) -> list[dict[str, Any]]:
    preflight = preflight_block_calibration_rows(manifest, affine_model)
    if repeats != 3:
        raise ValueError("block calibration requires exactly three SP1 repeats")
    output_rows = []
    with tempfile.TemporaryDirectory() as temporary_name:
        temporary = pathlib.Path(temporary_name)
        for row_index, row in enumerate(manifest.block_calibration_rows):
            payload = _controlled_block_row_payload(row)
            spec_path = temporary / f"block-row-{row_index}.json"
            report_path = temporary / f"block-report-{row_index}.jsonl"
            spec_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            repeat_rows = []
            for repeat_index in range(repeats):
                report_path.unlink(missing_ok=True)
                subprocess.run(
                    [
                        str(guest_launcher),
                        "--stage", "controlled-block",
                        "--proof-type", "sp1",
                        "--mode", "execute",
                        "--sp1-prover", "local",
                        "--input", str(spec_path),
                        "--jsonl-out", str(report_path),
                    ],
                    check=True,
                )
                reports = list(iter_jsonl(report_path))
                if len(reports) != 1:
                    raise ValueError("controlled-block launcher must emit exactly one row")
                report = reports[0]
                controlled = report.get("controlled_block")
                if not isinstance(controlled, Mapping):
                    raise ValueError("controlled-block launcher is missing its result")
                normalized = {
                    **payload,
                    "schema_version": 1,
                    "purpose": "block_calibration",
                    "status": controlled.get("status"),
                    "repeat_index": repeat_index,
                    "calibration_id": calibration_run_id,
                    "relation_artifact_sha256": relation_artifact_sha256,
                    "relation_raw_rows_sha256": relation_raw_rows_sha256,
                    "preflight_fit_rank": preflight["fit_rank"],
                }
                if controlled.get("status") != "accepted":
                    normalized.update(
                        reasons=list(controlled.get("reasons", [])),
                        error=controlled.get("error"),
                    )
                    output_rows.append(normalized)
                    break
                observation = controlled.get("observation")
                if not isinstance(observation, Mapping):
                    raise ValueError("accepted controlled-block row is missing host observation")
                validate_sp1_execution_provenance(report, workload_kind="overhead")
                backend_input_sha256 = observation.get("backend_input_sha256")
                if not _is_sha256(backend_input_sha256):
                    raise ValueError("controlled-block host observation has invalid input identity")
                guest_input_sha256 = str(report.get("guest_input_sha256", ""))
                normalized.update(
                    prover_gas=report.get("gas"),
                    total_instruction_count=report.get("total_instruction_count"),
                    exit_code=report.get("exit_code"),
                    public_values=report.get("public_values"),
                    backend_input_sha256=backend_input_sha256,
                    backend="sp1",
                    mode="execute",
                    sp1_prover="local",
                    primary_api="ExecutionReport::gas",
                    sp1_execution_engine=report.get("sp1_execution_engine"),
                    sp1_gas_trace_chunk_threshold=report.get(
                        "sp1_gas_trace_chunk_threshold"
                    ),
                    sp1_gas_trace_chunk_slots=report.get(
                        "sp1_gas_trace_chunk_slots"
                    ),
                    execution_row_id=controlled_execution_row_id(
                        row.row_id,
                        backend="sp1",
                        execution_engine="standard",
                        run_id=calibration_run_id,
                        repeat_index=repeat_index,
                        backend_input_sha256=backend_input_sha256,
                    ),
                    actual_raw_gas_by_key=observation.get("actual_raw_gas_by_key"),
                    actual_features=observation.get("actual_features"),
                    actual_diagnostics=observation.get("actual_diagnostics"),
                    actual_final_state_root=observation.get("actual_final_state_root"),
                    host_public_output=observation.get("public_output"),
                    guest_input_sha256=guest_input_sha256,
                    reported_row_id=controlled.get("row_id"),
                    observation_row_id=observation.get("row_id"),
                )
                repeat_rows.append(normalized)
            if len(repeat_rows) != repeats:
                continue
            stable_fields = (
                "prover_gas",
                "total_instruction_count",
                "exit_code",
                "public_values",
                "host_public_output",
                "backend_input_sha256",
                "guest_input_sha256",
                "reported_row_id",
                "observation_row_id",
                "actual_raw_gas_by_key",
                "actual_features",
                "actual_diagnostics",
                "actual_final_state_root",
            )
            repeat_mismatches = [
                field
                for field in stable_fields
                if len({canonical_json(item.get(field)) for item in repeat_rows}) != 1
            ]
            if any(item["exit_code"] != 0 for item in repeat_rows):
                repeat_mismatches.append("successful_exit")
            if any(
                item["reported_row_id"] != row.row_id
                or item["observation_row_id"] != row.row_id
                for item in repeat_rows
            ):
                repeat_mismatches.append("semantic_row_identity")
            if any(
                item["actual_raw_gas_by_key"]
                != payload["expected_raw_gas_by_key"]
                for item in repeat_rows
            ):
                repeat_mismatches.append("host_trace_ledger")
            if any(
                item["actual_features"] != payload["expected_features"]
                for item in repeat_rows
            ):
                repeat_mismatches.append("host_trace_features")
            if any(
                item["actual_diagnostics"] != payload["expected_diagnostics"]
                for item in repeat_rows
            ):
                repeat_mismatches.append("host_trace_diagnostics")
            if any(
                item["actual_final_state_root"]
                != payload["expected_final_state_root"]
                for item in repeat_rows
            ):
                repeat_mismatches.append("host_final_state_root")
            if any(
                item["guest_input_sha256"] != "0x" + item["backend_input_sha256"]
                for item in repeat_rows
            ):
                repeat_mismatches.append("report_observation_input_identity")
            if any(
                str(item["public_values"]).lower()
                != str(item["host_public_output"]).lower()
                for item in repeat_rows
            ):
                repeat_mismatches.append("trace_public_output")
            if repeat_mismatches:
                output_rows.append(
                    {
                        **payload,
                        "schema_version": 1,
                        "purpose": "block_calibration",
                        "status": "rejected",
                        "reasons": ["repeat_instability"],
                        "repeat_mismatches": sorted(set(repeat_mismatches)),
                        "repeat_indices": [item["repeat_index"] for item in repeat_rows],
                        "calibration_id": calibration_run_id,
                        "relation_artifact_sha256": relation_artifact_sha256,
                        "relation_raw_rows_sha256": relation_raw_rows_sha256,
                        "preflight_fit_rank": preflight["fit_rank"],
                    }
                )
                continue
            output_rows.extend(repeat_rows)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("".join(json.dumps(row, sort_keys=True) + "\n" for row in output_rows))
    return output_rows


def cmd_run_block_calibration(args: argparse.Namespace) -> None:
    calibration_run = _resolve_repo_path(args.calibration_run, field_name="calibration_run")
    execution_identity = validate_calibration_execution_identity(calibration_run)
    manifest, frozen_identity = verify_frozen_controlled_manifest(
        calibration_run,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
    )
    if execution_identity != frozen_identity:
        raise ValueError("block calibration identity changed during validation")
    relations_path = _canonical_run_artifact(
        calibration_run,
        _resolve_repo_path(args.relations, field_name="opcode_relations"),
        "opcode-relations.json",
    )
    raw_rows_path = calibration_run / "raw" / "formal-relations.jsonl"
    if not raw_rows_path.is_file():
        raise ValueError("block calibration requires canonical formal relation raw rows")
    artifact = json.loads(relations_path.read_text())
    raw_rows = list(iter_jsonl(raw_rows_path))
    expected_provenance = {
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(canonical_json(execution_identity)),
        "implementation_revision": execution_identity["implementation_revision"],
        "controlled_manifest_sha256": execution_identity["controlled_manifest_sha256"],
        "controlled_manifest_rows_sha256": execution_identity[
            "controlled_manifest_rows_sha256"
        ],
    }
    validate_opcode_relations_artifact(
        manifest, artifact, raw_rows, expected_provenance
    )
    if artifact.get("purpose") in {"proposal", "integration_smoke"}:
        raise ValueError("block calibration rejects proposal/integration-smoke purpose")
    affine_model = _affine_model_from_validated_artifact(manifest, artifact)
    output = _canonical_run_artifact(
        calibration_run,
        _resolve_repo_path(args.out, field_name="block_calibration_rows"),
        "block-calibration-rows.jsonl",
    )
    rows = run_block_calibration_rows(
        manifest=manifest,
        affine_model=affine_model,
        guest_launcher=_resolve_repo_path(args.guest_launcher, field_name="guest_launcher"),
        calibration_run_id=calibration_run.name,
        relation_artifact_sha256=artifact["artifact_sha256"],
        relation_raw_rows_sha256=artifact["raw_rows_sha256"],
        out=output,
        repeats=args.repeats,
    )
    print(f"wrote {len(rows)} controlled block calibration observation(s)")


_ACCEPTED_BLOCK_CALIBRATION_ROW_FIELDS = {
    "row_id",
    "workload_family",
    "split",
    "block_count",
    "transaction_count",
    "program",
    "expected_final_state_root",
    "expected_raw_gas_by_key",
    "expected_features",
    "expected_diagnostics",
    "schema_version",
    "purpose",
    "status",
    "repeat_index",
    "calibration_id",
    "relation_artifact_sha256",
    "relation_raw_rows_sha256",
    "preflight_fit_rank",
    "prover_gas",
    "total_instruction_count",
    "exit_code",
    "public_values",
    "backend_input_sha256",
    "backend",
    "mode",
    "sp1_prover",
    "primary_api",
    "sp1_execution_engine",
    "sp1_gas_trace_chunk_threshold",
    "sp1_gas_trace_chunk_slots",
    "execution_row_id",
    "actual_raw_gas_by_key",
    "actual_features",
    "actual_diagnostics",
    "actual_final_state_root",
    "host_public_output",
    "guest_input_sha256",
    "reported_row_id",
    "observation_row_id",
}


def _exact_json_equal(actual: Any, expected: Any) -> bool:
    """Compare JSON values without allowing Python's bool/int aliases."""
    if isinstance(expected, Mapping):
        return (
            isinstance(actual, Mapping)
            and set(actual) == set(expected)
            and all(
                _exact_json_equal(actual[key], expected[key])
                for key in expected
            )
        )
    if isinstance(expected, list):
        return (
            isinstance(actual, list)
            and len(actual) == len(expected)
            and all(
                _exact_json_equal(actual_item, expected_item)
                for actual_item, expected_item in zip(actual, expected)
            )
        )
    return type(actual) is type(expected) and actual == expected


def _validated_block_calibration_rows(
    manifest: Manifest,
    relation_artifact: Mapping[str, Any],
    raw_rows: list[Mapping[str, Any]],
) -> tuple[BlockCalibrationRow, ...]:
    if (
        relation_artifact.get("purpose") != FORMAL_RELATION_PURPOSE
        or relation_artifact.get("status") != "accepted"
    ):
        raise ValueError("block calibration requires an accepted opcode relation artifact")
    relation_sha256 = relation_artifact.get("artifact_sha256")
    relation_raw_sha256 = relation_artifact.get("raw_rows_sha256")
    provenance = relation_artifact.get("provenance")
    calibration_id = (
        provenance.get("calibration_id") if isinstance(provenance, Mapping) else None
    )
    if (
        not _is_sha256(relation_sha256)
        or not _is_sha256(relation_raw_sha256)
        or not isinstance(calibration_id, str)
    ):
        raise ValueError("block calibration relation provenance is invalid")

    expected = {row.row_id: row for row in manifest.block_calibration_rows}
    if any(
        not isinstance(row, Mapping)
        or set(row) != _ACCEPTED_BLOCK_CALIBRATION_ROW_FIELDS
        for row in raw_rows
    ):
        raise ValueError("block calibration accepted row schema is incomplete or unexpected")
    integer_fields = (
        "schema_version",
        "repeat_index",
        "preflight_fit_rank",
        "block_count",
        "transaction_count",
        "total_instruction_count",
        "exit_code",
    )
    if any(
        type(row.get(field)) is not int
        for row in raw_rows
        for field in integer_fields
    ):
        raise ValueError("block calibration accepted row schema requires exact integers")
    actual_ids = {str(row.get("row_id")) for row in raw_rows}
    if actual_ids != set(expected):
        raise ValueError("block calibration raw row identities differ from the manifest")
    collapsed = []
    for row_id, spec in expected.items():
        repeats = [row for row in raw_rows if row.get("row_id") == row_id]
        if len(repeats) != 3 or sorted(
            row.get("repeat_index") for row in repeats
        ) != [0, 1, 2]:
            raise ValueError("block calibration requires exactly three indexed repeats")
        required = {
            "schema_version": 1,
            "purpose": "block_calibration",
            "status": "accepted",
            "calibration_id": calibration_id,
            "relation_artifact_sha256": relation_sha256,
            "relation_raw_rows_sha256": relation_raw_sha256,
            "preflight_fit_rank": 8,
            "workload_family": spec.workload_family,
            "split": spec.split,
            "reported_row_id": row_id,
            "observation_row_id": row_id,
            "backend": "sp1",
            "mode": "execute",
            "sp1_prover": "local",
            "primary_api": "ExecutionReport::gas",
            "sp1_execution_engine": "standard",
            "exit_code": 0,
        }
        if any(
            not _exact_json_equal(row.get(key), value)
            for row in repeats
            for key, value in required.items()
        ):
            raise ValueError("block calibration raw row provenance or identity differs")
        payload = _controlled_block_row_payload(spec)
        if any(
            not _exact_json_equal(repeat.get(field), value)
            for repeat in repeats
            for field, value in payload.items()
        ):
            raise ValueError("block calibration raw row semantics differ from the manifest")
        stable_fields = (
            "prover_gas",
            "total_instruction_count",
            "public_values",
            "host_public_output",
            "actual_raw_gas_by_key",
            "actual_features",
            "actual_diagnostics",
            "actual_final_state_root",
            "backend_input_sha256",
            "guest_input_sha256",
        )
        if any(
            len({canonical_json(row.get(field)) for row in repeats}) != 1
            for field in stable_fields
        ):
            raise ValueError("block calibration repeats are not deterministic")
        first = repeats[0]
        for repeat in repeats:
            validate_sp1_execution_provenance(repeat, workload_kind="overhead")
            backend_input_sha256 = repeat.get("backend_input_sha256")
            repeat_index = repeat["repeat_index"]
            expected_execution_row_id = (
                controlled_execution_row_id(
                    row_id,
                    backend="sp1",
                    execution_engine="standard",
                    run_id=calibration_id,
                    repeat_index=repeat_index,
                    backend_input_sha256=backend_input_sha256,
                )
                if _is_sha256(backend_input_sha256)
                else None
            )
            if (
                repeat.get("execution_row_id") != expected_execution_row_id
                or repeat.get("guest_input_sha256")
                != "0x" + str(backend_input_sha256)
            ):
                raise ValueError("block calibration execution row identity differs")
            instruction_count = repeat.get("total_instruction_count")
            if type(instruction_count) is not int or instruction_count <= 0:
                raise ValueError("block calibration instruction count is invalid")
            public_values = _normalized_hex(
                repeat.get("public_values"), field_name="block SP1 public output"
            )
            host_public_output = _normalized_hex(
                repeat.get("host_public_output"),
                field_name="block host public output",
            )
            if public_values != host_public_output:
                raise ValueError("block calibration public output join differs")
        if (
            not _exact_json_equal(
                first.get("actual_raw_gas_by_key"),
                dict(spec.expected_raw_gas_by_key),
            )
            or not _exact_json_equal(
                first.get("actual_features"), dict(spec.expected_features)
            )
            or not _exact_json_equal(
                first.get("actual_diagnostics"), dict(spec.expected_diagnostics)
            )
            or not _exact_json_equal(
                first.get("actual_final_state_root"), spec.expected_final_state_root
            )
        ):
            raise ValueError("block calibration traced inputs differ from the manifest")
        collapsed.append(
            BlockCalibrationRow(
                row_id=row_id,
                workload_family=spec.workload_family,
                split=spec.split,
                prover_gas=_decimal(first.get("prover_gas"), label="block prover gas"),
                raw_gas_by_key=dict(spec.expected_raw_gas_by_key),
                feature_counts=dict(spec.expected_features),
            )
        )
    if len(raw_rows) != 3 * len(expected):
        raise ValueError("block calibration raw rows contain duplicate observations")
    return tuple(collapsed)


def _dynamic_observations_from_relation_artifact(
    relation_artifact: Mapping[str, Any],
) -> tuple[DynamicRelationObservation, ...]:
    rows = [
        *relation_artifact.get("equations", []),
        *relation_artifact.get("dynamic_holdouts", []),
    ]
    observations = []
    for row in rows:
        dynamic_key = row.get("dynamic_key") if isinstance(row, Mapping) else None
        if dynamic_key is None:
            continue
        signed = row.get("signed_raw_gas_by_key")
        if not isinstance(signed, Mapping):
            raise ValueError("dynamic relation coefficient map is missing")
        observations.append(
            DynamicRelationObservation(
                dynamic_key=str(dynamic_key),
                scenario_id=str(row.get("scenario_id")),
                split=str(row.get("split")),
                equation=RelationEquation(
                    relation_id=str(row.get("relation_id")),
                    coefficients={key: Fraction(value) for key, value in signed.items()},
                    slope=_decimal(row.get("slope_p"), label="dynamic relation slope"),
                ),
            )
        )
    return tuple(observations)


def _serialize_decimal_tree(value: Any) -> Any:
    if isinstance(value, Decimal):
        return _decimal_text(value)
    if isinstance(value, Fraction):
        return _fraction_text(value)
    if isinstance(value, Mapping):
        return {key: _serialize_decimal_tree(item) for key, item in value.items()}
    if isinstance(value, (tuple, list)):
        return [_serialize_decimal_tree(item) for item in value]
    return value


@_isolated_decimal_context
def fit_block_calibration_artifact(
    manifest: Manifest,
    affine_model: AffineOpcodeModel,
    relation_artifact: Mapping[str, Any],
    raw_rows: list[Mapping[str, Any]],
) -> dict[str, Any]:
    """Fit and serialize the canonical controlled block calibration artifact."""
    rows = _validated_block_calibration_rows(manifest, relation_artifact, raw_rows)
    result = fit_block_calibration(affine_model, rows, tuple(Q_FORMULA))
    dynamic = validate_dynamic_holdouts(
        affine_model,
        result.opcode_multipliers,
        _dynamic_observations_from_relation_artifact(relation_artifact),
        tuple(manifest.dynamic_raw_gas_keys),
    )
    normalization_key = manifest.normalization_reference_key
    if normalization_key not in result.opcode_multipliers:
        raise ValueError("block calibration ADD normalization reference is missing")
    normalization = result.opcode_multipliers[normalization_key]
    with localcontext(_OPCODE_DECIMAL_CONTEXT):
        normalized_multipliers = {
            key: value / normalization
            for key, value in result.opcode_multipliers.items()
        }
    artifact = {
        "schema_version": 1,
        "purpose": "block_calibration",
        "status": result.status,
        "provenance": dict(relation_artifact["provenance"]),
        "relation_artifact_sha256": relation_artifact["artifact_sha256"],
        "relation_raw_rows_sha256": relation_artifact["raw_rows_sha256"],
        "raw_block_rows_sha256": sha256_bytes(canonical_json(raw_rows)),
        "parameter_order": list(result.parameter_order),
        "formulas": {
            "fit": "p_hat = x * mu_zero + [x * B, q] * [theta, beta]",
            "opcode": "mu = mu_zero + B * theta",
            "ape": "abs(predicted_prover_gas - actual_prover_gas) / actual_prover_gas",
        },
        "gates": {
            "exact_fit_rank": 8,
            "positive_parameters": True,
            "positive_opcode_multipliers": True,
            "fit_mape_max": "0.05",
            "fit_max_ape_max": "0.10",
            "holdout_max_ape_max": "0.10",
            "dynamic_relation_ape_max": "0.10",
            "dynamic_implied_multiplier_spread_max": "0.05",
            "leave_one_family_out_drift_max": "0.05",
        },
        "anchors": _serialize_decimal_tree(result.anchors),
        "fixed_costs": _serialize_decimal_tree(result.fixed_costs),
        "opcode_multipliers": _serialize_decimal_tree(result.opcode_multipliers),
        "normalization_reference_key": normalization_key,
        "opcode_multipliers_add_normalized": _serialize_decimal_tree(
            normalized_multipliers
        ),
        "fit_mape": _decimal_text(result.fit_mape),
        "fit_max_ape": _decimal_text(result.fit_max_ape),
        "holdout_max_ape": _decimal_text(result.holdout_max_ape),
        "exact_fit_matrix": _serialize_decimal_tree(result.exact_design_matrix),
        "exact_fit_rank": result.exact_rank,
        "column_scales": _serialize_decimal_tree(result.column_scales),
        "solver_residual": _decimal_text(result.solver_residual),
        "predictions": _serialize_decimal_tree(result.predictions),
        "leave_one_family_out": _serialize_decimal_tree(result.leave_one_family_out),
        "dynamic_holdouts": _serialize_decimal_tree(dynamic),
    }
    artifact["artifact_sha256"] = sha256_bytes(canonical_json(artifact))
    return artifact


def cmd_fit_block_calibration(args: argparse.Namespace) -> None:
    runs_path = _resolve_repo_path(args.runs, field_name="block_calibration_rows")
    calibration_run = runs_path.parent
    if runs_path.name != "block-calibration-rows.jsonl":
        raise ValueError("block calibration runs must use the canonical persisted artifact")
    execution_identity = validate_calibration_execution_identity(calibration_run)
    manifest, frozen_identity = verify_frozen_controlled_manifest(
        calibration_run,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
    )
    if execution_identity != frozen_identity:
        raise ValueError("block calibration identity changed during validation")
    relations_path = _canonical_run_artifact(
        calibration_run,
        _resolve_repo_path(args.relations, field_name="opcode_relations"),
        "opcode-relations.json",
    )
    output = _canonical_run_artifact(
        calibration_run,
        _resolve_repo_path(args.out, field_name="block_calibration"),
        "block-calibration.json",
    )
    relation_rows_path = calibration_run / "raw" / "formal-relations.jsonl"
    if not relation_rows_path.is_file():
        raise ValueError("block calibration requires canonical formal relation raw rows")
    relation_artifact = json.loads(relations_path.read_text())
    relation_rows = list(iter_jsonl(relation_rows_path))
    expected_provenance = {
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(canonical_json(execution_identity)),
        "implementation_revision": execution_identity["implementation_revision"],
        "controlled_manifest_sha256": execution_identity["controlled_manifest_sha256"],
        "controlled_manifest_rows_sha256": execution_identity[
            "controlled_manifest_rows_sha256"
        ],
    }
    validate_opcode_relations_artifact(
        manifest, relation_artifact, relation_rows, expected_provenance
    )
    artifact = fit_block_calibration_artifact(
        manifest,
        _affine_model_from_validated_artifact(manifest, relation_artifact),
        relation_artifact,
        list(iter_jsonl(runs_path)),
    )
    _atomic_write_json(output, artifact)
    print(f"fit {len(artifact['parameter_order'])} block calibration parameter(s)")


CONTROLLED_GENERATOR_ROUNDS = (8, 32, 128, 512, 2048)
CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT = 128


def controlled_round_counts(generator_max_count: int) -> tuple[int, ...]:
    try:
        index = CONTROLLED_GENERATOR_ROUNDS.index(generator_max_count)
    except ValueError as exc:
        raise ValueError("unknown controlled generator round") from exc
    return tuple(dict.fromkeys((*CONTROLLED_PREFIXES[index], generator_max_count)))


def run_controlled_overhead_round(
    *,
    guest_launcher: pathlib.Path,
    calibration_run_id: str,
    generator_max_count: int,
    include_startup: bool,
    out: pathlib.Path,
) -> None:
    """Execute each frozen overhead point three times through sp1-shasta-proposal."""
    if generator_max_count > CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT:
        raise ValueError(
            "controlled overhead generator exceeds the frozen "
            f"{CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT} bound"
        )
    out.parent.mkdir(parents=True, exist_ok=True)
    rows: list[dict[str, Any]] = []
    for target_count in controlled_round_counts(generator_max_count):
        for repeat_index in range(3):
            stem = f"overhead-{generator_max_count}-{target_count}-{repeat_index}"
            spec_path = out.with_name(f"{stem}.json")
            report_path = out.with_name(f"{stem}.reports.jsonl")
            run_startup = include_startup and target_count == 0
            spec_path.write_text(
                json.dumps(
                    {
                        "target_count": target_count,
                        "include_startup": run_startup,
                    },
                    indent=2,
                    sort_keys=True,
                )
                + "\n"
            )
            subprocess.run(
                [
                    str(guest_launcher),
                    "--stage",
                    "controlled-overhead",
                    "--proof-type",
                    "sp1",
                    "--mode",
                    "execute",
                    "--sp1-prover",
                    "local",
                    "--input",
                    str(spec_path),
                    "--jsonl-out",
                    str(report_path),
                ],
                check=True,
            )
            for report in iter_jsonl(report_path):
                validate_sp1_execution_provenance(report, workload_kind="overhead")
                controlled = report.get("controlled_overhead")
                if not isinstance(controlled, Mapping):
                    raise ValueError("controlled overhead report is missing typed metadata")
                status = controlled.get("status")
                if status == "rejected":
                    rows.append(
                        {
                            "case": "block_base_one_vs_two_minimal_blocks",
                            "overhead_key_id": "block_base",
                            "lane": "target",
                            "target_count": target_count,
                            "generator_max_count": generator_max_count,
                            "repeat_index": repeat_index,
                            "status": "rejected",
                            "sp1_execution_engine": report["sp1_execution_engine"],
                            "reasons": controlled.get("reasons", []),
                            "error": controlled.get("error"),
                        }
                    )
                    continue
                if status != "accepted" or not isinstance(
                    controlled.get("observation"), Mapping
                ):
                    raise ValueError("controlled overhead report has invalid status")
                observation = dict(controlled["observation"])
                workload_id = observation.get("workload_id")
                workload_spec = observation.get("workload_spec")
                backend_input_sha256 = observation.get("backend_input_sha256")
                if not _is_sha256(workload_id) or not _is_sha256(
                    backend_input_sha256
                ):
                    raise ValueError("controlled overhead identity is not canonical")
                if not isinstance(workload_spec, Mapping) or controlled_workload_id(
                    workload_spec
                ) != workload_id:
                    raise ValueError("controlled overhead workload identity mismatch")
                if report.get("guest_input_sha256") != observation.get(
                    "guest_input_sha256"
                ):
                    raise ValueError("controlled overhead GuestInput identity mismatch")
                row = {
                    **report,
                    "case": observation["case_id"],
                    "overhead_key_id": observation["overhead_key_id"],
                    "lane": observation["lane"],
                    "baseline_kind": observation.get("baseline_kind"),
                    "target_count": observation["target_count"],
                    "generator_max_count": generator_max_count,
                    "repeat_index": repeat_index,
                    "status": "accepted",
                    "workload_id": workload_id,
                    "workload_spec": workload_spec,
                    "backend_input_sha256": backend_input_sha256,
                    "expected_feature_deltas": observation.get(
                        "expected_feature_deltas", {}
                    ),
                    "expected_operation_deltas": observation.get(
                        "expected_operation_deltas", {}
                    ),
                    "observed_operation_deltas": observation.get(
                        "observed_operation_deltas", {}
                    ),
                    "absolute_feature_counts": observation.get(
                        "absolute_feature_counts", {}
                    ),
                    "absolute_operation_pricing_units": observation.get(
                        "absolute_operation_pricing_units", {}
                    ),
                    "operation_phase_ownership": observation.get(
                        "operation_phase_ownership"
                    ),
                    "system_operation_ownership": observation.get(
                        "system_operation_ownership"
                    ),
                    "anchor_operation_ownership": observation.get(
                        "anchor_operation_ownership"
                    ),
                }
                row["execution_row_id"] = controlled_execution_row_id(
                    workload_id,
                    backend="sp1",
                    execution_engine=row["sp1_execution_engine"],
                    run_id=calibration_run_id,
                    repeat_index=repeat_index,
                    backend_input_sha256=backend_input_sha256,
                )
                if "prover_gas" not in row and "gas" in row:
                    row["prover_gas"] = row["gas"]
                rows.append(row)
    with out.open("w") as output:
        for row in rows:
            output.write(json.dumps(row, sort_keys=True) + "\n")


def _stable_overhead_declaration(
    rows: Iterable[Mapping[str, Any]], field_name: str
) -> dict[str, int]:
    values = [row.get(field_name, {}) for row in rows]
    if not values or any(not isinstance(value, Mapping) for value in values):
        raise ValueError(f"controlled overhead rows are missing {field_name}")
    canonical = {canonical_json(value) for value in values}
    if len(canonical) != 1:
        raise ValueError(f"controlled overhead {field_name} changed across repeats")
    return {str(key): int(value) for key, value in values[0].items()}


def _stable_operation_deltas(
    rows: Iterable[Mapping[str, Any]], field_name: str
) -> dict[str, dict[str, Any]]:
    values = [row.get(field_name, {}) for row in rows]
    if not values or any(not isinstance(value, Mapping) for value in values):
        raise ValueError(f"controlled overhead rows are missing {field_name}")
    canonical = {canonical_json(value) for value in values}
    if len(canonical) != 1:
        raise ValueError(f"controlled overhead {field_name} changed across repeats")
    result = {}
    for key, value in values[0].items():
        if not isinstance(value, Mapping):
            raise ValueError("controlled operation delta must declare basis and units")
        basis = value.get("pricing_basis")
        if basis not in {"raw_gas_slope", "fixed_per_event"}:
            raise ValueError("controlled operation delta has unknown pricing basis")
        units = value.get("units")
        if isinstance(units, bool) or not isinstance(units, int) or units == 0:
            raise ValueError("controlled operation delta units must be a nonzero integer")
        result[str(key)] = {"pricing_basis": basis, "units": units}
    return result


def _overhead_repeat_point(rows: Iterable[Mapping[str, Any]]) -> dict[str, Any]:
    point = _controlled_repeat_point(rows)
    point["isolation"] = {
        "status": "passed",
        "bytecode_size": 0,
        "input_size": 0,
        "non_target_counts": {},
        "non_target_raw_gas": 0,
    }
    return point


def _operation_costs(
    manifest: Manifest, case_results: Iterable[Mapping[str, Any]]
) -> dict[str, dict[str, Any]]:
    rows = list(case_results)
    values = construct_measurement_values(manifest, rows)
    by_case = {str(row.get("case_id")): row for row in rows}
    key_specs = {key.id: key for key in manifest.measurement_keys}
    result = {}
    for key, value in values.items():
        if value.get("status") != "accepted":
            continue
        field_name = "c_p" if "c_p" in value else "f_p"
        if field_name in value:
            operation = {
                "pricing_basis": value["pricing_basis"],
                "cost": _decimal(value[field_name], label=f"operation cost {key}"),
            }
            required = [by_case.get(case_id) for case_id in key_specs[key].required_case_ids]
            secondary_values = []
            for row in required:
                secondary = row.get("secondary") if isinstance(row, Mapping) else None
                if not isinstance(secondary, Mapping) or secondary.get("status") != "available":
                    secondary_values = []
                    break
                secondary_slope = _decimal(
                    secondary.get("g_s"), label=f"secondary operation cost {key}"
                )
                if value["pricing_basis"] == "raw_gas_slope":
                    raw_gas = _decimal(row.get("target_raw_gas"), label=f"raw gas {key}")
                    if raw_gas <= 0:
                        secondary_values = []
                        break
                    secondary_slope /= raw_gas
                secondary_values.append(secondary_slope)
            if secondary_values:
                minimum, maximum = min(secondary_values), max(secondary_values)
                if minimum > 0 and maximum / minimum - Decimal(1) <= Decimal("0.05"):
                    operation["secondary_cost"] = sum(secondary_values) / Decimal(
                        len(secondary_values)
                    )
            result[key] = operation
    return result


def _paired_overhead_points(
    manifest: Manifest,
    key_id: str,
    case_id: str,
    rows: list[Mapping[str, Any]],
    operation_costs: Mapping[str, Mapping[str, Any]],
    accepted_overheads: Mapping[str, Decimal],
    accepted_secondary_overheads: Mapping[str, Decimal],
) -> tuple[list[dict[str, Any]], list[str]]:
    points = []
    reasons = []
    counts = sorted(
        {
            int(row["target_count"])
            for row in rows
            if row.get("case") == case_id
            and row.get("lane") == "target"
            and row.get("status") == "accepted"
        }
    )
    for count in counts:
        target_rows = [
            row
            for row in rows
            if row.get("case") == case_id
            and row.get("lane") == "target"
            and int(row.get("target_count", -1)) == count
            and row.get("status") == "accepted"
        ]
        control_rows = [
            row
            for row in rows
            if row.get("case") == case_id
            and row.get("lane") == "control"
            and int(row.get("target_count", -1)) == count
            and row.get("status") == "accepted"
        ]
        target = _overhead_repeat_point(target_rows)
        control = _overhead_repeat_point(control_rows)
        feature_deltas = _stable_overhead_declaration(
            target_rows, "expected_feature_deltas"
        )
        operation_deltas = _stable_operation_deltas(
            target_rows, "observed_operation_deltas"
        )
        missing_operations = [
            operation
            for operation in operation_deltas
            if operation not in operation_costs
        ]
        if missing_operations:
            reasons.append("unmeasured_operation_delta")
            continue
        basis_mismatches = [
            operation
            for operation, delta in operation_deltas.items()
            if delta["pricing_basis"]
            != operation_costs[operation]["pricing_basis"]
        ]
        if basis_mismatches:
            reasons.append("operation_basis_mismatch")
            continue
        operation_cost = sum(
            Decimal(delta["units"]) * operation_costs[operation]["cost"]
            for operation, delta in operation_deltas.items()
        )
        secondary_dependencies_available = all(
            "secondary_cost" in operation_costs[operation]
            for operation in operation_deltas
        ) and all(
            child in accepted_secondary_overheads
            for child in manifest.subtract_closure.get(key_id, ())
        )
        secondary_operation_cost = (
            sum(
                Decimal(delta["units"])
                * operation_costs[operation]["secondary_cost"]
                for operation, delta in operation_deltas.items()
            )
            if secondary_dependencies_available
            else None
        )
        own_units = feature_deltas.get(key_id, 0)
        gas_repeats = []
        instruction_repeats = []
        for target_gas, control_gas, target_s, control_s in zip(
            target["prover_gas_repeats"],
            control["prover_gas_repeats"],
            target["instruction_count_repeats"],
            control["instruction_count_repeats"],
        ):
            delta = (
                _decimal(target_gas, label="overhead target proverGas")
                - _decimal(control_gas, label="overhead control proverGas")
                - operation_cost
            )
            if own_units:
                delta = residualize_overhead_delta(
                    manifest,
                    key_id,
                    target_minus_control=delta,
                    feature_deltas=feature_deltas,
                    accepted_costs=accepted_overheads,
                ) * Decimal(own_units)
            elif delta != 0 or any(feature_deltas.values()):
                reasons.append("zero_count_nonzero_delta")
            gas_repeats.append(_decimal_text(delta))
            if secondary_dependencies_available:
                secondary_delta = (
                    _decimal(target_s, label="overhead target instruction count")
                    - _decimal(control_s, label="overhead control instruction count")
                    - secondary_operation_cost
                )
                if own_units:
                    secondary_delta = residualize_overhead_delta(
                        manifest,
                        key_id,
                        target_minus_control=secondary_delta,
                        feature_deltas=feature_deltas,
                        accepted_costs=accepted_secondary_overheads,
                    ) * Decimal(own_units)
                instruction_repeats.append(_decimal_text(secondary_delta))
        points.append(
            {
                "count": count,
                "prover_gas_repeats": gas_repeats,
                "instruction_count_repeats": instruction_repeats,
                "secondary_unavailable_reason": (
                    None
                    if secondary_dependencies_available
                    else "unresidualized_dependencies"
                ),
                "case_input_sha256_repeats": ["paired"] * 3,
                "exit_code_repeats": [0, 0, 0],
                "public_values_repeats": ["paired"] * 3,
                "isolation": {
                    "status": "passed",
                    "bytecode_size": 0,
                    "input_size": 0,
                    "non_target_counts": {},
                    "non_target_raw_gas": 0,
                },
            }
        )
    return points, list(dict.fromkeys(reasons))


def fit_controlled_overheads(
    manifest: Manifest,
    rows: Iterable[Mapping[str, Any]],
    measurement_case_results: Iterable[Mapping[str, Any]],
    *,
    generator_max_count: int,
    overhead_generator_max_count: int | None = None,
) -> dict[str, Any]:
    """Residualize the four frozen controlled overhead identities."""
    overhead_generator_max_count = (
        generator_max_count
        if overhead_generator_max_count is None
        else overhead_generator_max_count
    )
    if (
        overhead_generator_max_count > CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT
        or overhead_generator_max_count > generator_max_count
    ):
        raise ValueError("invalid controlled overhead generator maximum")
    row_list = list(rows)
    operation_costs = _operation_costs(manifest, measurement_case_results)
    accepted: dict[str, Decimal] = {}
    accepted_secondary: dict[str, Decimal] = {}
    case_results: list[dict[str, Any]] = []
    overhead_results: dict[str, dict[str, Any]] = {}
    by_key = {key.id: key for key in manifest.overhead_keys}
    required_keys = sorted(
        (key for key in manifest.overhead_keys if key.formula_role == "required"),
        key=lambda key: len(manifest.subtract_closure.get(key.id, ())),
    )
    for key in required_keys:
        if key.id == "proposal_startup":
            continue
        required_values = []
        key_case_results = []
        for case_id in key.required_case_ids:
            missing_dependencies = sorted(
                child
                for child in manifest.subtract_closure.get(key.id, ())
                if child not in accepted
            )
            if missing_dependencies:
                result = {
                    "case_id": case_id,
                    "overhead_key_id": key.id,
                    "status": "rejected",
                    "reasons": ["unmeasured_overhead_dependency"],
                    "dependency_ids": missing_dependencies,
                    "secondary": {
                        "status": "failed",
                        "reason": "unresidualized_dependencies",
                        "dependency_ids": missing_dependencies,
                    },
                }
            elif any(
                row.get("case") == case_id and row.get("status") == "rejected"
                for row in row_list
            ):
                result = {
                    "case_id": case_id,
                    "overhead_key_id": key.id,
                    "status": "rejected",
                    "reasons": ["generation_failure"],
                }
            else:
                points, reasons = _paired_overhead_points(
                    manifest,
                    key.id,
                    case_id,
                    row_list,
                    operation_costs,
                    accepted,
                    accepted_secondary,
                )
                result = {
                    "case_id": case_id,
                    "overhead_key_id": key.id,
                    **(
                        {"status": "rejected", "reasons": reasons}
                        if reasons
                        else evaluate_controlled_sweep(
                            points,
                            pricing_basis="fixed_per_event",
                            target_raw_gas=None,
                            generator_max_count=overhead_generator_max_count,
                        )
                    ),
                }
                if not reasons and any(
                    point.get("secondary_unavailable_reason")
                    == "unresidualized_dependencies"
                    for point in points
                ):
                    result["secondary"] = {
                        "status": "failed",
                        "reason": "unresidualized_dependencies",
                    }
            key_case_results.append(result)
            case_results.append(result)
            if result.get("status") == "accepted":
                required_values.append(
                    _decimal(result["f_p"], label=f"overhead case {case_id}")
                )
        if len(required_values) != len(key.required_case_ids):
            overhead_results[key.id] = {
                "status": "required_case_incomplete",
                "required_case_ids": list(key.required_case_ids),
            }
            continue
        minimum, maximum = min(required_values), max(required_values)
        if minimum <= 0 or maximum / minimum - Decimal(1) > Decimal("0.05"):
            overhead_results[key.id] = {
                "status": "scenario_dependent",
                "required_case_ids": list(key.required_case_ids),
            }
            continue
        value = sum(required_values) / Decimal(len(required_values))
        accepted[key.id] = value
        secondary_values = [
            _decimal(result["secondary"]["g_s"], label=f"secondary overhead {key.id}")
            for result in key_case_results
            if isinstance(result.get("secondary"), Mapping)
            and result["secondary"].get("status") == "available"
        ]
        secondary_value = None
        if len(secondary_values) == len(key.required_case_ids):
            minimum_s, maximum_s = min(secondary_values), max(secondary_values)
            if minimum_s > 0 and maximum_s / minimum_s - Decimal(1) <= Decimal("0.05"):
                secondary_value = sum(secondary_values) / Decimal(len(secondary_values))
                accepted_secondary[key.id] = secondary_value
        overhead_results[key.id] = {
            "status": "accepted",
            "o_p": _decimal_text(value),
            "required_case_ids": list(key.required_case_ids),
            "checkpoint_evidence": {
                result["case_id"]: result.get("checkpoint")
                for result in key_case_results
            },
            "secondary": (
                {"status": "available", "o_s": _decimal_text(secondary_value)}
                if secondary_value is not None
                else {"status": "failed", "reason": "unresidualized_dependencies"}
            ),
        }

    startup = by_key["proposal_startup"]
    startup_residuals = []
    startup_secondary_residuals = []
    startup_complete = True
    for case_id in startup.required_case_ids:
        missing_dependencies = sorted(
            child
            for child in manifest.subtract_closure.get("proposal_startup", ())
            if child not in accepted
        )
        if missing_dependencies:
            startup_complete = False
            case_results.append(
                {
                    "case_id": case_id,
                    "overhead_key_id": "proposal_startup",
                    "status": "rejected",
                    "reasons": ["unmeasured_overhead_dependency"],
                    "dependency_ids": missing_dependencies,
                    "secondary": {
                        "status": "failed",
                        "reason": "unresidualized_dependencies",
                        "dependency_ids": missing_dependencies,
                    },
                }
            )
            continue
        case_rows = [
            row
            for row in row_list
            if row.get("case") == case_id and row.get("status") == "accepted"
        ]
        try:
            point = _overhead_repeat_point(case_rows)
            features = _stable_overhead_declaration(
                case_rows, "expected_feature_deltas"
            )
            operations = _stable_operation_deltas(
                case_rows, "observed_operation_deltas"
            )
            missing = [
                operation for operation in operations if operation not in operation_costs
            ]
            if missing:
                raise ValueError("unmeasured startup operation delta")
            basis_mismatches = [
                operation
                for operation, delta in operations.items()
                if delta["pricing_basis"]
                != operation_costs[operation]["pricing_basis"]
            ]
            if basis_mismatches:
                raise ValueError("startup operation pricing basis mismatch")
            operation_cost = sum(
                Decimal(delta["units"]) * operation_costs[operation]["cost"]
                for operation, delta in operations.items()
            )
            secondary_dependencies_available = all(
                "secondary_cost" in operation_costs[operation]
                for operation in operations
            ) and all(
                child in accepted_secondary
                for child in manifest.subtract_closure.get("proposal_startup", ())
            )
            secondary_operation_cost = (
                sum(
                    Decimal(delta["units"])
                    * operation_costs[operation]["secondary_cost"]
                    for operation, delta in operations.items()
                )
                if secondary_dependencies_available
                else None
            )
            residuals = [
                _decimal_text(
                    residualize_overhead_delta(
                        manifest,
                        "proposal_startup",
                        target_minus_control=_decimal(
                            value, label="startup proverGas"
                        )
                        - operation_cost,
                        feature_deltas=features,
                        accepted_costs=accepted,
                    )
                )
                for value in point["prover_gas_repeats"]
            ]
            startup_residuals.append(residuals)
            if secondary_dependencies_available:
                secondary_residuals = [
                    _decimal_text(
                        residualize_overhead_delta(
                            manifest,
                            "proposal_startup",
                            target_minus_control=_decimal(
                                value, label="startup instruction count"
                            )
                            - secondary_operation_cost,
                            feature_deltas=features,
                            accepted_costs=accepted_secondary,
                        )
                    )
                    for value in point["instruction_count_repeats"]
                ]
                startup_secondary_residuals.append(secondary_residuals)
            case_results.append(
                {
                    "case_id": case_id,
                    "overhead_key_id": "proposal_startup",
                    "status": "accepted",
                    "repeat_residuals": residuals,
                }
            )
        except (KeyError, ValueError) as error:
            startup_complete = False
            case_results.append(
                {
                    "case_id": case_id,
                    "overhead_key_id": "proposal_startup",
                    "status": "rejected",
                    "reasons": [str(error)],
                }
            )
    if startup_complete and len(startup_residuals) == len(startup.required_case_ids):
        value = fixed_startup_residual(startup_residuals)
        accepted["proposal_startup"] = value
        secondary_value = None
        if len(startup_secondary_residuals) == len(startup.required_case_ids):
            secondary_value = fixed_startup_residual(startup_secondary_residuals)
            accepted_secondary["proposal_startup"] = secondary_value
        overhead_results["proposal_startup"] = {
            "status": "accepted",
            "o_p": _decimal_text(value),
            "baseline_kind": "mathematical_zero_baseline",
            "secondary": (
                {"status": "available", "o_s": _decimal_text(secondary_value)}
                if secondary_value is not None
                else {"status": "failed", "reason": "unresidualized_dependencies"}
            ),
        }
    else:
        overhead_results["proposal_startup"] = {
            "status": "required_case_incomplete"
        }
    return {
        "schema_version": 1,
        "generator_max_count": generator_max_count,
        "overhead_generator_max_count": overhead_generator_max_count,
        "status": (
            "accepted" if set(accepted) == set(Q_FORMULA) else "rejected"
        ),
        "o_p": {key: _decimal_text(accepted[key]) for key in Q_FORMULA if key in accepted},
        "o_s": {
            key: _decimal_text(accepted_secondary[key])
            for key in Q_FORMULA
            if key in accepted_secondary
        },
        "overhead_results": overhead_results,
        "case_results": case_results,
    }


def controlled_round_decision(
    case_results: Iterable[Mapping[str, Any]], generator_max_count: int
) -> str:
    results = list(case_results)
    if not results:
        raise ValueError("controlled round produced no case results")
    expansion_reasons = {"exhausted_sweep", "unmeasured_overhead_dependency"}
    needs_larger_footprint = any(
        result.get("status") == "rejected"
        and expansion_reasons.intersection(result.get("reasons", []))
        for result in results
    )
    if needs_larger_footprint and generator_max_count != CONTROLLED_GENERATOR_ROUNDS[-1]:
        return "expand_next_round"
    return "complete"


def validate_persisted_controlled_decisions(
    calibration_run: pathlib.Path, decisions: Mapping[str, Any]
) -> dict[int, Mapping[str, Any]]:
    rounds = decisions.get("rounds")
    if decisions.get("schema_version") != 1 or not isinstance(rounds, list):
        raise ValueError("invalid persisted controlled decisions")
    observed = [int(record.get("generator_max_count", -1)) for record in rounds]
    if observed != list(CONTROLLED_GENERATOR_ROUNDS[: len(observed)]):
        raise ValueError("persisted controlled rounds must be a unique contiguous prefix")
    validated: dict[int, Mapping[str, Any]] = {}
    for index, record in enumerate(rounds):
        generator_max_count = observed[index]
        expected_overhead_generator_max_count = min(
            generator_max_count, CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT
        )
        if (
            record.get("overhead_generator_max_count")
            != expected_overhead_generator_max_count
        ):
            raise ValueError("persisted controlled overhead generator maximum changed")
        raw_path = calibration_run / record["raw_runs"]
        fit_path = calibration_run / record["fit"]
        if (
            not raw_path.is_file()
            or sha256_file(raw_path) != record.get("raw_runs_sha256")
            or not fit_path.is_file()
            or sha256_file(fit_path) != record.get("fit_sha256")
        ):
            raise ValueError("persisted controlled round artifact changed")
        fit_payload = json.loads(fit_path.read_text())
        if fit_payload.get("generator_max_count") != generator_max_count:
            raise ValueError("persisted controlled fit footprint changed")
        overhead_path = calibration_run / record["overhead_fit"]
        overhead_runs_path = calibration_run / record["overhead_runs"]
        if (
            not overhead_path.is_file()
            or sha256_file(overhead_path) != record.get("overhead_fit_sha256")
            or not overhead_runs_path.is_file()
            or sha256_file(overhead_runs_path) != record.get("overhead_runs_sha256")
        ):
            raise ValueError("persisted controlled overhead artifact changed")
        overhead_payload = json.loads(overhead_path.read_text())
        if (
            overhead_payload.get("generator_max_count") != generator_max_count
            or overhead_payload.get("overhead_generator_max_count")
            != expected_overhead_generator_max_count
        ):
            raise ValueError("persisted controlled overhead footprint changed")
        if generator_max_count > CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT:
            source = validated.get(CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT)
            if source is None or (
                record.get("overhead_runs") != source.get("overhead_runs")
                or record.get("overhead_runs_sha256")
                != source.get("overhead_runs_sha256")
            ):
                raise ValueError(
                    "persisted controlled overhead raw does not reuse the sealed "
                    f"{CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT} round"
                )
        decision = controlled_round_decision(
            [
                *fit_payload.get("case_results", []),
                *overhead_payload.get("case_results", []),
            ],
            generator_max_count,
        )
        if decision != record.get("decision"):
            raise ValueError("persisted controlled round decision changed")
        if decision == "complete" and index != len(rounds) - 1:
            raise ValueError("persisted controlled rounds continue after completion")
        validated[generator_max_count] = record
    return validated


def _canonical_run_artifact(
    calibration_run: pathlib.Path, supplied: pathlib.Path, filename: str
) -> pathlib.Path:
    canonical = (calibration_run / filename).resolve()
    if supplied.resolve() != canonical:
        raise ValueError(f"{filename} must be the canonical persisted run artifact")
    return canonical


def load_terminal_controlled_artifacts(
    calibration_run: pathlib.Path,
    identity: Mapping[str, Any],
) -> dict[str, Any]:
    """Verify and load the one complete adaptive round sealed by this run."""
    experiment_path = calibration_run / "experiment.json"
    decisions_path = calibration_run / "controlled-decisions.json"
    decisions_seal_path = calibration_run / "controlled-decisions.sha256"
    if not experiment_path.is_file() or not decisions_path.is_file():
        raise ValueError("calibration run is missing experiment or controlled decisions")
    if (
        not decisions_seal_path.is_file()
        or _read_digest(decisions_seal_path) != sha256_file(decisions_path)
    ):
        raise ValueError("controlled decisions seal does not match persisted decisions")
    experiment = json.loads(experiment_path.read_text())
    declaration = experiment_provenance_declaration(experiment)
    if declaration["calibration_id"] != calibration_run.name:
        raise ValueError("experiment calibration_id does not match calibration directory")
    for key in (
        "implementation_revision",
        "controlled_manifest_sha256",
        "controlled_manifest_rows_sha256",
    ):
        if declaration[key] != identity.get(key):
            raise ValueError(f"experiment {key} does not match frozen manifest identity")

    decisions = json.loads(decisions_path.read_text())
    validated = validate_persisted_controlled_decisions(calibration_run, decisions)
    if not validated:
        raise ValueError("controlled decisions have no terminal round")
    terminal_count = next(reversed(validated))
    terminal = validated[terminal_count]
    if terminal.get("decision") != "complete":
        raise ValueError("controlled decisions have no terminal complete round")

    fit_path = calibration_run / "controlled-fit.json"
    overhead_path = calibration_run / "controlled-overheads.json"
    if (
        not fit_path.is_file()
        or sha256_file(fit_path) != terminal.get("fit_sha256")
        or not overhead_path.is_file()
        or sha256_file(overhead_path) != terminal.get("overhead_fit_sha256")
    ):
        raise ValueError("canonical controlled artifacts do not match terminal decision")
    fit = json.loads(fit_path.read_text())
    overhead = json.loads(overhead_path.read_text())
    if (
        fit.get("generator_max_count") != terminal_count
        or overhead.get("generator_max_count") != terminal_count
        or overhead.get("overhead_generator_max_count")
        != min(terminal_count, CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT)
    ):
        raise ValueError("canonical controlled artifact footprint is not terminal")
    return {
        "experiment": experiment,
        "provenance_declaration": declaration,
        "terminal": terminal,
        "fit": fit,
        "overheads": overhead,
        "fit_sha256": terminal["fit_sha256"],
        "overheads_sha256": terminal["overhead_fit_sha256"],
        "controlled_decisions_sha256": sha256_file(decisions_path),
        "generator_max_count": terminal_count,
        "overhead_generator_max_count": overhead["overhead_generator_max_count"],
    }


def _sealed_candidate_provenance(
    artifacts: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        **artifacts["provenance_declaration"],
        "controlled_decisions_sha256": artifacts["controlled_decisions_sha256"],
        "controlled_fit_sha256": artifacts["fit_sha256"],
        "controlled_overheads_sha256": artifacts["overheads_sha256"],
        "terminal_generator_max_count": artifacts["generator_max_count"],
        "terminal_overhead_generator_max_count": artifacts[
            "overhead_generator_max_count"
        ],
    }


def _controlled_sample_artifact(
    manifest: Manifest,
    artifacts: Mapping[str, Any],
    candidate_sha256: str,
) -> dict[str, Any]:
    payload = build_controlled_cycle_cost_samples(
        manifest,
        artifacts["fit"]["case_results"],
        artifacts["overheads"],
    )
    payload["calibration_id"] = artifacts["provenance_declaration"]["calibration_id"]
    payload["implementation_revision"] = artifacts["provenance_declaration"][
        "implementation_revision"
    ]
    payload["candidate_sha256"] = candidate_sha256
    payload["controlled_manifest_sha256"] = artifacts["provenance_declaration"][
        "controlled_manifest_sha256"
    ]
    payload["controlled_manifest_rows_sha256"] = artifacts[
        "provenance_declaration"
    ]["controlled_manifest_rows_sha256"]
    payload["controlled_decisions_sha256"] = artifacts[
        "controlled_decisions_sha256"
    ]
    payload["controlled_fit_sha256"] = artifacts["fit_sha256"]
    payload["controlled_overheads_sha256"] = artifacts["overheads_sha256"]
    return payload


def cmd_run_controlled(args: argparse.Namespace) -> None:
    calibration_run = _resolve_repo_path(args.calibration_run, field_name="calibration_run")
    validate_calibration_execution_identity(calibration_run)
    manifest_path = _resolve_repo_path(
        args.controlled_manifest, field_name="controlled_manifest"
    )
    manifest, identity = verify_frozen_controlled_manifest(calibration_run, manifest_path)
    fixtures_root = _resolve_repo_path(args.fixtures, field_name="fixtures")
    final_runs = _resolve_repo_path(args.out, field_name="controlled_runs")
    decisions_path = calibration_run / "controlled-decisions.json"
    decisions_seal_path = calibration_run / "controlled-decisions.sha256"
    decisions = {"schema_version": 1, "rounds": []}
    if decisions_path.exists():
        if (
            not decisions_seal_path.is_file()
            or _read_digest(decisions_seal_path) != sha256_file(decisions_path)
        ):
            raise ValueError("controlled decisions seal does not match persisted decisions")
        decisions = json.loads(decisions_path.read_text())

    previous_rounds = validate_persisted_controlled_decisions(calibration_run, decisions)
    for generator_max_count in CONTROLLED_GENERATOR_ROUNDS:
        previous = previous_rounds.get(generator_max_count)
        if previous is not None:
            raw_path = calibration_run / previous["raw_runs"]
            decision = previous["decision"]
            if decision == "complete":
                final_runs.parent.mkdir(parents=True, exist_ok=True)
                final_runs.write_bytes(raw_path.read_bytes())
                overhead_fit = calibration_run / previous["overhead_fit"]
                (calibration_run / "controlled-overheads.json").write_bytes(
                    overhead_fit.read_bytes()
                )
                fit_path = calibration_run / previous["fit"]
                (calibration_run / "controlled-fit.json").write_bytes(
                    fit_path.read_bytes()
                )
                print(f"resumed complete controlled round at {generator_max_count}")
                return
            if decision != "expand_next_round":
                raise ValueError("invalid persisted controlled round decision")
            continue

        round_name = f"generator-max-{generator_max_count}"
        round_fixtures = fixtures_root / round_name
        generate_cases(
            manifest,
            round_fixtures,
            provenance={
                "calibration_id": calibration_run.name,
                "controlled_manifest_sha256": identity[
                    "controlled_manifest_sha256"
                ],
                "controlled_manifest_rows_sha256": identity[
                    "controlled_manifest_rows_sha256"
                ],
            },
            generator_max_count=generator_max_count,
        )
        round_runs = calibration_run / f"controlled-runs.{round_name}.jsonl"
        cmd_run(
            argparse.Namespace(
                fixtures=round_fixtures,
                guest_launcher=args.guest_launcher,
                elf=args.elf,
                precompile_elf=args.precompile_elf,
                opcode_stage="revm-opcode-lab",
                calibration_run=calibration_run,
                controlled_manifest=manifest_path,
                out=round_runs,
                repeats=3,
            )
        )
        results = fit_controlled_costs(manifest, iter_jsonl(round_runs))
        fit_payload = {
            "schema_version": 1,
            "generator_max_count": generator_max_count,
            "case_results": results,
        }
        round_fit = calibration_run / f"controlled-fit.{round_name}.json"
        round_fit.write_text(json.dumps(fit_payload, indent=2, sort_keys=True) + "\n")
        overhead_generator_max_count = min(
            generator_max_count, CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT
        )
        if generator_max_count <= CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT:
            overhead_runs = (
                calibration_run / f"controlled-overhead-runs.{round_name}.jsonl"
            )
            run_controlled_overhead_round(
                guest_launcher=args.guest_launcher,
                calibration_run_id=calibration_run.name,
                generator_max_count=overhead_generator_max_count,
                include_startup=generator_max_count == CONTROLLED_GENERATOR_ROUNDS[0],
                out=overhead_runs,
            )
        else:
            source_record = next(
                (
                    record
                    for record in decisions["rounds"]
                    if record.get("generator_max_count")
                    == CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT
                ),
                None,
            )
            if source_record is None:
                raise ValueError(
                    f"controlled {generator_max_count} round has no sealed "
                    f"{CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT} overhead raw"
                )
            overhead_runs = calibration_run / source_record["overhead_runs"]
            if (
                not overhead_runs.is_file()
                or sha256_file(overhead_runs)
                != source_record.get("overhead_runs_sha256")
            ):
                raise ValueError(
                    f"sealed {CONTROLLED_OVERHEAD_GENERATOR_MAX_COUNT} overhead raw "
                    f"changed before {generator_max_count} reuse"
                )
        overhead_rows = list(iter_jsonl(overhead_runs))
        if generator_max_count != CONTROLLED_GENERATOR_ROUNDS[0]:
            first_record = decisions["rounds"][0]
            first_rows = iter_jsonl(calibration_run / first_record["overhead_runs"])
            overhead_rows.extend(
                row
                for row in first_rows
                if row.get("overhead_key_id") == "proposal_startup"
            )
        overhead_artifact = fit_controlled_overheads(
            manifest,
            overhead_rows,
            results,
            generator_max_count=generator_max_count,
            overhead_generator_max_count=overhead_generator_max_count,
        )
        round_overhead_fit = calibration_run / f"controlled-overheads.{round_name}.json"
        round_overhead_fit.write_text(
            json.dumps(overhead_artifact, indent=2, sort_keys=True) + "\n"
        )
        decision = controlled_round_decision(
            [*results, *overhead_artifact["case_results"]], generator_max_count
        )
        record = {
            "generator_max_count": generator_max_count,
            "raw_runs": str(round_runs.relative_to(calibration_run)),
            "raw_runs_sha256": sha256_file(round_runs),
            "fit": str(round_fit.relative_to(calibration_run)),
            "fit_sha256": sha256_file(round_fit),
            "overhead_runs": str(overhead_runs.relative_to(calibration_run)),
            "overhead_runs_sha256": sha256_file(overhead_runs),
            "overhead_generator_max_count": overhead_generator_max_count,
            "overhead_fit": str(round_overhead_fit.relative_to(calibration_run)),
            "overhead_fit_sha256": sha256_file(round_overhead_fit),
            "decision": decision,
        }
        decisions["rounds"].append(record)
        decisions_path.write_text(json.dumps(decisions, indent=2, sort_keys=True) + "\n")
        decisions_seal_path.write_text(sha256_file(decisions_path) + "\n")
        if decision == "complete":
            final_runs.parent.mkdir(parents=True, exist_ok=True)
            final_runs.write_bytes(round_runs.read_bytes())
            (calibration_run / "controlled-overheads.json").write_bytes(
                round_overhead_fit.read_bytes()
            )
            (calibration_run / "controlled-fit.json").write_bytes(
                round_fit.read_bytes()
            )
            print(f"completed controlled round at {generator_max_count}")
            return
    raise ValueError("controlled adaptive sweep exhausted every frozen generator round")


def cmd_build_candidate(args: argparse.Namespace) -> None:
    run = _resolve_repo_path(args.run, field_name="calibration_run")
    manifest, identity = verify_frozen_controlled_manifest(
        run,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
    )
    artifacts = load_terminal_controlled_artifacts(run, identity)
    fit_path = _canonical_run_artifact(
        run,
        _resolve_repo_path(args.fit, field_name="controlled_fit"),
        "controlled-fit.json",
    )
    overhead_path = _canonical_run_artifact(
        run,
        _resolve_repo_path(args.overheads, field_name="controlled_overheads"),
        "controlled-overheads.json",
    )
    if (
        sha256_file(fit_path) != artifacts["fit_sha256"]
        or sha256_file(overhead_path) != artifacts["overheads_sha256"]
    ):
        raise ValueError("controlled fit/overhead do not match terminal decision")
    fit = artifacts["fit"]
    overhead_artifact = artifacts["overheads"]
    if overhead_artifact.get("status") != "accepted" or not isinstance(
        overhead_artifact.get("o_p"), Mapping
    ):
        raise ValueError("controlled overhead artifact is not accepted")
    provenance_path = _canonical_run_artifact(
        run,
        _resolve_repo_path(args.provenance, field_name="candidate_provenance"),
        "provenance.json",
    )
    supplied_provenance = json.loads(provenance_path.read_text())
    if supplied_provenance != artifacts["provenance_declaration"]:
        raise ValueError("candidate provenance does not match experiment identity")
    provenance = _sealed_candidate_provenance(artifacts)
    schedule = current_uzen_schedule()
    preview = build_candidate_components(
        manifest,
        fit["case_results"],
        overhead_artifact["o_p"],
        provenance,
        schedule,
    )
    samples = _controlled_sample_artifact(
        manifest, artifacts, preview["candidate_sha256"]
    )
    components = seal_candidate_directory(
        run,
        manifest,
        fit["case_results"],
        overhead_artifact["o_p"],
        provenance,
        schedule,
        controlled_cycle_samples=samples,
    )
    if components["candidate_sha256"] != preview["candidate_sha256"]:
        raise ValueError("candidate identity changed while sealing controlled samples")
    print(f"sealed candidate {components['candidate_sha256']}")


def cmd_build_sp1_bridge(args: argparse.Namespace) -> None:
    run = _resolve_repo_path(args.run, field_name="calibration_run")
    manifest, identity = verify_frozen_controlled_manifest(
        run,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
    )
    artifacts = load_terminal_controlled_artifacts(run, identity)
    samples_path = _canonical_run_artifact(
        run,
        _resolve_repo_path(args.samples, field_name="controlled_samples"),
        "samples/controlled-cycle-cost-samples.json",
    )
    payload = json.loads(samples_path.read_text())
    candidate = verify_candidate_directory(run)
    expected = _controlled_sample_artifact(
        manifest, artifacts, candidate["candidate_sha256"]
    )
    if payload != expected:
        raise ValueError("controlled samples do not match sealed candidate/run identity")
    samples = payload.get("samples", payload)
    if not isinstance(samples, Mapping):
        raise ValueError("controlled samples must be a JSON object")
    bridge = seal_bridge_directory(
        run,
        manifest,
        samples,
        controlled_sample_artifact=(payload if "samples" in payload else None),
    )
    print(f"sealed SP1 bridge {bridge['bridge_sha256']}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    generate = subcommands.add_parser("generate", help="generate opcode case metadata")
    generate.add_argument("--manifest", type=pathlib.Path, required=True)
    generate.add_argument("--calibration-run", type=pathlib.Path, required=True)
    generate.add_argument("--out", type=pathlib.Path, required=True)
    generate.add_argument(
        "--generator-max-count",
        type=int,
        choices=[8, 32, 128, 512, 2048],
        default=8,
        help="frozen controlled sweep checkpoint bound",
    )
    generate.set_defaults(func=cmd_generate, matched_control_diagnostic=False)

    matched_generate = subcommands.add_parser(
        "generate-matched-control",
        help="generate diagnostic matched-control opcode pairs",
    )
    matched_generate.add_argument("--manifest", type=pathlib.Path, required=True)
    matched_generate.add_argument("--calibration-run", type=pathlib.Path, required=True)
    matched_generate.add_argument("--out", type=pathlib.Path, required=True)
    matched_generate.add_argument(
        "--case",
        dest="matched_control_cases",
        action="append",
        required=True,
        help="manifest case ID to include; repeat for multiple cases",
    )
    matched_generate.add_argument(
        "--operand-profile",
        choices=tuple(MATCHED_CONTROL_OPERAND_PROFILES),
        default="zero",
        help="fixed matched-control operand profile",
    )
    matched_generate.add_argument(
        "--generator-max-count",
        type=int,
        choices=[8, 32, 128, 512, 2048],
        default=8,
        help="diagnostic fixed-footprint checkpoint bound",
    )
    matched_generate.set_defaults(func=cmd_generate, matched_control_diagnostic=True)

    relation_generate = subcommands.add_parser(
        "generate-relations",
        help="generate the frozen formal opcode relation campaign",
    )
    relation_generate.add_argument("--manifest", type=pathlib.Path, required=True)
    relation_generate.add_argument("--calibration-run", type=pathlib.Path, required=True)
    relation_generate.add_argument("--out", type=pathlib.Path, required=True)
    relation_generate.add_argument(
        "--generator-max-count",
        type=int,
        choices=tuple(OUT_OF_FIT_CHECKPOINTS.values()),
        default=8,
    )
    relation_generate.set_defaults(
        func=cmd_generate_relations,
        formal_relation_purpose=FORMAL_RELATION_PURPOSE,
    )

    run = subcommands.add_parser("run", help="run generated guest-input cases")
    _add_controlled_run_arguments(run)
    run.set_defaults(func=cmd_run, expected_purpose=None)

    matched_run = subcommands.add_parser(
        "run-matched-control",
        help="run diagnostic matched-control opcode pairs",
    )
    _add_controlled_run_arguments(
        matched_run,
        opcode_stage_choices=("revm-opcode-lab",),
        opcode_stage_default="revm-opcode-lab",
        opcode_elf_default=pathlib.Path(
            "crates/guests/elf/sp1_revm_opcode_lab.elf"
        ),
        opcode_elf_choices=(
            pathlib.Path("crates/guests/elf/sp1_revm_opcode_lab.elf"),
        ),
    )
    matched_run.set_defaults(
        func=cmd_run,
        expected_purpose=MATCHED_CONTROL_PURPOSE,
        opcode_stage="revm-opcode-lab",
    )

    relation_run = subcommands.add_parser(
        "run-relations",
        help="run every formal opcode relation lane three times",
    )
    _add_controlled_run_arguments(
        relation_run,
        opcode_stage_choices=("revm-opcode-lab",),
        opcode_stage_default="revm-opcode-lab",
        opcode_elf_default=pathlib.Path(
            "crates/guests/elf/sp1_revm_opcode_lab.elf"
        ),
        opcode_elf_choices=(
            pathlib.Path("crates/guests/elf/sp1_revm_opcode_lab.elf"),
        ),
    )
    relation_run.set_defaults(
        func=cmd_run,
        expected_purpose=FORMAL_RELATION_PURPOSE,
        repeats=3,
        opcode_stage="revm-opcode-lab",
    )

    run_controlled = subcommands.add_parser(
        "run-controlled",
        help="run every controlled SP1 workload three times",
    )
    _add_controlled_run_arguments(run_controlled)
    run_controlled.set_defaults(
        func=cmd_run_controlled,
        repeats=3,
        opcode_stage="revm-opcode-lab",
        expected_purpose=None,
    )

    run_block = subcommands.add_parser(
        "run-block-calibration",
        help="host-trace and run every frozen production-guest block row three times",
    )
    run_block.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    run_block.add_argument("--calibration-run", type=pathlib.Path, required=True)
    run_block.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    run_block.add_argument("--relations", type=pathlib.Path, required=True)
    run_block.add_argument("--out", type=pathlib.Path, required=True)
    run_block.set_defaults(func=cmd_run_block_calibration, repeats=3)

    fit_controlled = subcommands.add_parser(
        "fit-controlled-costs",
        help="apply the frozen repeat, isolation, fit, and checkpoint gates",
    )
    fit_controlled.add_argument("--runs", type=pathlib.Path, required=True)
    fit_controlled.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    fit_controlled.add_argument("--out", type=pathlib.Path, required=True)
    fit_controlled.set_defaults(func=cmd_fit_controlled_costs)

    fit_relations = subcommands.add_parser(
        "fit-relations",
        help="fit and seal the exact formal opcode relation artifact",
    )
    fit_relations.add_argument("--runs", type=pathlib.Path, required=True)
    fit_relations.add_argument("--calibration-run", type=pathlib.Path)
    fit_relations.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    fit_relations.add_argument("--out", type=pathlib.Path, required=True)
    fit_relations.set_defaults(func=cmd_fit_relations)

    fit_block = subcommands.add_parser(
        "fit-block-calibration",
        help="fit and seal opcode anchors plus fixed/base costs",
    )
    fit_block.add_argument("--relations", type=pathlib.Path, required=True)
    fit_block.add_argument("--runs", type=pathlib.Path, required=True)
    fit_block.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    fit_block.add_argument("--out", type=pathlib.Path, required=True)
    fit_block.set_defaults(func=cmd_fit_block_calibration)

    candidate = subcommands.add_parser(
        "build-candidate", help="seal the controlled SP1 proverGas candidate"
    )
    candidate.add_argument("--run", type=pathlib.Path, required=True)
    candidate.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    candidate.add_argument("--fit", type=pathlib.Path, required=True)
    candidate.add_argument("--overheads", type=pathlib.Path, required=True)
    candidate.add_argument("--provenance", type=pathlib.Path, required=True)
    candidate.set_defaults(func=cmd_build_candidate)

    bridge = subcommands.add_parser(
        "build-sp1-bridge", help="seal the SP1 proverGas/instruction-count bridge"
    )
    bridge.add_argument("--run", type=pathlib.Path, required=True)
    bridge.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    bridge.add_argument("--samples", type=pathlib.Path, required=True)
    bridge.set_defaults(func=cmd_build_sp1_bridge)

    run_proposal = subcommands.add_parser(
        "run-proposal",
        help="run one proposal GuestInput through guest-launcher and write normalized raw JSONL",
    )
    run_proposal.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    run_proposal.add_argument("--guest-input", type=pathlib.Path, required=True)
    run_proposal.add_argument("--proof-type", choices=["sp1", "risc0"], required=True)
    run_proposal.add_argument("--case", required=True)
    run_proposal.add_argument("--target-raw-gas", type=int, required=True)
    run_proposal.add_argument("--target-count", type=int, default=1)
    run_proposal.add_argument("--risc0-execution-po2", type=int, default=20)
    run_proposal.add_argument("--purpose", choices=["ad_hoc", "integration_smoke"], default="ad_hoc")
    run_proposal.add_argument("--smoke-record", type=pathlib.Path)
    run_proposal.add_argument("--network")
    run_proposal.add_argument("--proposal-id", type=int)
    run_proposal.add_argument("--out", type=pathlib.Path, required=True)
    run_proposal.set_defaults(func=cmd_run_proposal)

    fit = subcommands.add_parser("fit", help="fit marginal workload coefficients")
    fit.add_argument("--runs", type=pathlib.Path, required=True)
    fit.add_argument("--out", type=pathlib.Path, required=True)
    fit.add_argument(
        "--metric",
        default="prover_gas",
        help="raw-run metric field to fit, e.g. prover_gas or risc0_padded_cycles",
    )
    fit.set_defaults(func=cmd_fit)

    damage = subcommands.add_parser("damage", help="compute eth-limit zkgas damage statistics")
    damage.add_argument("--fit", type=pathlib.Path, required=True)
    damage.add_argument("--manifest", type=pathlib.Path, required=True)
    damage.add_argument("--eth-gas-limit", type=int, required=True)
    damage.add_argument("--zk-gas-limit", type=int, required=True)
    damage.add_argument("--out", type=pathlib.Path, required=True)
    damage.set_defaults(func=cmd_damage)

    inventory = subcommands.add_parser("inventory", help="report Unzen coverage inventory")
    inventory.add_argument("--manifest", type=pathlib.Path, required=True)
    inventory.add_argument("--out", type=pathlib.Path, required=True)
    inventory.set_defaults(func=cmd_inventory)

    prepare_corpus_parser = subcommands.add_parser(
        "prepare-corpus", help="acquire the preselected final-validation GuestInputs"
    )
    prepare_corpus_parser.add_argument("--corpus-root", type=pathlib.Path, required=True)
    prepare_corpus_parser.add_argument("--manifest", type=pathlib.Path)
    prepare_corpus_parser.add_argument("--l1-rpc", action="append", required=True, metavar="NETWORK=URL")
    prepare_corpus_parser.add_argument("--l2-rpc", action="append", required=True, metavar="NETWORK=URL")
    prepare_corpus_parser.add_argument("--chain-spec-hash", action="append", required=True, metavar="NETWORK=SHA256")
    prepare_corpus_parser.add_argument("--chain-spec-file", action="append", required=True, metavar="NETWORK=PATH")
    prepare_corpus_parser.set_defaults(func=cmd_prepare_corpus)

    smoke = subcommands.add_parser("prepare-integration-smoke", help="freeze one disjoint integration smoke row")
    smoke.add_argument("--network", required=True)
    smoke.add_argument("--proposal-id", type=int, required=True)
    smoke.add_argument("--guest-input", type=pathlib.Path, required=True)
    smoke.add_argument("--purpose", default="integration_smoke")
    smoke.add_argument("--out", type=pathlib.Path)
    smoke.set_defaults(func=cmd_prepare_integration_smoke)

    publish = subcommands.add_parser("publish-corpus", help="publish a sealed corpus archive")
    publish.add_argument("--archive", type=pathlib.Path, required=True)
    publish.add_argument("--object-uri", required=True)
    publish.add_argument("--manifest", type=pathlib.Path, required=True)
    publish.set_defaults(func=cmd_publish_corpus)

    calibration = subcommands.add_parser("prepare-calibration", help="freeze controlled calibration provenance")
    calibration.add_argument("--out", type=pathlib.Path, required=True)
    calibration.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    calibration.add_argument("--implementation-revision")
    calibration.add_argument("--run-path-file", type=pathlib.Path)
    calibration.set_defaults(func=cmd_prepare_calibration)

    validation = subcommands.add_parser("prepare-validation", help="bind candidate, bridge, and frozen corpus")
    validation.add_argument("--out", type=pathlib.Path, required=True)
    validation.add_argument("--run", type=pathlib.Path, required=True)
    validation.add_argument("--corpus", type=pathlib.Path, required=True)
    validation.set_defaults(func=cmd_prepare_validation)
    return parser


def cmd_run(args: argparse.Namespace) -> None:
    repeats = getattr(args, "repeats", 1)
    if repeats <= 0:
        raise ValueError("controlled run repeats must be positive")
    expected_purpose = getattr(args, "expected_purpose", None)
    if expected_purpose == FORMAL_RELATION_PURPOSE and repeats != 3:
        raise ValueError("formal relation run requires exactly three repeats")
    relation_or_diagnostic = expected_purpose in {
        MATCHED_CONTROL_PURPOSE,
        FORMAL_RELATION_PURPOSE,
    }
    if (
        relation_or_diagnostic
        and args.opcode_stage != "revm-opcode-lab"
    ):
        raise ValueError("matched-control run requires revm-opcode-lab")
    if relation_or_diagnostic and args.elf != pathlib.Path(
        "crates/guests/elf/sp1_revm_opcode_lab.elf"
    ):
        raise ValueError("matched-control run requires the frozen revm opcode-lab ELF")
    calibration_run = _resolve_repo_path(args.calibration_run, field_name="calibration_run")
    validate_calibration_execution_identity(calibration_run)
    fixtures = _resolve_repo_path(args.fixtures, field_name="fixtures")
    out = _resolve_repo_path(args.out, field_name="runs_output")
    _, identity = verify_frozen_controlled_manifest(
        calibration_run,
        _resolve_repo_path(args.controlled_manifest, field_name="controlled_manifest"),
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    cases_by_kind: dict[str, list[tuple[dict[str, Any], pathlib.Path]]] = {}
    loaded_cases = []
    loaded_guest_inputs = {}
    for case_path in sorted(fixtures.glob("**/case.json")):
        case = json.loads(case_path.read_text())
        validate_fixture_purpose(case, expected_purpose=expected_purpose)
        if (
            case.get("calibration_id") != calibration_run.name
            or case.get("controlled_manifest_sha256") != identity["controlled_manifest_sha256"]
            or case.get("controlled_manifest_rows_sha256")
            != identity["controlled_manifest_rows_sha256"]
        ):
            raise ValueError("controlled manifest provenance does not match calibration identity")
        input_path = case_path.with_name("guest-input.json")
        if input_path.exists():
            expected_input_sha256 = case.get("fixture_sha256")
            if (
                not _is_sha256(expected_input_sha256)
                or sha256_file(input_path) != expected_input_sha256
            ):
                raise ValueError("controlled GuestInput does not match sealed case provenance")
            cases_by_kind.setdefault(case.get("kind", "opcode"), []).append((case, input_path))
            loaded_cases.append(case)
            if relation_or_diagnostic:
                try:
                    guest_input = json.loads(input_path.read_text())
                except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                    raise ValueError("controlled GuestInput is not valid JSON") from exc
                if not isinstance(guest_input, Mapping):
                    raise ValueError("controlled GuestInput must be a JSON object")
                loaded_guest_inputs[expected_input_sha256] = guest_input
    matched_fixture_pairs = None
    if relation_or_diagnostic:
        matched_fixture_pairs = validate_matched_control_fixture_pairs(
            loaded_cases,
            expected_purpose=expected_purpose,
            calibration_execution_identity={
                "calibration_id": calibration_run.name,
                "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
                "implementation_revision": identity["implementation_revision"],
                "controlled_manifest_sha256": identity[
                    "controlled_manifest_sha256"
                ],
                "controlled_manifest_rows_sha256": identity[
                    "controlled_manifest_rows_sha256"
                ],
            },
            guest_inputs=loaded_guest_inputs,
        )
    report_paths = []
    for kind, cases in sorted(cases_by_kind.items()):
        stage = args.opcode_stage if kind == "opcode" else "precompile-lab"
        if kind == "opcode" and args.elf == pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"):
            elf_path = pathlib.Path(
                "crates/guests/elf/sp1_revm_opcode_lab.elf"
                if stage == "revm-opcode-lab"
                else "crates/guests/elf/sp1_opcode_lab.elf"
            )
        else:
            elf_path = args.elf if kind == "opcode" else args.precompile_elf
        report_path = out.with_name(f"{out.stem}.{stage}.jsonl")
        run_guest_inputs(
            guest_launcher=args.guest_launcher,
            elf_path=elf_path,
            input_paths=[
                input_path
                for _, input_path in cases
                for _repeat_index in range(repeats)
            ],
            reports_jsonl=report_path,
            stage=stage,
        )
        report_paths.append(report_path)
    case_by_input = {
        str(input_path): case for cases in cases_by_kind.values() for case, input_path in cases
    }
    def normalized_raw_runs() -> Iterable[dict[str, Any]]:
        repeat_index_by_input: dict[str, int] = {}
        for report_path in report_paths:
            for report in iter_jsonl(report_path):
                case = case_by_input[report["input"]]
                raw_run = raw_run_from_report(case, report)
                repeat_index = repeat_index_by_input.get(report["input"], 0)
                repeat_index_by_input[report["input"]] = repeat_index + 1
                raw_run["repeat_index"] = repeat_index
                if "workload_id" in raw_run and "backend_input_sha256" in raw_run:
                    raw_run["execution_row_id"] = controlled_execution_row_id(
                        raw_run["workload_id"],
                        backend="sp1",
                        execution_engine=raw_run["sp1_execution_engine"],
                        run_id=calibration_run.name,
                        repeat_index=repeat_index,
                        backend_input_sha256=raw_run["backend_input_sha256"],
                    )
                yield raw_run

    diagnostic_report = None
    if expected_purpose == MATCHED_CONTROL_PURPOSE:
        assert matched_fixture_pairs is not None
        raw_runs = list(normalized_raw_runs())
        diagnostic_report = build_matched_control_report(
            raw_runs,
            expected_pair_ids=set(matched_fixture_pairs),
            expected_repeats=repeats,
        )
        with out.open("w") as output:
            for raw_run in raw_runs:
                output.write(json.dumps(raw_run, sort_keys=True) + "\n")
        ran = len(raw_runs)
    elif expected_purpose == FORMAL_RELATION_PURPOSE:
        raw_runs = list(normalized_raw_runs())
        validate_formal_dynamic_raw_gas_preflight(raw_runs)
        with out.open("w") as output:
            for raw_run in raw_runs:
                output.write(json.dumps(raw_run, sort_keys=True) + "\n")
        ran = len(raw_runs)
    else:
        ran = 0
        with out.open("w") as output:
            for raw_run in normalized_raw_runs():
                output.write(json.dumps(raw_run, sort_keys=True) + "\n")
                ran += 1
    if diagnostic_report is not None:
        diagnostic_path = out.with_name(f"{out.stem}.matched-control.json")
        diagnostic_path.write_text(
            json.dumps(diagnostic_report, indent=2, sort_keys=True) + "\n"
        )
        print(f"wrote matched-control report to {diagnostic_path}")
    print(f"ran {ran} executable case(s)")


def cmd_run_proposal(args: argparse.Namespace) -> None:
    if args.purpose == "integration_smoke":
        if args.smoke_record is None or args.network is None or args.proposal_id is None:
            raise ValueError(
                "run-proposal integration_smoke requires --smoke-record --network --proposal-id"
            )
        verify_prepared_integration_smoke(
            _resolve_repo_path(args.smoke_record, field_name="smoke_record"),
            network=args.network,
            proposal_id=args.proposal_id,
            guest_input=args.guest_input,
        )
    run_proposal_guest_input(
        guest_launcher=args.guest_launcher,
        guest_input=args.guest_input,
        proof_type=args.proof_type,
        case_name=args.case,
        target_raw_gas=args.target_raw_gas,
        target_count=args.target_count,
        out=args.out,
        risc0_execution_po2=args.risc0_execution_po2,
    )
    print(f"wrote proposal raw run to {args.out}")


def main() -> None:
    parser = build_parser()
    args = parser.parse_args()
    with localcontext(_OPCODE_DECIMAL_CONTEXT):
        args.func(args)


if __name__ == "__main__":
    main()
