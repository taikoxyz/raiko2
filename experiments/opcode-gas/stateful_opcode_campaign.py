#!/usr/bin/env python3
"""Canonical fixture contract for the stateful SLOAD/SSTORE calibration."""

from __future__ import annotations

import json
import pathlib
import re
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping, Sequence

from opcode_gas import canonical_json, encode_fixed_microprograms, sha256_bytes


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

        if json.loads(canonical_json(value)) != expected:
            raise ValueError("stateful campaign manifest differs from the frozen contract")
        return cls(
            schema_version=SCHEMA_VERSION,
            purpose=PURPOSE,
            fit_counts=FIT_COUNTS,
            holdout_count=HOLDOUT_COUNT,
            checkpoint_count=CHECKPOINT_COUNT,
            structural_zero_count=STRUCTURAL_ZERO_COUNT,
            repeats=REPEATS,
            reference_registry=MappingProxyType(dict(REFERENCE_REGISTRY)),
            scenarios=tuple(scenarios),
        )


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
    low_hash = sha256_bytes(low_masked)
    high_hash = sha256_bytes(high_masked)
    if (
        low.program_shape_sha256 != low_hash
        or high.program_shape_sha256 != high_hash
        or low_hash != high_hash
    ):
        raise ValueError("low/high program shape hash differs")


def _program_contract(program: StatefulProgram) -> dict[str, Any]:
    return {
        "lane": program.lane,
        "measured": program.measured,
        "operand_immediate_spans": [
            span.as_payload() for span in program.operand_immediate_spans
        ],
        "program_shape_sha256": program.program_shape_sha256,
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
    opcode = scenario.measurement_opcode if lane == "target" else scenario.reference_opcode
    tx_gas_limit = max(100_000, 1_000_000 + GENERATOR_MAX_COUNT * scenario.target_raw_gas)
    guest_input = {
        "case": case_id,
        "scenario": scenario.name,
        "opcode": opcode,
        "target_count": count,
        "target_raw_gas": scenario.target_raw_gas,
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
        "case": case_id,
        "scenario": scenario.name,
        "lane": lane,
        "count": count,
        "opcode": opcode,
        "target_count": count,
        "target_raw_gas": scenario.target_raw_gas,
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
        "guest_input_sha256": sha256_bytes(canonical_json(guest_input)),
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


def cmd_generate(args: Any) -> None:
    manifest = load_stateful_campaign_manifest(args.manifest)
    written = generate_stateful_fixtures(manifest, args.out)
    print(f"wrote {len(written)} stateful fixture(s)")
