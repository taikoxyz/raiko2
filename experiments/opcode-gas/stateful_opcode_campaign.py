#!/usr/bin/env python3
"""Canonical fixture contract for the stateful SLOAD/SSTORE calibration."""

from __future__ import annotations

import functools
import json
import pathlib
import re
import tempfile
from dataclasses import dataclass
from decimal import Context, Decimal, InvalidOperation, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from types import MappingProxyType
from typing import Any, Mapping, Sequence

import opcode_gas
from composite_estimator import load_registry_payload
from hierarchical_model import ModelKind, OpcodeEvent, OpcodeRegistry, predict_opcode_event

from opcode_gas import (
    canonical_json,
    decode_fixed_microprograms,
    encode_fixed_microprograms,
    sha256_bytes,
)


SCHEMA_VERSION = 1
PURPOSE = "stateful_opcode_calibration"
FIT_COUNTS = (1, 2, 4, 8, 16)
HOLDOUT_COUNT = 32
CHECKPOINT_COUNT = 64
STRUCTURAL_ZERO_COUNT = 0
REPEATS = 3
STATEFUL_EXECUTION_PAIR_CHUNK_SIZE = 8
PRIMARY_COUNTS = (0, *FIT_COUNTS, HOLDOUT_COUNT, CHECKPOINT_COUNT)
DIAGNOSTIC_COUNTS = (0, CHECKPOINT_COUNT)
GENERATOR_MAX_COUNT = CHECKPOINT_COUNT
STATEFUL_DECIMAL_CONTEXT = Context(prec=80, rounding=ROUND_HALF_EVEN, traps=[])
STATEFUL_REPLAY_RESIDUAL_MAX = Fraction(1, 10**75)
CONTROLLED_RUN_NOISE_FLOOR = MappingProxyType({
    "prover_gas": 143,
    "provenance_contract_id": "sp1-controlled-block-fixed-envelope-cross-input-v1",
    "scope": "signal_gate_only",
})
REFERENCE_REGISTRY = {
    "path": "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json",
    "artifact_sha256": "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b",
}
STATEFUL_RESULT_SCHEMA_VERSION = 1
STATEFUL_RESULT_PURPOSE = "stateful_opcode_calibration_result"
STATEFUL_RESULT_INVENTORY = frozenset(
    {
        "result.json",
        "campaign-manifest.json",
        "calibration-identity.json",
        "campaign-identity.json",
        "rows.jsonl",
        "campaign-decisions.json",
        "campaign-decisions.sha256",
        "source-registry.json",
        "model-report.json",
    }
)
STATEFUL_RESULT_OWNERSHIP = MappingProxyType(
    {
        "measured": (
            "stateful REVM execution cost including storage execution, journal "
            "updates, and result-state construction"
        ),
        "excluded": (
            "witness materialization",
            "persistent dirty-state commit",
            "trie hashing",
            "final state root",
        ),
    }
)
STATEFUL_RESULT_FIT_SOURCE_PATHS = (
    "Cargo.toml",
    "Cargo.lock",
    "experiments/opcode-gas/stateful_opcode_campaign.py",
    "experiments/opcode-gas/opcode_gas.py",
    "experiments/opcode-gas/calibration_model.py",
    "experiments/opcode-gas/composite_estimator.py",
    "experiments/opcode-gas/hierarchical_model.py",
    "experiments/opcode-gas/manifests/sp1-calibration-v1.toml",
    "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json",
)
STATEFUL_RESULT_DESCENDANT_PATHS = (
    "experiments/opcode-gas/README.md",
    "docs/plans/2026-09-26-zkgas-calibration-progress.md",
)
EXECUTION_CONTRACT = {
    "elf_path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
    "evm_spec": "osaka",
    "mode": "execute",
    "proof_type": "sp1",
    "sp1_execution_engine": "gas-estimator",
    "stage": "revm-opcode-lab",
}

SLOAD = 0x54
SSTORE = 0x55
NOT = 0x19
POP = 0x50
JUMPDEST = 0x5B
PUSH32 = 0x7F
STOP = 0x00

ZERO = 0
ONE = 1
TWO = 2
HIGH = 1 << 255
HIGH2 = HIGH + 1
HIGH_SLOT = HIGH

_U256_RE = re.compile(r"0x[0-9a-f]{64}\Z")


def u256_hex(value: int) -> str:
    if type(value) is not int or not 0 <= value < 1 << 256:
        raise ValueError("value is outside U256")
    return f"0x{value:064x}"


def parse_u256(value: Any) -> int:
    if not isinstance(value, str) or _U256_RE.fullmatch(value) is None:
        raise ValueError("expected canonical U256 as 0x-prefixed 64-digit lowercase hex")
    return int(value[2:], 16)


def _load(name: str, original: int, access: str, **extra: Any) -> dict[str, Any]:
    return {
        "name": name,
        "measurement_opcode": SLOAD,
        "reference_opcode": NOT,
        "slot": u256_hex(extra.pop("slot", ZERO)),
        "original_value": u256_hex(original),
        "access": access,
        "operation": {"kind": "load", "expected_value": u256_hex(original)},
        "target_raw_gas": 2_100 if access == "cold" else 100,
        "diagnostic": bool(extra.pop("diagnostic", False)),
        "low_variant": extra.pop("low_variant", None),
        **extra,
    }


def _store(
    name: str,
    original: int,
    current: int,
    new: int,
    access: str,
    **extra: Any,
) -> dict[str, Any]:
    if current != original:
        raw_gas = 100
    elif new == current:
        raw_gas = 2_200 if access == "cold" else 100
    elif current == 0:
        raw_gas = 22_100 if access == "cold" else 20_000
    else:
        raw_gas = 5_000 if access == "cold" else 2_900
    return {
        "name": name,
        "measurement_opcode": SSTORE,
        "reference_opcode": POP,
        "slot": u256_hex(extra.pop("slot", ZERO)),
        "original_value": u256_hex(original),
        "access": access,
        "operation": {
            "kind": "store",
            "current_value": u256_hex(current),
            "new_value": u256_hex(new),
        },
        "target_raw_gas": raw_gas,
        "diagnostic": bool(extra.pop("diagnostic", False)),
        "low_variant": extra.pop("low_variant", None),
        **extra,
    }


def _canonical_scenarios() -> list[dict[str, Any]]:
    rows = [
        _load("sload_cold_zero", ZERO, "cold"),
        _load("sload_cold_nonzero", ONE, "cold"),
        _load("sload_warm_zero", ZERO, "warm"),
        _load("sload_warm_nonzero", ONE, "warm"),
        _store("sstore_noop_zero_cold", ZERO, ZERO, ZERO, "cold"),
        _store("sstore_noop_zero_warm", ZERO, ZERO, ZERO, "warm"),
        _store("sstore_noop_nonzero_cold", ONE, ONE, ONE, "cold"),
        _store("sstore_noop_nonzero_warm", ONE, ONE, ONE, "warm"),
        _store("sstore_set_cold", ZERO, ZERO, ONE, "cold"),
        _store("sstore_set_warm", ZERO, ZERO, ONE, "warm"),
        _store("sstore_clear_cold", ONE, ONE, ZERO, "cold"),
        _store("sstore_clear_warm", ONE, ONE, ZERO, "warm"),
        _store("sstore_reset_nonzero_cold", ONE, ONE, TWO, "cold"),
        _store("sstore_reset_nonzero_warm", ONE, ONE, TWO, "warm"),
        _store("sstore_dirty_rewrite", ZERO, ONE, TWO, "warm"),
        _store("sstore_restore_zero", ZERO, ONE, ZERO, "warm"),
        _store("sstore_dirty_rewrite_nonzero", ONE, TWO, ZERO, "warm"),
        _store("sstore_restore_nonzero", ONE, TWO, ONE, "warm"),
        _load(
            "sload_cold_high_value",
            HIGH,
            "cold",
            diagnostic=True,
            low_variant="sload_cold_nonzero",
        ),
        _load(
            "sload_warm_high_value",
            HIGH,
            "warm",
            diagnostic=True,
            low_variant="sload_warm_nonzero",
        ),
    ]
    rows.extend(
        [
            _store(
                f"sstore_noop_high_value_{access}",
                HIGH,
                HIGH,
                HIGH,
                access,
                diagnostic=True,
                low_variant=f"sstore_noop_nonzero_{access}",
            )
            for access in ("cold", "warm")
        ]
    )
    rows.extend(
        [
            _store(
                f"sstore_set_high_value_{access}",
                ZERO,
                ZERO,
                HIGH,
                access,
                diagnostic=True,
                low_variant=f"sstore_set_{access}",
            )
            for access in ("cold", "warm")
        ]
    )
    rows.extend(
        [
            _store(
                f"sstore_clear_high_value_{access}",
                HIGH,
                HIGH,
                ZERO,
                access,
                diagnostic=True,
                low_variant=f"sstore_clear_{access}",
            )
            for access in ("cold", "warm")
        ]
    )
    rows.extend(
        [
            _store(
                f"sstore_reset_high_value_{access}",
                HIGH,
                HIGH,
                HIGH2,
                access,
                diagnostic=True,
                low_variant=f"sstore_reset_nonzero_{access}",
            )
            for access in ("cold", "warm")
        ]
    )
    rows.extend(
        [
            _store(
                "sstore_dirty_rewrite_high_value",
                ZERO,
                HIGH,
                HIGH2,
                "warm",
                diagnostic=True,
                low_variant="sstore_dirty_rewrite",
            ),
            _store(
                "sstore_restore_high_value",
                HIGH,
                HIGH2,
                HIGH,
                "warm",
                diagnostic=True,
                low_variant="sstore_restore_nonzero",
            ),
        ]
    )
    for access in ("cold", "warm"):
        rows.append(
            _load(
                f"sload_{access}_nonzero_high_slot",
                ONE,
                access,
                slot=HIGH_SLOT,
                diagnostic=True,
                low_variant=f"sload_{access}_nonzero",
            )
        )
    high_slot_stores = (
        ("sstore_noop_nonzero", ONE, ONE, ONE),
        ("sstore_set", ZERO, ZERO, ONE),
        ("sstore_clear", ONE, ONE, ZERO),
        ("sstore_reset_nonzero", ONE, ONE, TWO),
    )
    for base_name, original, current, new in high_slot_stores:
        for access in ("cold", "warm"):
            rows.append(
                _store(
                    f"{base_name}_{access}_high_slot",
                    original,
                    current,
                    new,
                    access,
                    slot=HIGH_SLOT,
                    diagnostic=True,
                    low_variant=f"{base_name}_{access}",
                )
            )
    return rows


def canonical_stateful_manifest_payload() -> dict[str, Any]:
    return {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "fit_counts": list(FIT_COUNTS),
        "holdout_count": HOLDOUT_COUNT,
        "checkpoint_count": CHECKPOINT_COUNT,
        "controlled_run_noise_floor": dict(CONTROLLED_RUN_NOISE_FLOOR),
        "structural_zero_count": STRUCTURAL_ZERO_COUNT,
        "repeats": REPEATS,
        "execution": dict(EXECUTION_CONTRACT),
        "reference_registry": dict(REFERENCE_REGISTRY),
        "scenarios": _canonical_scenarios(),
    }


@dataclass(frozen=True)
class StatefulScenario:
    name: str
    measurement_opcode: int
    reference_opcode: int
    slot: int
    original_value: int
    access: str
    operation_kind: str
    expected_value: int | None
    current_value: int | None
    new_value: int | None
    target_raw_gas: int
    diagnostic: bool
    low_variant: str | None

    @property
    def dirty(self) -> bool:
        return self.operation_kind == "store" and self.current_value != self.original_value

    @property
    def counts(self) -> tuple[int, ...]:
        return DIAGNOSTIC_COUNTS if self.diagnostic else PRIMARY_COUNTS

    def storage_payload(self, lane: str) -> dict[str, Any]:
        operation: dict[str, Any]
        if self.operation_kind == "load":
            operation = {
                "kind": "load",
                "expected_value": u256_hex(self.expected_value),
            }
        else:
            operation = {
                "kind": "store",
                "current_value": u256_hex(self.current_value),
                "new_value": u256_hex(self.new_value),
            }
        return {
            "measurement_opcode": self.measurement_opcode,
            "lane": lane,
            "slot": u256_hex(self.slot),
            "original_value": u256_hex(self.original_value),
            "access": self.access,
            "operation": operation,
        }


@dataclass(frozen=True)
class StatefulCampaignManifest:
    schema_version: int
    purpose: str
    fit_counts: tuple[int, ...]
    holdout_count: int
    checkpoint_count: int
    structural_zero_count: int
    repeats: int
    controlled_run_noise_floor: Mapping[str, Any]
    execution: Mapping[str, str]
    reference_registry: Mapping[str, str]
    scenarios: tuple[StatefulScenario, ...]

    @property
    def primary_counts(self) -> tuple[int, ...]:
        return (self.structural_zero_count, *self.fit_counts, self.holdout_count, self.checkpoint_count)

    @property
    def diagnostic_counts(self) -> tuple[int, ...]:
        return (self.structural_zero_count, self.checkpoint_count)

    def scenario(self, name: str) -> StatefulScenario:
        for scenario in self.scenarios:
            if scenario.name == name:
                return scenario
        raise ValueError(f"undeclared stateful scenario: {name}")

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "StatefulCampaignManifest":
        expected = canonical_stateful_manifest_payload()
        if not isinstance(value, Mapping):
            raise ValueError("stateful campaign manifest must be an object")
        rows = value.get("scenarios")
        if not isinstance(rows, list):
            raise ValueError("stateful campaign scenarios must be a list")
        expected_names = [row["name"] for row in expected["scenarios"]]
        names = [row.get("name") if isinstance(row, Mapping) else None for row in rows]
        if names != expected_names:
            raise ValueError("stateful scenario inventory differs from the frozen contract")

        scenarios = []
        for row in rows:
            if not isinstance(row, Mapping):
                raise ValueError("stateful scenario must be an object")
            slot = parse_u256(row.get("slot"))
            original = parse_u256(row.get("original_value"))
            operation = row.get("operation")
            if not isinstance(operation, Mapping):
                raise ValueError("stateful scenario operation must be an object")
            kind = operation.get("kind")
            expected_value = current_value = new_value = None
            if kind == "load":
                if set(operation) != {"kind", "expected_value"}:
                    raise ValueError("stateful load operation fields differ")
                expected_value = parse_u256(operation.get("expected_value"))
            elif kind == "store":
                if set(operation) != {"kind", "current_value", "new_value"}:
                    raise ValueError("stateful store operation fields differ")
                current_value = parse_u256(operation.get("current_value"))
                new_value = parse_u256(operation.get("new_value"))
            else:
                raise ValueError("stateful operation kind differs")
            scenarios.append(
                StatefulScenario(
                    name=row["name"],
                    measurement_opcode=row.get("measurement_opcode"),
                    reference_opcode=row.get("reference_opcode"),
                    slot=slot,
                    original_value=original,
                    access=row.get("access"),
                    operation_kind=kind,
                    expected_value=expected_value,
                    current_value=current_value,
                    new_value=new_value,
                    target_raw_gas=row.get("target_raw_gas"),
                    diagnostic=row.get("diagnostic"),
                    low_variant=row.get("low_variant"),
                )
            )

        manifest = cls(
            schema_version=SCHEMA_VERSION,
            purpose=PURPOSE,
            fit_counts=FIT_COUNTS,
            holdout_count=HOLDOUT_COUNT,
            checkpoint_count=CHECKPOINT_COUNT,
            structural_zero_count=STRUCTURAL_ZERO_COUNT,
            repeats=REPEATS,
            controlled_run_noise_floor=MappingProxyType(
                dict(CONTROLLED_RUN_NOISE_FLOOR)
            ),
            execution=MappingProxyType(dict(EXECUTION_CONTRACT)),
            reference_registry=MappingProxyType(dict(REFERENCE_REGISTRY)),
            scenarios=tuple(scenarios),
        )
        validate_stateful_manifest_program_shapes(manifest)
        if json.loads(canonical_json(value)) != expected:
            raise ValueError("stateful campaign manifest differs from the frozen contract")
        return manifest


@dataclass(frozen=True)
class StatefulCampaignRowSpec:
    scenario: str
    lane: str
    relation_count: int
    repeat_index: int
    logical_identity: str


def stateful_campaign_row_specs(
    manifest: StatefulCampaignManifest,
) -> tuple[StatefulCampaignRowSpec, ...]:
    rows = []
    for scenario in manifest.scenarios:
        for count in scenario.counts:
            for lane in ("control", "target"):
                for repeat_index in range(manifest.repeats):
                    identity = sha256_bytes(
                        canonical_json(
                            {
                                "kind": "stateful_campaign_row_spec",
                                "scenario": scenario.name,
                                "lane": lane,
                                "relation_count": count,
                                "repeat_index": repeat_index,
                            }
                        )
                    )
                    rows.append(
                        StatefulCampaignRowSpec(
                            scenario=scenario.name,
                            lane=lane,
                            relation_count=count,
                            repeat_index=repeat_index,
                            logical_identity=identity,
                        )
                    )
    return tuple(rows)


def load_stateful_campaign_manifest(path: pathlib.Path) -> StatefulCampaignManifest:
    try:
        payload = json.loads(path.read_bytes())
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError("stateful campaign manifest is not valid JSON") from error
    if not isinstance(payload, Mapping):
        raise ValueError("stateful campaign manifest must be a JSON object")
    return StatefulCampaignManifest.from_mapping(payload)


@dataclass(frozen=True)
class OperandImmediateSpan:
    role: str
    start: int
    end: int

    def as_payload(self) -> dict[str, Any]:
        return {"role": self.role, "start": self.start, "end": self.end}


@dataclass(frozen=True)
class StatefulProgram:
    lane: str
    measured: bool
    bytecode: bytes
    operand_immediate_spans: tuple[OperandImmediateSpan, ...]
    program_shape_sha256: str
    opcode_counts: Mapping[int, int]


def _push32(program: bytearray, spans: list[OperandImmediateSpan], role: str, value: int) -> None:
    program.append(PUSH32)
    start = len(program)
    program.extend(value.to_bytes(32, "big"))
    spans.append(OperandImmediateSpan(role=role, start=start, end=start + 32))


def _mask_immediates(bytecode: bytes, spans: Sequence[OperandImmediateSpan]) -> bytes:
    masked = bytearray(bytecode)
    previous_end = 0
    for span in spans:
        if (
            span.start <= previous_end
            or span.end - span.start != 32
            or span.start < 1
            or span.end > len(masked)
        ):
            raise ValueError("operand spans are invalid or out of order")
        if masked[span.start - 1] != PUSH32:
            raise ValueError("operand span does not follow a PUSH32 opcode")
        masked[span.start : span.end] = bytes(32)
        previous_end = span.end
    return bytes(masked)


def _program_opcode_counts(bytecode: bytes) -> Mapping[int, int]:
    counts: dict[int, int] = {}
    cursor = 0
    while cursor < len(bytecode):
        opcode = bytecode[cursor]
        counts[opcode] = counts.get(opcode, 0) + 1
        cursor += 1
        if 0x60 <= opcode <= 0x7F:
            cursor += opcode - 0x5F
            if cursor > len(bytecode):
                raise ValueError("truncated PUSH immediate in stateful program")
    return MappingProxyType(dict(sorted(counts.items())))


def build_stateful_program(
    scenario: StatefulScenario, *, lane: str, measured: bool
) -> StatefulProgram:
    if lane not in {"target", "control"}:
        raise ValueError("stateful lane must be target or control")
    program = bytearray()
    spans: list[OperandImmediateSpan] = []

    if scenario.dirty:
        _push32(program, spans, "prefix_current_value", scenario.current_value)
        _push32(program, spans, "prefix_slot", scenario.slot)
        program.append(SSTORE)

    target_site = lane == "target" and measured
    if scenario.operation_kind == "load":
        _push32(program, spans, "slot", scenario.slot)
        program.append(SLOAD if target_site else scenario.reference_opcode)
        program.extend((POP, STOP))
    else:
        _push32(program, spans, "new_value", scenario.new_value)
        _push32(program, spans, "slot", scenario.slot)
        if target_site:
            program.extend((SSTORE, JUMPDEST, STOP))
        else:
            program.extend((POP, POP, STOP))

    bytecode = bytes(program)
    shape_hash = sha256_bytes(_mask_immediates(bytecode, spans))
    return StatefulProgram(
        lane=lane,
        measured=measured,
        bytecode=bytecode,
        operand_immediate_spans=tuple(spans),
        program_shape_sha256=shape_hash,
        opcode_counts=_program_opcode_counts(bytecode),
    )


def validate_program_shape_pair(low: StatefulProgram, high: StatefulProgram) -> None:
    low_shape = tuple((span.role, span.start, span.end) for span in low.operand_immediate_spans)
    high_shape = tuple((span.role, span.start, span.end) for span in high.operand_immediate_spans)
    if low_shape != high_shape:
        raise ValueError("low/high operand spans differ")
    low_masked = _mask_immediates(low.bytecode, low.operand_immediate_spans)
    high_masked = _mask_immediates(high.bytecode, high.operand_immediate_spans)
    if low_masked != high_masked:
        raise ValueError("low/high program shape differs outside operand spans")
    if dict(low.opcode_counts) != dict(high.opcode_counts):
        raise ValueError("low/high reference execution ledger differs")
    low_hash = sha256_bytes(low_masked)
    high_hash = sha256_bytes(high_masked)
    if (
        low.program_shape_sha256 != low_hash
        or high.program_shape_sha256 != high_hash
        or low_hash != high_hash
    ):
        raise ValueError("low/high program shape hash differs")


