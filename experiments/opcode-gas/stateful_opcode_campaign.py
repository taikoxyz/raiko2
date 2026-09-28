#!/usr/bin/env python3
"""Canonical fixture contract for the stateful SLOAD/SSTORE calibration."""

from __future__ import annotations

import json
import pathlib
import re
import tempfile
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping, Sequence

import opcode_gas

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
PRIMARY_COUNTS = (0, *FIT_COUNTS, HOLDOUT_COUNT, CHECKPOINT_COUNT)
DIAGNOSTIC_COUNTS = (0, CHECKPOINT_COUNT)
GENERATOR_MAX_COUNT = CHECKPOINT_COUNT
REFERENCE_REGISTRY = {
    "path": "experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json",
    "artifact_sha256": "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b",
}
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
    if expected_bundle.get("schema_version") != 1:
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
    if not isinstance(expected_identity, Mapping) or set(expected_identity) != identity_fields:
        raise ValueError("canonical Rust identity evidence has an invalid shape")
    if expected_identity.get("schema_version") != 1:
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
    if trace.get("schema_version") != 2:
        raise ValueError("stateful trace schema differs")
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

    pair_groups: dict[tuple[str, int, int], dict[str, StatefulCampaignRowSpec]] = {}
    for spec in specs:
        pair_groups.setdefault(
            (spec.scenario, spec.relation_count, spec.repeat_index), {}
        )[spec.lane] = spec
    missing_specs: list[StatefulCampaignRowSpec] = []
    for pair_key, lanes in pair_groups.items():
        for lane in ("control", "target"):
            key = (pair_key[0], pair_key[1], lane, pair_key[2])
            if key not in existing_reports:
                missing_specs.append(lanes[lane])

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

    if verification_only and missing_specs:
        raise ValueError("portable stateful verification found a missing row")

    new_reports: dict[tuple[str, int, str, int], Mapping[str, Any]] = {}
    if missing_specs:
        input_paths = [
            fixtures[(spec.scenario, spec.relation_count, spec.lane)][1]
            for spec in missing_specs
        ]
        with tempfile.TemporaryDirectory(prefix="stateful-opcode-reports-") as temporary:
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
        for spec, input_path, report in zip(missing_specs, input_paths, reports, strict=True):
            key = (spec.scenario, spec.relation_count, spec.lane, spec.repeat_index)
            new_reports[key] = _portable_formal_report(
                report,
                input_path=input_path,
                fixtures_root=fixtures_root,
                expected_public_values=bundle_evidence[
                    (spec.scenario, spec.relation_count, spec.lane)
                ]["expected_public_values"],
            )

    all_reports = {**existing_reports, **new_reports}
    records: dict[tuple[str, int, str, int], dict[str, Any]] = {}
    for pair_key, lanes in pair_groups.items():
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
            target_report=all_reports[(scenario, count, "target", repeat_index)],
            control_report=all_reports[(scenario, count, "control", repeat_index)],
            target_input=target_input,
            control_input=control_input,
            fixtures_root=fixtures_root,
            launcher_sha256=launcher_sha256,
            elf_sha256=elf_sha256,
            calibration_run_id=calibration_run_id,
        )
        for spec, record in (
            (lanes["target"], target_record),
            (lanes["control"], control_record),
        ):
            key = (scenario, count, spec.lane, repeat_index)
            old = existing_payloads.get(key)
            if old is not None and canonical_json(old) != canonical_json(record):
                raise ValueError("persisted stateful row differs from exact replay")
            records[key] = record
    public_values_by_lane: dict[tuple[str, int, str], set[str]] = {}
    for key, record in records.items():
        lane_key = key[:3]
        public_values_by_lane.setdefault(lane_key, set()).add(
            record["formal_report"]["public_values"]
        )
    if any(len(values) != 1 for values in public_values_by_lane.values()):
        raise ValueError("stateful repeated public output differs")
    for spec in specs:
        key = (spec.scenario, spec.relation_count, spec.lane, spec.repeat_index)
        if key not in existing_payloads:
            if verification_only:
                raise ValueError("portable stateful verification found a missing row")
            opcode_gas.persist_immutable_bytes(
                _row_path(run, spec), canonical_json(records[key]) + b"\n"
            )
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
    registry_artifact_sha256 = opcode_gas._validate_content_addressed_artifact(
        registry, label="stateful source registry"
    )
    registry_payload = registry.get("registry")
    named_keys = (
        registry_payload.get("named_opcode_keys")
        if isinstance(registry_payload, Mapping)
        else None
    )
    if not isinstance(named_keys, list) or any(
        not isinstance(key, str) for key in named_keys
    ):
        raise ValueError("stateful source registry has invalid opcode inventory")
    opcode_gas._validate_operation_core_registry(registry, set(named_keys))
    if registry_artifact_sha256 != manifest.reference_registry["artifact_sha256"]:
        raise ValueError("stateful source registry artifact hash differs")

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


def cmd_generate(args: Any) -> None:
    manifest = load_stateful_campaign_manifest(args.manifest)
    written = generate_stateful_fixtures(manifest, args.out)
    print(f"wrote {len(written)} stateful fixture(s)")
