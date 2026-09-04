#!/usr/bin/env python3
"""SP1 opcode prover-gas experiment scaffold."""

from __future__ import annotations

import argparse
import functools
import json
import pathlib
import statistics
import subprocess
import tomllib
from dataclasses import asdict, dataclass
from types import MappingProxyType
from typing import Any, Iterable, Mapping


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


@dataclass(frozen=True)
class Manifest:
    name: str
    backend: str
    variants: list[int]
    cases: list[CaseSpec]


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
    return UnzenSchedule(
        opcode_multipliers=MappingProxyType(
            _parse_schedule_rows(
                data,
                rows_key="opcodes",
                identifier_key="opcode",
                max_identifier=0xFF,
            )
        ),
        precompile_multipliers=MappingProxyType(
            _parse_schedule_rows(
                data,
                rows_key="precompiles",
                identifier_key="address",
                max_identifier=(1 << 160) - 1,
            )
        ),
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
        identifier = parse_opcode(row[identifier_key])
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


def load_manifest(path: pathlib.Path, schedule: UnzenSchedule | None = None) -> Manifest:
    data = tomllib.loads(path.read_text())
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


def generate_cases(manifest: Manifest, out_dir: pathlib.Path) -> list[pathlib.Path]:
    written = []
    for case in manifest.cases:
        for variant in manifest.variants:
            case_dir = out_dir / manifest.name / case.name / f"count-{variant}"
            case_dir.mkdir(parents=True, exist_ok=True)
            if case.kind == "opcode":
                if case.opcode is None:
                    raise ValueError(f"opcode case {case.name} is missing opcode")
                generated = build_bytecode(case, variant)
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
            elif case.kind == "precompile":
                if case.address is None:
                    raise ValueError(f"precompile case {case.name} is missing address")
                precompile_input = build_precompile_input(case)
                payload = {
                    "suite": manifest.name,
                    "backend": manifest.backend,
                    "kind": case.kind,
                    "case": case.name,
                    "address": f"0x{case.address:02x}",
                    "scenario": case.scenario,
                    "template": case.template,
                    "target_count": variant,
                    "input_size": case.input_size,
                    "target_raw_gas": case.target_raw_gas,
                    "target_feature": variant * case.target_raw_gas,
                    "input": precompile_input,
                    "guest_input_status": "precompile_lab_guest_input",
                }
                guest_input = {
                    "case": case.name,
                    "scenario": case.scenario,
                    "address": case.address,
                    "target_count": variant,
                    "input_size": case.input_size,
                    "target_raw_gas": case.target_raw_gas,
                    "input": precompile_input,
                }
            else:
                raise ValueError(f"unknown case kind: {case.kind}")
            path = case_dir / "case.json"
            path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            (case_dir / "guest-input.json").write_text(
                json.dumps(guest_input, indent=2, sort_keys=True) + "\n"
            )
            written.append(path)
    return written


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
    out.write_text(json.dumps(raw_run_from_report(case, report), sort_keys=True) + "\n")


def raw_run_from_report(case: dict[str, Any], report: dict[str, Any]) -> dict[str, Any]:
    raw_run = {**case, **report}
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


def cmd_generate(args: argparse.Namespace) -> None:
    manifest = load_manifest(args.manifest)
    written = generate_cases(manifest, args.out)
    print(f"wrote {len(written)} case metadata files")


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


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="command", required=True)

    generate = subcommands.add_parser("generate", help="generate opcode case metadata")
    generate.add_argument("--manifest", type=pathlib.Path, required=True)
    generate.add_argument("--out", type=pathlib.Path, required=True)
    generate.set_defaults(func=cmd_generate)

    run = subcommands.add_parser("run", help="run generated guest-input cases")
    run.add_argument("--fixtures", type=pathlib.Path, required=True)
    run.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    run.add_argument(
        "--elf",
        type=pathlib.Path,
        default=pathlib.Path("crates/guests/elf/sp1_opcode_lab.elf"),
        help="SP1 opcode-lab guest ELF",
    )
    run.add_argument(
        "--precompile-elf",
        type=pathlib.Path,
        default=pathlib.Path("crates/guests/elf/sp1_precompile_lab.elf"),
        help="SP1 precompile-lab guest ELF",
    )
    run.add_argument(
        "--opcode-stage",
        choices=["opcode-lab", "revm-opcode-lab"],
        default="opcode-lab",
        help="SP1 opcode lab stage to run for opcode fixtures",
    )
    run.add_argument("--out", type=pathlib.Path, required=True)
    run.set_defaults(func=cmd_run)

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
    return parser


def cmd_run(args: argparse.Namespace) -> None:
    args.out.parent.mkdir(parents=True, exist_ok=True)
    cases_by_kind: dict[str, list[tuple[dict[str, Any], pathlib.Path]]] = {}
    for case_path in sorted(args.fixtures.glob("**/case.json")):
        case = json.loads(case_path.read_text())
        input_path = case_path.with_name("guest-input.json")
        if input_path.exists():
            cases_by_kind.setdefault(case.get("kind", "opcode"), []).append((case, input_path))
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
        report_path = args.out.with_name(f"{args.out.stem}.{stage}.jsonl")
        run_guest_inputs(
            guest_launcher=args.guest_launcher,
            elf_path=elf_path,
            input_paths=[input_path for _, input_path in cases],
            reports_jsonl=report_path,
            stage=stage,
        )
        report_paths.append(report_path)
    case_by_input = {
        str(input_path): case for cases in cases_by_kind.values() for case, input_path in cases
    }
    with args.out.open("w") as out:
        ran = 0
        for report_path in report_paths:
            for report in iter_jsonl(report_path):
                case = case_by_input[report["input"]]
                out.write(json.dumps(raw_run_from_report(case, report), sort_keys=True) + "\n")
                ran += 1
    print(f"ran {ran} executable case(s)")


def cmd_run_proposal(args: argparse.Namespace) -> None:
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
    args.func(args)


if __name__ == "__main__":
    main()