def _reference_ledger(scenario: StatefulScenario) -> dict[str, int]:
    target = build_stateful_program(scenario, lane="target", measured=True)
    control = build_stateful_program(scenario, lane="control", measured=True)
    opcodes = sorted(set(target.opcode_counts) | set(control.opcode_counts))
    ledger = {
        f"opcode:0x{opcode:02x}": target.opcode_counts.get(opcode, 0)
        - control.opcode_counts.get(opcode, 0)
        for opcode in opcodes
    }
    ledger = {key: value for key, value in ledger.items() if value}
    if ledger.get(f"opcode:0x{scenario.measurement_opcode:02x}") != 1:
        raise ValueError("stateful reference ledger does not contain one measurement opcode")
    if sum(ledger.values()) != 0:
        raise ValueError("stateful reference execution ledger is not zero sum")
    return ledger


def validate_stateful_manifest_program_shapes(
    manifest: StatefulCampaignManifest,
) -> None:
    for high in manifest.scenarios:
        if not high.diagnostic:
            continue
        if high.low_variant is None:
            raise ValueError("diagnostic scenario is missing its low variant")
        low = manifest.scenario(high.low_variant)
        for lane in ("target", "control"):
            for measured in (False, True):
                validate_program_shape_pair(
                    build_stateful_program(low, lane=lane, measured=measured),
                    build_stateful_program(high, lane=lane, measured=measured),
                )
        if _reference_ledger(low) != _reference_ledger(high):
            raise ValueError("low/high reference execution ledger differs")


def _program_contract(program: StatefulProgram) -> dict[str, Any]:
    return {
        "lane": program.lane,
        "measured": program.measured,
        "operand_immediate_spans": [
            span.as_payload() for span in program.operand_immediate_spans
        ],
        "program_shape_sha256": program.program_shape_sha256,
        "opcode_counts": {
            f"0x{opcode:02x}": count for opcode, count in program.opcode_counts.items()
        },
    }


def stateful_case_id(scenario: StatefulScenario, *, lane: str, count: int) -> str:
    return sha256_bytes(
        canonical_json(
            {
                "schema_version": SCHEMA_VERSION,
                "purpose": PURPOSE,
                "scenario": scenario.name,
                "lane": lane,
                "count": count,
                "storage": scenario.storage_payload(lane),
            }
        )
    )


def stateful_pair_id(scenario: StatefulScenario, *, count: int) -> str:
    return sha256_bytes(
        canonical_json(
            {
                "schema_version": SCHEMA_VERSION,
                "purpose": PURPOSE,
                "scenario": scenario.name,
                "relation_count": count,
                "storage": {
                    key: value
                    for key, value in scenario.storage_payload("target").items()
                    if key != "lane"
                },
            }
        )
    )


def generate_stateful_fixture(
    manifest: StatefulCampaignManifest,
    scenario_name: str,
    *,
    lane: str,
    count: int,
) -> dict[str, dict[str, Any]]:
    scenario = manifest.scenario(scenario_name)
    if lane not in {"target", "control"}:
        raise ValueError("stateful lane must be target or control")
    if type(count) is not int or count not in scenario.counts:
        raise ValueError("stateful count differs from the frozen scenario sweep")

    active = build_stateful_program(scenario, lane=lane, measured=True)
    inactive = build_stateful_program(scenario, lane=lane, measured=False)
    programs = [
        active.bytecode if lane == "target" and index < count else inactive.bytecode
        for index in range(GENERATOR_MAX_COUNT)
    ]
    if len({len(program) for program in programs}) != 1:
        raise AssertionError("stateful target/reference programs must have equal length")
    encoded = encode_fixed_microprograms(programs)
    storage = scenario.storage_payload(lane)
    case_id = stateful_case_id(scenario, lane=lane, count=count)
    pair_id = stateful_pair_id(scenario, count=count)
    opcode = scenario.measurement_opcode if lane == "target" else scenario.reference_opcode
    if lane == "target":
        concrete_count = count
        concrete_raw_gas = scenario.target_raw_gas
    elif scenario.measurement_opcode == SLOAD:
        concrete_count = GENERATOR_MAX_COUNT
        concrete_raw_gas = 3
    else:
        concrete_count = GENERATOR_MAX_COUNT * 2
        concrete_raw_gas = 2
    tx_gas_limit = max(100_000, 1_000_000 + GENERATOR_MAX_COUNT * scenario.target_raw_gas)
    guest_input = {
        "case": case_id,
        "scenario": scenario.name,
        "opcode": opcode,
        "target_count": concrete_count,
        "target_raw_gas": concrete_raw_gas,
        "tx_gas_limit": tx_gas_limit,
        "bytecode": "0x" + encoded.hex(),
        "generator_max_count": GENERATOR_MAX_COUNT,
        "fixed_bytecode_len": len(encoded),
        "storage": storage,
    }
    case_record = {
        "schema_version": SCHEMA_VERSION,
        "purpose": PURPOSE,
        "kind": "opcode",
        "case_id": case_id,
        "pair_id": pair_id,
        "case": case_id,
        "scenario": scenario.name,
        "lane": lane,
        "relation_count": count,
        "opcode": opcode,
        "target_count": concrete_count,
        "target_raw_gas": concrete_raw_gas,
        "tx_gas_limit": tx_gas_limit,
        "bytecode": guest_input["bytecode"],
        "generator_max_count": GENERATOR_MAX_COUNT,
        "fixed_bytecode_len": len(encoded),
        "repeats": manifest.repeats,
        "diagnostic": scenario.diagnostic,
        "low_variant": scenario.low_variant,
        "storage": storage,
        "active_program": _program_contract(active),
        "inactive_program": _program_contract(inactive),
        "reference_ledger": _reference_ledger(scenario),
        "guest_input_json_payload_sha256": sha256_bytes(canonical_json(guest_input)),
        "guest_input_json_file_sha256": sha256_bytes(canonical_json(guest_input) + b"\n"),
    }
    return {"case_record": case_record, "guest_input": guest_input}


def generate_stateful_fixtures(
    manifest: StatefulCampaignManifest, output: pathlib.Path
) -> list[pathlib.Path]:
    written = []
    for scenario in manifest.scenarios:
        for count in scenario.counts:
            for lane in ("control", "target"):
                fixture = generate_stateful_fixture(
                    manifest, scenario.name, lane=lane, count=count
                )
                directory = output / scenario.name / str(count) / lane
                directory.mkdir(parents=True, exist_ok=True)
                case_path = directory / "case.json"
                input_path = directory / "guest-input.json"
                case_path.write_bytes(canonical_json(fixture["case_record"]) + b"\n")
                input_path.write_bytes(canonical_json(fixture["guest_input"]) + b"\n")
                written.append(case_path)
    return written


def _require_sha256(value: Any, field: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise ValueError(f"{field} must be a lowercase SHA256")
    return value


def _expected_executed_opcode_counts(case_record: Mapping[str, Any]) -> dict[str, int]:
    active_slots = (
        case_record["relation_count"] if case_record["lane"] == "target" else 0
    )
    counts: dict[str, int] = {}
    for index in range(case_record["generator_max_count"]):
        program = (
            case_record["active_program"]
            if index < active_slots
            else case_record["inactive_program"]
        )
        for opcode, count in program["opcode_counts"].items():
            key = f"opcode:{opcode}"
            counts[key] = counts.get(key, 0) + count
    return dict(sorted(counts.items()))


def _validate_semantic_check(
    case_record: Mapping[str, Any],
    semantic: Any,
    backend_input_sha256: str,
) -> str:
    if not isinstance(semantic, Mapping):
        raise ValueError("stateful semantic check is missing")
    if semantic.get("schema_version") != 1 or semantic.get("passed") is not True:
        raise ValueError("stateful semantic check did not pass")
    if semantic.get("backend_input_sha256") != backend_input_sha256:
        raise ValueError("semantic backend-input identity differs from trace")
    programs = semantic.get("programs")
    max_count = case_record["generator_max_count"]
    if semantic.get("checked_programs") != max_count or not isinstance(programs, list):
        raise ValueError("semantic program inventory differs from frozen layout")
    if len(programs) != max_count:
        raise ValueError("semantic program inventory differs from frozen layout")
    storage = case_record["storage"]
    operation = storage["operation"]
    target_lane = case_record["lane"] == "target"
    relation_count = case_record["relation_count"]
    dirty = (
        operation["kind"] == "store"
        and operation["current_value"] != storage["original_value"]
    )
    for index, program in enumerate(programs):
        if not isinstance(program, Mapping) or program.get("program_index") != index:
            raise ValueError("semantic program order differs from frozen layout")
        if program.get("result_status") != "success":
            raise ValueError("semantic program result is not successful")
        active = target_lane and index < relation_count
        canonical_loads = (
            [operation["expected_value"]]
            if operation["kind"] == "load" and active
            else []
        )
        if (
            program.get("expected_load_values") != canonical_loads
            or program.get("observed_load_values") != canonical_loads
        ):
            raise ValueError("semantic SLOAD expected/observed values differ")
        canonical_final_storage = None
        if operation["kind"] == "store":
            canonical_final_storage = (
                operation["new_value"]
                if active
                else operation["current_value"]
                if dirty
                else storage["original_value"]
            )
        if (
            program.get("expected_final_storage") != canonical_final_storage
            or program.get("observed_final_storage") != canonical_final_storage
        ):
            raise ValueError("semantic SSTORE expected/observed state differs")
    return sha256_bytes(canonical_json(semantic))


def _admit_expected_identity_bundle(
    fixture: Mapping[str, Any], expected_bundle: Mapping[str, Any]
) -> dict[str, Any]:
    guest_input = fixture.get("guest_input")
    if not isinstance(guest_input, Mapping):
        raise ValueError("stateful fixture is missing its guest input")
    if not isinstance(expected_bundle, Mapping) or set(expected_bundle) != {
        "schema_version",
        "expected_public_values",
        "identity",
        "report",
    }:
        raise ValueError("canonical Rust identity bundle has an invalid shape")
    bundle_schema = expected_bundle.get("schema_version")
    if bundle_schema not in (1, 2):
        raise ValueError("canonical Rust identity bundle schema differs")
    expected_public_values = expected_bundle.get("expected_public_values")
    if (
        not isinstance(expected_public_values, str)
        or re.fullmatch(r"0x[0-9a-f]{64}", expected_public_values) is None
    ):
        raise ValueError("canonical Rust guest output is invalid")
    expected_identity = expected_bundle.get("identity")
    identity_fields = {
        "schema_version",
        "input",
        "backend_input_sha256",
        "backend_input_len",
        "workload_id",
        "transaction_envelope_sha256",
        "access_list_sha256",
        "prestate_sha256",
    }
    if bundle_schema == 2:
        identity_fields.add("block_environment_sha256")
    if not isinstance(expected_identity, Mapping) or set(expected_identity) != identity_fields:
        raise ValueError("canonical Rust identity evidence has an invalid shape")
    if expected_identity.get("schema_version") != bundle_schema:
        raise ValueError("canonical Rust identity evidence schema differs")
    if expected_identity.get("input") != guest_input:
        raise ValueError("canonical Rust identity input differs from generated guest input")
    expected_backend_sha256 = _require_sha256(
        expected_identity.get("backend_input_sha256"),
        "canonical Rust backend_input_sha256",
    )
    expected_backend_len = expected_identity.get("backend_input_len")
    if type(expected_backend_len) is not int or expected_backend_len <= 0:
        raise ValueError("canonical Rust backend-input length is invalid")
    for field in (
        "workload_id",
        "transaction_envelope_sha256",
        "access_list_sha256",
        "prestate_sha256",
    ):
        _require_sha256(expected_identity.get(field), f"canonical Rust {field}")
    if bundle_schema == 2:
        _require_sha256(
            expected_identity.get("block_environment_sha256"),
            "canonical Rust block_environment_sha256",
        )
    identity_evidence_sha256 = sha256_bytes(canonical_json(expected_identity))

    expected_native_report = expected_bundle.get("report")
    if not isinstance(expected_native_report, Mapping) or set(expected_native_report) != {
        "guest_input_sha256",
        "guest_input_bincode_length",
        "controlled_trace",
    }:
        raise ValueError("canonical Rust native report has an invalid shape")
    expected_native_trace = expected_native_report.get("controlled_trace")
    expected_native_semantic = (
        expected_native_trace.get("semantic_check")
        if isinstance(expected_native_trace, Mapping)
        else None
    )
    if (
        expected_native_report.get("guest_input_sha256")
        != f"0x{expected_backend_sha256}"
        or expected_native_report.get("guest_input_bincode_length")
        != expected_backend_len
        or not isinstance(expected_native_trace, Mapping)
        or expected_native_trace.get("kind") != "revm_opcode"
        or expected_native_trace.get("backend_input_sha256")
        != expected_backend_sha256
        or expected_native_trace.get("backend_input_len") != expected_backend_len
        or not isinstance(expected_native_semantic, Mapping)
        or expected_native_semantic.get("backend_input_sha256")
        != expected_backend_sha256
    ):
        raise ValueError("canonical Rust native report differs from its identity evidence")
    return {
        "backend_input_sha256": expected_backend_sha256,
        "backend_input_len": expected_backend_len,
        "expected_public_values": expected_public_values,
        "identity": expected_identity,
        "identity_evidence_sha256": identity_evidence_sha256,
        "native_trace": expected_native_trace,
    }


def _project_legacy_stateful_identity_bundle(
    bundle: Mapping[str, Any], persisted_report: Mapping[str, Any]
) -> dict[str, Any]:
    """Project current native semantics onto the frozen schema-1/trace-2 wire identities.

    The old sealed result predates the context-input bincode suffix and explicit block digest.
    Replay still executes the canonical input natively; only the historical wire/public identity
    fields are taken from the already content-addressed row so exact old evidence remains replayable.
    """
    projected = json.loads(canonical_json(bundle))
    identity = projected.get("identity")
    native_report = projected.get("report")
    native_trace = (
        native_report.get("controlled_trace")
        if isinstance(native_report, Mapping)
        else None
    )
    persisted_trace = persisted_report.get("controlled_trace")
    if (
        projected.get("schema_version") != 2
        or not isinstance(identity, dict)
        or identity.get("schema_version") != 2
        or not isinstance(native_trace, dict)
        or native_trace.get("schema_version") != 3
        or not isinstance(persisted_trace, Mapping)
        or persisted_trace.get("schema_version") != 2
    ):
        return projected
    historical_fields = (
        "backend_input_sha256",
        "backend_input_len",
        "workload_id",
        "transaction_envelope_sha256",
        "access_list_sha256",
        "prestate_sha256",
    )
    for field in historical_fields:
        if field not in persisted_trace:
            raise ValueError("legacy stateful trace identity field is missing")
        identity[field] = persisted_trace[field]
        native_trace[field] = persisted_trace[field]
    identity.pop("block_environment_sha256", None)
    identity["schema_version"] = 1
    native_trace.pop("block_environment_sha256", None)
    native_trace["schema_version"] = 2
    semantic = native_trace.get("semantic_check")
    if isinstance(semantic, dict):
        semantic["backend_input_sha256"] = persisted_trace["backend_input_sha256"]
    projected["schema_version"] = 1
    public_values = persisted_report.get("public_values")
    if not isinstance(public_values, str):
        raise ValueError("legacy stateful public output is missing")
    projected["expected_public_values"] = public_values
    native_report["guest_input_sha256"] = (
        f"0x{persisted_trace['backend_input_sha256']}"
    )
    native_report["guest_input_bincode_length"] = persisted_trace[
        "backend_input_len"
    ]
    return projected


def admit_stateful_fixture_trace(
    manifest: StatefulCampaignManifest,
    fixture: Mapping[str, Any],
    report_matches: Sequence[Mapping[str, Any]],
    *,
    expected_bundle: Mapping[str, Any],
    repeat_index: int,
) -> dict[str, Any]:
    """Admits one lane against a replayed Rust identity/native-trace bundle.

    The caller must regenerate ``expected_bundle`` from the explicitly supplied canonical
    guest-input with the reviewed host helper before reading formal run reports. A bundle copied
    from a run directory or derived from ``report_matches`` is not an independent evidence source.
    """

    if type(repeat_index) is not int or not 0 <= repeat_index < manifest.repeats:
        raise ValueError("repeat index differs from the frozen campaign")
    if len(report_matches) != 1:
        raise ValueError("stateful fixture requires exactly one host trace report")
    if not isinstance(fixture, Mapping):
        raise ValueError("stateful fixture must be an object")
    case_record = fixture.get("case_record")
    guest_input = fixture.get("guest_input")
    if not isinstance(case_record, Mapping) or not isinstance(guest_input, Mapping):
        raise ValueError("stateful fixture is missing case or guest input")
    scenario_name = case_record.get("scenario")
    lane = case_record.get("lane")
    relation_count = case_record.get("relation_count")
    scenario = manifest.scenario(scenario_name)
    expected_fixture = generate_stateful_fixture(
        manifest, scenario_name, lane=lane, count=relation_count
    )
    if json.loads(canonical_json(fixture)) != expected_fixture:
        raise ValueError("stateful fixture differs from canonical generated layout")

    expected = _admit_expected_identity_bundle(fixture, expected_bundle)
    expected_identity = expected["identity"]
    expected_backend_sha256 = expected["backend_input_sha256"]
    expected_backend_len = expected["backend_input_len"]
    expected_native_trace = expected["native_trace"]
    identity_evidence_sha256 = expected["identity_evidence_sha256"]

    report = report_matches[0]
    if not isinstance(report, Mapping):
        raise ValueError("host trace report must be an object")
    trace = report.get("controlled_trace")
    if not isinstance(trace, Mapping) or trace.get("kind") != "revm_opcode":
        raise ValueError("host trace report is not a REVM opcode trace")
    backend_input_sha256 = _require_sha256(
        trace.get("backend_input_sha256"), "trace backend_input_sha256"
    )
    if (
        backend_input_sha256 != expected_backend_sha256
        or report.get("guest_input_sha256") != f"0x{expected_backend_sha256}"
    ):
        raise ValueError("canonical/report/trace backend-input identity differs")
    if (
        trace.get("backend_input_len") != expected_backend_len
        or report.get("guest_input_bincode_length") != expected_backend_len
    ):
        raise ValueError("canonical/report/trace backend-input length differs")
    if trace.get("storage") != guest_input["storage"]:
        raise ValueError("backend-input identity storage/lane differs from fixture")
    trace_schema = trace.get("schema_version")
    if trace_schema not in (2, 3):
        raise ValueError("stateful trace schema differs")
    if (expected_identity.get("schema_version"), trace_schema) not in ((1, 2), (2, 3)):
        raise ValueError("stateful trace and identity schema migration differs")
    if (
        trace.get("evm_spec") != "osaka"
        or trace.get("revm_version") != "41.0.0"
        or trace.get("shared_constructor") != "raiko2-opcode-lab"
    ):
        raise ValueError("REVM identity differs from the stateful campaign")
    for field in (
        "workload_id",
        "transaction_envelope_sha256",
        "access_list_sha256",
        "prestate_sha256",
    ):
        if trace.get(field) != expected_identity[field]:
            raise ValueError(f"trace {field} differs from canonical Rust identity")
    if trace_schema == 3 and (
        trace.get("block_environment_sha256")
        != expected_identity["block_environment_sha256"]
    ):
        raise ValueError("trace block environment differs from canonical Rust identity")
    _require_sha256(trace.get("bytecode_sha256"), "bytecode_sha256")

    bytecode = bytes.fromhex(guest_input["bytecode"][2:])
    programs = decode_fixed_microprograms(bytecode)
    expected_program_hashes = [sha256_bytes(program) for program in programs]
    if trace.get("bytecode_len") != len(bytecode):
        raise ValueError("trace bytecode length differs from guest input")
    if trace.get("bytecode_sha256") != sha256_bytes(bytecode):
        raise ValueError("trace bytecode identity differs from guest input")
    if trace.get("program_sha256") != expected_program_hashes:
        raise ValueError("trace inactive-slot layout differs from guest input")
    if (
        trace.get("target_opcode") != guest_input["opcode"]
        or trace.get("declared_target_count") != guest_input["target_count"]
        or trace.get("declared_target_raw_gas") != guest_input["target_raw_gas"]
        or trace.get("tx_gas_limit") != guest_input["tx_gas_limit"]
    ):
        raise ValueError("trace declaration differs from guest input")
    if (
        trace.get("executed_target_count") != guest_input["target_count"]
        or trace.get("executed_target_raw_gas")
        != guest_input["target_count"] * guest_input["target_raw_gas"]
    ):
        raise ValueError("trace concrete target execution differs from declaration")

    expected_counts = _expected_executed_opcode_counts(case_record)
    if trace.get("executed_opcode_counts") != expected_counts:
        raise ValueError("trace exact opcode ledger differs from canonical programs")
    raw_gas = trace.get("executed_opcode_raw_gas")
    if not isinstance(raw_gas, Mapping) or set(raw_gas) != set(expected_counts):
        raise ValueError("trace exact raw-gas ledger differs from opcode ledger")
    if any(type(value) is not int or value < 0 for value in raw_gas.values()):
        raise ValueError("trace exact raw-gas ledger contains an invalid value")
    if trace.get("total_raw_gas") != sum(raw_gas.values()):
        raise ValueError("trace total raw gas differs from exact ledger")

    target_key = f"opcode:0x{guest_input['opcode']:02x}"
    target_ledger_raw_gas = raw_gas.get(target_key, 0)
    if target_ledger_raw_gas < trace["executed_target_raw_gas"]:
        raise ValueError("trace target raw gas is inconsistent with exact ledger")
    expected_non_target_counts = dict(expected_counts)
    remaining_target_count = (
        expected_non_target_counts.get(target_key, 0) - trace["executed_target_count"]
    )
    if remaining_target_count:
        expected_non_target_counts[target_key] = remaining_target_count
    else:
        expected_non_target_counts.pop(target_key, None)
    if trace.get("non_target_counts") != expected_non_target_counts:
        raise ValueError("trace non-target count ledger differs from exact ledger")
    expected_non_target_raw_gas = (
        trace["total_raw_gas"] - trace["executed_target_raw_gas"]
    )
    if trace.get("non_target_raw_gas") != expected_non_target_raw_gas:
        raise ValueError("trace non-target raw gas differs from exact ledger")

    expected_measurement_count = relation_count if lane == "target" else 0
    expected_measurement_raw_gas = (
        relation_count * scenario.target_raw_gas if lane == "target" else 0
    )
    if (
        trace.get("executed_measurement_count") != expected_measurement_count
        or trace.get("executed_measurement_raw_gas")
        != expected_measurement_raw_gas
    ):
        raise ValueError("trace measured storage execution differs from scenario")
    expected_prefix_count = (
        GENERATOR_MAX_COUNT
        if scenario.operation_kind == "store" and scenario.dirty
        else 0
    )
    if trace.get("executed_prefix_count") != expected_prefix_count:
        raise ValueError("trace dirty-prefix execution differs from scenario")
    measurement_key = f"opcode:0x{scenario.measurement_opcode:02x}"
    if expected_counts.get(measurement_key, 0) != (
        expected_measurement_count + expected_prefix_count
    ):
        raise ValueError("trace measurement/prefix count differs from exact ledger")
    if raw_gas.get(measurement_key, 0) < expected_measurement_raw_gas:
        raise ValueError("trace measurement raw gas differs from exact ledger")
    if trace.get("result_statuses") != {"success": GENERATOR_MAX_COUNT}:
        raise ValueError("trace result status differs from successful frozen execution")

    semantic_check_sha256 = _validate_semantic_check(
        case_record, trace.get("semantic_check"), backend_input_sha256
    )
    if canonical_json(trace) != canonical_json(expected_native_trace):
        raise ValueError("formal trace differs from independently replayed native trace")
    trace_sha256 = sha256_bytes(canonical_json(trace))
    row_identity = sha256_bytes(
        canonical_json(
            {
                "kind": "stateful_trace_admission",
                "scenario": scenario_name,
                "lane": lane,
                "relation_count": relation_count,
                "repeat_index": repeat_index,
                "backend_input_sha256": backend_input_sha256,
                "identity_evidence_sha256": identity_evidence_sha256,
                "trace_sha256": trace_sha256,
                "semantic_check_sha256": semantic_check_sha256,
            }
        )
    )
    return {
        "scenario": scenario_name,
        "lane": lane,
        "relation_count": relation_count,
        "repeat_index": repeat_index,
        "pair_id": case_record["pair_id"],
        "measurement_opcode": scenario.measurement_opcode,
        "reference_ledger": dict(case_record["reference_ledger"]),
        "backend_input_sha256": backend_input_sha256,
        "trace_sha256": trace_sha256,
        "semantic_check_sha256": semantic_check_sha256,
        "identity_evidence_sha256": identity_evidence_sha256,
        "row_identity": row_identity,
        "transaction_envelope_sha256": trace["transaction_envelope_sha256"],
        "access_list_sha256": trace["access_list_sha256"],
        "prestate_sha256": trace["prestate_sha256"],
        "tx_gas_limit": trace["tx_gas_limit"],
        "bytecode_len": trace["bytecode_len"],
        "executed_opcode_counts": dict(trace["executed_opcode_counts"]),
        "executed_opcode_raw_gas": dict(trace["executed_opcode_raw_gas"]),
        "revm_identity": {
            "evm_spec": trace["evm_spec"],
            "revm_version": trace["revm_version"],
            "shared_constructor": trace["shared_constructor"],
        },
    }


def stateful_ordered_pair_identity(
    *,
    scenario: str,
    measurement_opcode: int,
    relation_count: int,
    repeat_index: int,
    target_hash: str,
    control_hash: str,
) -> str:
    _require_sha256(target_hash, "target_hash")
    _require_sha256(control_hash, "control_hash")
    return sha256_bytes(
        canonical_json(
            {
                "kind": "stateful_ordered_pair",
                "scenario": scenario,
                "measurement_opcode": measurement_opcode,
                "relation_count": relation_count,
                "repeat_index": repeat_index,
                "target_hash": target_hash,
                "control_hash": control_hash,
            }
        )
    )


def stateful_execution_row_identity(
    *,
    scenario: str,
    lane: str,
    relation_count: int,
    repeat_index: int,
    backend_input_sha256: str,
    elf_sha256: str,
    launcher_sha256: str,
    trace_sha256: str,
) -> str:
    if not isinstance(scenario, str) or not scenario:
        raise ValueError("stateful row scenario is invalid")
    if lane not in {"target", "control"}:
        raise ValueError("stateful row lane is invalid")
    if type(relation_count) is not int or relation_count < 0:
        raise ValueError("stateful row relation count is invalid")
    if type(repeat_index) is not int or repeat_index not in range(REPEATS):
        raise ValueError("stateful row repeat index differs from the frozen campaign")
    for field, value in (
        ("backend_input_sha256", backend_input_sha256),
        ("elf_sha256", elf_sha256),
        ("launcher_sha256", launcher_sha256),
        ("trace_sha256", trace_sha256),
    ):
        _require_sha256(value, field)
    return sha256_bytes(
        canonical_json(
            {
                "kind": "stateful_campaign_execution_row",
                "scenario": scenario,
                "lane": lane,
                "relation_count": relation_count,
                "repeat_index": repeat_index,
                "backend_input_sha256": backend_input_sha256,
                "elf_sha256": elf_sha256,
                "launcher_sha256": launcher_sha256,
                "trace_sha256": trace_sha256,
            }
        )
    )


def admit_stateful_pair(
    manifest: StatefulCampaignManifest,
    target_fixture: Mapping[str, Any],
    target_reports: Sequence[Mapping[str, Any]],
    control_fixture: Mapping[str, Any],
    control_reports: Sequence[Mapping[str, Any]],
    *,
    target_bundle: Mapping[str, Any],
    control_bundle: Mapping[str, Any],
    repeat_index: int,
) -> dict[str, Any]:
    """Admits an ordered target/control relation and rejects every pair confound."""

    target_record = target_fixture.get("case_record", {})
    control_record = control_fixture.get("case_record", {})
    if target_record.get("lane") != "target":
        raise ValueError("stateful pair first fixture must be the target lane")
    if control_record.get("lane") != "control":
        raise ValueError("stateful pair second fixture must be the control lane")
    if (
        target_record.get("pair_id") != control_record.get("pair_id")
        or target_record.get("scenario") != control_record.get("scenario")
        or target_record.get("relation_count") != control_record.get("relation_count")
    ):
        raise ValueError("stateful target and control do not belong to the same pair")
    target = admit_stateful_fixture_trace(
        manifest,
        target_fixture,
        target_reports,
        expected_bundle=target_bundle,
        repeat_index=repeat_index,
    )
    control = admit_stateful_fixture_trace(
        manifest,
        control_fixture,
        control_reports,
        expected_bundle=control_bundle,
        repeat_index=repeat_index,
    )
    if target["backend_input_sha256"] == control["backend_input_sha256"]:
        raise ValueError("stateful target/control require distinct backend-input hashes")
    if target["reference_ledger"] != control["reference_ledger"]:
        raise ValueError("stateful pair reference ledger differs")
    comparisons = (
        ("transaction envelope", "transaction_envelope_sha256"),
        ("access list", "access_list_sha256"),
        ("prestate", "prestate_sha256"),
        ("gas limit", "tx_gas_limit"),
        ("bytecode length", "bytecode_len"),
        ("REVM identity", "revm_identity"),
    )
    for label, field in comparisons:
        if target[field] != control[field]:
            raise ValueError(f"stateful pair {label} differs")

    all_keys = set(target["executed_opcode_counts"]) | set(
        control["executed_opcode_counts"]
    )
    signed_total = {
        key: target["executed_opcode_counts"].get(key, 0)
        - control["executed_opcode_counts"].get(key, 0)
        for key in sorted(all_keys)
    }
    signed_total = {key: value for key, value in signed_total.items() if value}
    relation_count = target["relation_count"]
    expected_total = {
        key: value * relation_count
        for key, value in target["reference_ledger"].items()
        if value * relation_count
    }
    if signed_total != expected_total:
        raise ValueError("stateful pair signed opcode ledger differs from reference ledger")

    ordered_pair_identity = stateful_ordered_pair_identity(
        scenario=target["scenario"],
        measurement_opcode=target["measurement_opcode"],
        relation_count=relation_count,
        repeat_index=repeat_index,
        target_hash=target["backend_input_sha256"],
        control_hash=control["backend_input_sha256"],
    )
    return {
        "scenario": target["scenario"],
        "measurement_opcode": target["measurement_opcode"],
        "relation_count": relation_count,
        "repeat_index": repeat_index,
        "target": target,
        "control": control,
        "signed_execution_ledger": target["reference_ledger"],
        "ordered_pair_identity": ordered_pair_identity,
    }


def validate_stateful_row_specs(
    specs: Sequence[StatefulCampaignRowSpec], *, repeats: int
) -> None:
    if repeats != REPEATS:
        raise ValueError("stateful campaign requires exactly three repeats")
    logical = [spec.logical_identity for spec in specs]
    if len(logical) != len(set(logical)):
        raise ValueError("stateful campaign row specs contain a duplicate")
    grouped: dict[tuple[str, int, str], set[int]] = {}
    for spec in specs:
        grouped.setdefault(
            (spec.scenario, spec.relation_count, spec.lane), set()
        ).add(spec.repeat_index)
    required = set(range(repeats))
    if any(repeat_indexes != required for repeat_indexes in grouped.values()):
        raise ValueError("stateful campaign rows require exactly three repeats")
    pairs: dict[tuple[str, int], set[str]] = {}
    for scenario, count, lane in grouped:
        pairs.setdefault((scenario, count), set()).add(lane)
    if any(lanes != {"control", "target"} for lanes in pairs.values()):
        raise ValueError("stateful campaign rows require target/control pairing")


def _fixture_paths(
    fixtures_root: pathlib.Path, spec: StatefulCampaignRowSpec
) -> tuple[pathlib.Path, pathlib.Path]:
    directory = fixtures_root / spec.scenario / str(spec.relation_count) / spec.lane
    return directory / "case.json", directory / "guest-input.json"


def _read_canonical_json_file(path: pathlib.Path, *, label: str) -> Mapping[str, Any]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{label} must be a regular non-symlink file")
    raw = path.read_bytes()
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is not valid JSON") from error
    if not isinstance(value, Mapping) or raw != canonical_json(value) + b"\n":
        raise ValueError(f"{label} is not canonical JSON")
    return value


def _require_safe_tree(root: pathlib.Path, *, label: str) -> None:
    """Reject every symlink or non-regular descendant without following links."""
    if root.is_symlink() or (root.exists() and not root.is_dir()):
        raise ValueError(f"{label} must be a regular non-symlink directory")
    if not root.exists():
        return
    for child in root.rglob("*"):
        if child.is_symlink():
            raise ValueError(f"{label} contains a symlink: {child}")
        if not child.is_dir() and not child.is_file():
            raise ValueError(f"{label} contains a non-regular entry: {child}")


def _require_contained_path(
    root: pathlib.Path, path: pathlib.Path, *, label: str, kind: str = "file"
) -> None:
    try:
        relative = path.relative_to(root)
    except ValueError as error:
        raise ValueError(f"{label} must stay within its declared root") from error
    current = root
    for part in relative.parts:
        current = current / part
        if current.is_symlink():
            raise ValueError(f"{label} path contains a symlink")
    if path.exists():
        expected = path.is_dir() if kind == "directory" else path.is_file()
        if not expected:
            raise ValueError(f"{label} must be a regular {kind}")


def _require_run_inventory(
    run: pathlib.Path, expected_rows: set[pathlib.Path]
) -> None:
    if not run.exists():
        return
    allowed_top = {
        run / "identity.json",
        run / "rows",
        run / "rows.jsonl",
        run / "decisions.json",
        run / "decisions.sha256",
    }
    unexpected_top = set(run.iterdir()) - allowed_top
    if unexpected_top:
        raise ValueError("stateful campaign run contains an unexpected artifact")
    rows = run / "rows"
    if rows.exists():
        actual = set(rows.iterdir())
        if not actual.issubset(expected_rows) or any(not path.is_file() for path in actual):
            raise ValueError("stateful row ledger contains a duplicate or unexpected row")


def _load_fixture_for_spec(
    manifest: StatefulCampaignManifest,
    fixtures_root: pathlib.Path,
    spec: StatefulCampaignRowSpec,
) -> tuple[dict[str, Any], pathlib.Path]:
    case_path, input_path = _fixture_paths(fixtures_root, spec)
    case_record = _read_canonical_json_file(case_path, label="stateful case")
    guest_input = _read_canonical_json_file(input_path, label="stateful guest input")
    fixture = {"case_record": dict(case_record), "guest_input": dict(guest_input)}
    expected = generate_stateful_fixture(
        manifest, spec.scenario, lane=spec.lane, count=spec.relation_count
    )
    if json.loads(canonical_json(fixture)) != expected:
        raise ValueError("stateful fixture differs from canonical generation")
    return fixture, input_path


def _portable_formal_report(
    report: Mapping[str, Any],
    *,
    input_path: pathlib.Path,
    fixtures_root: pathlib.Path,
    expected_public_values: str | None = None,
) -> dict[str, Any]:
    if not isinstance(report, Mapping):
        raise ValueError("stateful execution report must be an object")
    supplied = report.get("input")
    expected_relative = str(input_path.relative_to(fixtures_root))
    supplied_matches = isinstance(supplied, str) and (
        supplied == expected_relative
        or (
            pathlib.Path(supplied).is_absolute()
            and pathlib.Path(supplied).resolve() == input_path.resolve()
        )
    )
    if not supplied_matches:
        raise ValueError("stateful execution report belongs to another guest input")
    portable = dict(report)
    portable["input"] = expected_relative
    public_values = portable.get("public_values")
    if (
        portable.get("stage") != EXECUTION_CONTRACT["stage"]
        or portable.get("mode") != EXECUTION_CONTRACT["mode"]
        or portable.get("proof_mode") != "compressed"
    ):
        raise ValueError("stateful execution report has the wrong execution context")
    if (
        not isinstance(public_values, str)
        or re.fullmatch(r"0x[0-9a-f]{64}", public_values) is None
    ):
        raise ValueError("stateful execution report has noncanonical public values")
    if expected_public_values is not None and public_values != expected_public_values:
        raise ValueError("formal public values differ from canonical guest output")
    if portable.get("exit_code") != 0:
        raise ValueError("stateful guest execution failed")
    prover_gas = portable.get("prover_gas", portable.get("gas"))
    if type(prover_gas) is not int or prover_gas <= 0:
        raise ValueError("stateful execution report has no positive prover gas")
    if portable.get("primary_workload_metric") != {
        "label": "prover_gas",
        "count": prover_gas,
    }:
        raise ValueError("stateful execution report has an invalid result shape")
    opcode_gas.validate_sp1_execution_provenance(
        portable, workload_kind="opcode", expected_engine="gas-estimator"
    )
    return portable


def _build_stateful_row(
    *,
    spec: StatefulCampaignRowSpec,
    fixture: Mapping[str, Any],
    report: Mapping[str, Any],
    admission: Mapping[str, Any],
    ordered_pair_identity: str,
    launcher_sha256: str,
    elf_sha256: str,
    calibration_run_id: str,
) -> dict[str, Any]:
    normalized = opcode_gas.raw_run_from_report(
        dict(fixture["case_record"]), dict(report)
    )
    normalized["repeat_index"] = spec.repeat_index
    normalized["execution_row_id"] = opcode_gas.controlled_execution_row_id(
        normalized["workload_id"],
        backend="sp1",
        execution_engine=normalized["sp1_execution_engine"],
        run_id=calibration_run_id,
        repeat_index=spec.repeat_index,
        backend_input_sha256=admission["backend_input_sha256"],
    )
    row_identity = stateful_execution_row_identity(
        scenario=spec.scenario,
        lane=spec.lane,
        relation_count=spec.relation_count,
        repeat_index=spec.repeat_index,
        backend_input_sha256=admission["backend_input_sha256"],
        elf_sha256=elf_sha256,
        launcher_sha256=launcher_sha256,
        trace_sha256=admission["trace_sha256"],
    )
    return {
        "schema_version": 1,
        "purpose": PURPOSE,
        "scenario": spec.scenario,
        "lane": spec.lane,
        "relation_count": spec.relation_count,
        "repeat_index": spec.repeat_index,
        "logical_identity": spec.logical_identity,
        "row_identity": row_identity,
        "ordered_pair_identity": ordered_pair_identity,
        "backend_input_sha256": admission["backend_input_sha256"],
        "elf_sha256": elf_sha256,
        "launcher_sha256": launcher_sha256,
        "trace_sha256": admission["trace_sha256"],
        "semantic_check_sha256": admission["semantic_check_sha256"],
        "identity_evidence_sha256": admission["identity_evidence_sha256"],
        "guest_input_json_file_sha256": fixture["case_record"][
            "guest_input_json_file_sha256"
        ],
        "formal_report_sha256": sha256_bytes(canonical_json(report)),
        "formal_report": dict(report),
        "normalized_report": normalized,
    }


def _pair_records(
    manifest: StatefulCampaignManifest,
    *,
    target_spec: StatefulCampaignRowSpec,
    control_spec: StatefulCampaignRowSpec,
    target_fixture: Mapping[str, Any],
    control_fixture: Mapping[str, Any],
    target_bundle: Mapping[str, Any],
    control_bundle: Mapping[str, Any],
    target_report: Mapping[str, Any],
    control_report: Mapping[str, Any],
    target_input: pathlib.Path,
    control_input: pathlib.Path,
    fixtures_root: pathlib.Path,
    launcher_sha256: str,
    elf_sha256: str,
    calibration_run_id: str,
) -> tuple[dict[str, Any], dict[str, Any]]:
    if target_spec.repeat_index != control_spec.repeat_index:
        raise ValueError("stateful target/control repeat pairing differs")
    target_expected = _admit_expected_identity_bundle(target_fixture, target_bundle)
    control_expected = _admit_expected_identity_bundle(control_fixture, control_bundle)
    target_portable = _portable_formal_report(
        target_report,
        input_path=target_input,
        fixtures_root=fixtures_root,
        expected_public_values=target_expected["expected_public_values"],
    )
    control_portable = _portable_formal_report(
        control_report,
        input_path=control_input,
        fixtures_root=fixtures_root,
        expected_public_values=control_expected["expected_public_values"],
    )
    pair = admit_stateful_pair(
        manifest,
        target_fixture,
        [target_portable],
        control_fixture,
        [control_portable],
        target_bundle=target_bundle,
        control_bundle=control_bundle,
        repeat_index=target_spec.repeat_index,
    )

    return (
        _build_stateful_row(
            spec=target_spec,
            fixture=target_fixture,
            report=target_portable,
            admission=pair["target"],
            ordered_pair_identity=pair["ordered_pair_identity"],
            launcher_sha256=launcher_sha256,
            elf_sha256=elf_sha256,
            calibration_run_id=calibration_run_id,
        ),
        _build_stateful_row(
            spec=control_spec,
            fixture=control_fixture,
            report=control_portable,
            admission=pair["control"],
            ordered_pair_identity=pair["ordered_pair_identity"],
            launcher_sha256=launcher_sha256,
            elf_sha256=elf_sha256,
            calibration_run_id=calibration_run_id,
        ),
    )


def _row_path(run: pathlib.Path, spec: StatefulCampaignRowSpec) -> pathlib.Path:
    return run / "rows" / f"{spec.logical_identity}.json"


def run_stateful_campaign_rows(
    manifest: StatefulCampaignManifest,
    specs: Sequence[StatefulCampaignRowSpec],
    *,
    fixtures_root: pathlib.Path,
    run: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    launcher_sha256: str,
    elf_sha256: str,
    batch_executor=opcode_gas.run_guest_inputs,
    identity_replayer=None,
    calibration_run_id: str | None = None,
    verification_only: bool = False,
) -> list[dict[str, Any]]:
    """Run or replay exact stateful rows without duplicating launcher execution logic."""
    validate_stateful_row_specs(specs, repeats=manifest.repeats)
    if calibration_run_id is None:
        calibration_run_id = run.name
    if not isinstance(calibration_run_id, str) or not calibration_run_id:
        raise ValueError("stateful calibration run identity is invalid")
    _require_safe_tree(fixtures_root, label="stateful fixture root")
    _require_safe_tree(run, label="stateful campaign run")
    _require_contained_path(
        run, run / "rows", label="stateful row directory", kind="directory"
    )
    expected_paths = {_row_path(run, spec) for spec in specs}
    _require_run_inventory(run, expected_paths)
    fixtures: dict[tuple[str, int, str], tuple[dict[str, Any], pathlib.Path]] = {}
    for spec in specs:
        key = (spec.scenario, spec.relation_count, spec.lane)
        if key not in fixtures:
            case_path, input_path = _fixture_paths(fixtures_root, spec)
            _require_contained_path(
                fixtures_root, case_path, label="stateful case"
            )
            _require_contained_path(
                fixtures_root, input_path, label="stateful guest input"
            )
            fixtures[key] = _load_fixture_for_spec(manifest, fixtures_root, spec)

    if identity_replayer is None:
        identity_replayer = opcode_gas.replay_revm_opcode_identity
    replay_cache: dict[tuple[str, str], Mapping[str, Any]] = {}
    bundles: dict[tuple[str, int, str], Mapping[str, Any]] = {}
    bundle_evidence: dict[tuple[str, int, str], Mapping[str, Any]] = {}
    # Provenance authority boundary: finish every native replay before reading any run row.
    for key, (fixture, input_path) in fixtures.items():
        bundle = identity_replayer(guest_launcher, input_path, replay_cache)
        bundles[key] = bundle
        bundle_evidence[key] = _admit_expected_identity_bundle(fixture, bundle)

    if not verification_only:
        run.mkdir(parents=True, exist_ok=True)
        (run / "rows").mkdir(exist_ok=True)
    elif not (run / "rows").is_dir():
        raise ValueError("portable stateful verification requires the row directory")
    actual_paths = set((run / "rows").glob("*.json"))
    if not actual_paths.issubset(expected_paths):
        raise ValueError("stateful row ledger contains a duplicate or unexpected row")

    existing_reports: dict[tuple[str, int, str, int], Mapping[str, Any]] = {}
    existing_payloads: dict[tuple[str, int, str, int], dict[str, Any]] = {}
    for spec in specs:
        path = _row_path(run, spec)
        if path.exists() or path.is_symlink():
            payload = _read_canonical_json_file(path, label="stateful row ledger entry")
            key = (spec.scenario, spec.relation_count, spec.lane, spec.repeat_index)
            existing_payloads[key] = dict(payload)
            report = payload.get("formal_report")
            if not isinstance(report, Mapping):
                raise ValueError("stateful row ledger entry is missing its formal report")
            existing_reports[key] = report

    if verification_only:
        for key, report in existing_reports.items():
            scenario, count, lane, _repeat_index = key
            fixture, input_path = fixtures[(scenario, count, lane)]
            portable = _portable_formal_report(
                report,
                input_path=input_path,
                fixtures_root=fixtures_root,
            )
            projected = _project_legacy_stateful_identity_bundle(
                bundles[(scenario, count, lane)], portable
            )
            bundles[(scenario, count, lane)] = projected
            bundle_evidence[(scenario, count, lane)] = (
                _admit_expected_identity_bundle(fixture, projected)
            )

    pair_groups: dict[tuple[str, int, int], dict[str, StatefulCampaignRowSpec]] = {}
    for spec in specs:
        pair_groups.setdefault(
            (spec.scenario, spec.relation_count, spec.repeat_index), {}
        )[spec.lane] = spec
    pending_pairs: list[
        tuple[
            tuple[str, int, int],
            dict[str, StatefulCampaignRowSpec],
            tuple[StatefulCampaignRowSpec, ...],
        ]
    ] = []
    for pair_key, lanes in pair_groups.items():
        missing = []
        for lane in ("control", "target"):
            key = (pair_key[0], pair_key[1], lane, pair_key[2])
            if key not in existing_reports:
                missing.append(lanes[lane])
        if missing:
            pending_pairs.append((pair_key, lanes, tuple(missing)))

    # Validate every surviving lane against the freshly replayed host identity before
    # executing a missing sibling. Exact pair/row equality is checked after recovery.
    for key, report in existing_reports.items():
        scenario, count, lane, repeat_index = key
        fixture, input_path = fixtures[(scenario, count, lane)]
        portable = _portable_formal_report(
            report,
            input_path=input_path,
            fixtures_root=fixtures_root,
            expected_public_values=bundle_evidence[
                (scenario, count, lane)
            ]["expected_public_values"],
        )
        admission = admit_stateful_fixture_trace(
            manifest,
            fixture,
            [portable],
            expected_bundle=bundles[(scenario, count, lane)],
            repeat_index=repeat_index,
        )
        spec = pair_groups[(scenario, count, repeat_index)][lane]
        target_hash = bundle_evidence[(scenario, count, "target")][
            "backend_input_sha256"
        ]
        control_hash = bundle_evidence[(scenario, count, "control")][
            "backend_input_sha256"
        ]
        ordered_pair_identity = stateful_ordered_pair_identity(
            scenario=scenario,
            measurement_opcode=manifest.scenario(scenario).measurement_opcode,
            relation_count=count,
            repeat_index=repeat_index,
            target_hash=target_hash,
            control_hash=control_hash,
        )
        expected_row = _build_stateful_row(
            spec=spec,
            fixture=fixture,
            report=portable,
            admission=admission,
            ordered_pair_identity=ordered_pair_identity,
            launcher_sha256=launcher_sha256,
            elf_sha256=elf_sha256,
            calibration_run_id=calibration_run_id,
        )
        payload = existing_payloads[key]
        if canonical_json(payload) != canonical_json(expected_row):
            raise ValueError("persisted stateful lane differs from fresh replay")

    if verification_only and pending_pairs:
        raise ValueError("portable stateful verification found a missing row")

    records: dict[tuple[str, int, str, int], dict[str, Any]] = {}

    def build_pair_records(
        pair_key: tuple[str, int, int],
        lanes: Mapping[str, StatefulCampaignRowSpec],
        reports: Mapping[tuple[str, int, str, int], Mapping[str, Any]],
    ) -> dict[tuple[str, int, str, int], dict[str, Any]]:
        scenario, count, repeat_index = pair_key
        target_key = (scenario, count, "target")
        control_key = (scenario, count, "control")
        target_fixture, target_input = fixtures[target_key]
        control_fixture, control_input = fixtures[control_key]
        target_record, control_record = _pair_records(
            manifest,
            target_spec=lanes["target"],
            control_spec=lanes["control"],
            target_fixture=target_fixture,
            control_fixture=control_fixture,
            target_bundle=bundles[target_key],
            control_bundle=bundles[control_key],
            target_report=reports[(scenario, count, "target", repeat_index)],
            control_report=reports[(scenario, count, "control", repeat_index)],
            target_input=target_input,
            control_input=control_input,
            fixtures_root=fixtures_root,
            launcher_sha256=launcher_sha256,
            elf_sha256=elf_sha256,
            calibration_run_id=calibration_run_id,
        )
        pair_records = {}
        for spec, record in (
            (lanes["target"], target_record),
            (lanes["control"], control_record),
        ):
            key = (scenario, count, spec.lane, repeat_index)
            old = existing_payloads.get(key)
            if old is not None and canonical_json(old) != canonical_json(record):
                raise ValueError("persisted stateful row differs from exact replay")
            pair_records[key] = record
        return pair_records

    def validate_repeated_public_outputs(
        candidate_records: Mapping[
            tuple[str, int, str, int], Mapping[str, Any]
        ],
    ) -> None:
        public_values_by_lane: dict[tuple[str, int, str], set[str]] = {}
        for key, record in candidate_records.items():
            public_values_by_lane.setdefault(key[:3], set()).add(
                record["formal_report"]["public_values"]
            )
        if any(len(values) != 1 for values in public_values_by_lane.values()):
            raise ValueError("stateful repeated public output differs")

    for pair_key, lanes in pair_groups.items():
        if all(
            (pair_key[0], pair_key[1], lane, pair_key[2]) in existing_reports
            for lane in ("control", "target")
        ):
            records.update(build_pair_records(pair_key, lanes, existing_reports))
    validate_repeated_public_outputs(records)

    for offset in range(0, len(pending_pairs), STATEFUL_EXECUTION_PAIR_CHUNK_SIZE):
        pair_chunk = pending_pairs[
            offset : offset + STATEFUL_EXECUTION_PAIR_CHUNK_SIZE
        ]
        missing_specs = tuple(
            spec for _pair_key, _lanes, missing in pair_chunk for spec in missing
        )
        input_paths = tuple(
            fixtures[(spec.scenario, spec.relation_count, spec.lane)][1]
            for spec in missing_specs
        )
        with tempfile.TemporaryDirectory(
            prefix="stateful-opcode-reports-"
        ) as temporary:
            reports_path = pathlib.Path(temporary) / "reports.jsonl"
            batch_executor(
                guest_launcher=guest_launcher,
                elf_path=elf,
                input_paths=input_paths,
                reports_jsonl=reports_path,
                stage=EXECUTION_CONTRACT["stage"],
            )
            reports = list(opcode_gas.iter_jsonl(reports_path))
        if len(reports) != len(missing_specs):
            raise ValueError("stateful batch execution report inventory differs")

        chunk_reports = {}
        for spec, input_path, report in zip(
            missing_specs, input_paths, reports, strict=True
        ):
            key = (spec.scenario, spec.relation_count, spec.lane, spec.repeat_index)
            chunk_reports[key] = _portable_formal_report(
                report,
                input_path=input_path,
                fixtures_root=fixtures_root,
                expected_public_values=bundle_evidence[
                    (spec.scenario, spec.relation_count, spec.lane)
                ]["expected_public_values"],
            )

        available_reports = {**existing_reports, **chunk_reports}
        chunk_records = {}
        for pair_key, lanes, _missing in pair_chunk:
            chunk_records.update(
                build_pair_records(pair_key, lanes, available_reports)
            )
        validate_repeated_public_outputs({**records, **chunk_records})

        for _pair_key, _lanes, missing in pair_chunk:
            for spec in missing:
                key = (
                    spec.scenario,
                    spec.relation_count,
                    spec.lane,
                    spec.repeat_index,
                )
                record = chunk_records[key]
                opcode_gas.persist_immutable_bytes(
                    _row_path(run, spec), canonical_json(record) + b"\n"
                )
                existing_payloads[key] = record
                existing_reports[key] = record["formal_report"]
        records.update(chunk_records)

    validate_repeated_public_outputs(records)
    if len(records) != len(specs):
        raise ValueError("stateful campaign row inventory differs after execution")
    return [
        records[(spec.scenario, spec.relation_count, spec.lane, spec.repeat_index)]
        for spec in specs
    ]


def _repository_relative(path: pathlib.Path, *, label: str) -> str:
    resolved = path.resolve()
    try:
        return str(resolved.relative_to(opcode_gas.REPO_ROOT.resolve()))
    except ValueError as error:
        raise ValueError(f"{label} must stay within the repository") from error


def _regular_file(path: pathlib.Path, *, label: str) -> pathlib.Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{label} must be a regular non-symlink file")
    return path


def _stateful_campaign_identity(
    *,
    manifest_path: pathlib.Path,
    manifest: StatefulCampaignManifest,
    calibration_run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    specs: Sequence[StatefulCampaignRowSpec],
    execution_identity_loader,
    launcher_validator,
) -> tuple[dict[str, Any], str, str]:
    _regular_file(manifest_path, label="stateful campaign manifest")
    expected_manifest = (
        opcode_gas.REPO_ROOT
        / "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json"
    ).resolve()
    if manifest_path.resolve() != expected_manifest:
        raise ValueError("stateful campaign requires the tracked frozen manifest")
    _require_safe_tree(fixtures_root, label="stateful fixture root")
    if not fixtures_root.is_dir():
        raise ValueError("stateful fixture root must be a regular directory")
    _regular_file(guest_launcher, label="stateful guest launcher")
    _regular_file(elf, label="stateful guest ELF")
    if _repository_relative(elf, label="stateful guest ELF") != EXECUTION_CONTRACT[
        "elf_path"
    ]:
        raise ValueError("stateful campaign requires the frozen REVM opcode-lab ELF")

    execution_identity = execution_identity_loader(calibration_run)
    if not isinstance(execution_identity, Mapping):
        raise ValueError("calibration execution identity is invalid")
    launcher_sha256 = launcher_validator(execution_identity, guest_launcher)
    _require_sha256(launcher_sha256, "launcher_sha256")
    elf_sha256 = opcode_gas.sha256_file(elf)
    guest_artifacts = execution_identity.get("guest_artifacts")
    if (
        not isinstance(guest_artifacts, Mapping)
        or guest_artifacts.get(EXECUTION_CONTRACT["elf_path"]) != elf_sha256
    ):
        raise ValueError("stateful guest ELF differs from calibration identity")

    registry_path = opcode_gas.REPO_ROOT / manifest.reference_registry["path"]
    _regular_file(registry_path, label="stateful source registry")
    try:
        registry_raw = registry_path.read_bytes()
        registry = json.loads(registry_raw)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("stateful source registry is invalid JSON") from error
    if (
        not isinstance(registry, Mapping)
        or registry_raw != opcode_gas._canonical_json_file_bytes(registry)
    ):
        raise ValueError("stateful source registry is not canonical JSON")
    reference_registry = load_stateful_reference_registry(
        registry,
        expected_artifact_sha256=manifest.reference_registry["artifact_sha256"],
    )
    registry_artifact_sha256 = reference_registry.artifact_sha256

    fixture_rows = []
    seen = set()
    for spec in specs:
        key = spec.scenario, spec.relation_count, spec.lane
        if key in seen:
            continue
        seen.add(key)
        case_path, input_path = _fixture_paths(fixtures_root, spec)
        _require_contained_path(fixtures_root, case_path, label="stateful case")
        _require_contained_path(
            fixtures_root, input_path, label="stateful guest input"
        )
        _regular_file(case_path, label="stateful case")
        _regular_file(input_path, label="stateful guest input")
        fixture_rows.append(
            {
                "scenario": spec.scenario,
                "relation_count": spec.relation_count,
                "lane": spec.lane,
                "case_sha256": opcode_gas.sha256_file(case_path),
                "guest_input_sha256": opcode_gas.sha256_file(input_path),
            }
        )
    implementation_revision = execution_identity.get("implementation_revision")
    if (
        not isinstance(implementation_revision, str)
        or re.fullmatch(r"[0-9a-f]{40}", implementation_revision) is None
    ):
        raise ValueError("calibration implementation revision is invalid")
    identity = {
        "schema_version": 1,
        "purpose": PURPOSE,
        "implementation_revision": implementation_revision,
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(
            canonical_json(execution_identity)
        ),
        "manifest": {
            "path": _repository_relative(
                manifest_path, label="stateful campaign manifest"
            ),
            "file_sha256": opcode_gas.sha256_file(manifest_path),
        },
        "execution": dict(manifest.execution),
        "guest_launcher": {
            "path": _repository_relative(
                guest_launcher, label="stateful guest launcher"
            ),
            "file_sha256": launcher_sha256,
        },
        "guest_elf": {
            "path": EXECUTION_CONTRACT["elf_path"],
            "file_sha256": elf_sha256,
        },
        "source_registry": {
            **dict(manifest.reference_registry),
            "file_sha256": opcode_gas.sha256_file(registry_path),
        },
        "fixtures_root": _repository_relative(
            fixtures_root, label="stateful fixture root"
        ),
        "fixture_inventory_sha256": sha256_bytes(canonical_json(fixture_rows)),
        "row_inventory_sha256": sha256_bytes(
            canonical_json([spec.logical_identity for spec in specs])
        ),
        "row_count": len(specs),
        "repeats": manifest.repeats,
    }
    return identity, launcher_sha256, elf_sha256


def _terminal_campaign_payloads(
    identity: Mapping[str, Any], records: Sequence[Mapping[str, Any]]
) -> tuple[bytes, dict[str, Any], bytes]:
    rows_bytes = b"".join(canonical_json(record) + b"\n" for record in records)
    decisions = {
        "schema_version": 1,
        "purpose": PURPOSE,
        "status": "complete",
        "campaign_identity_sha256": sha256_bytes(canonical_json(identity)),
        "row_count": len(records),
        "pair_count": len(records) // 2,
        "repeats": REPEATS,
        "row_ledger_sha256": sha256_bytes(rows_bytes),
    }
    decisions_bytes = canonical_json(decisions) + b"\n"
    return rows_bytes, decisions, decisions_bytes


def run_stateful_opcode_campaign(
    *,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    run: pathlib.Path,
    batch_executor=opcode_gas.run_guest_inputs,
    identity_replayer=None,
    execution_identity_loader=opcode_gas.validate_calibration_execution_identity,
    launcher_validator=opcode_gas.validate_calibration_guest_launcher,
) -> dict[str, Any]:
    if run.is_symlink() or (run.exists() and not run.is_dir()):
        raise ValueError("stateful campaign run must be a regular directory")
    _require_safe_tree(run, label="stateful campaign run")
    terminal_paths = tuple(
        run / name for name in ("rows.jsonl", "decisions.json", "decisions.sha256")
    )
    terminal_present = [path.exists() or path.is_symlink() for path in terminal_paths]
    if any(terminal_present):
        if not all(terminal_present):
            raise ValueError("terminal stateful campaign artifact set is incomplete")
        return verify_stateful_opcode_campaign(
            manifest_path=manifest_path,
            calibration_run=calibration_run,
            fixtures_root=fixtures_root,
            guest_launcher=guest_launcher,
            elf=elf,
            run=run,
            identity_replayer=identity_replayer,
            execution_identity_loader=execution_identity_loader,
            launcher_validator=launcher_validator,
        )
    manifest = load_stateful_campaign_manifest(manifest_path)
    specs = stateful_campaign_row_specs(manifest)
    validate_stateful_row_specs(specs, repeats=manifest.repeats)
    identity, launcher_sha256, elf_sha256 = _stateful_campaign_identity(
        manifest_path=manifest_path,
        manifest=manifest,
        calibration_run=calibration_run,
        fixtures_root=fixtures_root,
        guest_launcher=guest_launcher,
        elf=elf,
        specs=specs,
        execution_identity_loader=execution_identity_loader,
        launcher_validator=launcher_validator,
    )
    run.mkdir(parents=True, exist_ok=True)
    opcode_gas.persist_immutable_bytes(
        run / "identity.json", canonical_json(identity) + b"\n"
    )
    records = run_stateful_campaign_rows(
        manifest,
        specs,
        fixtures_root=fixtures_root,
        run=run,
        guest_launcher=guest_launcher,
        elf=elf,
        launcher_sha256=launcher_sha256,
        elf_sha256=elf_sha256,
        batch_executor=batch_executor,
        identity_replayer=identity_replayer,
        calibration_run_id=identity["calibration_id"],
    )
    rows_bytes, decisions, decisions_bytes = _terminal_campaign_payloads(
        identity, records
    )
    opcode_gas.persist_immutable_bytes(run / "rows.jsonl", rows_bytes)
    opcode_gas.persist_immutable_bytes(run / "decisions.json", decisions_bytes)
    opcode_gas.persist_immutable_bytes(
        run / "decisions.sha256",
        (sha256_bytes(decisions_bytes) + "\n").encode(),
    )
    return {
        "status": decisions["status"],
        "row_count": decisions["row_count"],
        "pair_count": decisions["pair_count"],
        "campaign_identity_sha256": decisions["campaign_identity_sha256"],
        "row_ledger_sha256": decisions["row_ledger_sha256"],
    }


def verify_stateful_opcode_campaign(
    *,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    run: pathlib.Path,
    identity_replayer=None,
    execution_identity_loader=opcode_gas.validate_calibration_execution_identity,
    launcher_validator=opcode_gas.validate_calibration_guest_launcher,
) -> dict[str, Any]:
    if run.is_symlink() or not run.is_dir():
        raise ValueError("stateful campaign run must be a regular directory")
    _require_safe_tree(run, label="stateful campaign run")
    manifest = load_stateful_campaign_manifest(manifest_path)
    specs = stateful_campaign_row_specs(manifest)
    identity, launcher_sha256, elf_sha256 = _stateful_campaign_identity(
        manifest_path=manifest_path,
        manifest=manifest,
        calibration_run=calibration_run,
        fixtures_root=fixtures_root,
        guest_launcher=guest_launcher,
        elf=elf,
        specs=specs,
        execution_identity_loader=execution_identity_loader,
        launcher_validator=launcher_validator,
    )
    identity_path = _regular_file(run / "identity.json", label="campaign identity")
    if identity_path.read_bytes() != canonical_json(identity) + b"\n":
        raise ValueError("persisted campaign identity differs from supplied artifacts")

    def reject_guest_execution(**_kwargs):
        raise ValueError("portable stateful verification cannot execute the SP1 guest")

    records = run_stateful_campaign_rows(
        manifest,
        specs,
        fixtures_root=fixtures_root,
        run=run,
        guest_launcher=guest_launcher,
        elf=elf,
        launcher_sha256=launcher_sha256,
        elf_sha256=elf_sha256,
        batch_executor=reject_guest_execution,
        identity_replayer=identity_replayer,
        calibration_run_id=identity["calibration_id"],
        verification_only=True,
    )
    rows_bytes, decisions, decisions_bytes = _terminal_campaign_payloads(
        identity, records
    )
    rows_path = _regular_file(run / "rows.jsonl", label="terminal row ledger")
    decisions_path = _regular_file(
        run / "decisions.json", label="terminal decision ledger"
    )
    seal_path = _regular_file(
        run / "decisions.sha256", label="terminal decision seal"
    )
    if rows_path.read_bytes() != rows_bytes:
        raise ValueError("terminal stateful row ledger differs from exact replay")
    if decisions_path.read_bytes() != decisions_bytes:
        raise ValueError("terminal stateful decisions differ from exact replay")
    if seal_path.read_bytes() != (sha256_bytes(decisions_bytes) + "\n").encode():
        raise ValueError("terminal stateful decision seal differs")
    return {
        "status": decisions["status"],
        "row_count": decisions["row_count"],
        "pair_count": decisions["pair_count"],
        "campaign_identity_sha256": decisions["campaign_identity_sha256"],
        "row_ledger_sha256": decisions["row_ledger_sha256"],
    }


@dataclass(frozen=True)
class StatefulReferenceRegistry:
    artifact_sha256: str
    typed_registry: OpcodeRegistry
    common_dispatch_exact: Fraction
    static_body_per_raw_gas: Mapping[str, Fraction]
    named_opcode_keys: frozenset[str]

    def __post_init__(self) -> None:
        object.__setattr__(
            self,
            "static_body_per_raw_gas",
            MappingProxyType(dict(self.static_body_per_raw_gas)),
        )
        object.__setattr__(self, "named_opcode_keys", frozenset(self.named_opcode_keys))


def _canonical_decimal_fraction(value: Any, *, label: str) -> Fraction:
    if not isinstance(value, str):
        raise ValueError(f"{label} must be a canonical Decimal string")
    try:
        decimal = Decimal(value)
    except InvalidOperation as error:
        raise ValueError(f"{label} must be a canonical Decimal string") from error
    if not decimal.is_finite() or opcode_gas._decimal_text(decimal) != value:
        raise ValueError(f"{label} must be a canonical Decimal string")
    return Fraction(decimal)


def _fraction_decimal_text(value: Fraction) -> str:
    integer_digits = max(1, len(str(abs(value.numerator) // value.denominator)))
    context = Context(
        prec=integer_digits + 90,
        rounding=ROUND_HALF_EVEN,
        traps=[],
    )
    with localcontext(context):
        projected = (Decimal(value.numerator) / Decimal(value.denominator)).quantize(
            Decimal(1).scaleb(-80),
            rounding=ROUND_HALF_EVEN,
        )
    denominator = value.denominator
    for factor in (2, 5):
        while denominator % factor == 0:
            denominator //= factor
    if denominator != 1:
        return format(projected, "f")
    return opcode_gas._decimal_text(projected)


def exact_fraction_payload(value: Fraction) -> dict[str, str]:
    if not isinstance(value, Fraction):
        raise TypeError("exact value must be a Fraction")
    payload = {
        "numerator": str(value.numerator),
        "denominator": str(value.denominator),
        "decimal": _fraction_decimal_text(value),
    }
    replay_exact_fraction(payload)
    return payload


def replay_exact_fraction(value: Mapping[str, Any]) -> Fraction:
    if not isinstance(value, Mapping) or set(value) != {
        "numerator",
        "denominator",
        "decimal",
    }:
        raise ValueError("exact Fraction payload has an invalid shape")
    numerator_raw = value["numerator"]
    denominator_raw = value["denominator"]
    if not isinstance(numerator_raw, str) or not isinstance(denominator_raw, str):
        raise ValueError("exact Fraction numerator/denominator must be strings")
    try:
        numerator = int(numerator_raw)
        denominator = int(denominator_raw)
    except ValueError as error:
        raise ValueError("exact Fraction numerator/denominator are invalid") from error
    if (
        str(numerator) != numerator_raw
        or str(denominator) != denominator_raw
        or denominator <= 0
    ):
        raise ValueError("exact Fraction numerator/denominator are noncanonical")
    exact = Fraction(numerator, denominator)
    if exact.numerator != numerator or exact.denominator != denominator:
        raise ValueError("exact Fraction numerator/denominator are not reduced")
    decimal_raw = value["decimal"]
    if not isinstance(decimal_raw, str):
        raise ValueError("Fraction projection must be a canonical Decimal string")
    try:
        decimal = Decimal(decimal_raw)
    except InvalidOperation as error:
        raise ValueError("Fraction projection must be a canonical Decimal string") from error
    if not decimal.is_finite():
        raise ValueError("Fraction projection must be a canonical Decimal string")
    projected = Fraction(decimal)
    if abs(projected - exact) > STATEFUL_REPLAY_RESIDUAL_MAX:
        raise ValueError("exact Fraction decimal replay residual exceeds 1e-75")
    if value["decimal"] != _fraction_decimal_text(exact):
        raise ValueError("exact Fraction decimal is not its 80-digit projection")
    return exact


def load_stateful_reference_registry(
    artifact: Mapping[str, Any], *, expected_artifact_sha256: str
) -> StatefulReferenceRegistry:
    if not isinstance(artifact, Mapping):
        raise ValueError("stateful reference registry must be an object")
    actual_sha256 = opcode_gas._validate_content_addressed_artifact(
        artifact, label="stateful reference registry"
    )
    if actual_sha256 != expected_artifact_sha256:
        raise ValueError("stateful reference registry differs from the sealed artifact")
    registry = artifact.get("registry")
    named = registry.get("named_opcode_keys") if isinstance(registry, Mapping) else None
    if not isinstance(named, list) or any(not isinstance(key, str) for key in named):
        raise ValueError("stateful reference registry has invalid named opcode keys")
    opcode_gas._validate_operation_core_registry(
        artifact, set(named)
    )
    if not isinstance(registry, Mapping):
        raise ValueError("stateful reference registry payload is missing")
    registry_snapshot = json.loads(canonical_json(registry))
    typed_registry = load_registry_payload(registry_snapshot)
    return StatefulReferenceRegistry(
        artifact_sha256=actual_sha256,
        typed_registry=typed_registry,
        common_dispatch_exact=_canonical_decimal_fraction(
            registry_snapshot["common_dispatch"],
            label="stateful reference common dispatch",
        ),
        static_body_per_raw_gas={
            model_id: _canonical_decimal_fraction(
                registry_snapshot["models"][model_id]["parameters"][
                    "body_per_raw_gas"
                ],
                label=f"stateful reference {model_id} body_per_raw_gas",
            )
            for model_id, model in typed_registry.models.items()
            if model.kind is ModelKind.STATIC_RAW_GAS
        },
        named_opcode_keys=frozenset(named),
    )


def _typed_reference_input(kind: str, value: Any) -> dict[str, int]:
    if not isinstance(value, Mapping):
        raise ValueError("stateful reference typed input must be an object")
    if kind != ModelKind.STATIC_RAW_GAS.value:
        raise ValueError(
            "structured stateful reference lacks exact Task 4 typed input"
        )
    if set(value) != {"raw_gas"} or any(
        type(item) is not int or item < 0 for item in value.values()
    ):
        raise ValueError("stateful reference typed input differs from its exact model")
    return dict(value)


def evaluate_typed_reference(
    registry: StatefulReferenceRegistry,
    opcode_key: str,
    typed_reference: Mapping[str, Any],
) -> dict[str, Any]:
    match = re.fullmatch(r"opcode:0x([0-9a-f]{2})", opcode_key)
    if match is None:
        raise ValueError("stateful reference opcode key is invalid")
    opcode = int(match.group(1), 16)
    if opcode_key not in registry.named_opcode_keys:
        raise ValueError("stateful reference opcode is missing from the sealed registry")
    model_id = registry.typed_registry.opcode_model_ids[opcode]
    if model_id is None:
        raise ValueError("stateful reference opcode is unsupported by the sealed registry")
    if not isinstance(typed_reference, Mapping) or set(typed_reference) != {
        "artifact_sha256",
        "model_id",
        "model_kind",
        "input",
    }:
        raise ValueError("stateful reference typed input has an invalid shape")
    model = registry.typed_registry.models.get(model_id)
    if model is None:
        raise ValueError("stateful reference model is missing")
    if (
        typed_reference["artifact_sha256"] != registry.artifact_sha256
        or typed_reference["model_id"] != model_id
        or typed_reference["model_kind"] != model.kind.value
    ):
        raise ValueError("stateful reference model identity differs")
    typed_input = _typed_reference_input(model.kind.value, typed_reference["input"])
    predicted = predict_opcode_event(
        registry.typed_registry,
        OpcodeEvent(opcode=opcode, raw_gas=typed_input["raw_gas"]),
    )
    exact = (
        registry.common_dispatch_exact
        + registry.static_body_per_raw_gas[model_id] * typed_input["raw_gas"]
    )
    predictor_residual = abs(Fraction(predicted) - exact)
    if predictor_residual > STATEFUL_REPLAY_RESIDUAL_MAX:
        raise ValueError("stateful reference typed predictor differs from exact static cost")
    return {
        "opcode_key": opcode_key,
        "artifact_sha256": registry.artifact_sha256,
        "model_id": model_id,
        "model_kind": model.kind.value,
        "input": typed_input,
        "common_dispatch_exact": exact_fraction_payload(
            registry.common_dispatch_exact
        ),
        "model_parameters_exact": {
            "body_per_raw_gas": exact_fraction_payload(
                registry.static_body_per_raw_gas[model_id]
            )
        },
        "predictor_decimal": opcode_gas._decimal_text(predicted),
        "predictor_residual_exact": exact_fraction_payload(predictor_residual),
        "predictor_residual_tolerance_exact": exact_fraction_payload(
            STATEFUL_REPLAY_RESIDUAL_MAX
        ),
        "predicted_cost_exact": exact_fraction_payload(exact),
    }


def resolve_signed_reference_cost(
    registry: StatefulReferenceRegistry,
    signed_execution_ledger: Mapping[str, Any],
    *,
    state_opcode_key: str,
    typed_inputs: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    if not isinstance(signed_execution_ledger, Mapping) or any(
        not isinstance(key, str) or type(value) is not int or value == 0
        for key, value in signed_execution_ledger.items()
    ):
        raise ValueError("stateful signed execution ledger is invalid")
    if signed_execution_ledger.get(state_opcode_key) != 1:
        raise ValueError("stateful signed execution ledger must contain one state opcode")
    if sum(signed_execution_ledger.values()) != 0:
        raise ValueError("stateful signed execution ledger is not zero sum")
    reference_keys = set(signed_execution_ledger) - {state_opcode_key}
    if set(typed_inputs) != reference_keys:
        raise ValueError("stateful signed execution ledger typed references differ")
    entries = []
    signed_cost = Fraction()
    for opcode_key in sorted(reference_keys):
        evidence = evaluate_typed_reference(
            registry, opcode_key, typed_inputs[opcode_key]
        )
        coefficient = signed_execution_ledger[opcode_key]
        predicted = replay_exact_fraction(evidence["predicted_cost_exact"])
        signed_cost += coefficient * predicted
        entries.append({**evidence, "signed_count": coefficient})
    return {
        "signed_execution_ledger": dict(sorted(signed_execution_ledger.items())),
        "entries": entries,
        "signed_reference_cost_exact": exact_fraction_payload(signed_cost),
    }


def _task4_row_trace_ledgers(
    row: Mapping[str, Any], *, label: str
) -> tuple[dict[str, int], dict[str, int]]:
    formal = row.get("formal_report")
    trace = formal.get("controlled_trace") if isinstance(formal, Mapping) else None
    counts = trace.get("executed_opcode_counts") if isinstance(trace, Mapping) else None
    raw_gas = (
        trace.get("executed_opcode_raw_gas") if isinstance(trace, Mapping) else None
    )
    if (
        not isinstance(counts, Mapping)
        or not isinstance(raw_gas, Mapping)
        or set(counts) != set(raw_gas)
        or any(
            not isinstance(key, str)
            or type(value) is not int
            or value < 0
            for ledger in (counts, raw_gas)
            for key, value in ledger.items()
        )
    ):
        raise ValueError(f"{label} Task 4 trace ledgers are invalid")
    return dict(counts), dict(raw_gas)


def extract_stateful_pair_observations(
    manifest: StatefulCampaignManifest,
    rows: Sequence[Mapping[str, Any]],
    registry: StatefulReferenceRegistry,
) -> dict[str, Any]:
    if not rows:
        raise ValueError("stateful Task 4 rows are empty")
    grouped: dict[tuple[str, int, int], dict[str, Mapping[str, Any]]] = {}
    for row in rows:
        if not isinstance(row, Mapping):
            raise ValueError("stateful Task 4 row is not an object")
        scenario_name = row.get("scenario")
        lane = row.get("lane")
        count = row.get("relation_count")
        repeat_index = row.get("repeat_index")
        scenario = manifest.scenario(scenario_name)
        if (
            lane not in {"target", "control"}
            or type(count) is not int
            or count not in scenario.counts
            or type(repeat_index) is not int
            or repeat_index not in range(manifest.repeats)
        ):
            raise ValueError("stateful Task 4 row identity differs from the manifest")
        key = scenario_name, count, repeat_index
        lanes = grouped.setdefault(key, {})
        if lane in lanes:
            raise ValueError("stateful Task 4 pair lane is duplicated")
        lanes[lane] = row

    pair_ledgers = []
    deltas: dict[str, dict[int, list[str | None]]] = {}
    raw_inputs: dict[str, dict[str, set[int]]] = {}
    present_scenarios = set()
    for (scenario_name, count, repeat_index), lanes in sorted(grouped.items()):
        if set(lanes) != {"target", "control"}:
            raise ValueError("stateful Task 4 target/control pair is incomplete")
        target = lanes["target"]
        control = lanes["control"]
        if target.get("ordered_pair_identity") != control.get("ordered_pair_identity"):
            raise ValueError("stateful Task 4 ordered pair identity differs")
        scenario = manifest.scenario(scenario_name)
        expected_per_unit = _reference_ledger(scenario)
        target_counts, target_raw = _task4_row_trace_ledgers(
            target, label="target"
        )
        control_counts, control_raw = _task4_row_trace_ledgers(
            control, label="control"
        )
        all_keys = set(target_counts) | set(control_counts)
        signed_total = {
            key: target_counts.get(key, 0) - control_counts.get(key, 0)
            for key in sorted(all_keys)
        }
        signed_total = {key: value for key, value in signed_total.items() if value}
        if count == 0:
            if signed_total:
                raise ValueError("stateful count-zero signed execution ledger is not empty")
        else:
            if any(value % count for value in signed_total.values()):
                raise ValueError("stateful signed execution ledger is not integral per slot")
            per_unit = {
                key: value // count for key, value in signed_total.items()
            }
            if per_unit != expected_per_unit:
                raise ValueError("stateful signed execution ledger differs from canonical pair")
            reference_keys = set(expected_per_unit) - {
                f"opcode:0x{scenario.measurement_opcode:02x}"
            }
            for opcode_key in reference_keys:
                coefficient = expected_per_unit[opcode_key]
                raw_delta = target_raw.get(opcode_key, 0) - control_raw.get(
                    opcode_key, 0
                )
                quotient = Fraction(raw_delta, coefficient * count)
                if quotient.denominator != 1 or quotient < 0:
                    raise ValueError(
                        "stateful reference raw-gas quotient is not an exact nonnegative integer"
                    )
                raw_inputs.setdefault(scenario_name, {}).setdefault(
                    opcode_key, set()
                ).add(quotient.numerator)
        target_normalized = target.get("normalized_report")
        control_normalized = control.get("normalized_report")
        target_gas = (
            target_normalized.get("prover_gas")
            if isinstance(target_normalized, Mapping)
            else None
        )
        control_gas = (
            control_normalized.get("prover_gas")
            if isinstance(control_normalized, Mapping)
            else None
        )
        if type(target_gas) is not int or type(control_gas) is not int:
            raise ValueError("stateful Task 4 proverGas observations must be exact integers")
        scenario_deltas = deltas.setdefault(
            scenario_name,
            {
                scenario_count: [None] * manifest.repeats
                for scenario_count in scenario.counts
            },
        )
        if scenario_deltas[count][repeat_index] is not None:
            raise ValueError("stateful Task 4 pair repeat is duplicated")
        scenario_deltas[count][repeat_index] = str(target_gas - control_gas)
        present_scenarios.add(scenario_name)
        pair_ledgers.append(
            {
                "scenario": scenario_name,
                "relation_count": count,
                "repeat_index": repeat_index,
                "ordered_pair_identity": target["ordered_pair_identity"],
                "signed_execution_ledger": expected_per_unit,
                "signed_execution_total": signed_total,
            }
        )

    scenario_data = {}
    typed_references = {}
    reference_evidence = {}
    for scenario_name in sorted(present_scenarios):
        scenario = manifest.scenario(scenario_name)
        ledger = _reference_ledger(scenario)
        reference_keys = set(ledger) - {
            f"opcode:0x{scenario.measurement_opcode:02x}"
        }
        scenario_raw = raw_inputs.get(scenario_name, {})
        if set(scenario_raw) != reference_keys:
            raise ValueError(
                "stateful reference typed input requires positive-count pair evidence"
            )
        typed = {}
        for opcode_key in sorted(reference_keys):
            candidates = scenario_raw[opcode_key]
            if len(candidates) != 1:
                raise ValueError(
                    "stateful reference raw-gas quotient differs across positive-count pairs"
                )
            opcode = int(opcode_key.removeprefix("opcode:0x"), 16)
            model_id = registry.typed_registry.opcode_model_ids[opcode]
            model = (
                registry.typed_registry.models.get(model_id)
                if model_id is not None
                else None
            )
            if model is None:
                raise ValueError("stateful reference opcode is unsupported")
            if model.kind is not ModelKind.STATIC_RAW_GAS:
                raise ValueError(
                    "structured stateful reference lacks exact Task 4 typed input"
                )
            typed[opcode_key] = {
                "artifact_sha256": registry.artifact_sha256,
                "model_id": model_id,
                "model_kind": model.kind.value,
                "input": {"raw_gas": next(iter(candidates))},
            }
        resolved = resolve_signed_reference_cost(
            registry,
            ledger,
            state_opcode_key=f"opcode:0x{scenario.measurement_opcode:02x}",
            typed_inputs=typed,
        )
        scenario_deltas = deltas[scenario_name]
        if any(
            any(value is None for value in scenario_deltas[count])
            for count in scenario.counts
        ):
            raise ValueError("stateful Task 4 required scenario count/repeat is missing")
        scenario_data[scenario_name] = {
            "deltas": {
                count: list(scenario_deltas[count]) for count in scenario.counts
            },
            "signed_execution_ledger": ledger,
            "signed_reference_cost_exact": resolved[
                "signed_reference_cost_exact"
            ],
        }
        typed_references[scenario_name] = typed
        reference_evidence[scenario_name] = resolved
    return {
        "scenario_data": scenario_data,
        "pair_ledgers": pair_ledgers,
        "typed_references": typed_references,
        "reference_evidence": reference_evidence,
    }


def fit_stateful_scenario(
    *,
    scenario: str,
    fit_deltas: Mapping[int, Sequence[str]],
    reference_cost: Fraction,
) -> dict[str, Any]:
    if not isinstance(scenario, str) or not scenario:
        raise ValueError("stateful scenario fit name is invalid")
    if set(fit_deltas) != set(FIT_COUNTS) or any(
        len(fit_deltas[count]) != REPEATS for count in FIT_COUNTS
    ):
        raise ValueError("stateful scenario fit rows differ from the frozen counts/repeats")
    if not isinstance(reference_cost, Fraction):
        raise TypeError("stateful scenario reference cost must be a Fraction")
    points = [
        (
            Fraction(count),
            _canonical_decimal_fraction(
                delta, label=f"stateful scenario {scenario} delta"
            ),
        )
        for count in FIT_COUNTS
        for delta in fit_deltas[count]
    ]
    point_count = Fraction(len(points))
    mean_x = sum((x for x, _ in points), Fraction()) / point_count
    mean_y = sum((y for _, y in points), Fraction()) / point_count
    denominator = sum(((x - mean_x) ** 2 for x, _ in points), Fraction())
    if denominator == 0:
        raise ValueError("stateful scenario fit count denominator is zero")
    slope = sum(
        ((x - mean_x) * (y - mean_y) for x, y in points), Fraction()
    ) / denominator
    intercept = mean_y - slope * mean_x
    absolute = slope - reference_cost
    residuals = [y - (intercept + slope * x) for x, y in points]
    ss_residual = sum((residual * residual for residual in residuals), Fraction())
    ss_total = sum(((y - mean_y) ** 2 for _, y in points), Fraction())
    r2 = Fraction(1) if ss_total == 0 else Fraction(1) - ss_residual / ss_total
    stderr_squared = ss_residual / Fraction(len(points) - 2) / denominator
    relative_stderr_squared = (
        None if slope == 0 else stderr_squared / (slope * slope)
    )
    max_residual = max(abs(residual) for residual in residuals)
    return {
        "scenario": scenario,
        "fit_counts": list(FIT_COUNTS),
        "fit_observation_count": len(points),
        "relative_slope_exact": exact_fraction_payload(slope),
        "nuisance_intercept_exact": exact_fraction_payload(intercept),
        "signed_reference_cost_exact": exact_fraction_payload(reference_cost),
        "absolute_stateful_cost_exact": exact_fraction_payload(absolute),
        "r2_exact": exact_fraction_payload(r2),
        "slope_standard_error_squared_exact": exact_fraction_payload(stderr_squared),
        "relative_slope_standard_error_squared_exact": (
            None
            if relative_stderr_squared is None
            else exact_fraction_payload(relative_stderr_squared)
        ),
        "max_fit_residual_exact": exact_fraction_payload(max_residual),
    }


def _median_fraction(values: Sequence[Fraction]) -> Fraction:
    if not values or len(values) % 2 == 0:
        raise ValueError("stateful median requires a nonempty odd sample count")
    return sorted(values)[len(values) // 2]


def _parse_stateful_delta_table(
    scenario: str, deltas: Mapping[int, Sequence[str]]
) -> dict[int, tuple[Fraction, ...]]:
    if set(deltas) != set(PRIMARY_COUNTS):
        raise ValueError("stateful scenario rows differ from the frozen count inventory")
    parsed = {}
    for count in PRIMARY_COUNTS:
        values = deltas[count]
        if not isinstance(values, Sequence) or isinstance(values, (str, bytes)):
            raise ValueError("stateful scenario repeat rows are invalid")
        if len(values) != REPEATS:
            raise ValueError("stateful scenario requires exactly three repeats")
        parsed[count] = tuple(
            _canonical_decimal_fraction(
                value, label=f"stateful scenario {scenario} count {count} delta"
            )
            for value in values
        )
    return parsed


def _sqrt_fraction_text(value: Fraction) -> str:
    if value < 0:
        raise ValueError("stateful square root input is negative")
    with localcontext(STATEFUL_DECIMAL_CONTEXT):
        projected = (
            Decimal(value.numerator) / Decimal(value.denominator)
        ).sqrt()
    return opcode_gas._decimal_text(projected)


def fit_and_gate_stateful_scenario(
    *,
    scenario: str,
    deltas: Mapping[int, Sequence[str]],
    reference_cost: Fraction,
) -> dict[str, Any]:
    noise_floor = Fraction(CONTROLLED_RUN_NOISE_FLOOR["prover_gas"])
    parsed = _parse_stateful_delta_table(scenario, deltas)
    fit = fit_stateful_scenario(
        scenario=scenario,
        fit_deltas={count: deltas[count] for count in FIT_COUNTS},
        reference_cost=reference_cost,
    )
    slope = replay_exact_fraction(fit["relative_slope_exact"])
    intercept = replay_exact_fraction(fit["nuisance_intercept_exact"])
    absolute = replay_exact_fraction(fit["absolute_stateful_cost_exact"])
    r2 = replay_exact_fraction(fit["r2_exact"])
    relative_stderr_squared_payload = fit[
        "relative_slope_standard_error_squared_exact"
    ]
    relative_stderr_squared = (
        None
        if relative_stderr_squared_payload is None
        else replay_exact_fraction(relative_stderr_squared_payload)
    )
    max_fit_residual = replay_exact_fraction(fit["max_fit_residual_exact"])
    signal = abs(slope) * (max(FIT_COUNTS) - min(FIT_COUNTS))
    residual_signal_ratio = (
        None if signal == 0 else max_fit_residual / signal
    )
    zero_values = parsed[STRUCTURAL_ZERO_COUNT]
    zero_baseline = _median_fraction(zero_values)
    count_zero_ratio = (
        None
        if signal == 0
        else max(abs(value - intercept) for value in zero_values) / signal
    )

    spread_numerators = []
    for count in (*FIT_COUNTS, HOLDOUT_COUNT, CHECKPOINT_COUNT):
        per_event = tuple(
            (delta - zero_baseline) / count for delta in parsed[count]
        )
        spread_numerators.append(max(per_event) - min(per_event))
    repeat_spread = (
        None if absolute == 0 else max(spread_numerators) / abs(absolute)
    )

    def marginal_ape(count: int) -> tuple[Fraction | None, bool]:
        predicted = Fraction(count) * slope
        apes = []
        for delta in parsed[count]:
            observed = delta - intercept
            if observed == 0:
                return None, True
            apes.append(abs(predicted - observed) / abs(observed))
        return max(apes), False

    holdout_ape, zero_holdout = marginal_ape(HOLDOUT_COUNT)
    checkpoint_ape, zero_checkpoint = marginal_ape(CHECKPOINT_COUNT)
    positive_signal = slope > 0 and absolute > 0 and signal > noise_floor
    gate_values: dict[str, tuple[bool, Fraction | None]] = {
        "positive_signal": (positive_signal, signal),
        "r2": (r2 >= Fraction(995, 1000), r2),
        "relative_slope_standard_error": (
            relative_stderr_squared is not None
            and relative_stderr_squared <= Fraction(1, 400),
            relative_stderr_squared,
        ),
        "residual_signal": (
            residual_signal_ratio is not None
            and residual_signal_ratio <= Fraction(1, 50),
            residual_signal_ratio,
        ),
        "count_zero_intercept": (
            count_zero_ratio is not None and count_zero_ratio <= Fraction(1, 50),
            count_zero_ratio,
        ),
        "repeat_spread": (
            repeat_spread is not None and repeat_spread <= Fraction(1, 20),
            repeat_spread,
        ),
        "holdout_ape": (
            holdout_ape is not None and holdout_ape <= Fraction(1, 10),
            holdout_ape,
        ),
        "checkpoint_ape": (
            checkpoint_ape is not None and checkpoint_ape <= Fraction(1, 10),
            checkpoint_ape,
        ),
    }
    gate_thresholds = {
        "positive_signal": noise_floor,
        "r2": Fraction(995, 1000),
        "relative_slope_standard_error": Fraction(1, 400),
        "residual_signal": Fraction(1, 50),
        "count_zero_intercept": Fraction(1, 50),
        "repeat_spread": Fraction(1, 20),
        "holdout_ape": Fraction(1, 10),
        "checkpoint_ape": Fraction(1, 10),
    }
    failures = [name for name, (passed, _value) in gate_values.items() if not passed]
    if signal == 0:
        failures.append("zero_signal")
    if absolute == 0:
        failures.append("zero_absolute_stateful_cost")
    if zero_holdout:
        failures.append("zero_holdout_marginal")
    if zero_checkpoint:
        failures.append("zero_checkpoint_marginal")
    gates = {
        name: {
            "passed": passed,
            "value_exact": None if value is None else exact_fraction_payload(value),
            "threshold_exact": exact_fraction_payload(gate_thresholds[name]),
        }
        for name, (passed, value) in gate_values.items()
    }
    gates["relative_slope_standard_error"]["value_decimal"] = (
        None
        if relative_stderr_squared is None
        else _sqrt_fraction_text(relative_stderr_squared)
    )
    return {
        "scenario": scenario,
        "fit": fit,
        "controlled_run_noise_floor": dict(CONTROLLED_RUN_NOISE_FLOOR),
        "noise_floor_exact": exact_fraction_payload(noise_floor),
        "gates": gates,
        "failures": failures,
        "passed": not failures,
    }


STATEFUL_MODEL_FAMILY_ORDER = (
    "M_fixed",
    "M_access",
    "M_typed",
    "M_raw_gas_diagnostic",
)
STATEFUL_PRODUCTION_MODEL_ORDER = STATEFUL_MODEL_FAMILY_ORDER[:3]
_SSTORE_BRANCHES = (
    "noop",
    "set",
    "clear",
    "reset",
    "dirty_rewrite",
    "restore_original",
)


def _stateful_store_branch(scenario: StatefulScenario) -> str:
    if scenario.operation_kind != "store":
        raise ValueError("stateful SSTORE branch requested for a load")
    if scenario.current_value != scenario.original_value:
        return (
            "restore_original"
            if scenario.new_value == scenario.original_value
            else "dirty_rewrite"
        )
    if scenario.new_value == scenario.current_value:
        return "noop"
    if scenario.current_value == 0:
        return "set"
    if scenario.new_value == 0:
        return "clear"
    return "reset"


def _stateful_model_parameter_order(family: str) -> tuple[str, ...]:
    if family == "M_fixed":
        return ("sload", "sstore")
    if family == "M_access":
        return (
            "sload_warm_body",
            "sload_cold_extra",
            "sstore_warm_body",
            "sstore_cold_extra",
        )
    if family == "M_typed":
        return (
            "sload_warm_body",
            "sload_cold_extra",
            *(f"sstore_branch:{branch}" for branch in _SSTORE_BRANCHES),
            "sstore_cold_extra",
        )
    if family == "M_raw_gas_diagnostic":
        return ("alpha", "beta")
    raise ValueError(f"unknown stateful model family: {family}")


def _stateful_model_features(
    family: str, scenario: StatefulScenario
) -> dict[str, Fraction]:
    if family == "M_fixed":
        return {
            "sload" if scenario.operation_kind == "load" else "sstore": Fraction(1)
        }
    if family in {"M_access", "M_typed"}:
        if scenario.operation_kind == "load":
            features = {"sload_warm_body": Fraction(1)}
            if scenario.access == "cold":
                features["sload_cold_extra"] = Fraction(1)
            return features
        if family == "M_access":
            features = {"sstore_warm_body": Fraction(1)}
        else:
            features = {
                f"sstore_branch:{_stateful_store_branch(scenario)}": Fraction(1)
            }
        if scenario.access == "cold":
            features["sstore_cold_extra"] = Fraction(1)
        return features
    if family == "M_raw_gas_diagnostic":
        return {
            "alpha": Fraction(1),
            "beta": Fraction(scenario.target_raw_gas),
        }
    raise ValueError(f"unknown stateful model family: {family}")


def _solve_fraction_linear_system(
    matrix: Sequence[Sequence[Fraction]], values: Sequence[Fraction]
) -> tuple[Fraction, ...]:
    size = len(values)
    if len(matrix) != size or any(len(row) != size for row in matrix):
        raise ValueError("stateful exact solver matrix is not square")
    augmented = [list(row) + [value] for row, value in zip(matrix, values)]
    for column in range(size):
        pivot = next(
            (row for row in range(column, size) if augmented[row][column] != 0),
            None,
        )
        if pivot is None:
            raise ValueError("stateful model fit is rank deficient")
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        augmented[column] = [value / divisor for value in augmented[column]]
        for row in range(size):
            if row == column:
                continue
            factor = augmented[row][column]
            if factor:
                augmented[row] = [
                    actual - factor * source
                    for actual, source in zip(augmented[row], augmented[column])
                ]
    return tuple(augmented[index][-1] for index in range(size))


def _solve_fraction_least_squares(
    matrix: Sequence[Sequence[Fraction]], values: Sequence[Fraction]
) -> tuple[Fraction, ...]:
    if not matrix or len(matrix) != len(values):
        raise ValueError("stateful exact least-squares input is empty or mismatched")
    width = len(matrix[0])
    if width == 0 or any(len(row) != width for row in matrix):
        raise ValueError("stateful exact least-squares rows differ")
    normal = [
        [
            sum((row[left] * row[right] for row in matrix), Fraction())
            for right in range(width)
        ]
        for left in range(width)
    ]
    projection = [
        sum((row[column] * value for row, value in zip(matrix, values)), Fraction())
        for column in range(width)
    ]
    return _solve_fraction_linear_system(normal, projection)


def _validate_stateful_scenario_data(
    manifest: StatefulCampaignManifest,
    scenario_data: Mapping[str, Mapping[str, Any]],
) -> dict[str, dict[str, Any]]:
    expected = {scenario.name for scenario in manifest.scenarios}
    if not isinstance(scenario_data, Mapping) or set(scenario_data) != expected:
        raise ValueError("stateful model required scenario inventory is incomplete")
    validated = {}
    for scenario in manifest.scenarios:
        row = scenario_data[scenario.name]
        if not isinstance(row, Mapping) or set(row) != {
            "deltas",
            "signed_execution_ledger",
            "signed_reference_cost_exact",
        }:
            raise ValueError("stateful model scenario data schema differs")
        ledger = row["signed_execution_ledger"]
        if ledger != _reference_ledger(scenario):
            raise ValueError("stateful model signed execution ledger differs")
        deltas = row["deltas"]
        if not isinstance(deltas, Mapping) or set(deltas) != set(scenario.counts):
            raise ValueError("stateful model scenario count inventory differs")
        parsed = {}
        for count in scenario.counts:
            repeats = deltas[count]
            if (
                not isinstance(repeats, Sequence)
                or isinstance(repeats, (str, bytes))
                or len(repeats) != manifest.repeats
            ):
                raise ValueError("stateful model scenario repeats differ")
            parsed[count] = tuple(
                _canonical_decimal_fraction(
                    value,
                    label=f"stateful model {scenario.name} count {count} delta",
                )
                for value in repeats
            )
        validated[scenario.name] = {
            "deltas": parsed,
            "reference_cost": replay_exact_fraction(
                row["signed_reference_cost_exact"]
            ),
            "signed_execution_ledger": dict(sorted(ledger.items())),
        }
    return validated


def _fit_stateful_model_family(
    manifest: StatefulCampaignManifest,
    data: Mapping[str, Mapping[str, Any]],
    family: str,
) -> dict[str, Any]:
    primary = tuple(scenario for scenario in manifest.scenarios if not scenario.diagnostic)
    parameter_order = _stateful_model_parameter_order(family)
    intercept_order = tuple(scenario.name for scenario in primary)
    columns = (*intercept_order, *parameter_order)
    matrix = []
    values = []
    fit_projection = []
    for scenario in primary:
        reference_cost = data[scenario.name]["reference_cost"]
        features = _stateful_model_features(family, scenario)
        for count in manifest.fit_counts:
            for repeat_index, delta in enumerate(data[scenario.name]["deltas"][count]):
                row = [Fraction(int(name == scenario.name)) for name in intercept_order]
                row.extend(
                    Fraction(count) * features.get(name, Fraction())
                    for name in parameter_order
                )
                matrix.append(row)
                values.append(delta - Fraction(count) * reference_cost)
                fit_projection.append(
                    {
                        "scenario": scenario.name,
                        "count": count,
                        "repeat_index": repeat_index,
                        "delta": exact_fraction_payload(delta),
                        "signed_reference_cost": exact_fraction_payload(reference_cost),
                    }
                )
    solution = _solve_fraction_least_squares(matrix, values)
    solved = dict(zip(columns, solution))
    residuals = [
        value - sum((coefficient * solved[name] for name, coefficient in zip(columns, row)), Fraction())
        for row, value in zip(matrix, values)
    ]
    return {
        "family": family,
        "parameter_order": list(parameter_order),
        "parameters_exact": {
            name: exact_fraction_payload(solved[name]) for name in parameter_order
        },
        "nuisance_intercepts_exact": {
            name: exact_fraction_payload(solved[name]) for name in intercept_order
        },
        "fit_observation_count": len(matrix),
        "fit_input_sha256": sha256_bytes(canonical_json(fit_projection)),
        "max_fit_residual_exact": exact_fraction_payload(
            max((abs(value) for value in residuals), default=Fraction())
        ),
    }


def _model_cost_from_frozen_fit(
    model: Mapping[str, Any], scenario: StatefulScenario
) -> Fraction:
    features = _stateful_model_features(model["family"], scenario)
    parameters = {
        name: replay_exact_fraction(value)
        for name, value in model["parameters_exact"].items()
    }
    return sum(
        (coefficient * parameters[name] for name, coefficient in features.items()),
        Fraction(),
    )


def _max_model_ape(
    *,
    scenario: StatefulScenario,
    count: int,
    deltas: Sequence[Fraction],
    intercept: Fraction,
    predicted_absolute: Fraction,
    reference_cost: Fraction,
) -> Fraction | None:
    predicted = Fraction(count) * (predicted_absolute + reference_cost)
    apes = []
    for delta in deltas:
        observed = delta - intercept
        if observed == 0:
            return None
        apes.append(abs(predicted - observed) / abs(observed))
    return max(apes)


def _high_limb_model_comparison(
    *,
    scenario: StatefulScenario,
    count: int,
    zero_deltas: Sequence[Fraction],
    checkpoint_deltas: Sequence[Fraction],
    predicted_absolute: Fraction,
    reference_cost: Fraction,
) -> dict[str, Any]:
    if not scenario.diagnostic or scenario.low_variant is None:
        raise ValueError("stateful high-limb comparison requires a diagnostic scenario")
    if count <= 0 or not zero_deltas or not checkpoint_deltas:
        raise ValueError("stateful high-limb comparison inputs are invalid")
    zero_intercept = _median_fraction(zero_deltas)
    observed_relative = (
        _median_fraction(checkpoint_deltas) - zero_intercept
    ) / count
    observed_absolute = observed_relative - reference_cost
    consistency = (
        None
        if predicted_absolute == 0
        else abs(observed_absolute - predicted_absolute) / abs(predicted_absolute)
    )
    ape = _max_model_ape(
        scenario=scenario,
        count=count,
        deltas=checkpoint_deltas,
        intercept=zero_intercept,
        predicted_absolute=predicted_absolute,
        reference_cost=reference_cost,
    )
    return {
        "scenario": scenario.name,
        "low_variant": scenario.low_variant,
        "structural_zero_median_exact": exact_fraction_payload(zero_intercept),
        "observed_relative_cost_exact": exact_fraction_payload(observed_relative),
        "signed_reference_cost_exact": exact_fraction_payload(reference_cost),
        "observed_absolute_cost_exact": exact_fraction_payload(observed_absolute),
        "model_absolute_cost_exact": exact_fraction_payload(predicted_absolute),
        "ape_exact": None if ape is None else exact_fraction_payload(ape),
        "consistency_error_exact": (
            None if consistency is None else exact_fraction_payload(consistency)
        ),
        "zero_marginal_denominator": ape is None,
        "zero_model_cost_denominator": consistency is None,
    }


def fit_stateful_model_report(
    manifest: StatefulCampaignManifest,
    scenario_data: Mapping[str, Mapping[str, Any]],
) -> dict[str, Any]:
    if dict(manifest.controlled_run_noise_floor) != dict(
        CONTROLLED_RUN_NOISE_FLOOR
    ):
        raise ValueError("stateful controlled-run noise floor contract differs")
    data = _validate_stateful_scenario_data(manifest, scenario_data)
    primary = tuple(scenario for scenario in manifest.scenarios if not scenario.diagnostic)
    diagnostics = tuple(scenario for scenario in manifest.scenarios if scenario.diagnostic)

    # Freeze every declared family from fit rows before reading any decision split.
    frozen_models = {
        family: _fit_stateful_model_family(manifest, data, family)
        for family in STATEFUL_MODEL_FAMILY_ORDER
    }
    frozen_fit_sha256 = sha256_bytes(canonical_json(frozen_models))

    scenario_reports = {
        scenario.name: fit_and_gate_stateful_scenario(
            scenario=scenario.name,
            deltas={
                count: [
                    _fraction_decimal_text(value)
                    for value in data[scenario.name]["deltas"][count]
                ]
                for count in manifest.primary_counts
            },
            reference_cost=data[scenario.name]["reference_cost"],
        )
        for scenario in primary
    }
    measurement_gates_pass = all(
        report["passed"] for report in scenario_reports.values()
    )

    model_reports = {}
    for family in STATEFUL_MODEL_FAMILY_ORDER:
        frozen = frozen_models[family]
        holdout_apes = []
        checkpoint_apes = []
        high_limb_rows = []
        zero_denominator = False
        zero_high_model_denominator = False
        for scenario in primary:
            intercept = replay_exact_fraction(
                frozen["nuisance_intercepts_exact"][scenario.name]
            )
            predicted_absolute = _model_cost_from_frozen_fit(frozen, scenario)
            reference_cost = data[scenario.name]["reference_cost"]
            holdout = _max_model_ape(
                scenario=scenario,
                count=manifest.holdout_count,
                deltas=data[scenario.name]["deltas"][manifest.holdout_count],
                intercept=intercept,
                predicted_absolute=predicted_absolute,
                reference_cost=reference_cost,
            )
            checkpoint = _max_model_ape(
                scenario=scenario,
                count=manifest.checkpoint_count,
                deltas=data[scenario.name]["deltas"][manifest.checkpoint_count],
                intercept=intercept,
                predicted_absolute=predicted_absolute,
                reference_cost=reference_cost,
            )
            if holdout is None or checkpoint is None:
                zero_denominator = True
            else:
                holdout_apes.append(holdout)
                checkpoint_apes.append(checkpoint)
        for scenario in diagnostics:
            low_scenario = manifest.scenario(scenario.low_variant)
            if _stateful_model_features(
                family, scenario
            ) != _stateful_model_features(family, low_scenario):
                raise ValueError(
                    "stateful high-limb scenario differs from its low model features"
                )
            predicted_absolute = _model_cost_from_frozen_fit(
                frozen, low_scenario
            )
            comparison = _high_limb_model_comparison(
                scenario=scenario,
                count=manifest.checkpoint_count,
                zero_deltas=data[scenario.name]["deltas"][
                    manifest.structural_zero_count
                ],
                checkpoint_deltas=data[scenario.name]["deltas"][
                    manifest.checkpoint_count
                ],
                predicted_absolute=predicted_absolute,
                reference_cost=data[scenario.name]["reference_cost"],
            )
            if comparison["zero_marginal_denominator"]:
                zero_denominator = True
            if comparison["zero_model_cost_denominator"]:
                zero_high_model_denominator = True
            high_limb_rows.append(comparison)
        max_holdout = max(holdout_apes, default=Fraction())
        max_checkpoint = max(checkpoint_apes, default=Fraction())
        high_values = [
            replay_exact_fraction(row["ape_exact"])
            for row in high_limb_rows
            if row["ape_exact"] is not None
        ]
        max_high = max(high_values, default=Fraction())
        consistency_values = [
            replay_exact_fraction(row["consistency_error_exact"])
            for row in high_limb_rows
            if row["consistency_error_exact"] is not None
        ]
        max_consistency = max(consistency_values, default=Fraction())
        reasons = []
        if not measurement_gates_pass:
            reasons.append("scenario_measurement_gates")
        if zero_denominator:
            reasons.append("zero_model_ape_denominator")
        if zero_high_model_denominator:
            reasons.append("zero_high_limb_model_denominator")
        if max_holdout > Fraction(1, 10):
            reasons.append("holdout_ape")
        if max_checkpoint > Fraction(1, 10):
            reasons.append("checkpoint_ape")
        if max_high > Fraction(1, 10):
            reasons.append("high_limb_ape")
        if max_consistency > Fraction(1, 10):
            reasons.append("high_limb_consistency")
        diagnostic_only = family == "M_raw_gas_diagnostic"
        if diagnostic_only:
            reasons.append("diagnostic_only")
        cold_increment_diagnostic = None
        if family in {"M_access", "M_typed"}:
            parameters = {
                name: replay_exact_fraction(value)
                for name, value in frozen["parameters_exact"].items()
            }
            load_cold = parameters["sload_cold_extra"]
            store_cold = parameters["sstore_cold_extra"]
            cold_increment_diagnostic = {
                "sload_cold_extra_exact": exact_fraction_payload(load_cold),
                "sstore_cold_extra_exact": exact_fraction_payload(store_cold),
                "difference_exact": exact_fraction_payload(load_cold - store_cold),
                "equal": load_cold == store_cold,
            }
        model_reports[family] = {
            **frozen,
            "diagnostic_only": diagnostic_only,
            "eligible": not reasons,
            "rejection_reasons": reasons,
            "maximum_ape_threshold_exact": exact_fraction_payload(Fraction(1, 10)),
            "maximum_holdout_ape_exact": exact_fraction_payload(max_holdout),
            "maximum_checkpoint_ape_exact": exact_fraction_payload(max_checkpoint),
            "maximum_high_limb_ape_exact": exact_fraction_payload(max_high),
            "maximum_high_limb_consistency_error_exact": exact_fraction_payload(
                max_consistency
            ),
            "cold_increment_diagnostic": cold_increment_diagnostic,
            "high_limb_comparisons": high_limb_rows,
        }

    selected = next(
        (
            family
            for family in STATEFUL_PRODUCTION_MODEL_ORDER
            if model_reports[family]["eligible"]
        ),
        None,
    )
    return {
        "schema_version": 1,
        "purpose": "stateful_opcode_model_comparison",
        "candidate_eligible": False,
        "controlled_run_noise_floor": dict(CONTROLLED_RUN_NOISE_FLOOR),
        "frozen_fit_family_order": list(STATEFUL_MODEL_FAMILY_ORDER),
        "frozen_fit_sha256": frozen_fit_sha256,
        "frozen_fit_models": frozen_models,
        "scenario_reports": scenario_reports,
        "model_reports": model_reports,
        "selection": {
            "order": list(STATEFUL_PRODUCTION_MODEL_ORDER),
            "selected_model": selected,
            "status": "selected" if selected is not None else "no_eligible_model",
        },
    }


def _fit_stateful_task4_rows(
    manifest: StatefulCampaignManifest,
    rows: Sequence[Mapping[str, Any]],
    registry_artifact: Mapping[str, Any],
) -> dict[str, Any]:
    registry = load_stateful_reference_registry(
        registry_artifact,
        expected_artifact_sha256=manifest.reference_registry["artifact_sha256"],
    )
    extracted = extract_stateful_pair_observations(manifest, rows, registry)
    report = fit_stateful_model_report(
        manifest,
        extracted["scenario_data"],
    )
    pair_ledgers = extracted["pair_ledgers"]
    return {
        **report,
        "reference_registry": dict(manifest.reference_registry),
        "pair_ledgers": pair_ledgers,
        "pair_ledger_sha256": sha256_bytes(canonical_json(pair_ledgers)),
        "typed_references": extracted["typed_references"],
        "reference_evidence": extracted["reference_evidence"],
    }


def _validate_verified_task4_row_identities(
    manifest: StatefulCampaignManifest,
    rows: Sequence[Mapping[str, Any]],
) -> None:
    specs = stateful_campaign_row_specs(manifest)
    if len(rows) != len(specs):
        raise ValueError("verified stateful Task 4 row inventory differs")
    grouped: dict[tuple[str, int, int], dict[str, Mapping[str, Any]]] = {}
    for spec, row in zip(specs, rows):
        if not isinstance(row, Mapping):
            raise ValueError("verified stateful Task 4 row is not an object")
        if (
            row.get("scenario") != spec.scenario
            or row.get("lane") != spec.lane
            or row.get("relation_count") != spec.relation_count
            or row.get("repeat_index") != spec.repeat_index
            or row.get("logical_identity") != spec.logical_identity
        ):
            raise ValueError("verified stateful Task 4 logical identity differs")
        expected_row_identity = stateful_execution_row_identity(
            scenario=spec.scenario,
            lane=spec.lane,
            relation_count=spec.relation_count,
            repeat_index=spec.repeat_index,
            backend_input_sha256=row.get("backend_input_sha256"),
            elf_sha256=row.get("elf_sha256"),
            launcher_sha256=row.get("launcher_sha256"),
            trace_sha256=row.get("trace_sha256"),
        )
        if row.get("row_identity") != expected_row_identity:
            raise ValueError("verified stateful Task 4 execution identity differs")
        formal = row.get("formal_report")
        if (
            not isinstance(formal, Mapping)
            or row.get("formal_report_sha256")
            != sha256_bytes(canonical_json(formal))
        ):
            raise ValueError("verified stateful Task 4 formal report hash differs")
        normalized = row.get("normalized_report")
        formal_gas = formal.get("prover_gas", formal.get("gas"))
        normalized_gas = (
            normalized.get("prover_gas")
            if isinstance(normalized, Mapping)
            else None
        )
        if (
            type(formal_gas) is not int
            or formal_gas <= 0
            or normalized_gas != formal_gas
        ):
            raise ValueError("verified stateful Task 4 proverGas binding differs")
        grouped.setdefault(
            (spec.scenario, spec.relation_count, spec.repeat_index), {}
        )[spec.lane] = row

    for (scenario_name, count, repeat_index), lanes in grouped.items():
        if set(lanes) != {"target", "control"}:
            raise ValueError("verified stateful Task 4 ordered pair is incomplete")
        scenario = manifest.scenario(scenario_name)
        expected_pair = stateful_ordered_pair_identity(
            scenario=scenario_name,
            measurement_opcode=scenario.measurement_opcode,
            relation_count=count,
            repeat_index=repeat_index,
            target_hash=lanes["target"]["backend_input_sha256"],
            control_hash=lanes["control"]["backend_input_sha256"],
        )
        if any(
            row.get("ordered_pair_identity") != expected_pair
            for row in lanes.values()
        ):
            raise ValueError("verified stateful Task 4 ordered pair identity differs")


def _validate_sealed_task4_row_evidence(
    rows: Sequence[Mapping[str, Any]], campaign_identity: Mapping[str, Any]
) -> None:
    expected_keys = {
        "schema_version",
        "purpose",
        "scenario",
        "lane",
        "relation_count",
        "repeat_index",
        "logical_identity",
        "row_identity",
        "ordered_pair_identity",
        "backend_input_sha256",
        "elf_sha256",
        "launcher_sha256",
        "trace_sha256",
        "semantic_check_sha256",
        "identity_evidence_sha256",
        "guest_input_json_file_sha256",
        "formal_report_sha256",
        "formal_report",
        "normalized_report",
    }
    elf = campaign_identity.get("guest_elf")
    launcher = campaign_identity.get("guest_launcher")
    expected_elf = elf.get("file_sha256") if isinstance(elf, Mapping) else None
    expected_launcher = (
        launcher.get("file_sha256") if isinstance(launcher, Mapping) else None
    )
    for row in rows:
        formal = row.get("formal_report")
        trace = formal.get("controlled_trace") if isinstance(formal, Mapping) else None
        semantic = trace.get("semantic_check") if isinstance(trace, Mapping) else None
        try:
            for field in (
                "backend_input_sha256",
                "elf_sha256",
                "launcher_sha256",
                "trace_sha256",
                "semantic_check_sha256",
                "identity_evidence_sha256",
                "guest_input_json_file_sha256",
                "formal_report_sha256",
            ):
                _require_sha256(row.get(field), f"sealed Task 4 row {field}")
        except ValueError as error:
            raise ValueError("sealed stateful Task 4 row evidence differs") from error
        if (
            set(row) != expected_keys
            or row.get("schema_version") != 1
            or row.get("purpose") != PURPOSE
            or not isinstance(trace, Mapping)
            or not isinstance(semantic, Mapping)
            or row.get("backend_input_sha256")
            != trace.get("backend_input_sha256")
            or row.get("elf_sha256") != expected_elf
            or row.get("launcher_sha256") != expected_launcher
            or row.get("trace_sha256") != sha256_bytes(canonical_json(trace))
            or row.get("semantic_check_sha256")
            != sha256_bytes(canonical_json(semantic))
        ):
            raise ValueError("sealed stateful Task 4 row evidence differs")


def fit_stateful_task4_run(
    *,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    run: pathlib.Path,
    identity_replayer=None,
    execution_identity_loader=opcode_gas.validate_calibration_execution_identity,
    launcher_validator=opcode_gas.validate_calibration_guest_launcher,
) -> dict[str, Any]:
    verification = verify_stateful_opcode_campaign(
        manifest_path=manifest_path,
        calibration_run=calibration_run,
        fixtures_root=fixtures_root,
        guest_launcher=guest_launcher,
        elf=elf,
        run=run,
        identity_replayer=identity_replayer,
        execution_identity_loader=execution_identity_loader,
        launcher_validator=launcher_validator,
    )
    rows_path = _regular_file(run / "rows.jsonl", label="terminal row ledger")
    rows_bytes = rows_path.read_bytes()
    if sha256_bytes(rows_bytes) != verification["row_ledger_sha256"]:
        raise ValueError("verified stateful Task 4 row ledger hash differs")
    rows = list(opcode_gas.iter_jsonl(rows_path))
    if rows_bytes != b"".join(canonical_json(row) + b"\n" for row in rows):
        raise ValueError("verified stateful Task 4 row ledger is not canonical JSONL")

    decisions_path = _regular_file(
        run / "decisions.json", label="terminal decision ledger"
    )
    seal_path = _regular_file(
        run / "decisions.sha256", label="terminal decision seal"
    )
    decisions_bytes = decisions_path.read_bytes()
    try:
        decisions = json.loads(decisions_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("verified stateful Task 4 decisions are invalid JSON") from error
    if (
        not isinstance(decisions, Mapping)
        or set(decisions)
        != {
            "schema_version",
            "purpose",
            "status",
            "campaign_identity_sha256",
            "row_count",
            "pair_count",
            "repeats",
            "row_ledger_sha256",
        }
        or decisions_bytes != canonical_json(decisions) + b"\n"
        or decisions.get("schema_version") != 1
        or decisions.get("purpose") != PURPOSE
        or decisions.get("status") != "complete"
        or decisions.get("campaign_identity_sha256")
        != verification["campaign_identity_sha256"]
        or decisions.get("row_ledger_sha256") != verification["row_ledger_sha256"]
        or decisions.get("row_count") != verification["row_count"]
        or decisions.get("pair_count") != verification["pair_count"]
        or decisions.get("repeats") != REPEATS
    ):
        raise ValueError("verified stateful Task 4 decisions differ")
    seal_bytes = seal_path.read_bytes()
    if seal_bytes != (sha256_bytes(decisions_bytes) + "\n").encode():
        raise ValueError("verified stateful Task 4 decision seal differs")

    manifest = load_stateful_campaign_manifest(manifest_path)
    _validate_verified_task4_row_identities(manifest, rows)
    registry_path = _regular_file(
        opcode_gas.REPO_ROOT / manifest.reference_registry["path"],
        label="stateful source registry",
    )
    try:
        registry_artifact = json.loads(registry_path.read_bytes())
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("stateful source registry is invalid JSON") from error
    report = _fit_stateful_task4_rows(manifest, rows, registry_artifact)
    return {
        **report,
        "task4_provenance": {
            "campaign_identity_sha256": verification[
                "campaign_identity_sha256"
            ],
            "row_ledger_sha256": verification["row_ledger_sha256"],
            "terminal_artifact_file_sha256": {
                "rows.jsonl": sha256_bytes(rows_bytes),
                "decisions.json": sha256_bytes(decisions_bytes),
                "decisions.sha256": sha256_bytes(seal_bytes),
            },
        },
    }


def _stateful_result_ownership() -> dict[str, Any]:
    return {
        "measured": STATEFUL_RESULT_OWNERSHIP["measured"],
        "excluded": list(STATEFUL_RESULT_OWNERSHIP["excluded"]),
    }


def _stateful_result_identity(
    *,
    implementation_revision: str,
    campaign_identity_sha256: str,
    row_ledger_sha256: str,
    manifest_file_sha256: str,
    calibration_identity_file_sha256: str,
    campaign_identity_file_sha256: str,
    decisions_file_sha256: str,
    decisions_seal_file_sha256: str,
    registry_artifact_sha256: str,
    registry_file_sha256: str,
) -> dict[str, Any]:
    return {
        "analysis_schema_version": STATEFUL_RESULT_SCHEMA_VERSION,
        "analysis_implementation_revision": implementation_revision,
        "task4": {
            "campaign_identity_sha256": campaign_identity_sha256,
            "row_ledger_sha256": row_ledger_sha256,
        },
        "input_hashes": {
            "campaign_manifest_file_sha256": manifest_file_sha256,
            "calibration_identity_file_sha256": calibration_identity_file_sha256,
            "campaign_identity_file_sha256": campaign_identity_file_sha256,
            "campaign_decisions_file_sha256": decisions_file_sha256,
            "campaign_decisions_seal_file_sha256": decisions_seal_file_sha256,
            "source_registry_artifact_sha256": registry_artifact_sha256,
            "source_registry_file_sha256": registry_file_sha256,
        },
    }


def _stateful_result_envelope(
    identity: Mapping[str, Any], model_report_bytes: bytes
) -> dict[str, Any]:
    identity_sha256 = sha256_bytes(canonical_json(identity))
    ownership = _stateful_result_ownership()
    envelope: dict[str, Any] = {
        "schema_version": STATEFUL_RESULT_SCHEMA_VERSION,
        "purpose": STATEFUL_RESULT_PURPOSE,
        "status": "sealed",
        "result_id": identity_sha256[:24],
        "result_identity_sha256": identity_sha256,
        "result_identity": dict(identity),
        "ownership": ownership,
        "candidate_eligible": False,
        "proposal_validated": False,
        "production_registry_modified": False,
        "output_hashes": {
            "model_report_file_sha256": sha256_bytes(model_report_bytes),
            "ownership_sha256": sha256_bytes(canonical_json(ownership)),
        },
    }
    envelope["artifact_sha256"] = sha256_bytes(canonical_json(envelope))
    return envelope


def _load_canonical_json_bytes(
    raw: bytes, *, label: str, pretty: bool
) -> Mapping[str, Any]:
    try:
        payload = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{label} is invalid JSON") from error
    if not isinstance(payload, Mapping):
        raise ValueError(f"{label} must be a JSON object")
    expected = (
        opcode_gas._canonical_json_file_bytes(payload)
        if pretty
        else canonical_json(payload) + b"\n"
    )
    if raw != expected:
        raise ValueError(f"{label} is not canonical JSON")
    return payload


@functools.lru_cache(maxsize=1)
def _current_stateful_calibration_contract_bytes() -> bytes:
    controlled_manifest = (
        opcode_gas.REPO_ROOT
        / "experiments/opcode-gas/manifests/sp1-calibration-v1.toml"
    )
    controlled_data = opcode_gas.tomllib.loads(controlled_manifest.read_text())
    schedule = opcode_gas.current_uzen_schedule()
    workspace = opcode_gas.tomllib.loads(
        (opcode_gas.REPO_ROOT / "Cargo.toml").read_text()
    )
    guest_artifacts = {
        str(path.relative_to(opcode_gas.REPO_ROOT)): opcode_gas.sha256_file(path)
        for path in sorted(
            (opcode_gas.REPO_ROOT / "crates/guests/elf").glob("sp1*")
        )
        if path.is_file()
        and (path.name.endswith(".elf") or path.name.endswith(".vk.bin"))
    }
    contract = {
        "alethia_reth_revision": workspace["workspace"]["dependencies"][
            "alethia-reth-chainspec"
        ]["rev"],
        "rust_version": opcode_gas._rust_version(),
        "sp1_sdk_version": opcode_gas._locked_package_version("sp1-sdk"),
        "controlled_manifest_sha256": opcode_gas.sha256_file(
            controlled_manifest
        ),
        "controlled_manifest_rows_sha256": (
            opcode_gas.controlled_manifest_rows_sha256(controlled_manifest)
        ),
        "complete_schedule_sha256": opcode_gas.schedule_sha256(schedule),
        "guest_artifacts": guest_artifacts,
        "guest_artifacts_sha256": sha256_bytes(canonical_json(guest_artifacts)),
        "normalization_reference_key": "opcode:0x01",
        "sp1_execution_parameters": opcode_gas.sp1_execution_parameters(),
        "primary_metric": "proverGas",
        "sp1_instruction_count": "secondary_non_gating",
        "workload_identity_schema_version": 1,
        "workload_canonicalization": "sha256(canonical_json(workload_spec))",
        "primary_formulas": {
            "candidate_cost": "g_p(k) / r(k)",
            "candidate_multiplier": "c_p(k) / c_p(opcode:0x01)",
        },
        "q_formula": list(opcode_gas.Q_FORMULA),
        "out_of_fit_checkpoint": {
            "mapping": opcode_gas.OUT_OF_FIT_CHECKPOINTS,
            "ape_max": 0.10,
        },
        "quality_gates": {"checkpoint_ape_max": 0.10},
        "bridge": {
            "bridge_key_ids": controlled_data["bridge_key_ids"],
            "model": "through_origin_equal_key_median",
            "controlled_ape_max": "0.10",
            "proposal_ape_max": "0.10",
            "missing_data": "insufficient_data_is_sealable_and_non_gating",
        },
        "version_identity": opcode_gas.calibration_version_identity(schedule),
    }
    return canonical_json(contract)


def _current_stateful_calibration_contract() -> dict[str, Any]:
    # Return a fresh tree so a caller cannot mutate the cached authority.
    return json.loads(_current_stateful_calibration_contract_bytes())


def _validate_sealed_calibration_identity(identity: Mapping[str, Any]) -> None:
    if not isinstance(identity, Mapping):
        raise ValueError("sealed stateful calibration identity must be an object")
    contract = _current_stateful_calibration_contract()
    expected_keys = {
        "implementation_revision",
        "guest_launcher_sha256",
        *contract.keys(),
    }
    try:
        _require_sha256(
            identity.get("guest_launcher_sha256"),
            "sealed calibration guest launcher",
        )
    except ValueError as error:
        raise ValueError("sealed stateful calibration identity differs") from error
    revision = identity.get("implementation_revision")
    if (
        set(identity) != expected_keys
        or not isinstance(revision, str)
        or re.fullmatch(r"[0-9a-f]{40}", revision) is None
        or any(identity.get(key) != value for key, value in contract.items())
    ):
        raise ValueError("sealed stateful calibration identity differs")


def _canonical_fixture_inventory_sha256(
    manifest: StatefulCampaignManifest,
) -> str:
    fixture_rows = []
    seen = set()
    for spec in stateful_campaign_row_specs(manifest):
        key = spec.scenario, spec.relation_count, spec.lane
        if key in seen:
            continue
        seen.add(key)
        fixture = generate_stateful_fixture(
            manifest, spec.scenario, lane=spec.lane, count=spec.relation_count
        )
        case_bytes = canonical_json(fixture["case_record"]) + b"\n"
        input_bytes = canonical_json(fixture["guest_input"]) + b"\n"
        fixture_rows.append(
            {
                "scenario": spec.scenario,
                "relation_count": spec.relation_count,
                "lane": spec.lane,
                "case_sha256": sha256_bytes(case_bytes),
                "guest_input_sha256": sha256_bytes(input_bytes),
            }
        )
    return sha256_bytes(canonical_json(fixture_rows))


def _validate_sealed_campaign_identity(
    campaign_identity: Mapping[str, Any],
    calibration_identity: Mapping[str, Any],
    manifest: StatefulCampaignManifest,
    *,
    manifest_file_sha256: str,
    registry_artifact_sha256: str,
    registry_file_sha256: str,
) -> None:
    expected_keys = {
        "schema_version",
        "purpose",
        "implementation_revision",
        "calibration_id",
        "calibration_identity_sha256",
        "manifest",
        "execution",
        "guest_launcher",
        "guest_elf",
        "source_registry",
        "fixtures_root",
        "fixture_inventory_sha256",
        "row_inventory_sha256",
        "row_count",
        "repeats",
    }
    calibration_sha256 = sha256_bytes(canonical_json(calibration_identity))
    expected_calibration_id = calibration_sha256[:24]
    guest_artifacts = calibration_identity.get("guest_artifacts")
    guest_launcher_sha256 = calibration_identity.get("guest_launcher_sha256")
    specs = stateful_campaign_row_specs(manifest)
    manifest_identity = campaign_identity.get("manifest")
    launcher_identity = campaign_identity.get("guest_launcher")
    elf_identity = campaign_identity.get("guest_elf")
    registry_identity = campaign_identity.get("source_registry")
    fixture_root = campaign_identity.get("fixtures_root")
    launcher_path = (
        launcher_identity.get("path")
        if isinstance(launcher_identity, Mapping)
        else None
    )
    fixture_path = pathlib.PurePosixPath(fixture_root) if isinstance(
        fixture_root, str
    ) else None
    launcher_relative = pathlib.PurePosixPath(launcher_path) if isinstance(
        launcher_path, str
    ) else None
    digest_fields = (
        campaign_identity.get("fixture_inventory_sha256"),
        campaign_identity.get("row_inventory_sha256"),
        guest_launcher_sha256,
    )
    try:
        for index, digest in enumerate(digest_fields):
            _require_sha256(digest, f"stateful campaign digest {index}")
    except ValueError as error:
        raise ValueError("stateful result campaign identity differs") from error
    if (
        set(campaign_identity) != expected_keys
        or campaign_identity.get("schema_version") != 1
        or campaign_identity.get("purpose") != PURPOSE
        or campaign_identity.get("implementation_revision")
        != calibration_identity.get("implementation_revision")
        or campaign_identity.get("calibration_id") != expected_calibration_id
        or campaign_identity.get("calibration_identity_sha256")
        != calibration_sha256
        or manifest_identity
        != {
            "path": "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json",
            "file_sha256": manifest_file_sha256,
        }
        or campaign_identity.get("execution") != dict(manifest.execution)
        or not isinstance(launcher_identity, Mapping)
        or set(launcher_identity) != {"path", "file_sha256"}
        or launcher_identity.get("file_sha256") != guest_launcher_sha256
        or launcher_relative is None
        or launcher_relative.is_absolute()
        or ".." in launcher_relative.parts
        or str(launcher_relative) != launcher_path
        or not isinstance(elf_identity, Mapping)
        or elf_identity
        != {
            "path": EXECUTION_CONTRACT["elf_path"],
            "file_sha256": (
                guest_artifacts.get(EXECUTION_CONTRACT["elf_path"])
                if isinstance(guest_artifacts, Mapping)
                else None
            ),
        }
        or registry_identity
        != {
            **dict(manifest.reference_registry),
            "artifact_sha256": registry_artifact_sha256,
            "file_sha256": registry_file_sha256,
        }
        or fixture_path is None
        or fixture_path.is_absolute()
        or ".." in fixture_path.parts
        or str(fixture_path) != fixture_root
        or campaign_identity.get("fixture_inventory_sha256")
        != _canonical_fixture_inventory_sha256(manifest)
        or campaign_identity.get("row_inventory_sha256")
        != sha256_bytes(
            canonical_json([spec.logical_identity for spec in specs])
        )
        or campaign_identity.get("row_count") != len(specs)
        or campaign_identity.get("repeats") != manifest.repeats
    ):
        raise ValueError("stateful result campaign identity differs")


def seal_stateful_opcode_result(
    *,
    out_root: pathlib.Path,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    run: pathlib.Path,
) -> dict[str, Any]:
    """Seal the sole verified Task 4 fit authority into an immutable result."""
    model_report = fit_stateful_task4_run(
        manifest_path=manifest_path,
        calibration_run=calibration_run,
        fixtures_root=fixtures_root,
        guest_launcher=guest_launcher,
        elf=elf,
        run=run,
    )
    if not isinstance(model_report, Mapping):
        raise ValueError("stateful Task 4 fit did not return a model report")
    calibration_identity = opcode_gas.validate_calibration_execution_identity(
        calibration_run
    )
    if not isinstance(calibration_identity, Mapping):
        raise ValueError("stateful calibration identity is invalid")

    manifest_bytes = _regular_file(
        manifest_path, label="stateful campaign manifest"
    ).read_bytes()
    manifest_payload = _load_canonical_json_bytes(
        manifest_bytes, label="stateful campaign manifest", pretty=True
    )
    manifest = StatefulCampaignManifest.from_mapping(manifest_payload)
    registry_path = opcode_gas.REPO_ROOT / manifest.reference_registry["path"]
    registry_bytes = _regular_file(
        registry_path, label="stateful source registry"
    ).read_bytes()
    registry_payload = _load_canonical_json_bytes(
        registry_bytes, label="stateful source registry", pretty=True
    )
    registry = load_stateful_reference_registry(
        registry_payload,
        expected_artifact_sha256=manifest.reference_registry["artifact_sha256"],
    )

    campaign_identity_bytes = _regular_file(
        run / "identity.json", label="stateful campaign identity"
    ).read_bytes()
    campaign_identity = _load_canonical_json_bytes(
        campaign_identity_bytes, label="stateful campaign identity", pretty=False
    )
    rows_bytes = _regular_file(
        run / "rows.jsonl", label="stateful row ledger"
    ).read_bytes()
    decisions_bytes = _regular_file(
        run / "decisions.json", label="stateful decision ledger"
    ).read_bytes()
    decisions = _load_canonical_json_bytes(
        decisions_bytes, label="stateful decision ledger", pretty=False
    )
    seal_bytes = _regular_file(
        run / "decisions.sha256", label="stateful decision seal"
    ).read_bytes()
    _validate_sealed_campaign_identity(
        campaign_identity,
        calibration_identity,
        manifest,
        manifest_file_sha256=sha256_bytes(manifest_bytes),
        registry_artifact_sha256=registry.artifact_sha256,
        registry_file_sha256=sha256_bytes(registry_bytes),
    )
    provenance = model_report.get("task4_provenance")
    terminal_hashes = (
        provenance.get("terminal_artifact_file_sha256")
        if isinstance(provenance, Mapping)
        else None
    )
    implementation_revision = calibration_identity.get("implementation_revision")
    if (
        not isinstance(provenance, Mapping)
        or not isinstance(terminal_hashes, Mapping)
        or not isinstance(implementation_revision, str)
        or re.fullmatch(r"[0-9a-f]{40}", implementation_revision) is None
        or campaign_identity.get("calibration_id") != calibration_run.name
        or decisions.get("campaign_identity_sha256")
        != sha256_bytes(canonical_json(campaign_identity))
        or decisions.get("row_ledger_sha256") != sha256_bytes(rows_bytes)
        or provenance.get("campaign_identity_sha256")
        != decisions.get("campaign_identity_sha256")
        or provenance.get("row_ledger_sha256")
        != decisions.get("row_ledger_sha256")
        or terminal_hashes
        != {
            "rows.jsonl": sha256_bytes(rows_bytes),
            "decisions.json": sha256_bytes(decisions_bytes),
            "decisions.sha256": sha256_bytes(seal_bytes),
        }
        or seal_bytes != (sha256_bytes(decisions_bytes) + "\n").encode()
    ):
        raise ValueError("stateful result source provenance differs")

    calibration_identity_bytes = opcode_gas._canonical_json_file_bytes(
        calibration_identity
    )
    model_report_bytes = opcode_gas._canonical_json_file_bytes(model_report)
    identity = _stateful_result_identity(
        implementation_revision=implementation_revision,
        campaign_identity_sha256=decisions["campaign_identity_sha256"],
        row_ledger_sha256=decisions["row_ledger_sha256"],
        manifest_file_sha256=sha256_bytes(manifest_bytes),
        calibration_identity_file_sha256=sha256_bytes(
            calibration_identity_bytes
        ),
        campaign_identity_file_sha256=sha256_bytes(campaign_identity_bytes),
        decisions_file_sha256=sha256_bytes(decisions_bytes),
        decisions_seal_file_sha256=sha256_bytes(seal_bytes),
        registry_artifact_sha256=registry.artifact_sha256,
        registry_file_sha256=sha256_bytes(registry_bytes),
    )
    envelope = _stateful_result_envelope(identity, model_report_bytes)
    files = {
        "result.json": opcode_gas._canonical_json_file_bytes(envelope),
        "campaign-manifest.json": manifest_bytes,
        "calibration-identity.json": calibration_identity_bytes,
        "campaign-identity.json": campaign_identity_bytes,
        "rows.jsonl": rows_bytes,
        "campaign-decisions.json": decisions_bytes,
        "campaign-decisions.sha256": seal_bytes,
        "source-registry.json": registry_bytes,
        "model-report.json": model_report_bytes,
    }
    directory = opcode_gas._publish_immutable_directory(
        out_root,
        envelope["result_id"],
        files,
        label="stateful result",
    )
    return {"result_id": envelope["result_id"], "directory": str(directory)}


def _validate_stateful_result_checkout(
    implementation_revision: str,
    calibration_identity: Mapping[str, Any],
    registry_path: str,
    result_id: str,
) -> None:
    """Allow only byte-identical fitting inputs in an evidence-only descendant."""
    head = opcode_gas.git_head()
    descendant = head != implementation_revision
    if descendant:
        if not opcode_gas.git_revision_is_ancestor(implementation_revision):
            raise ValueError("stateful result implementation revision is not an ancestor")
        changed_paths = opcode_gas.git_changed_paths_since(implementation_revision)
        result_prefix = f"experiments/opcode-gas/derivations/{result_id}/"
        if any(
            not any(
                path.startswith(allowed)
                if allowed.endswith("/")
                else path == allowed
                for allowed in STATEFUL_RESULT_DESCENDANT_PATHS
            )
            and not path.startswith(result_prefix)
            for path in changed_paths
        ):
            raise ValueError("stateful result descendant changes non-evidence source")
    opcode_gas.assert_generated_paths_only(opcode_gas.git_worktree_status())

    relevant_paths = (*STATEFUL_RESULT_FIT_SOURCE_PATHS, registry_path)
    guest_artifacts = calibration_identity.get("guest_artifacts")
    if not isinstance(guest_artifacts, Mapping) or not guest_artifacts:
        raise ValueError("stateful result calibration guest artifacts are missing")
    for relative, expected_sha256 in guest_artifacts.items():
        if (
            not isinstance(relative, str)
            or pathlib.PurePosixPath(relative).is_absolute()
            or ".." in pathlib.PurePosixPath(relative).parts
            or str(pathlib.PurePosixPath(relative)) != relative
        ):
            raise ValueError("stateful result calibration guest artifact path differs")
        _require_sha256(expected_sha256, "stateful result guest artifact")
        path = opcode_gas.REPO_ROOT / relative
        if (
            path.is_symlink()
            or not path.is_file()
            or not path.resolve().is_relative_to(opcode_gas.REPO_ROOT.resolve())
            or opcode_gas.sha256_file(path) != expected_sha256
        ):
            raise ValueError("stateful result calibration guest artifact differs")
        if descendant and opcode_gas.git_file_bytes_at_revision(
            implementation_revision, relative
        ) != path.read_bytes():
            raise ValueError("stateful result guest artifact changed after measurement")

    if descendant:
        for relative in relevant_paths:
            path = opcode_gas.REPO_ROOT / relative
            if (
                path.is_symlink()
                or not path.is_file()
                or opcode_gas.git_file_bytes_at_revision(
                    implementation_revision, relative
                )
                != path.read_bytes()
            ):
                raise ValueError(
                    "stateful result fitting source changed after measurement"
                )


def _replay_sealed_task4_rows(
    manifest: StatefulCampaignManifest,
    rows: Sequence[Mapping[str, Any]],
    campaign_identity: Mapping[str, Any],
    calibration_identity: Mapping[str, Any],
    *,
    guest_launcher: pathlib.Path,
    identity_replayer=None,
) -> list[dict[str, Any]]:
    launcher_sha256 = opcode_gas.validate_calibration_guest_launcher(
        calibration_identity, guest_launcher
    )
    if (
        campaign_identity.get("guest_launcher", {}).get("file_sha256")
        != launcher_sha256
    ):
        raise ValueError("sealed stateful host-native helper identity differs")
    if identity_replayer is None:
        identity_replayer = opcode_gas.replay_revm_opcode_identity
    specs = stateful_campaign_row_specs(manifest)
    if len(rows) != len(specs):
        raise ValueError("sealed stateful Task 4 row inventory differs")
    rows_by_identity = {
        row.get("logical_identity"): row
        for row in rows
        if isinstance(row, Mapping)
    }
    if len(rows_by_identity) != len(specs):
        raise ValueError("sealed stateful Task 4 row identity is duplicated")

    def reject_guest_execution(**_kwargs):
        raise ValueError("sealed stateful replay cannot execute the SP1 guest")

    with tempfile.TemporaryDirectory(
        prefix="stateful-result-replay."
    ) as temporary:
        root = pathlib.Path(temporary)
        fixtures_root = root / "fixtures"
        run = root / "run"
        rows_root = run / "rows"
        rows_root.mkdir(parents=True)
        generate_stateful_fixtures(manifest, fixtures_root)
        for spec in specs:
            row = rows_by_identity.get(spec.logical_identity)
            if row is None:
                raise ValueError("sealed stateful Task 4 logical row is missing")
            (_row_path(run, spec)).write_bytes(canonical_json(row) + b"\n")
        replayed = run_stateful_campaign_rows(
            manifest,
            specs,
            fixtures_root=fixtures_root,
            run=run,
            guest_launcher=guest_launcher,
            elf=opcode_gas.REPO_ROOT / EXECUTION_CONTRACT["elf_path"],
            launcher_sha256=launcher_sha256,
            elf_sha256=campaign_identity["guest_elf"]["file_sha256"],
            batch_executor=reject_guest_execution,
            identity_replayer=identity_replayer,
            calibration_run_id=campaign_identity["calibration_id"],
            verification_only=True,
        )
    if canonical_json(replayed) != canonical_json(rows):
        raise ValueError("sealed stateful Task 4 rows differ from exact replay")
    return replayed


def verify_stateful_opcode_result(
    directory: pathlib.Path,
    *,
    guest_launcher: pathlib.Path,
    identity_replayer=None,
) -> dict[str, Any]:
    """Replay a sealed result from its directory without executing guest code."""
    absolute = directory.absolute()
    if (
        not directory.is_dir()
        or directory.is_symlink()
        or absolute != directory.resolve()
        or {path.name for path in directory.iterdir()} != STATEFUL_RESULT_INVENTORY
    ):
        raise ValueError("stateful result directory inventory differs")
    paths = {name: directory / name for name in STATEFUL_RESULT_INVENTORY}
    if any(
        path.is_symlink()
        or not path.is_file()
        or path.absolute().parent != absolute
        or not path.resolve().is_relative_to(absolute)
        for path in paths.values()
    ):
        raise ValueError("stateful result directory inventory differs")

    raw = {name: path.read_bytes() for name, path in paths.items()}
    envelope = _load_canonical_json_bytes(
        raw["result.json"], label="stateful result", pretty=True
    )
    manifest_payload = _load_canonical_json_bytes(
        raw["campaign-manifest.json"],
        label="stateful campaign manifest",
        pretty=True,
    )
    calibration_identity = _load_canonical_json_bytes(
        raw["calibration-identity.json"],
        label="stateful calibration identity",
        pretty=True,
    )
    campaign_identity = _load_canonical_json_bytes(
        raw["campaign-identity.json"],
        label="stateful campaign identity",
        pretty=False,
    )
    decisions = _load_canonical_json_bytes(
        raw["campaign-decisions.json"],
        label="stateful campaign decisions",
        pretty=False,
    )
    registry_payload = _load_canonical_json_bytes(
        raw["source-registry.json"],
        label="stateful source registry",
        pretty=True,
    )
    model_report = _load_canonical_json_bytes(
        raw["model-report.json"], label="stateful model report", pretty=True
    )
    _validate_sealed_calibration_identity(calibration_identity)
    manifest = StatefulCampaignManifest.from_mapping(manifest_payload)
    registry = load_stateful_reference_registry(
        registry_payload,
        expected_artifact_sha256=manifest.reference_registry["artifact_sha256"],
    )

    tracked_manifest = (
        opcode_gas.REPO_ROOT
        / "experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json"
    )
    tracked_registry = opcode_gas.REPO_ROOT / manifest.reference_registry["path"]
    if (
        raw["campaign-manifest.json"]
        != _regular_file(
            tracked_manifest, label="tracked stateful campaign manifest"
        ).read_bytes()
        or raw["source-registry.json"]
        != _regular_file(
            tracked_registry, label="tracked stateful source registry"
        ).read_bytes()
    ):
        raise ValueError("stateful result tracked source artifact differs")

    implementation_revision = calibration_identity.get("implementation_revision")
    if (
        not isinstance(implementation_revision, str)
        or re.fullmatch(r"[0-9a-f]{40}", implementation_revision) is None
    ):
        raise ValueError("stateful result implementation revision differs")
    _validate_stateful_result_checkout(
        implementation_revision,
        calibration_identity,
        manifest.reference_registry["path"],
        directory.name,
    )
    _validate_sealed_campaign_identity(
        campaign_identity,
        calibration_identity,
        manifest,
        manifest_file_sha256=sha256_bytes(raw["campaign-manifest.json"]),
        registry_artifact_sha256=registry.artifact_sha256,
        registry_file_sha256=sha256_bytes(raw["source-registry.json"]),
    )

    try:
        rows = list(opcode_gas.iter_jsonl(paths["rows.jsonl"]))
    except (ValueError, json.JSONDecodeError) as error:
        raise ValueError("stateful result row ledger is invalid") from error
    if raw["rows.jsonl"] != b"".join(
        canonical_json(row) + b"\n" for row in rows
    ):
        raise ValueError("stateful result row ledger is not canonical JSONL")
    expected_rows, expected_decisions, expected_decisions_bytes = (
        _terminal_campaign_payloads(campaign_identity, rows)
    )
    if (
        raw["rows.jsonl"] != expected_rows
        or raw["campaign-decisions.json"] != expected_decisions_bytes
        or raw["campaign-decisions.sha256"]
        != (sha256_bytes(expected_decisions_bytes) + "\n").encode()
        or decisions != expected_decisions
    ):
        raise ValueError("stateful result Task 4 terminal payload differs")
    _validate_verified_task4_row_identities(manifest, rows)
    _validate_sealed_task4_row_evidence(rows, campaign_identity)
    rows = _replay_sealed_task4_rows(
        manifest,
        rows,
        campaign_identity,
        calibration_identity,
        guest_launcher=guest_launcher,
        identity_replayer=identity_replayer,
    )

    replayed_report = _fit_stateful_task4_rows(manifest, rows, registry_payload)
    replayed_report["task4_provenance"] = {
        "campaign_identity_sha256": expected_decisions[
            "campaign_identity_sha256"
        ],
        "row_ledger_sha256": expected_decisions["row_ledger_sha256"],
        "terminal_artifact_file_sha256": {
            "rows.jsonl": sha256_bytes(raw["rows.jsonl"]),
            "decisions.json": sha256_bytes(raw["campaign-decisions.json"]),
            "decisions.sha256": sha256_bytes(
                raw["campaign-decisions.sha256"]
            ),
        },
    }
    if canonical_json(model_report) != canonical_json(replayed_report):
        raise ValueError("stateful result model report differs from exact replay")

    identity = _stateful_result_identity(
        implementation_revision=implementation_revision,
        campaign_identity_sha256=expected_decisions[
            "campaign_identity_sha256"
        ],
        row_ledger_sha256=expected_decisions["row_ledger_sha256"],
        manifest_file_sha256=sha256_bytes(raw["campaign-manifest.json"]),
        calibration_identity_file_sha256=sha256_bytes(
            raw["calibration-identity.json"]
        ),
        campaign_identity_file_sha256=sha256_bytes(
            raw["campaign-identity.json"]
        ),
        decisions_file_sha256=sha256_bytes(raw["campaign-decisions.json"]),
        decisions_seal_file_sha256=sha256_bytes(
            raw["campaign-decisions.sha256"]
        ),
        registry_artifact_sha256=registry.artifact_sha256,
        registry_file_sha256=sha256_bytes(raw["source-registry.json"]),
    )
    expected_envelope = _stateful_result_envelope(
        identity, raw["model-report.json"]
    )
    if canonical_json(envelope) != canonical_json(expected_envelope):
        raise ValueError("stateful result identity, ownership, or output differs")
    if directory.name != expected_envelope["result_id"]:
        raise ValueError("stateful result directory name differs from identity")
    return {
        "status": "sealed",
        "result_id": expected_envelope["result_id"],
        "selected_model": model_report.get("selection", {}).get(
            "selected_model"
        ),
    }


def cmd_generate(args: Any) -> None:
    manifest = load_stateful_campaign_manifest(args.manifest)
    written = generate_stateful_fixtures(manifest, args.out)
    print(f"wrote {len(written)} stateful fixture(s)")
