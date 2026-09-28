#!/usr/bin/env python3
"""Deterministic context-opcode fixtures, exact fitting, and sealed replay."""

from __future__ import annotations

import copy
import argparse
import hashlib
import json
import os
import pathlib
import re
import shutil
import stat
import subprocess
import tempfile
from decimal import Decimal, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from typing import Any, Mapping


CONTEXT_OPCODES = (0x30, 0x33, 0x34, 0x35, 0x36, 0x42)
CONTROL_OPCODES = (0x5F, 0x90)
ADAPTIVE_ROUNDS = (8, 32, 128, 512, 2048)
ADAPTIVE_PREFIXES = (
    (0, 1, 2, 4),
    (0, 1, 2, 4, 8, 16),
    (0, 1, 2, 4, 8, 16, 32, 64),
    (0, 1, 2, 4, 8, 16, 32, 64, 128, 256),
    (0, 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024),
)
OSAKA_CANARY_RELATION_IDS = (
    "opcode:0x01:canonical",
    "opcode:0x02:canonical",
    "opcode:0x10:canonical",
    "opcode:0x1b:canonical",
    "opcode:0x57:canonical",
    "opcode:0x19:canonical",
    "opcode:0x5b:canonical",
    "opcode:0x0a:exp-bytes-1",
    "opcode:0x20:input-32",
    "opcode:0x51:offset-0x00",
    "opcode:0x5e:copy-32",
)
CONTEXT_RESULT_INVENTORY = {
    "result.json",
    "campaign-manifest.json",
    "rows.jsonl",
    "source-registry.json",
    "compatibility-canary.json",
    "source-identity.json",
    "adaptive-evidence.json",
}
PREFIX_PLACEMENT = "active_prefix"
TAIL_PLACEMENT = "active_tail"
QUALITY_GATES = {
    "activation_gap_signal_ratio_trigger_gt": "0.02",
    "checkpoint_ape_max": "0.10",
    "r2_min": "0.995",
    "relative_slope_stderr_max": "0.05",
    "residual_signal_max": "0.02",
    "signal_min_baseline_fraction": "0.01",
    "signal_min_repeat_noise_multiple": "20",
    "tail_holdout_ape_max": "0.10",
}
CONTEXT_MANIFEST_CANONICAL_SHA256 = (
    "d9c6cdef20ccb911d28532c8703be34a335e57cc49bdda355420eecd1df467af"
)
HISTORICAL_ANCHOR_DERIVATION_SHA256 = (
    "b61fda990d258ea8dbb909572d0df8efb12adca0b14e8bb01bc8f2c437a33115"
)
HISTORICAL_OSAKA_OBSERVATIONS_SHA256 = (
    "0b1322415c78a38f4546ecc2722d15c2d8fd663cc3e3620b1c60c5ef3b387a72"
)
HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256 = (
    "765100575ffef6a22bbfd8423c60615a9f54d1af086d75f7a18282282303c784"
)
HISTORICAL_ANCHOR_FIT_ARTIFACT_SHA256 = (
    "0b60afdb9130a40978a4fc3842a61e0cdc30faaff58e78c511bb85939844ac9c"
)
HISTORICAL_ANCHOR_PRIMARY_ARTIFACT_SHA256 = (
    "d62529402f2463f3c852f646fb169a0298db9f3c6deabae86441378347dee355"
)
HISTORICAL_ANCHOR_RAW_ROWS_SHA256 = (
    "682aaf33b3e3bc6c0171e759a960a956dd5fe3a90a1c2d91d5e038e353628451"
)
HISTORICAL_ANCHOR_RAW_FILE_SHA256 = (
    "949a3de95b3312c996c05ebf7ca41eb8185e2dff76250b49fb7b88466c2593e2"
)


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _sha256(value: Any, label: str) -> str:
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise ValueError(f"{label} must be a lowercase SHA256")
    return value


def fraction_from_decimal(value: str) -> Fraction:
    if not isinstance(value, str):
        raise ValueError("exact decimal must be a string")
    return Fraction(Decimal(value))


def fraction_payload(value: Fraction) -> dict[str, str]:
    integer_digits = max(1, len(str(abs(value.numerator // value.denominator))))
    with localcontext() as context:
        context.prec = integer_digits + 96
        projected = (Decimal(value.numerator) / Decimal(value.denominator)).quantize(
            Decimal(1).scaleb(-80), rounding=ROUND_HALF_EVEN
        )
    text = format(projected, "f").rstrip("0").rstrip(".")
    if text in {"", "-0"}:
        text = "0"
    replay = Fraction(Decimal(text))
    if abs(replay - value) > Fraction(1, 10**75):
        raise ValueError("exact Decimal projection residual exceeds 1e-75")
    return {
        "numerator": str(value.numerator),
        "denominator": str(value.denominator),
        "decimal": text,
    }


def fraction_from_payload(payload: Mapping[str, Any]) -> Fraction:
    if set(payload) != {"numerator", "denominator", "decimal"}:
        raise ValueError("exact fraction payload shape differs")
    value = Fraction(int(payload["numerator"]), int(payload["denominator"]))
    expected = fraction_payload(value)
    if dict(payload) != expected:
        raise ValueError("exact fraction payload projection differs")
    return value


def load_context_manifest(path: pathlib.Path) -> dict[str, Any]:
    raw = pathlib.Path(path).read_bytes()
    value = json.loads(raw)
    if not isinstance(value, dict) or value.get("purpose") != "context_opcode_calibration":
        raise ValueError("context campaign manifest purpose differs")
    if sha256_bytes(canonical_json(value)) != CONTEXT_MANIFEST_CANONICAL_SHA256:
        raise ValueError("context campaign manifest differs from the frozen V1 contract")
    adaptive = value.get("adaptive_round_contract")
    if not isinstance(adaptive, dict) or adaptive != {
        "checkpoints": list(ADAPTIVE_ROUNDS),
        "generator_rounds": list(ADAPTIVE_ROUNDS),
        "prefixes": [list(prefix) for prefix in ADAPTIVE_PREFIXES],
        "selection": "advance_only_failed_scenarios_freeze_passed_scenarios",
    }:
        raise ValueError("context adaptive round contract differs")
    if value.get("repeats") != 3:
        raise ValueError("context campaign requires exactly three repeats")
    if value.get("quality_gates") != QUALITY_GATES:
        raise ValueError("context campaign quality gates differ")
    if value.get("compatibility_canary") != {
        "control_anchor_opcodes": [0x5F, 0x90],
        "control_opcode_lab_elf_path": "crates/guests/elf/sp1_opcode_lab.elf",
        "control_relation_drift_mape_max": "0.05",
        "historical_anchor_calibration_id": "09ebb08d76d3f461086b0cf4",
        "legacy_relation_drift_mape_max": "0.05",
        "legacy_relation_ids": list(OSAKA_CANARY_RELATION_IDS),
        "per_relation_drift_ape_max": "0.10",
        "revm_opcode_lab_elf_path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
        "reuse_only_no_refit": True,
    }:
        raise ValueError("context compatibility canary contract differs")
    scenarios = value.get("scenarios")
    if not isinstance(scenarios, list) or len(scenarios) != 15:
        raise ValueError("context campaign scenario inventory differs")
    if [scenario.get("opcode") for scenario in scenarios] != [
        0x30,
        0x33,
        0x34,
        0x34,
        0x35,
        0x35,
        0x35,
        0x35,
        0x36,
        0x36,
        0x36,
        0x36,
        0x36,
        0x42,
        0x42,
    ]:
        raise ValueError("context campaign required sibling ordering differs")
    if any(scenario.get("reference_opcode") not in CONTROL_OPCODES for scenario in scenarios):
        raise ValueError("context campaign reference opcode differs")
    if any(
        scenario["reference_opcode"] != (0x90 if scenario["opcode"] == 0x35 else 0x5F)
        for scenario in scenarios
    ):
        raise ValueError("context campaign control shape differs")
    if [scenario["timestamp"] for scenario in scenarios if scenario["opcode"] == 0x42] != [0, 17]:
        raise ValueError("TIMESTAMP siblings must be explicit zero and non-default nonzero")
    return value


def _word(value: int) -> bytes:
    return value.to_bytes(32, "big")


def _program(opcode: int, scenario: Mapping[str, Any]) -> bytes:
    if scenario["opcode"] == 0x35:
        return b"\x5f\x7f" + _word(int(scenario["offset"])) + bytes([opcode, 0x50, 0x50, 0x00])
    return bytes([opcode, 0x50, 0x00])


def _encode_programs(programs: list[bytes]) -> bytes:
    encoded = bytearray(b"\xefMP\x01")
    encoded.extend(len(programs).to_bytes(4, "big"))
    for program in programs:
        encoded.extend(len(program).to_bytes(4, "big"))
        encoded.extend(program)
    return bytes(encoded)


def _block_environment(timestamp: int) -> dict[str, Any]:
    return {
        "basefee": 0,
        "beneficiary": "0x" + "00" * 20,
        "blob_excess_gas_and_price": {"blob_gasprice": "1", "excess_blob_gas": 0},
        "difficulty": "0x" + "00" * 32,
        "gas_limit": 2**64 - 1,
        "number": "0x" + "00" * 32,
        "prevrandao": "0x" + "00" * 32,
        "slot_num": 0,
        "timestamp": "0x" + _word(timestamp).hex(),
    }


def generate_context_fixture(
    manifest: Mapping[str, Any],
    scenario_name: str,
    lane: str,
    count: int,
    *,
    generator_bound: int = 8,
    placement: str = PREFIX_PLACEMENT,
) -> dict[str, Any]:
    if lane not in {"target", "control"}:
        raise ValueError("context fixture lane differs")
    if placement not in {PREFIX_PLACEMENT, TAIL_PLACEMENT}:
        raise ValueError("context fixture placement differs")
    if placement == TAIL_PLACEMENT and count != 1:
        raise ValueError("context tail fixture must contain one active event")
    if generator_bound not in ADAPTIVE_ROUNDS or (
        placement == PREFIX_PLACEMENT
        and count not in ADAPTIVE_PREFIXES[ADAPTIVE_ROUNDS.index(generator_bound)]
        and count != generator_bound
    ):
        raise ValueError("context count is outside the frozen adaptive round")
    scenario = next((row for row in manifest["scenarios"] if row["name"] == scenario_name), None)
    if scenario is None:
        raise ValueError("unknown context scenario")
    target_program = _program(scenario["opcode"], scenario)
    control_program = _program(scenario["reference_opcode"], scenario)
    active_indices = (
        range(count) if placement == PREFIX_PLACEMENT else (generator_bound - 1,)
    )
    programs = [
        target_program if lane == "target" and index in active_indices else control_program
        for index in range(generator_bound)
    ]
    bytecode = _encode_programs(programs)
    concrete_opcode = scenario["opcode"] if lane == "target" else scenario["reference_opcode"]
    concrete_count = count if lane == "target" else generator_bound
    raw_gas = 3 if scenario["opcode"] == 0x35 else 2
    environment = _block_environment(int(scenario["timestamp"]))
    return {
        "case": (
            f"context-{scenario_name}-{placement}-{lane}-{count}-bound-{generator_bound}"
        ),
        "scenario": scenario_name,
        "lane": lane,
        "relation_count": count,
        "placement": placement,
        "opcode": concrete_opcode,
        "target_count": concrete_count,
        "target_raw_gas": raw_gas,
        "tx_gas_limit": manifest["tx_gas_limit"],
        "bytecode": "0x" + bytecode.hex(),
        "fixed_bytecode_len": len(bytecode),
        "generator_max_count": generator_bound,
        "storage": None,
        "tx_value": scenario["tx_value"],
        "calldata": scenario["calldata"],
        "block_timestamp": scenario["timestamp"],
        "active_program": (target_program if lane == "target" else control_program).hex(),
        "inactive_program": control_program.hex(),
        "environment_sha256": sha256_bytes(canonical_json(environment)),
        "identity_contract": {"schema_version": 2, "block_environment_sha256": "required"},
        "trace_schema_version": 3,
    }


def load_control_reference(registry: Mapping[str, Any], opcode: int) -> dict[str, Any]:
    key = f"opcode:0x{opcode:02x}"
    if opcode not in CONTROL_OPCODES:
        raise ValueError("context control opcode is not frozen")
    if registry.get("reference_registry") is not None:
        raise ValueError("context registry wrapper shape differs")
    dispatch_only = registry.get("approximation_policy", {}).get("dispatch_only_opcode_keys")
    if not isinstance(dispatch_only, list) or key in dispatch_only:
        raise ValueError("context control reference is an approximation")
    model = registry.get("registry", {}).get("models", {}).get(key)
    parameters = model.get("parameters") if isinstance(model, Mapping) else None
    if (
        not isinstance(model, Mapping)
        or model.get("kind") != "static_raw_gas"
        or not isinstance(parameters, Mapping)
        or set(parameters) != {"body_per_raw_gas"}
    ):
        raise ValueError("context control reference is not a sealed measured static model")
    body = fraction_from_decimal(parameters["body_per_raw_gas"])
    if body <= 0:
        raise ValueError("context control reference body must be positive")
    return {
        "model_id": key,
        "model_kind": "static_raw_gas",
        "stored_body_per_raw_gas": fraction_payload(body),
        "registry_artifact_sha256": _sha256(registry.get("artifact_sha256"), "registry artifact"),
    }


def recover_scaled_target_body(
    *,
    delta_lab: Fraction,
    body_scale: Fraction,
    target_raw_gas: int,
    control_raw_gas: int,
    stored_control_body: Fraction,
) -> Fraction:
    if body_scale <= 0 or target_raw_gas <= 0 or control_raw_gas <= 0:
        raise ValueError("context recovery has a non-positive denominator or scale")
    return (body_scale * delta_lab + control_raw_gas * stored_control_body) / target_raw_gas


def predict_static_event(*, common_dispatch: Fraction, raw_gas: int, stored_body: Fraction) -> Fraction:
    if raw_gas < 0:
        raise ValueError("raw gas must be nonnegative")
    return common_dispatch + raw_gas * stored_body


def synthetic_passing_scenario_reports(manifest: Mapping[str, Any]) -> dict[str, Any]:
    return {
        scenario["name"]: {
            "status": "passed",
            "selected_generator_bound": 8,
            "selected_counts": [0, 1, 2, 4],
            "checkpoint_count": 8,
            "repeat_count": 3,
            "delta_lab_exact": fraction_payload(Fraction(100)),
            "quality_gates": {
                "activation": "passed",
                "checkpoint": "passed",
                "fit": "passed",
                "repeat_noise": "passed",
                "residual": "passed",
                "signal": "passed",
                "tail_holdout": "not_triggered",
            },
        }
        for scenario in manifest["scenarios"]
    }


def synthetic_passing_rows(manifest: Mapping[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for scenario in manifest["scenarios"]:
        samples = [
            *((PREFIX_PLACEMENT, count) for count in (*ADAPTIVE_PREFIXES[0], ADAPTIVE_ROUNDS[0])),
            (TAIL_PLACEMENT, 1),
        ]
        for placement, count in samples:
            for repeat in range(manifest["repeats"]):
                control_gas = 10_000 + repeat
                for lane in ("control", "target"):
                    fixture = generate_context_fixture(
                        manifest,
                        scenario["name"],
                        lane,
                        count,
                        generator_bound=8,
                        placement=placement,
                    )
                    rows.append(
                        {
                            "scenario": scenario["name"],
                            "lane": lane,
                            "count": count,
                            "placement": placement,
                            "repeat_index": repeat,
                            "generator_bound": 8,
                            "prover_gas": control_gas + (100 * count if lane == "target" else 0),
                            "fixture": fixture,
                            "identity": {
                                "schema_version": 2,
                                "block_environment_sha256": fixture["environment_sha256"],
                            },
                            "trace": {
                                "schema_version": 3,
                                "block_environment_sha256": fixture["environment_sha256"],
                            },
                        }
                    )
    return rows


def admit_context_row(manifest: Mapping[str, Any], row: Mapping[str, Any]) -> None:
    if set(row) != {
        "scenario",
        "lane",
        "count",
        "placement",
        "repeat_index",
        "generator_bound",
        "prover_gas",
        "fixture",
        "identity",
        "trace",
    }:
        raise ValueError("context row shape differs")
    if row["lane"] not in {"target", "control"} or type(row["prover_gas"]) is not int:
        raise ValueError("context row lane or prover gas differs")
    if type(row["repeat_index"]) is not int or not 0 <= row["repeat_index"] < manifest["repeats"]:
        raise ValueError("context row repeat differs")
    expected = generate_context_fixture(
        manifest,
        row["scenario"],
        row["lane"],
        row["count"],
        generator_bound=row["generator_bound"],
        placement=row["placement"],
    )
    if row["fixture"] != expected:
        raise ValueError("context row fixture or scenario environment differs")
    identity = row["identity"]
    trace = row["trace"]
    if not isinstance(identity, Mapping) or identity != {
        "schema_version": 2,
        "block_environment_sha256": expected["environment_sha256"],
    }:
        raise ValueError("context row identity or environment digest differs")
    if not isinstance(trace, Mapping) or trace != {
        "schema_version": 3,
        "block_environment_sha256": expected["environment_sha256"],
    }:
        raise ValueError("context row trace or environment digest differs")


def _linear_fit(points: list[tuple[int, Fraction]]) -> tuple[Fraction, Fraction, Fraction]:
    count = Fraction(len(points))
    sum_x = sum((Fraction(x) for x, _y in points), Fraction())
    sum_y = sum((y for _x, y in points), Fraction())
    denominator = count * sum(Fraction(x * x) for x, _y in points) - sum_x * sum_x
    if denominator == 0:
        raise ValueError("context fit denominator is zero")
    slope = (
        count * sum((Fraction(x) * y for x, y in points), Fraction()) - sum_x * sum_y
    ) / denominator
    intercept = (sum_y - slope * sum_x) / count
    residual = max((abs(y - (intercept + slope * x)) for x, y in points), default=Fraction())
    return slope, intercept, residual


def _exact_fit_evidence(points: list[tuple[int, Fraction]]) -> dict[str, Fraction]:
    slope, intercept, residual = _linear_fit(points)
    count = Fraction(len(points))
    mean_y = sum((value for _x, value in points), Fraction()) / count
    residuals = [value - (intercept + slope * x) for x, value in points]
    ss_res = sum((value * value for value in residuals), Fraction())
    ss_total = sum(((value - mean_y) ** 2 for _x, value in points), Fraction())
    mean_x = sum((Fraction(x) for x, _value in points), Fraction()) / count
    centered_x = sum(((Fraction(x) - mean_x) ** 2 for x, _value in points), Fraction())
    stderr_squared = (
        ss_res / Fraction(len(points) - 2) / centered_x
        if len(points) > 2 and centered_x
        else Fraction()
    )
    signal = abs(max(value for _x, value in points) - min(value for _x, value in points))
    return {
        "slope": slope,
        "intercept": intercept,
        "r2": Fraction(1) - ss_res / ss_total if ss_total else Fraction(),
        "stderr_squared": stderr_squared,
        "signal": signal,
        "max_residual": residual,
    }


def _ape(observed: Fraction, predicted: Fraction) -> Fraction | None:
    if observed == predicted == 0:
        return Fraction()
    if observed == 0 or predicted == 0 or (observed > 0) != (predicted > 0):
        return None
    return abs(observed - predicted) / abs(observed)


def fit_context_campaign_rows(
    manifest: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    *,
    selected_scenarios: list[str] | None = None,
) -> dict[str, Any]:
    for row in rows:
        admit_context_row(manifest, row)
    reports = {}
    noise_floor = Fraction(manifest["controlled_run_noise_floor"]["prover_gas"])
    allowed = (
        {scenario["name"] for scenario in manifest["scenarios"]}
        if selected_scenarios is None
        else set(selected_scenarios)
    )
    if selected_scenarios is not None and (
        len(allowed) != len(selected_scenarios)
        or not allowed <= {scenario["name"] for scenario in manifest["scenarios"]}
    ):
        raise ValueError("context selected scenario inventory differs")
    for scenario in manifest["scenarios"]:
        name = scenario["name"]
        if name not in allowed:
            continue
        scenario_rows = [row for row in rows if row["scenario"] == name]
        bounds = {row["generator_bound"] for row in scenario_rows}
        if len(bounds) != 1:
            raise ValueError(f"context scenario has zero or multiple selected rounds: {name}")
        bound = next(iter(bounds))
        round_index = ADAPTIVE_ROUNDS.index(bound)
        fit_counts = ADAPTIVE_PREFIXES[round_index]
        expected_samples = [
            *((PREFIX_PLACEMENT, count) for count in (*fit_counts, bound)),
            (TAIL_PLACEMENT, 1),
        ]
        deltas = {}
        repeat_spreads = []
        for placement, count in expected_samples:
            repeat_deltas = []
            for repeat in range(manifest["repeats"]):
                pair = [
                    row
                    for row in scenario_rows
                    if row["placement"] == placement
                    and row["count"] == count
                    and row["repeat_index"] == repeat
                ]
                if len(pair) != 2 or {row["lane"] for row in pair} != {"target", "control"}:
                    raise ValueError(f"context scenario pair inventory differs: {name}")
                by_lane = {row["lane"]: row for row in pair}
                repeat_deltas.append(
                    Fraction(by_lane["target"]["prover_gas"] - by_lane["control"]["prover_gas"])
                )
            repeat_spreads.append(max(repeat_deltas) - min(repeat_deltas))
            deltas[(placement, count)] = sum(repeat_deltas, Fraction()) / len(repeat_deltas)
        positive = [
            (count, deltas[(PREFIX_PLACEMENT, count)])
            for count in fit_counts
            if count > 0
        ]
        fit = _exact_fit_evidence(positive)
        slope = fit["slope"]
        intercept = fit["intercept"]
        signal = fit["signal"]
        residual = fit["max_residual"]
        repeat_noise = max(repeat_spreads)
        checkpoint_observed = deltas[(PREFIX_PLACEMENT, bound)] - intercept
        checkpoint_predicted = slope * bound
        checkpoint_ape = _ape(checkpoint_observed, checkpoint_predicted)
        activation_gap = deltas[(PREFIX_PLACEMENT, 0)] - intercept
        activation_ratio = abs(activation_gap) / signal if signal else None
        tail_triggered = activation_ratio is None or activation_ratio > Fraction(2, 100)
        tail_observed = (
            deltas[(TAIL_PLACEMENT, 1)] - deltas[(PREFIX_PLACEMENT, 0)]
        )
        tail_ape = _ape(tail_observed, slope)
        tail_passed = tail_ape is not None and tail_ape <= Fraction(1, 10)
        signal_threshold = max(
            noise_floor,
            abs(intercept) * Fraction(1, 100),
            repeat_noise * 20,
        )
        r2_passed = fit["r2"] >= Fraction(995, 1000)
        stderr_passed = (
            slope != 0
            and fit["stderr_squared"]
            <= slope * slope * Fraction(5, 100) * Fraction(5, 100)
        )
        gates = {
            "activation": "passed",
            "checkpoint": "passed" if checkpoint_ape is not None and checkpoint_ape <= Fraction(1, 10) else "failed",
            "fit": "passed" if slope > 0 else "failed",
            "r2": "passed" if r2_passed else "failed",
            "slope_stderr": "passed" if stderr_passed else "failed",
            "repeat_noise": "passed" if repeat_noise == 0 else "failed",
            "residual": "passed" if residual <= abs(signal) * Fraction(2, 100) else "failed",
            "signal": "passed" if signal >= signal_threshold else "failed",
            "tail_holdout": (
                "passed" if tail_triggered and tail_passed
                else "failed" if tail_triggered
                else "not_triggered"
            ),
        }
        reports[name] = {
            "status": "passed" if set(gates.values()) <= {"passed", "not_triggered"} else "failed",
            "selected_generator_bound": bound,
            "selected_counts": list(fit_counts),
            "checkpoint_count": bound,
            "repeat_count": manifest["repeats"],
            "delta_lab_exact": fraction_payload(slope),
            "intercept_exact": fraction_payload(intercept),
            "activation_gap_exact": fraction_payload(activation_gap),
            "activation_ratio_exact": (
                fraction_payload(activation_ratio) if activation_ratio is not None else None
            ),
            "r2_exact": fraction_payload(fit["r2"]),
            "slope_stderr_squared_exact": fraction_payload(fit["stderr_squared"]),
            "checkpoint_ape_exact": (
                fraction_payload(checkpoint_ape) if checkpoint_ape is not None else None
            ),
            "max_residual_exact": fraction_payload(residual),
            "repeat_noise_exact": fraction_payload(repeat_noise),
            "signal_exact": fraction_payload(signal),
            "signal_threshold_exact": fraction_payload(signal_threshold),
            "tail_holdout": {
                "placement": TAIL_PLACEMENT,
                "triggered": tail_triggered,
                "observed_marginal_exact": fraction_payload(tail_observed),
                "predicted_marginal_exact": fraction_payload(slope),
                "ape_exact": fraction_payload(tail_ape) if tail_ape is not None else None,
                "status": "passed" if tail_passed else "failed",
            },
            "quality_gates": gates,
        }
    return reports


def select_promotable_context_keys(
    manifest: Mapping[str, Any], scenario_reports: Mapping[str, Any]
) -> set[str]:
    selected = set()
    for opcode in CONTEXT_OPCODES:
        siblings = [scenario["name"] for scenario in manifest["scenarios"] if scenario["opcode"] == opcode]
        if siblings and all(
            isinstance(scenario_reports.get(name), Mapping)
            and scenario_reports[name].get("status") == "passed"
            and set(scenario_reports[name].get("quality_gates", {}).values())
            <= {"passed", "not_triggered"}
            for name in siblings
        ):
            selected.add(f"opcode:0x{opcode:02x}")
    return selected


def _guest_input_from_fixture(fixture: Mapping[str, Any]) -> dict[str, Any]:
    guest_input = {
        field: fixture[field]
        for field in (
            "case",
            "scenario",
            "opcode",
            "target_count",
            "target_raw_gas",
            "tx_gas_limit",
            "bytecode",
            "generator_max_count",
            "fixed_bytecode_len",
        )
    }
    if int(fixture["tx_value"], 16):
        guest_input["tx_value"] = fixture["tx_value"]
    if fixture["calldata"] != "0x":
        guest_input["calldata"] = fixture["calldata"]
    guest_input["block_timestamp"] = fixture["block_timestamp"]
    return guest_input


def _decode_programs(bytecode_hex: str) -> list[bytes]:
    value = bytes.fromhex(bytecode_hex.removeprefix("0x"))
    if value[:4] != b"\xefMP\x01" or len(value) < 8:
        raise ValueError("context fixture has an invalid fixed-program envelope")
    count = int.from_bytes(value[4:8], "big")
    cursor = 8
    programs = []
    for _index in range(count):
        if cursor + 4 > len(value):
            raise ValueError("context fixture program envelope is truncated")
        length = int.from_bytes(value[cursor : cursor + 4], "big")
        cursor += 4
        if cursor + length > len(value):
            raise ValueError("context fixture program is truncated")
        programs.append(value[cursor : cursor + length])
        cursor += length
    if cursor != len(value):
        raise ValueError("context fixture program envelope has trailing bytes")
    return programs


def _synthetic_identity_for_fixture(fixture: Mapping[str, Any]) -> dict[str, Any]:
    guest_input = _guest_input_from_fixture(fixture)
    backend = sha256_bytes(canonical_json(guest_input))
    workload = sha256_bytes(canonical_json({"context_fixture": fixture}))
    tx_hash = sha256_bytes(
        canonical_json(
            {
                "value": fixture["tx_value"],
                "calldata": fixture["calldata"],
                "gas_limit": fixture["tx_gas_limit"],
            }
        )
    )
    programs = _decode_programs(fixture["bytecode"])
    trace = {
        "kind": "revm_opcode",
        "schema_version": 3,
        "workload_id": workload,
        "backend_input_sha256": backend,
        "backend_input_len": len(canonical_json(guest_input)),
        "target_opcode": fixture["opcode"],
        "declared_target_count": fixture["target_count"],
        "declared_target_raw_gas": fixture["target_raw_gas"],
        "tx_gas_limit": fixture["tx_gas_limit"],
        "executed_target_count": fixture["target_count"],
        "executed_target_raw_gas": fixture["target_count"] * fixture["target_raw_gas"],
        "bytecode_len": fixture["fixed_bytecode_len"],
        "bytecode_sha256": sha256_bytes(bytes.fromhex(fixture["bytecode"][2:])),
        "program_sha256": [sha256_bytes(program) for program in programs],
        "evm_spec": "osaka",
        "revm_version": "41.0.0",
        "shared_constructor": "raiko2-opcode-lab",
        "transaction_envelope_sha256": tx_hash,
        "block_environment_sha256": fixture["environment_sha256"],
        "access_list_sha256": sha256_bytes(b"[]"),
        "prestate_sha256": sha256_bytes(b"null"),
        "result_statuses": {"success": fixture["generator_max_count"]},
        "semantic_check": {
            "schema_version": 1,
            "backend_input_sha256": backend,
            "checked_programs": fixture["generator_max_count"],
            "passed": True,
        },
    }
    identity = {
        "schema_version": 2,
        "input": guest_input,
        "backend_input_sha256": backend,
        "backend_input_len": trace["backend_input_len"],
        "workload_id": workload,
        "transaction_envelope_sha256": tx_hash,
        "block_environment_sha256": fixture["environment_sha256"],
        "access_list_sha256": trace["access_list_sha256"],
        "prestate_sha256": trace["prestate_sha256"],
    }
    return {
        "schema_version": 2,
        "expected_public_values": "0x" + sha256_bytes(canonical_json(guest_input)),
        "identity": identity,
        "report": {
            "guest_input_sha256": "0x" + backend,
            "guest_input_bincode_length": trace["backend_input_len"],
            "controlled_trace": trace,
        },
    }


def synthetic_identity_replayer(input_path: pathlib.Path) -> dict[str, Any]:
    """Complete deterministic Rust-oracle shape used only by unit tests."""
    fixture = json.loads(pathlib.Path(input_path).read_bytes())
    return _synthetic_identity_for_fixture(fixture)


def synthetic_execution_reports(
    fixtures: list[pathlib.Path], *, fail_first_round: str | None = None
) -> list[dict[str, Any]]:
    repeats: dict[str, int] = {}
    reports = []
    for path in fixtures:
        fixture = json.loads(pathlib.Path(path).read_bytes())
        repeat = repeats.get(str(path), 0)
        repeats[str(path)] = repeat + 1
        slope = (
            10
            if fail_first_round == fixture["scenario"]
            and fixture["generator_max_count"] == ADAPTIVE_ROUNDS[0]
            else 100
        )
        gas = 10_000 + repeat + (slope * fixture["relation_count"] if fixture["lane"] == "target" else 0)
        oracle = synthetic_identity_replayer(path)
        reports.append(
            {
                "stage": "revm-opcode-lab",
                "mode": "execute",
                "proof_mode": "compressed",
                "input": str(path),
                "prover_gas": gas,
                "gas": gas,
                "primary_workload_metric": {"label": "prover_gas", "count": gas},
                "public_values": oracle["expected_public_values"],
                "exit_code": 0,
                "sp1_execution_engine": "gas-estimator",
                **oracle["report"],
            }
        )
    return reports


def _admit_executed_context_row(
    manifest: Mapping[str, Any],
    fixture: Mapping[str, Any],
    identity_bundle: Mapping[str, Any],
    report: Mapping[str, Any],
) -> None:
    if set(identity_bundle) != {"schema_version", "expected_public_values", "identity", "report"}:
        raise ValueError("context native identity bundle shape differs")
    identity = identity_bundle.get("identity")
    native = identity_bundle.get("report")
    native_trace = native.get("controlled_trace") if isinstance(native, Mapping) else None
    trace = report.get("controlled_trace")
    guest_input = _guest_input_from_fixture(fixture)
    if (
        identity_bundle.get("schema_version") != 2
        or not isinstance(identity, Mapping)
        or identity.get("schema_version") != 2
        or identity.get("input") != guest_input
        or identity.get("block_environment_sha256") != fixture["environment_sha256"]
        or not isinstance(native_trace, Mapping)
        or native_trace.get("schema_version") != 3
        or native_trace.get("block_environment_sha256") != fixture["environment_sha256"]
        or trace != native_trace
    ):
        raise ValueError("context native identity, trace, or environment differs")
    backend = identity.get("backend_input_sha256")
    if (
        re.fullmatch(r"[0-9a-f]{64}", str(backend)) is None
        or native.get("guest_input_sha256") != "0x" + backend
        or report.get("guest_input_sha256") != "0x" + backend
        or report.get("guest_input_bincode_length") != identity.get("backend_input_len")
        or report.get("public_values") != identity_bundle.get("expected_public_values")
    ):
        raise ValueError("context backend input or public commitment differs")
    programs = _decode_programs(fixture["bytecode"])
    if (
        trace.get("kind") != "revm_opcode"
        or trace.get("evm_spec") != "osaka"
        or trace.get("shared_constructor") != "raiko2-opcode-lab"
        or trace.get("target_opcode") != fixture["opcode"]
        or trace.get("declared_target_count") != fixture["target_count"]
        or trace.get("executed_target_count") != fixture["target_count"]
        or trace.get("declared_target_raw_gas") != fixture["target_raw_gas"]
        or trace.get("executed_target_raw_gas")
        != fixture["target_count"] * fixture["target_raw_gas"]
        or trace.get("bytecode_len") != fixture["fixed_bytecode_len"]
        or trace.get("bytecode_sha256")
        != sha256_bytes(bytes.fromhex(fixture["bytecode"][2:]))
        or trace.get("program_sha256") != [sha256_bytes(program) for program in programs]
        or trace.get("result_statuses") != {"success": fixture["generator_max_count"]}
    ):
        raise ValueError("context executed trace differs from canonical fixture")
    semantic = trace.get("semantic_check")
    if (
        not isinstance(semantic, Mapping)
        or semantic.get("backend_input_sha256") != backend
        or semantic.get("checked_programs") != fixture["generator_max_count"]
        or semantic.get("passed") is not True
    ):
        raise ValueError("context semantic check differs")
    gas = report.get("prover_gas", report.get("gas"))
    if (
        report.get("stage") != manifest["execution"]["stage"]
        or report.get("mode") != manifest["execution"]["mode"]
        or report.get("proof_mode") != "compressed"
        or report.get("sp1_execution_engine") != manifest["execution"]["sp1_execution_engine"]
        or type(gas) is not int
        or gas <= 0
        or report.get("primary_workload_metric") != {"label": "prover_gas", "count": gas}
        or report.get("exit_code") != 0
    ):
        raise ValueError("context formal execution report differs")


def _default_context_executor(
    *,
    fixtures: list[pathlib.Path],
    reports_jsonl: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
) -> None:
    input_list = reports_jsonl.with_suffix(".inputs.json")
    input_list.parent.mkdir(parents=True, exist_ok=True)
    _write_canonical(input_list, [str(path) for path in fixtures])
    subprocess.run(
        [
            str(guest_launcher),
            "--stage",
            "revm-opcode-lab",
            "--proof-type",
            "sp1",
            "--mode",
            "execute",
            "--sp1-prover",
            "local",
            "--sp1-execution-engine",
            "gas-estimator",
            "--elf",
            str(elf),
            "--input-list",
            str(input_list),
            "--jsonl-out",
            str(reports_jsonl),
        ],
        check=True,
    )


def _default_identity_replayer(
    guest_launcher: pathlib.Path, input_path: pathlib.Path
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="context-identity-") as directory:
        output = pathlib.Path(directory) / "identity.json"
        subprocess.run(
            [
                str(guest_launcher),
                "--stage",
                "revm-opcode-identity",
                "--proof-type",
                "native",
                "--input",
                str(input_path),
                "--json-out",
                str(output),
            ],
            check=True,
        )
        value = json.loads(output.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("context native identity helper returned a non-object")
    return value


def _default_anchor_identity_replayer(
    guest_launcher: pathlib.Path, input_path: pathlib.Path
) -> dict[str, Any]:
    with tempfile.TemporaryDirectory(prefix="context-anchor-identity-") as directory:
        output = pathlib.Path(directory) / "identity.json"
        subprocess.run(
            [
                str(guest_launcher),
                "--stage",
                "opcode-anchor-identity",
                "--proof-type",
                "native",
                "--input",
                str(input_path),
                "--json-out",
                str(output),
            ],
            check=True,
        )
        value = json.loads(output.read_bytes())
    if not isinstance(value, dict):
        raise ValueError("context anchor identity helper returned a non-object")
    return value


def _capture_current_anchor_replay(
    *,
    fixtures: pathlib.Path,
    guest_launcher: pathlib.Path,
    execution_identity: Mapping[str, Any],
) -> dict[str, Any]:
    """Capture the canonical current-input identity oracle without executing SP1."""
    import opcode_gas

    guest_artifacts = execution_identity.get("guest_artifacts")
    expected_elf = (
        guest_artifacts.get("crates/guests/elf/sp1_opcode_lab.elf")
        if isinstance(guest_artifacts, Mapping)
        else None
    )
    manifest, inventory = opcode_gas._validated_anchor_probe_fixture_manifest(
        fixtures, expected_elf_sha256=expected_elf
    )
    expected_launcher = execution_identity.get("guest_launcher_sha256")
    if (
        not isinstance(expected_launcher, str)
        or sha256_bytes(pathlib.Path(guest_launcher).read_bytes()) != expected_launcher
        or manifest.get("guest_launcher_sha256") != expected_launcher
    ):
        raise ValueError("context anchor fixture launcher identity differs")
    inputs = []
    native_identities = []
    for fixture in inventory.values():
        relative = pathlib.PurePosixPath(str(fixture["guest_input_path"]))
        input_path = fixtures.joinpath(*relative.parts)
        value = json.loads(input_path.read_bytes())
        if input_path.read_bytes() != canonical_json(value) + b"\n":
            raise ValueError("context anchor fixture input is noncanonical")
        bundle = _default_anchor_identity_replayer(
            pathlib.Path(guest_launcher), input_path
        )
        if (
            bundle.get("schema_version") != 1
            or re.fullmatch(
                r"0x[0-9a-f]{64}", str(bundle.get("guest_input_sha256"))
            )
            is None
            or type(bundle.get("guest_input_bincode_length")) is not int
            or re.fullmatch(
                r"0x[0-9a-f]{64}", str(bundle.get("expected_public_values"))
            )
            is None
        ):
            raise ValueError("context anchor native input identity differs")
        inputs.append(
            {
                "guest_input_path": relative.as_posix(),
                "input": value,
            }
        )
        native_identities.append(
            {
                "guest_input_path": relative.as_posix(),
                "backend_input_sha256": bundle["guest_input_sha256"][2:],
                "backend_input_len": bundle["guest_input_bincode_length"],
                "expected_public_values": bundle["expected_public_values"],
            }
        )
    return {
        "calibration_identity": copy.deepcopy(execution_identity),
        "calibration_identity_sha256": sha256_bytes(
            canonical_json(execution_identity)
        ),
        "fixture_manifest": copy.deepcopy(manifest),
        "fixture_inputs": inputs,
        "native_input_identities": native_identities,
    }


def _validate_current_anchor_replay(
    replay: Any,
    current_fit: Mapping[str, Any],
    current_rows: list[Mapping[str, Any]],
) -> None:
    """Replay fixture/input/row joins from embedded evidence without SP1."""
    import opcode_gas

    if not isinstance(replay, Mapping) or set(replay) != {
        "calibration_identity",
        "calibration_identity_sha256",
        "fixture_manifest",
        "fixture_inputs",
        "native_input_identities",
    }:
        raise ValueError("context current anchor replay inventory differs")
    identity = replay.get("calibration_identity")
    manifest = replay.get("fixture_manifest")
    inputs = replay.get("fixture_inputs")
    native = replay.get("native_input_identities")
    if (
        not isinstance(identity, Mapping)
        or replay.get("calibration_identity_sha256")
        != sha256_bytes(canonical_json(identity))
        or not isinstance(manifest, Mapping)
        or not isinstance(inputs, list)
        or not isinstance(native, list)
    ):
        raise ValueError("context current anchor replay identity differs")
    with tempfile.TemporaryDirectory(prefix="context-anchor-replay-") as directory:
        root = pathlib.Path(directory)
        (root / "anchor-probe-manifest.json").write_bytes(
            canonical_json(manifest) + b"\n"
        )
        for record in inputs:
            if not isinstance(record, Mapping) or set(record) != {
                "guest_input_path",
                "input",
            }:
                raise ValueError("context current anchor fixture input differs")
            relative = pathlib.PurePosixPath(str(record["guest_input_path"]))
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("context current anchor fixture path differs")
            path = root.joinpath(*relative.parts)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(canonical_json(record["input"]) + b"\n")
        checked_manifest, inventory = (
            opcode_gas._validated_anchor_probe_fixture_manifest(
                root,
                expected_elf_sha256=current_fit.get("elf_sha256"),
            )
        )
    if checked_manifest != manifest:
        raise ValueError("context current anchor fixture manifest differs")
    opcode_gas._validate_anchor_probe_rows_against_fixtures(
        current_fit, current_rows, manifest, inventory
    )
    bound_identity = {
        **identity,
        "calibration_id": current_fit.get("run_provenance", {}).get(
            "calibration_id"
        ),
        "calibration_identity_sha256": replay["calibration_identity_sha256"],
    }
    opcode_gas.validated_anchor_probe_for_execution_identity(
        current_fit, current_rows, bound_identity
    )
    expected_paths = [fixture["guest_input_path"] for fixture in inventory.values()]
    if [row.get("guest_input_path") for row in native] != expected_paths:
        raise ValueError("context current anchor native identity inventory differs")
    by_path = {}
    for record in native:
        if (
            not isinstance(record, Mapping)
            or set(record)
            != {
                "guest_input_path",
                "backend_input_sha256",
                "backend_input_len",
                "expected_public_values",
            }
            or re.fullmatch(
                r"[0-9a-f]{64}", str(record.get("backend_input_sha256"))
            )
            is None
            or type(record.get("backend_input_len")) is not int
            or record["backend_input_len"] <= 0
            or re.fullmatch(
                r"0x[0-9a-f]{64}", str(record.get("expected_public_values"))
            )
            is None
        ):
            raise ValueError("context current anchor native identity differs")
        by_path[record["guest_input_path"]] = record
    for row in current_rows:
        record = by_path.get(row.get("guest_input_path"))
        if (
            record is None
            or row.get("guest_input_sha256")
            != f"0x{record['backend_input_sha256']}"
            or row.get("guest_input_bincode_length")
            != record["backend_input_len"]
            or row.get("public_values") != record["expected_public_values"]
        ):
            raise ValueError("context current anchor row input identity differs")


def _replay_current_anchor_native_inputs(
    canary: Mapping[str, Any], guest_launcher: pathlib.Path
) -> None:
    """Recompute the current anchor input identities with the sealed launcher."""
    replay = canary.get("current_anchor", {}).get("replay")
    if not isinstance(replay, Mapping):
        raise ValueError("context current anchor replay is missing")
    expected = replay.get("native_input_identities")
    inputs = replay.get("fixture_inputs")
    if not isinstance(expected, list) or not isinstance(inputs, list):
        raise ValueError("context current anchor replay differs")
    with tempfile.TemporaryDirectory(prefix="context-anchor-native-") as directory:
        root = pathlib.Path(directory)
        actual = []
        for record in inputs:
            relative = pathlib.PurePosixPath(str(record.get("guest_input_path")))
            path = root.joinpath(*relative.parts)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(canonical_json(record.get("input")) + b"\n")
            bundle = _default_anchor_identity_replayer(guest_launcher, path)
            actual.append(
                {
                    "guest_input_path": relative.as_posix(),
                    "backend_input_sha256": str(
                        bundle.get("guest_input_sha256", "")
                    ).removeprefix("0x"),
                    "backend_input_len": bundle.get(
                        "guest_input_bincode_length"
                    ),
                    "expected_public_values": bundle.get(
                        "expected_public_values"
                    ),
                }
            )
    if actual != expected:
        raise ValueError("context current anchor native identity replay differs")


def _current_anchor_source_ledger(
    replay: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    fit: Mapping[str, Any],
    executor_reports_file_sha256: str,
) -> dict[str, Any]:
    inputs = replay["fixture_inputs"]
    return {
        "schema_version": 1,
        "runner_contract": "canonical_anchor_probe_gas_estimator_v1",
        "raw_rows_file_sha256": sha256_bytes(
            b"".join(canonical_json(row) + b"\n" for row in rows)
        ),
        "fit_file_sha256": sha256_bytes(
            (json.dumps(fit, indent=2, sort_keys=True) + "\n").encode()
        ),
        "executor_reports_file_sha256": _sha256(
            executor_reports_file_sha256,
            "context current anchor executor reports file",
        ),
        "fixture_manifest_file_sha256": sha256_bytes(
            canonical_json(replay["fixture_manifest"]) + b"\n"
        ),
        "fixture_input_file_sha256s": {
            record["guest_input_path"]: sha256_bytes(
                canonical_json(record["input"]) + b"\n"
            )
            for record in inputs
        },
    }


def _validate_current_anchor_source_ledger(
    ledger: Any,
    replay: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    fit: Mapping[str, Any],
) -> None:
    if not isinstance(ledger, Mapping) or ledger != _current_anchor_source_ledger(
        replay,
        rows,
        fit,
        ledger.get("executor_reports_file_sha256"),
    ):
        raise ValueError("context current anchor source ledger differs")


def _persist_immutable(path: pathlib.Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists():
        if path.is_symlink() or not path.is_file() or path.read_bytes() != data:
            raise ValueError(f"context immutable artifact conflicts: {path.name}")
        return
    with path.open("xb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def _atomic_replace_bytes(path: pathlib.Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = pathlib.Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if temporary.exists():
            temporary.unlink()


def _validate_safe_campaign_root(path: pathlib.Path, *, label: str) -> pathlib.Path:
    absolute = pathlib.Path(path).absolute()
    for component in (absolute, *absolute.parents):
        if component.is_symlink():
            raise ValueError(f"context {label} path contains a symlink")
    if absolute.exists():
        if not absolute.is_dir() or any(child.is_symlink() for child in absolute.rglob("*")):
            raise ValueError(f"context {label} tree contains a symlink")
    return absolute


def _default_context_source_validator(
    *,
    calibration_run: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    control_opcode_lab_elf: pathlib.Path | None,
) -> dict[str, Any]:
    import opcode_gas

    repo_root = pathlib.Path(__file__).resolve().parents[2]
    expected_launcher = (repo_root / "target/release/guest-launcher").resolve()
    expected_elf = (repo_root / "crates/guests/elf/sp1_revm_opcode_lab.elf").resolve()
    expected_control_elf = (
        repo_root / "crates/guests/elf/sp1_opcode_lab.elf"
    ).resolve()
    if (
        control_opcode_lab_elf is None
        or guest_launcher.resolve() != expected_launcher
        or elf.resolve() != expected_elf
        or control_opcode_lab_elf.resolve() != expected_control_elf
    ):
        raise ValueError(
            "context campaign requires the release launcher and both canonical ELFs"
        )
    identity = opcode_gas.validate_calibration_execution_identity(calibration_run)
    launcher_sha256 = opcode_gas.validate_calibration_guest_launcher(identity, guest_launcher)
    elf_sha256 = opcode_gas.sha256_file(elf)
    control_elf_sha256 = opcode_gas.sha256_file(control_opcode_lab_elf)
    if identity.get("guest_artifacts", {}).get(
        "crates/guests/elf/sp1_revm_opcode_lab.elf"
    ) != elf_sha256 or identity.get("guest_artifacts", {}).get(
        "crates/guests/elf/sp1_opcode_lab.elf"
    ) != control_elf_sha256:
        raise ValueError("context campaign ELFs differ from calibration identity")
    return {
        "evidence_mode": "production_execution",
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
        "calibration_identity": copy.deepcopy(identity),
        "implementation_revision": identity["implementation_revision"],
        "launcher_sha256": launcher_sha256,
        "revm_elf_sha256": elf_sha256,
        "control_opcode_lab_elf_sha256": control_elf_sha256,
    }


def _round_samples(bound: int) -> list[tuple[str, int]]:
    prefix = ADAPTIVE_PREFIXES[ADAPTIVE_ROUNDS.index(bound)]
    return [
        *((PREFIX_PLACEMENT, count) for count in (*prefix, bound)),
        (TAIL_PLACEMENT, 1),
    ]


def _canonical_round_rows(
    *,
    manifest: Mapping[str, Any],
    selected: list[str],
    bound: int,
    fixtures_root: pathlib.Path,
    reports: list[Mapping[str, Any]],
    guest_launcher: pathlib.Path,
    identity_replayer,
) -> list[dict[str, Any]]:
    specs = [
        (scenario, placement, count, lane, repeat)
        for scenario in selected
        for placement, count in _round_samples(bound)
        for lane in ("control", "target")
        for repeat in range(manifest["repeats"])
    ]
    if len(reports) != len(specs):
        raise ValueError("context executor returned the wrong report count")
    identities: dict[pathlib.Path, Mapping[str, Any]] = {}
    rows = []
    for spec, raw_report in zip(specs, reports):
        scenario, placement, count, lane, repeat = spec
        input_path = (
            fixtures_root
            / scenario
            / str(bound)
            / placement
            / str(count)
            / lane
            / "guest-input.json"
        )
        fixture = json.loads(input_path.read_bytes())
        bundle = identities.get(input_path)
        if bundle is None:
            try:
                bundle = identity_replayer(guest_launcher, input_path)
            except TypeError:
                bundle = identity_replayer(input_path)
            identities[input_path] = bundle
        report = dict(raw_report)
        supplied_input = report.get("input")
        if not isinstance(supplied_input, str) or pathlib.Path(supplied_input).resolve() != input_path.resolve():
            raise ValueError("context execution report belongs to another fixture")
        report["input"] = str(input_path.relative_to(fixtures_root))
        _admit_executed_context_row(manifest, fixture, bundle, report)
        gas = report.get("prover_gas", report.get("gas"))
        rows.append(
            {
                "scenario": scenario,
                "lane": lane,
                "count": count,
                "placement": placement,
                "repeat_index": repeat,
                "generator_bound": bound,
                "prover_gas": gas,
                "fixture": fixture,
                "identity": copy.deepcopy(bundle),
                "formal_report": report,
            }
        )
    return rows


def _fit_production_rows(
    manifest: Mapping[str, Any], rows: list[Mapping[str, Any]]
) -> dict[str, Any]:
    for row in rows:
        if set(row) != {
            "scenario",
            "lane",
            "count",
            "placement",
            "repeat_index",
            "generator_bound",
            "prover_gas",
            "fixture",
            "identity",
            "formal_report",
        }:
            raise ValueError("context production row shape differs")
        if (
            row["fixture"]
            != generate_context_fixture(
                manifest,
                row["scenario"],
                row["lane"],
                row["count"],
                generator_bound=row["generator_bound"],
                placement=row["placement"],
            )
            or type(row["repeat_index"]) is not int
            or not 0 <= row["repeat_index"] < manifest["repeats"]
            or row["prover_gas"]
            != row["formal_report"].get(
                "prover_gas", row["formal_report"].get("gas")
            )
        ):
            raise ValueError("context production row fixture or metric differs")
        _admit_executed_context_row(
            manifest, row["fixture"], row["identity"], row["formal_report"]
        )
    projected = [
        {
            "scenario": row["scenario"],
            "lane": row["lane"],
            "count": row["count"],
            "placement": row["placement"],
            "repeat_index": row["repeat_index"],
            "generator_bound": row["generator_bound"],
            "prover_gas": row["prover_gas"],
            "fixture": row["fixture"],
            "identity": {
                "schema_version": row["identity"]["identity"]["schema_version"],
                "block_environment_sha256": row["identity"]["identity"][
                    "block_environment_sha256"
                ],
            },
            "trace": {
                "schema_version": row["formal_report"]["controlled_trace"]["schema_version"],
                "block_environment_sha256": row["formal_report"]["controlled_trace"][
                    "block_environment_sha256"
                ],
            },
        }
        for row in rows
    ]
    selected = list(dict.fromkeys(row["scenario"] for row in rows))
    return fit_context_campaign_rows(
        manifest, projected, selected_scenarios=selected
    )


def _read_context_round(
    *,
    manifest: Mapping[str, Any],
    run: pathlib.Path,
    record: Mapping[str, Any],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    bound = record.get("generator_bound")
    selected = record.get("selected_scenarios")
    raw_path = run / str(record.get("raw_rows"))
    fit_path = run / str(record.get("fit"))
    executor_path = run / str(record.get("executor_reports"))
    if (
        bound not in ADAPTIVE_ROUNDS
        or not isinstance(selected, list)
        or raw_path.is_symlink()
        or not raw_path.is_file()
        or sha256_bytes(raw_path.read_bytes()) != record.get("raw_rows_sha256")
        or fit_path.is_symlink()
        or not fit_path.is_file()
        or sha256_bytes(fit_path.read_bytes()) != record.get("fit_sha256")
        or executor_path.is_symlink()
        or not executor_path.is_file()
        or sha256_bytes(executor_path.read_bytes())
        != record.get("executor_reports_sha256")
    ):
        raise ValueError("context persisted adaptive round differs")
    rows = [json.loads(line) for line in raw_path.read_bytes().splitlines()]
    fit = json.loads(fit_path.read_bytes())
    executor_reports = [
        json.loads(line) for line in executor_path.read_bytes().splitlines()
    ]
    if len(executor_reports) != len(rows):
        raise ValueError("context persisted executor report count differs")
    for executor_report, row in zip(executor_reports, rows):
        normalized = copy.deepcopy(executor_report)
        source_input = normalized.get("input")
        expected_input = row["formal_report"].get("input")
        if (
            not isinstance(source_input, str)
            or not isinstance(expected_input, str)
            or not pathlib.PurePath(source_input).as_posix().endswith(expected_input)
        ):
            raise ValueError("context persisted executor input differs")
        normalized["input"] = expected_input
        if normalized != row["formal_report"]:
            raise ValueError("context persisted executor report differs")
    replay = _fit_production_rows(manifest, rows)
    if set(replay) != set(selected) or fit != replay:
        raise ValueError("context adaptive round fit differs on replay")
    return rows, fit


def run_context_opcode_campaign(
    *,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path | None = None,
    run: pathlib.Path,
    fixtures_root: pathlib.Path,
    guest_launcher: pathlib.Path,
    elf: pathlib.Path,
    control_opcode_lab_elf: pathlib.Path | None = None,
    executor=_default_context_executor,
    identity_replayer=_default_identity_replayer,
    source_validator=_default_context_source_validator,
    phase_hook=lambda _phase: None,
) -> dict[str, Any]:
    """Execute/resume frozen adaptive rounds; accepted scenarios are never re-run."""
    production_runner = (
        executor is _default_context_executor
        and identity_replayer is _default_identity_replayer
        and source_validator is _default_context_source_validator
    )
    manifest = load_context_manifest(manifest_path)
    run = _validate_safe_campaign_root(run, label="run")
    fixtures_root = _validate_safe_campaign_root(fixtures_root, label="fixtures")
    if production_runner:
        if calibration_run is None:
            raise ValueError("context production runner requires a calibration run")
        calibration_root = pathlib.Path(calibration_run).resolve(strict=True)
        if (
            run != calibration_root / "context-campaign"
            or fixtures_root != calibration_root / "context-fixtures"
        ):
            raise ValueError("context production runner path contract differs")
    if (
        run == fixtures_root
        or run.is_relative_to(fixtures_root)
        or fixtures_root.is_relative_to(run)
    ):
        raise ValueError("context run and fixture roots overlap")
    source = source_validator(
        calibration_run=calibration_run,
        guest_launcher=guest_launcher,
        elf=elf,
        control_opcode_lab_elf=control_opcode_lab_elf,
    )
    identity = {
        "schema_version": 1,
        "purpose": "context_opcode_adaptive_campaign",
        "evidence_mode": (
            "production_execution" if production_runner else "synthetic_test_only"
        ),
        "runner_contract": (
            "canonical_release_guest_launcher_v1"
            if production_runner
            else "injected_test_hooks"
        ),
        "manifest_sha256": sha256_bytes(canonical_json(manifest)),
        "manifest_file_sha256": sha256_bytes(pathlib.Path(manifest_path).read_bytes()),
        "source": source,
    }
    terminal_paths = (run / "rows.jsonl", run / "terminal.json")
    if any(path.exists() for path in terminal_paths):
        if not all(path.is_file() and not path.is_symlink() for path in terminal_paths):
            raise ValueError("context terminal run is incomplete")
        evidence = load_context_adaptive_evidence(run, manifest)
        if evidence["identity"] != identity:
            raise ValueError("context terminal run identity differs")
        ledger = evidence["production_run_ledger"]
        return {
            "status": "complete",
            "scenario_reports": evidence["terminal"]["scenario_reports"],
            "rows": evidence["terminal"]["rows"],
            "identity": identity,
            "decisions": ledger["decisions"],
            "terminal": ledger["terminal"],
        }
    run.mkdir(parents=True, exist_ok=True)
    fixtures_root.mkdir(parents=True, exist_ok=True)
    _persist_immutable(run / "identity.json", canonical_json(identity) + b"\n")
    decisions_path = run / "decisions.json"
    decisions_seal = run / "decisions.sha256"
    decisions = {"schema_version": 1, "identity_sha256": sha256_bytes(canonical_json(identity)), "rounds": []}
    if decisions_path.exists():
        decisions_bytes = decisions_path.read_bytes()
        decisions = json.loads(decisions_bytes)
        expected_seal = (sha256_bytes(decisions_bytes) + "\n").encode()
        if decisions_seal.is_symlink() or (
            decisions_seal.exists() and not decisions_seal.is_file()
        ):
            raise ValueError("context adaptive decision seal differs")
        if not decisions_seal.exists() or decisions_seal.read_bytes() != expected_seal:
            _atomic_replace_bytes(decisions_seal, expected_seal)
    elif decisions_seal.exists():
        raise ValueError("context adaptive decision seal exists without ledger")
    if decisions.get("schema_version") != 1 or decisions.get("identity_sha256") != sha256_bytes(canonical_json(identity)):
        raise ValueError("context adaptive decision identity differs")

    remaining = [scenario["name"] for scenario in manifest["scenarios"]]
    terminal: dict[str, tuple[dict[str, Any], list[dict[str, Any]]]] = {}
    for index, record in enumerate(decisions.get("rounds", [])):
        if index >= len(ADAPTIVE_ROUNDS) or record.get("generator_bound") != ADAPTIVE_ROUNDS[index]:
            raise ValueError("context adaptive rounds are not a contiguous frozen prefix")
        if record.get("selected_scenarios") != remaining:
            raise ValueError("context adaptive round selection differs")
        rows, reports = _read_context_round(manifest=manifest, run=run, record=record)
        next_remaining = []
        for scenario in remaining:
            report = reports[scenario]
            scenario_rows = [row for row in rows if row["scenario"] == scenario]
            if report["status"] == "passed" or index == len(ADAPTIVE_ROUNDS) - 1:
                terminal[scenario] = (report, scenario_rows)
            else:
                next_remaining.append(scenario)
        if record.get("next_scenarios") != next_remaining:
            raise ValueError("context adaptive next-round selection differs")
        remaining = next_remaining

    if remaining:
        start = len(decisions["rounds"])
        for index in range(start, len(ADAPTIVE_ROUNDS)):
            bound = ADAPTIVE_ROUNDS[index]
            selected = list(remaining)
            fixture_paths = []
            for scenario in selected:
                for placement, count in _round_samples(bound):
                    for lane in ("control", "target"):
                        fixture = generate_context_fixture(
                            manifest,
                            scenario,
                            lane,
                            count,
                            generator_bound=bound,
                            placement=placement,
                        )
                        path = fixtures_root / scenario / str(bound) / placement / str(count) / lane / "guest-input.json"
                        _persist_immutable(path, canonical_json(fixture) + b"\n")
                        fixture_paths.extend([path] * manifest["repeats"])
            reports_path = run / "executor" / f"context-round-{bound}.jsonl"
            reports = None
            if reports_path.exists():
                if reports_path.is_symlink() or not reports_path.is_file():
                    raise ValueError("context orphan executor output is not a regular file")
                try:
                    orphan_bytes = reports_path.read_bytes()
                    orphan_rows = [json.loads(line) for line in orphan_bytes.splitlines()]
                except (OSError, UnicodeDecodeError, json.JSONDecodeError):
                    orphan_rows = []
                if len(orphan_rows) == len(fixture_paths) and orphan_bytes.endswith(b"\n"):
                    reports = orphan_rows
                else:
                    reports_path.unlink()
            if reports is None:
                executor(
                    fixtures=fixture_paths,
                    reports_jsonl=reports_path,
                    guest_launcher=guest_launcher,
                    elf=elf,
                )
                reports = [
                    json.loads(line) for line in reports_path.read_bytes().splitlines()
                ]
            phase_hook("after_executor_output")
            rows = _canonical_round_rows(
                manifest=manifest,
                selected=selected,
                bound=bound,
                fixtures_root=fixtures_root,
                reports=reports,
                guest_launcher=guest_launcher,
                identity_replayer=identity_replayer,
            )
            round_reports = _fit_production_rows(manifest, rows)
            raw_relative = pathlib.Path("raw") / f"context-round-{bound}.jsonl"
            fit_relative = pathlib.Path("fit") / f"context-round-{bound}.json"
            raw_bytes = b"".join(canonical_json(row) + b"\n" for row in rows)
            fit_bytes = canonical_json(round_reports) + b"\n"
            _persist_immutable(run / raw_relative, raw_bytes)
            _persist_immutable(run / fit_relative, fit_bytes)
            phase_hook("after_round_rows")
            next_remaining = []
            for scenario in selected:
                scenario_rows = [row for row in rows if row["scenario"] == scenario]
                if round_reports[scenario]["status"] == "passed" or index == len(ADAPTIVE_ROUNDS) - 1:
                    terminal[scenario] = (round_reports[scenario], scenario_rows)
                else:
                    next_remaining.append(scenario)
            decisions["rounds"].append(
                {
                    "generator_bound": bound,
                    "selected_scenarios": selected,
                    "raw_rows": str(raw_relative),
                    "raw_rows_sha256": sha256_bytes(raw_bytes),
                    "fit": str(fit_relative),
                    "fit_sha256": sha256_bytes(fit_bytes),
                    "executor_reports": str(reports_path.relative_to(run)),
                    "executor_reports_sha256": sha256_bytes(
                        reports_path.read_bytes()
                    ),
                    "terminal_decisions": [
                        {"scenario": scenario, "status": round_reports[scenario]["status"]}
                        for scenario in selected
                    ],
                    "next_scenarios": next_remaining,
                }
            )
            decisions_bytes = canonical_json(decisions) + b"\n"
            _atomic_replace_bytes(decisions_path, decisions_bytes)
            phase_hook("after_decision_ledger")
            _atomic_replace_bytes(
                decisions_seal,
                (sha256_bytes(decisions_bytes) + "\n").encode(),
            )
            phase_hook("after_decision_seal")
            remaining = next_remaining
            if not remaining:
                break
    if remaining:
        raise AssertionError("context adaptive campaign ended without terminal scenarios")
    ordered_names = [scenario["name"] for scenario in manifest["scenarios"]]
    reports = {name: terminal[name][0] for name in ordered_names}
    rows = [row for name in ordered_names for row in terminal[name][1]]
    rows_bytes = b"".join(canonical_json(row) + b"\n" for row in rows)
    _persist_immutable(run / "rows.jsonl", rows_bytes)
    terminal_payload = {
        "schema_version": 1,
        "status": "complete",
        "identity_sha256": decisions["identity_sha256"],
        "decisions_sha256": sha256_bytes(decisions_path.read_bytes()),
        "rows_sha256": sha256_bytes(rows_bytes),
        "scenario_reports": reports,
    }
    _persist_immutable(run / "terminal.json", canonical_json(terminal_payload) + b"\n")
    return {
        "status": "complete",
        "scenario_reports": reports,
        "rows": rows,
        "identity": identity,
        "decisions": decisions,
        "terminal": terminal_payload,
    }


def synthetic_passing_production_rows(
    manifest: Mapping[str, Any],
) -> list[dict[str, Any]]:
    """Full formal-row shape used only to exercise sealed replay in unit tests."""
    rows = []
    for scenario in manifest["scenarios"]:
        for placement, count in _round_samples(ADAPTIVE_ROUNDS[0]):
            for lane in ("control", "target"):
                fixture = generate_context_fixture(
                    manifest,
                    scenario["name"],
                    lane,
                    count,
                    generator_bound=ADAPTIVE_ROUNDS[0],
                    placement=placement,
                )
                identity = _synthetic_identity_for_fixture(fixture)
                for repeat in range(manifest["repeats"]):
                    gas = 10_000 + repeat + (
                        100 * count if lane == "target" else 0
                    )
                    report = {
                        "stage": "revm-opcode-lab",
                        "mode": "execute",
                        "proof_mode": "compressed",
                        "input": f"synthetic/{scenario['name']}/{lane}/{count}",
                        "prover_gas": gas,
                        "gas": gas,
                        "primary_workload_metric": {
                            "label": "prover_gas",
                            "count": gas,
                        },
                        "public_values": identity["expected_public_values"],
                        "exit_code": 0,
                        "sp1_execution_engine": "gas-estimator",
                        **identity["report"],
                    }
                    rows.append(
                        {
                            "scenario": scenario["name"],
                            "lane": lane,
                            "count": count,
                            "placement": placement,
                            "repeat_index": repeat,
                            "generator_bound": ADAPTIVE_ROUNDS[0],
                            "prover_gas": gas,
                            "fixture": fixture,
                            "identity": identity,
                            "formal_report": report,
                        }
                    )
    return rows


def _fit_any_context_rows(
    manifest: Mapping[str, Any], rows: list[Mapping[str, Any]]
) -> dict[str, Any]:
    if rows and "formal_report" in rows[0]:
        return _fit_production_rows(manifest, rows)
    return fit_context_campaign_rows(manifest, rows)


def build_context_adaptive_evidence(
    *,
    manifest: Mapping[str, Any],
    identity: Mapping[str, Any],
    rounds: list[Mapping[str, Any]],
) -> dict[str, Any]:
    """Canonical replay ledger for all adaptive rows, fits, and decisions."""
    remaining = [scenario["name"] for scenario in manifest["scenarios"]]
    terminal_rows: dict[str, list[Mapping[str, Any]]] = {}
    terminal_reports: dict[str, Mapping[str, Any]] = {}
    canonical_rounds = []
    for index, supplied in enumerate(rounds):
        bound = supplied.get("generator_bound")
        rows = list(supplied.get("rows", []))
        fit = _fit_any_context_rows(manifest, rows)
        if (
            index >= len(ADAPTIVE_ROUNDS)
            or bound != ADAPTIVE_ROUNDS[index]
            or supplied.get("selected_scenarios") != remaining
            or set(fit) != set(remaining)
            or supplied.get("fit") != fit
        ):
            raise ValueError("context adaptive evidence round differs")
        next_remaining = []
        for scenario in remaining:
            scenario_rows = [row for row in rows if row["scenario"] == scenario]
            if fit[scenario]["status"] == "passed" or index == len(ADAPTIVE_ROUNDS) - 1:
                terminal_rows[scenario] = scenario_rows
                terminal_reports[scenario] = fit[scenario]
            else:
                next_remaining.append(scenario)
        if supplied.get("next_scenarios") != next_remaining:
            raise ValueError("context adaptive evidence decision differs")
        canonical_rounds.append(
            {
                "generator_bound": bound,
                "selected_scenarios": list(remaining),
                "next_scenarios": next_remaining,
                "rows": copy.deepcopy(rows),
                "rows_sha256": sha256_bytes(canonical_json(rows)),
                "fit": copy.deepcopy(fit),
                "fit_sha256": sha256_bytes(canonical_json(fit)),
            }
        )
        remaining = next_remaining
        if not remaining:
            break
    if remaining or len(canonical_rounds) != len(rounds):
        raise ValueError("context adaptive evidence is not terminal")
    ordered = [scenario["name"] for scenario in manifest["scenarios"]]
    terminal = {
        "scenario_reports": {
            name: copy.deepcopy(terminal_reports[name]) for name in ordered
        },
        "rows": [
            copy.deepcopy(row) for name in ordered for row in terminal_rows[name]
        ],
    }
    evidence = {
        "schema_version": 1,
        "purpose": "context_opcode_adaptive_evidence",
        "identity": copy.deepcopy(identity),
        "rounds": canonical_rounds,
        "terminal": terminal,
    }
    evidence["artifact_sha256"] = sha256_bytes(canonical_json(evidence))
    return evidence


def validate_context_adaptive_evidence(
    manifest: Mapping[str, Any],
    evidence: Mapping[str, Any],
    source_identity: Mapping[str, Any],
) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    unsigned = dict(evidence)
    artifact_sha256 = unsigned.pop("artifact_sha256", None)
    identity = evidence.get("identity")
    if isinstance(identity, Mapping) and identity.get("evidence_mode") != "production_execution":
        raise ValueError("synthetic context adaptive evidence cannot be sealed")
    if (
        set(evidence)
        != {
            "schema_version",
            "purpose",
            "identity",
            "rounds",
            "terminal",
            "production_run_ledger",
            "artifact_sha256",
        }
        or evidence.get("schema_version") != 1
        or evidence.get("purpose") != "context_opcode_adaptive_evidence"
        or artifact_sha256 != sha256_bytes(canonical_json(unsigned))
        or not isinstance(identity, Mapping)
        or identity.get("schema_version") != 1
        or identity.get("purpose") != "context_opcode_adaptive_campaign"
        or identity.get("evidence_mode") != "production_execution"
        or identity.get("runner_contract")
        != "canonical_release_guest_launcher_v1"
        or identity.get("manifest_sha256") != sha256_bytes(canonical_json(manifest))
        or not isinstance(identity.get("source"), Mapping)
    ):
        raise ValueError("context adaptive evidence identity differs")
    source = identity["source"]
    _validate_production_campaign_source(source)
    if any(
        source.get(campaign_field) != source_identity[result_field]
        for campaign_field, result_field in (
            ("calibration_id", "calibration_id"),
            ("calibration_identity_sha256", "calibration_identity_sha256"),
            ("implementation_revision", "execution_revision"),
            ("revm_elf_sha256", "elf_sha256"),
            ("control_opcode_lab_elf_sha256", "control_opcode_lab_elf_sha256"),
            ("launcher_sha256", "launcher_sha256"),
        )
    ):
        raise ValueError("context adaptive evidence source differs")
    rounds = evidence.get("rounds")
    if not isinstance(rounds, list):
        raise ValueError("context adaptive evidence rounds differ")
    replay = build_context_adaptive_evidence(
        manifest=manifest,
        identity=identity,
        rounds=rounds,
    )
    ledger = evidence.get("production_run_ledger")
    portable = dict(evidence)
    portable.pop("artifact_sha256")
    portable.pop("production_run_ledger")
    portable["artifact_sha256"] = sha256_bytes(canonical_json(portable))
    if replay != portable:
        raise ValueError("context adaptive evidence replay differs")
    _validate_context_production_run_ledger(identity, replay, ledger)
    terminal = replay["terminal"]
    return terminal["rows"], terminal["scenario_reports"]


def _validate_context_production_run_ledger(
    identity: Mapping[str, Any],
    evidence: Mapping[str, Any],
    ledger: Any,
) -> None:
    if not isinstance(ledger, Mapping):
        raise ValueError("context production run ledger is missing")
    decisions = ledger.get("decisions")
    terminal = ledger.get("terminal")
    rounds = evidence["rounds"]
    if (
        set(ledger)
        != {
            "schema_version",
            "purpose",
            "identity_file_sha256",
            "decisions",
            "decisions_file_sha256",
            "decisions_seal_sha256",
            "terminal",
            "terminal_file_sha256",
            "rows_file_sha256",
            "executor_source_file_sha256s",
            "portable_verification_scope",
        }
        or ledger.get("schema_version") != 1
        or ledger.get("purpose") != "context_create_only_run_ledger"
        or ledger.get("portable_verification_scope")
        != "integrity_and_exact_fit_replay_not_prover_gas_reauthentication"
        or not isinstance(decisions, Mapping)
        or not isinstance(terminal, Mapping)
        or ledger.get("identity_file_sha256")
        != sha256_bytes(canonical_json(identity) + b"\n")
        or ledger.get("decisions_file_sha256")
        != sha256_bytes(canonical_json(decisions) + b"\n")
        or ledger.get("decisions_seal_sha256")
        != sha256_bytes(
            (sha256_bytes(canonical_json(decisions) + b"\n") + "\n").encode()
        )
        or ledger.get("terminal_file_sha256")
        != sha256_bytes(canonical_json(terminal) + b"\n")
        or ledger.get("rows_file_sha256")
        != sha256_bytes(
            b"".join(
                canonical_json(row) + b"\n"
                for row in evidence["terminal"]["rows"]
            )
        )
        or decisions.get("identity_sha256")
        != sha256_bytes(canonical_json(identity))
        or terminal.get("identity_sha256") != decisions.get("identity_sha256")
        or terminal.get("decisions_sha256")
        != ledger.get("decisions_file_sha256")
        or terminal.get("rows_sha256") != ledger.get("rows_file_sha256")
        or not isinstance(decisions.get("rounds"), list)
        or len(decisions["rounds"]) != len(rounds)
        or ledger.get("executor_source_file_sha256s")
        != [record.get("executor_reports_sha256") for record in decisions["rounds"]]
        or any(
            re.fullmatch(r"[0-9a-f]{64}", str(value)) is None
            for value in ledger.get("executor_source_file_sha256s", [])
        )
    ):
        raise ValueError("context production run ledger differs")
    for record, round_evidence in zip(decisions["rounds"], rounds):
        raw_bytes = b"".join(
            canonical_json(row) + b"\n" for row in round_evidence["rows"]
        )
        fit_bytes = canonical_json(round_evidence["fit"]) + b"\n"
        if (
            record.get("generator_bound") != round_evidence["generator_bound"]
            or record.get("selected_scenarios")
            != round_evidence["selected_scenarios"]
            or record.get("next_scenarios") != round_evidence["next_scenarios"]
            or record.get("raw_rows_sha256") != sha256_bytes(raw_bytes)
            or record.get("fit_sha256") != sha256_bytes(fit_bytes)
        ):
            raise ValueError("context production run round ledger differs")


def _validate_production_campaign_source(source: Mapping[str, Any]) -> None:
    import opcode_gas

    expected_fields = {
        "evidence_mode",
        "calibration_id",
        "calibration_identity_sha256",
        "calibration_identity",
        "implementation_revision",
        "launcher_sha256",
        "revm_elf_sha256",
        "control_opcode_lab_elf_sha256",
    }
    calibration_identity = source.get("calibration_identity")
    guest_artifacts = (
        calibration_identity.get("guest_artifacts")
        if isinstance(calibration_identity, Mapping)
        else None
    )
    required_guest_artifacts = {
        "crates/guests/elf/sp1_revm_opcode_lab.elf": source.get(
            "revm_elf_sha256"
        ),
        "crates/guests/elf/sp1_revm_opcode_lab.vk.bin": None,
        "crates/guests/elf/sp1_opcode_lab.elf": source.get(
            "control_opcode_lab_elf_sha256"
        ),
        "crates/guests/elf/sp1_opcode_lab.vk.bin": None,
    }
    if (
        set(source) != expected_fields
        or source.get("evidence_mode") != "production_execution"
        or not isinstance(calibration_identity, Mapping)
        or source.get("calibration_identity_sha256")
        != sha256_bytes(canonical_json(calibration_identity))
        or source.get("calibration_id")
        != str(source.get("calibration_identity_sha256"))[:24]
        or calibration_identity.get("implementation_revision")
        != source.get("implementation_revision")
        or re.fullmatch(r"[0-9a-f]{40}", str(source.get("implementation_revision")))
        is None
        or calibration_identity.get("guest_launcher_sha256")
        != source.get("launcher_sha256")
        or re.fullmatch(r"[0-9a-f]{64}", str(source.get("launcher_sha256")))
        is None
        or not isinstance(guest_artifacts, Mapping)
        or calibration_identity.get("guest_artifacts_sha256")
        != sha256_bytes(canonical_json(guest_artifacts))
        or any(
            path not in guest_artifacts
            or re.fullmatch(r"[0-9a-f]{64}", str(guest_artifacts[path])) is None
            or (expected is not None and guest_artifacts[path] != expected)
            for path, expected in required_guest_artifacts.items()
        )
        or not isinstance(calibration_identity.get("rust_version"), str)
        or not isinstance(calibration_identity.get("sp1_sdk_version"), str)
    ):
        raise ValueError("context production calibration identity differs")
    opcode_gas.validate_calibration_version_identity(calibration_identity)


def synthetic_passing_adaptive_evidence(
    manifest: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    source_identity: Mapping[str, Any],
) -> dict[str, Any]:
    """Structurally complete one-round evidence used only by unit tests."""
    fit = _fit_any_context_rows(manifest, rows)
    names = [scenario["name"] for scenario in manifest["scenarios"]]
    return build_context_adaptive_evidence(
        manifest=manifest,
        identity={
            "schema_version": 1,
            "purpose": "context_opcode_adaptive_campaign",
            "evidence_mode": "synthetic_test_only",
            "manifest_sha256": sha256_bytes(canonical_json(manifest)),
            "manifest_file_sha256": "0" * 64,
            "source": {
                "calibration_id": "synthetic-test-only",
                "calibration_identity_sha256": "1" * 64,
                "implementation_revision": "2" * 40,
                "launcher_sha256": source_identity["launcher_sha256"],
                "revm_elf_sha256": source_identity["elf_sha256"],
                "control_opcode_lab_elf_sha256": source_identity[
                    "control_opcode_lab_elf_sha256"
                ],
            },
        },
        rounds=[
            {
                "generator_bound": ADAPTIVE_ROUNDS[0],
                "selected_scenarios": names,
                "next_scenarios": [],
                "rows": rows,
                "fit": fit,
            }
        ],
    )


def load_context_adaptive_evidence(
    run: pathlib.Path, manifest: Mapping[str, Any]
) -> dict[str, Any]:
    """Load and replay the complete immutable adaptive-run ledger."""
    run = pathlib.Path(run)
    if run.is_symlink() or not run.is_dir():
        raise ValueError("context adaptive run must be a non-symlink directory")
    required = {
        "identity.json",
        "decisions.json",
        "decisions.sha256",
        "rows.jsonl",
        "terminal.json",
    }
    if any((run / name).is_symlink() or not (run / name).is_file() for name in required):
        raise ValueError("context adaptive run is incomplete")
    identity_bytes = (run / "identity.json").read_bytes()
    decisions_bytes = (run / "decisions.json").read_bytes()
    terminal_bytes = (run / "terminal.json").read_bytes()
    identity = json.loads(identity_bytes)
    decisions = json.loads(decisions_bytes)
    terminal = json.loads(terminal_bytes)
    if (
        identity_bytes != canonical_json(identity) + b"\n"
        or decisions_bytes != canonical_json(decisions) + b"\n"
        or terminal_bytes != canonical_json(terminal) + b"\n"
        or (run / "decisions.sha256").read_bytes()
        != (sha256_bytes(decisions_bytes) + "\n").encode()
        or decisions.get("identity_sha256") != sha256_bytes(canonical_json(identity))
        or terminal.get("identity_sha256") != decisions.get("identity_sha256")
        or terminal.get("decisions_sha256") != sha256_bytes(decisions_bytes)
    ):
        raise ValueError("context adaptive run identity or decision seal differs")
    rounds = []
    for record in decisions.get("rounds", []):
        rows, fit = _read_context_round(manifest=manifest, run=run, record=record)
        rounds.append(
            {
                "generator_bound": record["generator_bound"],
                "selected_scenarios": record["selected_scenarios"],
                "next_scenarios": record["next_scenarios"],
                "rows": rows,
                "fit": fit,
            }
        )
    evidence = build_context_adaptive_evidence(
        manifest=manifest, identity=identity, rounds=rounds
    )
    terminal_rows_bytes = (run / "rows.jsonl").read_bytes()
    terminal_rows = [json.loads(line) for line in terminal_rows_bytes.splitlines()]
    if (
        terminal.get("status") != "complete"
        or terminal.get("rows_sha256") != sha256_bytes(terminal_rows_bytes)
        or terminal_rows != evidence["terminal"]["rows"]
        or terminal.get("scenario_reports")
        != evidence["terminal"]["scenario_reports"]
    ):
        raise ValueError("context adaptive terminal evidence differs")
    ledger = {
        "schema_version": 1,
        "purpose": "context_create_only_run_ledger",
        "identity_file_sha256": sha256_bytes(identity_bytes),
        "decisions": copy.deepcopy(decisions),
        "decisions_file_sha256": sha256_bytes(decisions_bytes),
        "decisions_seal_sha256": sha256_bytes(
            (run / "decisions.sha256").read_bytes()
        ),
        "terminal": copy.deepcopy(terminal),
        "terminal_file_sha256": sha256_bytes(terminal_bytes),
        "rows_file_sha256": sha256_bytes(terminal_rows_bytes),
        "executor_source_file_sha256s": [
            record["executor_reports_sha256"] for record in decisions["rounds"]
        ],
        "portable_verification_scope": (
            "integrity_and_exact_fit_replay_not_prover_gas_reauthentication"
        ),
    }
    unsigned = dict(evidence)
    unsigned.pop("artifact_sha256")
    unsigned["production_run_ledger"] = ledger
    unsigned["artifact_sha256"] = sha256_bytes(canonical_json(unsigned))
    evidence = unsigned
    return evidence


def synthetic_passing_compatibility_canary(
    *, elf_sha256: str = "1" * 64, launcher_sha256: str = "2" * 64
) -> dict[str, Any]:
    legacy = {
        "status": "passed",
        "relation_ids": list(OSAKA_CANARY_RELATION_IDS),
        "quality_gates": {"per_relation_drift_ape_max": "0.10", "drift_mape_max": "0.05"},
    }
    return {
        "schema_version": 1,
        "purpose": "context_opcode_elf_compatibility_canary",
        "evidence_mode": "synthetic_test_only",
        "elf_sha256": elf_sha256,
        "launcher_sha256": launcher_sha256,
        "legacy_osaka_canary": legacy,
        "control_relations": {
            key: {"status": "passed", "drift_ape": "0", "repeat_count": 3}
            for key in ("opcode:0x5f", "opcode:0x90")
        },
        "status": "passed",
    }


def _capture_legacy_osaka_replay(
    *,
    output_root: pathlib.Path,
    controlled_manifest: pathlib.Path,
    historical_observations: list[Mapping[str, Any]],
) -> dict[str, Any]:
    decisions_path = output_root / "canary-decisions.json"
    decisions = json.loads(decisions_path.read_bytes())
    rounds = []
    for record in decisions.get("rounds", []):
        raw_path = output_root / str(record["raw_runs"])
        result_path = output_root / str(record["result"])
        if any(path.is_symlink() or not path.is_file() for path in (raw_path, result_path)):
            raise ValueError("context legacy canary persisted source differs")
        rows = [json.loads(line) for line in raw_path.read_bytes().splitlines()]
        result = json.loads(result_path.read_bytes())
        if (
            sha256_bytes(raw_path.read_bytes()) != record.get("raw_runs_sha256")
            or sha256_bytes(result_path.read_bytes()) != record.get("result_sha256")
        ):
            raise ValueError("context legacy canary persisted hash differs")
        rounds.append(
            {
                "generator_max_count": record["generator_max_count"],
                "relation_ids": copy.deepcopy(record["relation_ids"]),
                "rows": rows,
                "rows_sha256": sha256_bytes(canonical_json(rows)),
                "result": result,
                "result_sha256": sha256_bytes(canonical_json(result)),
                "source_raw_file_sha256": record["raw_runs_sha256"],
                "source_result_file_sha256": record["result_sha256"],
            }
        )
    manifest_bytes = pathlib.Path(controlled_manifest).read_bytes()
    return {
        "schema_version": 1,
        "purpose": "context_legacy_osaka_portable_replay",
        "historical_derivation_artifact_sha256": (
            HISTORICAL_ANCHOR_DERIVATION_SHA256
        ),
        "historical_relation_artifact_sha256": (
            HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
        ),
        "historical_observations": copy.deepcopy(historical_observations),
        "historical_observations_sha256": sha256_bytes(
            canonical_json(historical_observations)
        ),
        "controlled_manifest_utf8": manifest_bytes.decode(),
        "controlled_manifest_file_sha256": sha256_bytes(manifest_bytes),
        "decisions": decisions,
        "decisions_sha256": sha256_bytes(canonical_json(decisions)),
        "rounds": rounds,
    }


def _replay_legacy_osaka_evidence(
    replay: Mapping[str, Any], provenance: Mapping[str, Any]
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    import opcode_gas

    historical = replay.get("historical_observations")
    rounds = replay.get("rounds")
    decisions = replay.get("decisions")
    manifest_text = replay.get("controlled_manifest_utf8")
    if (
        replay.get("schema_version") != 1
        or replay.get("purpose") != "context_legacy_osaka_portable_replay"
        or replay.get("historical_derivation_artifact_sha256")
        != HISTORICAL_ANCHOR_DERIVATION_SHA256
        or replay.get("historical_relation_artifact_sha256")
        != HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256
        or not isinstance(historical, list)
        or sha256_bytes(canonical_json(historical))
        != HISTORICAL_OSAKA_OBSERVATIONS_SHA256
        or replay.get("historical_observations_sha256")
        != HISTORICAL_OSAKA_OBSERVATIONS_SHA256
        or not isinstance(manifest_text, str)
        or sha256_bytes(manifest_text.encode())
        != provenance.get("controlled_manifest_sha256")
        or replay.get("controlled_manifest_file_sha256")
        != provenance.get("controlled_manifest_sha256")
        or not isinstance(decisions, Mapping)
        or replay.get("decisions_sha256")
        != sha256_bytes(canonical_json(decisions))
        or not isinstance(rounds, list)
        or len(rounds) != len(decisions.get("rounds", []))
    ):
        raise ValueError("context legacy Osaka replay source differs")
    with tempfile.TemporaryDirectory(prefix="context-osaka-replay-") as directory:
        manifest_path = pathlib.Path(directory) / "controlled-manifest.toml"
        manifest_path.write_text(manifest_text)
        manifest = opcode_gas.load_manifest(manifest_path)
    formal_provenance = {
        field: provenance[field]
        for field in opcode_gas.FORMAL_RELATION_PROVENANCE_FIELDS
    }
    observations = {}
    for record, source_record in zip(rounds, decisions["rounds"]):
        bound = record.get("generator_max_count")
        relation_ids = record.get("relation_ids")
        rows = record.get("rows")
        result_payload = record.get("result")
        if (
            type(bound) is not int
            or not isinstance(relation_ids, list)
            or not isinstance(rows, list)
            or not isinstance(result_payload, Mapping)
            or source_record.get("generator_max_count") != bound
            or source_record.get("relation_ids") != relation_ids
            or record.get("rows_sha256") != sha256_bytes(canonical_json(rows))
            or record.get("result_sha256")
            != sha256_bytes(canonical_json(result_payload))
            or record.get("source_raw_file_sha256")
            != source_record.get("raw_runs_sha256")
            or record.get("source_result_file_sha256")
            != source_record.get("result_sha256")
        ):
            raise ValueError("context legacy Osaka replay round differs")
        subset = opcode_gas._osaka_relation_manifest(manifest, relation_ids)
        results = opcode_gas.fit_formal_relation_round(
            subset,
            rows,
            relation_ids,
            bound,
            expected_provenance=formal_provenance,
        )
        expected_result = opcode_gas._formal_relation_result_payload(
            bound, relation_ids, results
        )
        if expected_result != result_payload:
            raise ValueError("context legacy Osaka fit replay differs")
        rows_by_relation = {
            relation_id: [
                row for row in rows if row.get("relation_id") == relation_id
            ]
            for relation_id in relation_ids
        }
        result_by_id = {result["relation_id"]: result for result in results}
        relation_by_id = {
            relation.id: relation for relation in subset.opcode_relations
        }
        for relation_id in relation_ids:
            observations[relation_id] = opcode_gas._osaka_observation_from_fit(
                relation_by_id[relation_id],
                bound,
                rows_by_relation[relation_id],
                result_by_id[relation_id],
            )
    current = [observations[key] for key in OSAKA_CANARY_RELATION_IDS]
    return historical, current


def _build_context_compatibility_canary(
    *,
    legacy_osaka_canary: Mapping[str, Any],
    historical_anchor_fit: Mapping[str, Any],
    current_anchor_fit: Mapping[str, Any],
    historical_raw_sha256: str,
    current_raw_sha256: str,
    historical_anchor_rows: list[Mapping[str, Any]] | None = None,
    current_anchor_rows: list[Mapping[str, Any]] | None = None,
    historical_derivation_artifact_sha256: str | None = None,
    legacy_osaka_replay: Mapping[str, Any] | None = None,
    current_anchor_replay: Mapping[str, Any] | None = None,
    current_anchor_source_ledger: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Combine existing non-fitting Osaka replay with exact absolute anchor slopes."""
    import opcode_gas

    opcode_gas._validate_osaka_compatibility_canary_artifact(
        legacy_osaka_canary
    )
    if not isinstance(legacy_osaka_replay, Mapping):
        raise ValueError("context compatibility canary requires portable Osaka replay")
    provenance = legacy_osaka_canary.get("provenance")
    if not isinstance(provenance, Mapping):
        raise ValueError("context compatibility canary provenance differs")
    replay_historical, replay_current = _replay_legacy_osaka_evidence(
        legacy_osaka_replay, provenance
    )
    replayed_legacy = opcode_gas.build_osaka_compatibility_canary(
        replay_historical,
        replay_current,
        baseline_artifact_sha256=HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256,
        expected_baseline_artifact_sha256=HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256,
    )
    replayed_legacy["provenance"] = copy.deepcopy(provenance)
    replayed_legacy["artifact_sha256"] = sha256_bytes(
        canonical_json(
            {
                key: value
                for key, value in replayed_legacy.items()
                if key != "artifact_sha256"
            }
        )
    )
    if replayed_legacy != legacy_osaka_canary:
        raise ValueError("context legacy Osaka compatibility replay differs")
    if historical_anchor_rows is None or current_anchor_rows is None:
        raise ValueError("context compatibility canary requires replayable anchor rows")
    historical_costs = opcode_gas.validated_anchor_probe_costs(
        historical_anchor_fit, historical_anchor_rows
    )
    current_costs = opcode_gas.validated_anchor_probe_costs(
        current_anchor_fit, current_anchor_rows
    )
    _validate_current_anchor_replay(
        current_anchor_replay, current_anchor_fit, current_anchor_rows
    )
    _validate_current_anchor_source_ledger(
        current_anchor_source_ledger,
        current_anchor_replay,
        current_anchor_rows,
        current_anchor_fit,
    )
    _sha256(
        historical_derivation_artifact_sha256,
        "historical anchor derivation artifact",
    )
    if (
        legacy_osaka_canary.get("status") != "passed"
        or legacy_osaka_canary.get("relation_ids") != list(OSAKA_CANARY_RELATION_IDS)
    ):
        raise ValueError("context legacy Osaka compatibility canary failed")
    historical = {
        row.get("anchor_key"): row
        for row in historical_anchor_fit.get("anchors", [])
        if isinstance(row, Mapping)
    }
    current = {
        row.get("anchor_key"): row
        for row in current_anchor_fit.get("anchors", [])
        if isinstance(row, Mapping)
    }
    controls = {}
    apes = []
    for key in ("opcode:0x5f", "opcode:0x90"):
        old = historical.get(key)
        new = current.get(key)
        if not isinstance(old, Mapping) or not isinstance(new, Mapping):
            raise ValueError(f"context compatibility anchor is missing: {key}")
        old_slope = fraction_from_decimal(old.get("prover_gas_per_operation"))
        new_slope = fraction_from_decimal(new.get("prover_gas_per_operation"))
        if (
            old_slope != Fraction(historical_costs[key])
            or new_slope != Fraction(current_costs[key])
        ):
            raise ValueError(f"context compatibility anchor replay differs: {key}")
        if old_slope <= 0 or new_slope <= 0:
            raise ValueError(f"context compatibility anchor has non-positive slope: {key}")
        ape = abs(new_slope - old_slope) / old_slope
        apes.append(ape)
        controls[key] = {
            "status": "passed" if ape <= Fraction(1, 10) else "failed",
            "historical_slope_exact": fraction_payload(old_slope),
            "current_slope_exact": fraction_payload(new_slope),
            "drift_ape_exact": fraction_payload(ape),
            "repeat_count": 3,
        }
    mape = sum(apes, Fraction()) / len(apes)
    status = (
        "passed"
        if all(row["status"] == "passed" for row in controls.values())
        and mape <= Fraction(5, 100)
        else "failed"
    )
    artifact = {
        "schema_version": 1,
        "purpose": "context_opcode_elf_compatibility_canary",
        "evidence_mode": "production_replay",
        "revm_elf_sha256": legacy_osaka_canary.get("provenance", {}).get(
            "guest_elf_sha256"
        ),
        "control_opcode_lab_elf_sha256": current_anchor_fit.get("elf_sha256"),
        "launcher_sha256": current_anchor_fit.get("guest_launcher_sha256"),
        "legacy_osaka_canary": copy.deepcopy(legacy_osaka_canary),
        "legacy_osaka_replay": copy.deepcopy(legacy_osaka_replay),
        "control_relations": controls,
        "control_mape_exact": fraction_payload(mape),
        "historical_anchor": {
            "calibration_id": "09ebb08d76d3f461086b0cf4",
            "derivation_artifact_sha256": historical_derivation_artifact_sha256,
            "fit_artifact_sha256": historical_anchor_fit.get("artifact_sha256"),
            "raw_rows_sha256": historical_anchor_fit.get("raw_rows_sha256"),
            "raw_rows_file_sha256": _sha256(
                historical_raw_sha256, "historical anchor raw rows file"
            ),
            "fit": copy.deepcopy(historical_anchor_fit),
            "rows": copy.deepcopy(historical_anchor_rows),
        },
        "current_anchor": {
            "fit_artifact_sha256": current_anchor_fit.get("artifact_sha256"),
            "raw_rows_sha256": current_anchor_fit.get("raw_rows_sha256"),
            "raw_rows_file_sha256": _sha256(
                current_raw_sha256, "current anchor raw rows file"
            ),
            "manifest_sha256": current_anchor_fit.get("anchor_probe_manifest_sha256"),
            "fit": copy.deepcopy(current_anchor_fit),
            "rows": copy.deepcopy(current_anchor_rows),
            "replay": copy.deepcopy(current_anchor_replay),
            "source_ledger": copy.deepcopy(current_anchor_source_ledger),
        },
        "quality_gates": {
            "per_control_drift_ape_max": "0.10",
            "control_drift_mape_max": "0.05",
        },
        "status": status,
    }
    artifact["artifact_sha256"] = sha256_bytes(canonical_json(artifact))
    return artifact


def validate_context_compatibility_canary(canary: Mapping[str, Any]) -> None:
    import opcode_gas

    legacy = canary.get("legacy_osaka_canary")
    legacy_replay = canary.get("legacy_osaka_replay")
    controls = canary.get("control_relations")
    if (
        canary.get("schema_version") != 1
        or canary.get("purpose") != "context_opcode_elf_compatibility_canary"
        or canary.get("evidence_mode") != "production_replay"
        or canary.get("status") != "passed"
        or re.fullmatch(r"[0-9a-f]{64}", str(canary.get("revm_elf_sha256"))) is None
        or re.fullmatch(
            r"[0-9a-f]{64}", str(canary.get("control_opcode_lab_elf_sha256"))
        )
        is None
        or re.fullmatch(r"[0-9a-f]{64}", str(canary.get("launcher_sha256"))) is None
        or not isinstance(legacy, Mapping)
        or legacy.get("status") != "passed"
        or legacy.get("relation_ids") != list(OSAKA_CANARY_RELATION_IDS)
    ):
        raise ValueError("context legacy compatibility canary differs")
    opcode_gas._validate_osaka_compatibility_canary_artifact(legacy)
    provenance = legacy.get("provenance")
    if (
        not isinstance(provenance, Mapping)
        or set(provenance)
        != {
            "calibration_id",
            "calibration_identity_sha256",
            "implementation_revision",
            "controlled_manifest_sha256",
            "controlled_manifest_rows_sha256",
            "complete_schedule_sha256",
            "guest_elf_sha256",
            "version_identity",
        }
        or not isinstance(provenance.get("calibration_id"), str)
        or not provenance["calibration_id"]
        or re.fullmatch(r"[0-9a-f]{40}", str(provenance.get("implementation_revision")))
        is None
        or any(
            re.fullmatch(r"[0-9a-f]{64}", str(provenance.get(field))) is None
            for field in (
                "calibration_identity_sha256",
                "controlled_manifest_sha256",
                "controlled_manifest_rows_sha256",
                "complete_schedule_sha256",
                "guest_elf_sha256",
            )
        )
        or provenance["guest_elf_sha256"] != canary["revm_elf_sha256"]
        or not isinstance(provenance.get("version_identity"), Mapping)
    ):
        raise ValueError("context legacy compatibility canary provenance differs")
    if not isinstance(legacy_replay, Mapping):
        raise ValueError("context legacy compatibility replay is missing")
    replay_historical, replay_current = _replay_legacy_osaka_evidence(
        legacy_replay, provenance
    )
    replayed_legacy = opcode_gas.build_osaka_compatibility_canary(
        replay_historical,
        replay_current,
        baseline_artifact_sha256=HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256,
        expected_baseline_artifact_sha256=HISTORICAL_OSAKA_RELATION_ARTIFACT_SHA256,
    )
    replayed_legacy["provenance"] = copy.deepcopy(provenance)
    replayed_legacy["artifact_sha256"] = sha256_bytes(
        canonical_json(
            {
                key: value
                for key, value in replayed_legacy.items()
                if key != "artifact_sha256"
            }
        )
    )
    if replayed_legacy != legacy:
        raise ValueError("context legacy Osaka compatibility replay differs")
    if not isinstance(controls, Mapping) or set(controls) != {"opcode:0x5f", "opcode:0x90"}:
        raise ValueError("context compatibility control relation inventory differs")
    apes = []
    for key, row in controls.items():
        if row.get("status") != "passed" or row.get("repeat_count") != 3:
            raise ValueError(f"context compatibility control relation failed: {key}")
        if set(row) != {
            "status",
            "historical_slope_exact",
            "current_slope_exact",
            "drift_ape_exact",
            "repeat_count",
        }:
            raise ValueError(f"context compatibility control evidence differs: {key}")
        old = fraction_from_payload(row["historical_slope_exact"])
        new = fraction_from_payload(row["current_slope_exact"])
        exact_ape = abs(new - old) / old
        ape = fraction_from_payload(row["drift_ape_exact"])
        if ape != exact_ape:
            raise ValueError(f"context compatibility control drift differs: {key}")
        if ape > Fraction(1, 10):
            raise ValueError(f"context compatibility control relation drift exceeds 10%: {key}")
        apes.append(ape)
    mape = sum(apes, Fraction()) / len(apes)
    if mape > Fraction(5, 100):
        raise ValueError("context compatibility control relation MAPE exceeds 5%")
    unsigned = dict(canary)
    artifact_sha256 = unsigned.pop("artifact_sha256", None)
    historical_anchor = canary.get("historical_anchor")
    current_anchor = canary.get("current_anchor")
    historical_fit = (
        historical_anchor.get("fit") if isinstance(historical_anchor, Mapping) else None
    )
    current_fit = (
        current_anchor.get("fit") if isinstance(current_anchor, Mapping) else None
    )
    historical_rows = (
        historical_anchor.get("rows")
        if isinstance(historical_anchor, Mapping)
        else None
    )
    current_rows = (
        current_anchor.get("rows") if isinstance(current_anchor, Mapping) else None
    )
    if (
        not isinstance(historical_fit, Mapping)
        or not isinstance(current_fit, Mapping)
        or not isinstance(historical_rows, list)
        or not isinstance(current_rows, list)
    ):
        raise ValueError("context compatibility anchor replay differs")
    _validate_current_anchor_replay(
        current_anchor.get("replay"), current_fit, current_rows
    )
    _validate_current_anchor_source_ledger(
        current_anchor.get("source_ledger"),
        current_anchor["replay"],
        current_rows,
        current_fit,
    )
    try:
        historical_costs = opcode_gas.validated_anchor_probe_costs(
            historical_fit, historical_rows
        )
        current_costs = opcode_gas.validated_anchor_probe_costs(
            current_fit, current_rows
        )
    except (KeyError, TypeError, ValueError) as error:
        raise ValueError("context compatibility anchor replay differs") from error
    historical_rows_file_sha256 = sha256_bytes(
        b"".join(canonical_json(row) + b"\n" for row in historical_rows)
    )
    current_rows_file_sha256 = sha256_bytes(
        b"".join(canonical_json(row) + b"\n" for row in current_rows)
    )
    if isinstance(historical_anchor, Mapping) and isinstance(current_anchor, Mapping):
        for key, row in controls.items():
            if (
                fraction_from_payload(row["historical_slope_exact"])
                != Fraction(historical_costs[key])
                or fraction_from_payload(row["current_slope_exact"])
                != Fraction(current_costs[key])
            ):
                raise ValueError("context compatibility anchor replay differs")
    if (
        canary.get("quality_gates")
        != {
            "per_control_drift_ape_max": "0.10",
            "control_drift_mape_max": "0.05",
        }
        or fraction_from_payload(canary["control_mape_exact"]) != mape
        or not isinstance(historical_anchor, Mapping)
        or set(historical_anchor)
        != {
            "calibration_id",
            "derivation_artifact_sha256",
            "fit_artifact_sha256",
            "raw_rows_sha256",
            "raw_rows_file_sha256",
            "fit",
            "rows",
        }
        or historical_anchor.get("calibration_id")
        != "09ebb08d76d3f461086b0cf4"
        or historical_anchor.get("derivation_artifact_sha256")
        != HISTORICAL_ANCHOR_DERIVATION_SHA256
        or historical_anchor.get("fit_artifact_sha256")
        != HISTORICAL_ANCHOR_FIT_ARTIFACT_SHA256
        or historical_fit.get("artifact_sha256")
        != HISTORICAL_ANCHOR_FIT_ARTIFACT_SHA256
        or historical_fit.get("primary_artifact_sha256")
        != HISTORICAL_ANCHOR_PRIMARY_ARTIFACT_SHA256
        or historical_anchor.get("raw_rows_sha256")
        != HISTORICAL_ANCHOR_RAW_ROWS_SHA256
        or historical_fit.get("raw_rows_sha256")
        != HISTORICAL_ANCHOR_RAW_ROWS_SHA256
        or historical_anchor.get("raw_rows_file_sha256")
        != HISTORICAL_ANCHOR_RAW_FILE_SHA256
        or historical_rows_file_sha256 != HISTORICAL_ANCHOR_RAW_FILE_SHA256
        or any(
            re.fullmatch(r"[0-9a-f]{64}", str(historical_anchor.get(field))) is None
            for field in (
                "derivation_artifact_sha256",
                "fit_artifact_sha256",
                "raw_rows_sha256",
                "raw_rows_file_sha256",
            )
        )
        or not isinstance(current_anchor, Mapping)
        or set(current_anchor)
        != {
            "fit_artifact_sha256",
            "raw_rows_sha256",
            "raw_rows_file_sha256",
            "manifest_sha256",
            "fit",
            "rows",
            "replay",
            "source_ledger",
        }
        or any(
            re.fullmatch(r"[0-9a-f]{64}", str(current_anchor.get(field))) is None
            for field in (
                "fit_artifact_sha256",
                "raw_rows_sha256",
                "raw_rows_file_sha256",
                "manifest_sha256",
            )
        )
        or current_anchor.get("fit_artifact_sha256")
        != current_fit.get("artifact_sha256")
        or current_anchor.get("raw_rows_sha256")
        != current_fit.get("raw_rows_sha256")
        or current_anchor.get("raw_rows_file_sha256")
        != current_rows_file_sha256
        or current_anchor.get("manifest_sha256")
        != current_fit.get("anchor_probe_manifest_sha256")
        or artifact_sha256 != sha256_bytes(canonical_json(unsigned))
    ):
        raise ValueError("context compatibility canary provenance differs")


def run_context_compatibility_canary(
    *,
    calibration_run: pathlib.Path,
    controlled_manifest: pathlib.Path,
    baseline_derivation: pathlib.Path,
    historical_manifest: pathlib.Path,
    historical_anchor_run: pathlib.Path,
    guest_launcher: pathlib.Path,
    revm_elf: pathlib.Path,
    control_opcode_lab_elf: pathlib.Path,
    output_root: pathlib.Path,
) -> dict[str, Any]:
    """Execute both frozen canary families through their reviewed source paths."""
    import opcode_gas

    calibration_run = pathlib.Path(calibration_run).resolve(strict=True)
    output_root = pathlib.Path(output_root).resolve(strict=False)
    expected_output_root = calibration_run / "context-compatibility"
    if (
        output_root.is_symlink()
        or output_root != expected_output_root
    ):
        raise ValueError("context compatibility output path differs")
    output_root = _validate_safe_campaign_root(
        output_root, label="compatibility output"
    )
    for label, path in (
        ("guest launcher", guest_launcher),
        ("REVM opcode-lab ELF", revm_elf),
        ("control opcode-lab ELF", control_opcode_lab_elf),
    ):
        path = pathlib.Path(path)
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"context compatibility {label} must be a regular file")
    identity = opcode_gas.validate_calibration_execution_identity(calibration_run)
    launcher_sha256 = opcode_gas.validate_calibration_guest_launcher(
        identity, guest_launcher
    )
    artifacts = identity.get("guest_artifacts")
    revm_sha = opcode_gas.sha256_file(revm_elf)
    control_sha = opcode_gas.sha256_file(control_opcode_lab_elf)
    if (
        not isinstance(artifacts, Mapping)
        or artifacts.get("crates/guests/elf/sp1_revm_opcode_lab.elf") != revm_sha
        or artifacts.get("crates/guests/elf/sp1_opcode_lab.elf") != control_sha
    ):
        raise ValueError("context compatibility guest artifacts differ from calibration identity")
    manifest, frozen_identity = opcode_gas.verify_frozen_controlled_manifest(
        calibration_run, controlled_manifest
    )
    if json.loads(canonical_json(identity)) != json.loads(canonical_json(frozen_identity)):
        raise ValueError("context compatibility calibration identity changed")
    baseline = opcode_gas.validate_historical_core_opcode_baseline(
        baseline_derivation, historical_manifest
    )
    portable_anchor = validate_portable_historical_anchor_source(
        historical_anchor_run, baseline_derivation
    )
    opcode_gas.validate_osaka_relation_subsets(
        manifest,
        baseline["manifest"],
        OSAKA_CANARY_RELATION_IDS,
        opcode_gas.OSAKA_SUPPLEMENT_RELATION_IDS,
    )
    version_identity = opcode_gas.validate_calibration_version_identity(identity)
    provenance = {
        "calibration_id": calibration_run.name,
        "calibration_identity_sha256": sha256_bytes(canonical_json(identity)),
        "implementation_revision": identity["implementation_revision"],
        "controlled_manifest_sha256": identity["controlled_manifest_sha256"],
        "controlled_manifest_rows_sha256": identity[
            "controlled_manifest_rows_sha256"
        ],
        "complete_schedule_sha256": identity["complete_schedule_sha256"],
        "guest_elf_sha256": revm_sha,
        "version_identity": version_identity,
    }
    formal_provenance = {
        field: provenance[field]
        for field in opcode_gas.FORMAL_RELATION_PROVENANCE_FIELDS
    }
    output_root.mkdir(parents=True, exist_ok=True)
    legacy_root = output_root / "legacy-osaka"
    historical_observations = opcode_gas._historical_osaka_canary_observations(
        baseline
    )
    execution_args = argparse.Namespace(
        guest_launcher=guest_launcher,
        elf=revm_elf,
        controlled_manifest=controlled_manifest,
    )
    current_observations = opcode_gas._run_osaka_canary_rounds(
        calibration_run=calibration_run,
        output_root=legacy_root,
        manifest=manifest,
        historical_observations=historical_observations,
        provenance=formal_provenance,
        version_identity=version_identity,
        args=execution_args,
    )
    legacy = opcode_gas.build_osaka_compatibility_canary(
        historical_observations,
        current_observations,
        baseline_artifact_sha256=baseline["relation_artifact"]["artifact_sha256"],
        expected_baseline_artifact_sha256=baseline["derivation"][
            "derivation_identity"
        ]["source"]["source_hashes"]["relation_artifact_sha256"],
    )
    legacy["provenance"] = provenance
    legacy["artifact_sha256"] = sha256_bytes(
        canonical_json({key: value for key, value in legacy.items() if key != "artifact_sha256"})
    )
    if legacy.get("status") != "passed":
        raise ValueError("context legacy Osaka compatibility canary failed")
    legacy_replay = _capture_legacy_osaka_replay(
        output_root=legacy_root,
        controlled_manifest=controlled_manifest,
        historical_observations=list(historical_observations),
    )

    historical_anchor_run = pathlib.Path(historical_anchor_run).resolve(strict=True)
    historical_fit = portable_anchor["fit"]
    anchor_root = output_root / "control-anchors"
    fixtures = anchor_root / "fixtures"
    raw_path = anchor_root / "raw.jsonl"
    fit_path = anchor_root / "fit.json"
    if fit_path.exists():
        if fit_path.is_symlink() or raw_path.is_symlink() or fixtures.is_symlink():
            raise ValueError("context persisted control-anchor source is a symlink")
        current_rows = [json.loads(line) for line in raw_path.read_bytes().splitlines()]
        current_fit = json.loads(fit_path.read_bytes())
        if opcode_gas.fit_anchor_probe_rows(current_rows) != current_fit:
            raise ValueError("context persisted control-anchor fit differs")
    else:
        if fixtures.exists() or raw_path.exists():
            raise ValueError("context control-anchor execution is partially persisted")
        run_provenance = opcode_gas._anchor_probe_run_provenance(
            calibration_run, identity
        )
        opcode_gas.generate_anchor_probe_fixtures(
            control_opcode_lab_elf,
            fixtures,
            guest_launcher=guest_launcher,
            run_provenance=run_provenance,
        )
        current_rows = opcode_gas.run_anchor_probe_fixtures(
            guest_launcher=guest_launcher,
            elf_path=control_opcode_lab_elf,
            fixtures_dir=fixtures,
            out_path=raw_path,
        )
        current_fit = opcode_gas.fit_anchor_probe_rows(current_rows)
        _persist_immutable(
            fit_path,
            (json.dumps(current_fit, indent=2, sort_keys=True) + "\n").encode(),
        )
    anchor_manifest, anchor_inventory = (
        opcode_gas._validated_anchor_probe_fixture_manifest(
            fixtures, expected_elf_sha256=control_sha
        )
    )
    opcode_gas._validate_anchor_probe_rows_against_fixtures(
        current_fit, current_rows, anchor_manifest, anchor_inventory
    )
    opcode_gas.validated_anchor_probe_for_execution_identity(
        current_fit, current_rows, identity
    )
    current_anchor_replay = _capture_current_anchor_replay(
        fixtures=fixtures,
        guest_launcher=pathlib.Path(guest_launcher),
        execution_identity=identity,
    )
    current_anchor_source_ledger = _current_anchor_source_ledger(
        current_anchor_replay,
        current_rows,
        current_fit,
        sha256_bytes(
            (anchor_root / "raw.guest-launcher.jsonl").read_bytes()
        ),
    )
    canary = _build_context_compatibility_canary(
        legacy_osaka_canary=legacy,
        historical_anchor_fit=historical_fit,
        current_anchor_fit=current_fit,
        historical_raw_sha256=opcode_gas.sha256_file(
            historical_anchor_run / "raw" / "anchor-probe.jsonl"
        ),
        current_raw_sha256=opcode_gas.sha256_file(raw_path),
        historical_anchor_rows=portable_anchor["rows"],
        current_anchor_rows=current_rows,
        historical_derivation_artifact_sha256=portable_anchor[
            "derivation_artifact_sha256"
        ],
        legacy_osaka_replay=legacy_replay,
        current_anchor_replay=current_anchor_replay,
        current_anchor_source_ledger=current_anchor_source_ledger,
    )
    if canary["launcher_sha256"] != launcher_sha256:
        raise ValueError("context compatibility launcher identity differs")
    validate_context_compatibility_canary(canary)
    _persist_immutable(
        output_root / "compatibility-canary.json",
        (json.dumps(canary, indent=2, sort_keys=True) + "\n").encode(),
    )
    return canary


def validate_portable_historical_anchor_source(
    historical_anchor_run: pathlib.Path,
    baseline_derivation: pathlib.Path,
) -> dict[str, Any]:
    """Replay the frozen 09ebb anchor package without consulting live artifacts."""
    import opcode_gas

    historical_anchor_run = pathlib.Path(historical_anchor_run)
    baseline_derivation = pathlib.Path(baseline_derivation)
    if (
        historical_anchor_run.is_symlink()
        or not historical_anchor_run.is_dir()
        or historical_anchor_run.name != "09ebb08d76d3f461086b0cf4"
        or baseline_derivation.is_symlink()
        or not baseline_derivation.is_dir()
        or baseline_derivation.name != "3e1d97c461cd2ef9a40e6a02"
    ):
        raise ValueError("context historical anchor package path differs")
    derivation_path = baseline_derivation / "derivation.json"
    if derivation_path.is_symlink() or not derivation_path.is_file():
        raise ValueError("context historical derivation is missing")
    derivation = json.loads(derivation_path.read_bytes())
    opcode_gas._validate_content_addressed_artifact(
        derivation, label="context historical derivation"
    )
    identity = derivation.get("derivation_identity")
    source = identity.get("source") if isinstance(identity, Mapping) else None
    source_hashes = source.get("source_hashes") if isinstance(source, Mapping) else None
    file_hashes = (
        source_hashes.get("source_files_sha256")
        if isinstance(source_hashes, Mapping)
        else None
    )
    if (
        derivation.get("derivation_id") != baseline_derivation.name
        or not isinstance(source, Mapping)
        or source.get("calibration_id") != historical_anchor_run.name
        or not isinstance(file_hashes, Mapping)
        or not file_hashes
    ):
        raise ValueError("context historical derivation source differs")
    resolved_run = historical_anchor_run.resolve(strict=True)
    for relative, expected_sha256 in file_hashes.items():
        if not isinstance(relative, str):
            raise ValueError("context historical anchor source file hash differs")
        relative_path = pathlib.PurePosixPath(relative)
        path = historical_anchor_run / pathlib.Path(relative)
        if (
            relative_path.is_absolute()
            or ".." in relative_path.parts
            or re.fullmatch(r"[0-9a-f]{64}", str(expected_sha256)) is None
            or path.is_symlink()
            or not path.is_file()
            or path.resolve(strict=True).is_relative_to(resolved_run) is False
            or sha256_bytes(path.read_bytes()) != expected_sha256
        ):
            raise ValueError("context historical anchor source file hash differs")
    experiment = json.loads((historical_anchor_run / "experiment.json").read_bytes())
    declaration = opcode_gas.experiment_provenance_declaration(experiment)
    calibration_identity = experiment.get("calibration_identity")
    if (
        declaration.get("calibration_id") != historical_anchor_run.name
        or declaration.get("implementation_revision")
        != source.get("implementation_revision")
        or declaration.get("calibration_identity_sha256")
        != source.get("calibration_identity_sha256")
        or not isinstance(calibration_identity, Mapping)
    ):
        raise ValueError("context historical calibration identity differs")
    fit_path = historical_anchor_run / "anchor-probe-fit.json"
    fit = json.loads(fit_path.read_bytes())
    costs, rows = opcode_gas.load_validated_anchor_probe_run(
        historical_anchor_run, fit, calibration_identity
    )
    if (
        fit.get("artifact_sha256")
        != source_hashes.get("anchor_probe_artifact_sha256")
        or fit.get("primary_artifact_sha256")
        != source_hashes.get("anchor_probe_primary_sha256")
    ):
        raise ValueError("context historical anchor fit differs from derivation")
    return {
        "calibration_id": historical_anchor_run.name,
        "calibration_identity": copy.deepcopy(calibration_identity),
        "derivation_artifact_sha256": derivation["artifact_sha256"],
        "fit": fit,
        "rows": rows,
        "raw_rows_file_sha256": file_hashes["raw/anchor-probe.jsonl"],
        "costs": {key: str(value) for key, value in costs.items()},
    }


def _sealed_file_bytes(
    manifest: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    registry: Mapping[str, Any],
    canary: Mapping[str, Any],
    source_identity: Mapping[str, Any],
    adaptive_evidence: Mapping[str, Any],
) -> dict[str, bytes]:
    row_bytes = b"".join(
        canonical_json(row) + b"\n"
        for row in sorted(
            rows,
            key=lambda row: (
                row["scenario"], row["generator_bound"], row["placement"], row["count"], row["repeat_index"], row["lane"]
            ),
        )
    )
    return {
        "campaign-manifest.json": canonical_json(manifest) + b"\n",
        "rows.jsonl": row_bytes,
        "source-registry.json": (json.dumps(registry, indent=2, sort_keys=True) + "\n").encode(),
        "compatibility-canary.json": canonical_json(canary) + b"\n",
        "source-identity.json": canonical_json(source_identity) + b"\n",
        "adaptive-evidence.json": canonical_json(adaptive_evidence) + b"\n",
    }


def _build_context_result(
    *,
    manifest: Mapping[str, Any],
    rows: list[Mapping[str, Any]],
    source_registry: Mapping[str, Any],
    compatibility_canary: Mapping[str, Any],
    source_identity: Mapping[str, Any],
    adaptive_evidence: Mapping[str, Any],
) -> dict[str, Any]:
    if sha256_bytes(canonical_json(manifest)) != CONTEXT_MANIFEST_CANONICAL_SHA256:
        raise ValueError("context result manifest differs from the frozen V1 contract")
    validate_context_compatibility_canary(compatibility_canary)
    expected_source_identity_fields = {
        "calibration_id",
        "calibration_identity_sha256",
        "execution_revision",
        "elf_sha256",
        "elf_path",
        "control_opcode_lab_elf_sha256",
        "control_opcode_lab_elf_path",
        "launcher_sha256",
        "launcher_path",
        "corrected_osaka_core_artifact_sha256",
        "corrected_osaka_core_file_sha256",
        "corrected_osaka_core_path",
        "operation_coverage_v5_artifact_sha256",
        "operation_coverage_v5_file_sha256",
        "operation_coverage_v5_path",
    }
    if set(source_identity) != expected_source_identity_fields:
        raise ValueError("context source identity field inventory differs")
    rows = sorted(
        (copy.deepcopy(row) for row in rows),
        key=lambda row: (
            row["scenario"],
            row["generator_bound"],
            row["placement"],
            row["count"],
            row["repeat_index"],
            row["lane"],
        ),
    )
    for field in (
        "calibration_identity_sha256",
        "elf_sha256",
        "control_opcode_lab_elf_sha256",
        "launcher_sha256",
        "corrected_osaka_core_artifact_sha256",
        "corrected_osaka_core_file_sha256",
        "operation_coverage_v5_artifact_sha256",
        "operation_coverage_v5_file_sha256",
    ):
        _sha256(source_identity.get(field), f"context {field}")
    if (
        not isinstance(source_identity.get("calibration_id"), str)
        or source_identity["calibration_id"]
        != source_identity["calibration_identity_sha256"][:24]
        or re.fullmatch(r"[0-9a-f]{40}", str(source_identity.get("execution_revision")))
        is None
    ):
        raise ValueError("context execution calibration identity differs")
    path_contract = {
        "elf_path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
        "control_opcode_lab_elf_path": "crates/guests/elf/sp1_opcode_lab.elf",
        "launcher_path": "target/release/guest-launcher",
        "operation_coverage_v5_path": (
            "experiments/opcode-gas/manifests/operation-coverage-v5.json"
        ),
    }
    if any(source_identity.get(field) != value for field, value in path_contract.items()):
        raise ValueError("context source path identity differs")
    coverage_v5 = manifest.get("operation_coverage_v5")
    if (
        not isinstance(coverage_v5, Mapping)
        or coverage_v5
        != {
            "artifact_sha256": "5e3f9aae0d3d9ae10a9b05bfbe877b2f9c4ab146bd7d6df66f915211c4a9108d",
            "file_sha256": "4c67852165636d1991e5dd21716dc225e8df8bbf79245763e5c6bca2ce505b89",
            "path": "experiments/opcode-gas/manifests/operation-coverage-v5.json",
        }
        or source_identity.get("operation_coverage_v5_artifact_sha256")
        != coverage_v5["artifact_sha256"]
        or source_identity.get("operation_coverage_v5_file_sha256")
        != coverage_v5["file_sha256"]
    ):
        raise ValueError("context operation coverage V5 source differs")
    corrected_bytes = (json.dumps(source_registry, indent=2, sort_keys=True) + "\n").encode()
    if (
        compatibility_canary.get("revm_elf_sha256") != source_identity["elf_sha256"]
        or compatibility_canary.get("control_opcode_lab_elf_sha256")
        != source_identity["control_opcode_lab_elf_sha256"]
        or compatibility_canary.get("launcher_sha256")
        != source_identity["launcher_sha256"]
        or source_registry.get("artifact_sha256")
        != source_identity["corrected_osaka_core_artifact_sha256"]
    ):
        raise ValueError("context compatibility canary or corrected source identity differs")
    current_anchor_fit = compatibility_canary.get("current_anchor", {}).get("fit")
    current_run_provenance = (
        current_anchor_fit.get("run_provenance")
        if isinstance(current_anchor_fit, Mapping)
        else None
    )
    if (
        not isinstance(current_run_provenance, Mapping)
        or current_run_provenance.get("calibration_id")
        != source_identity["calibration_id"]
        or current_run_provenance.get("calibration_identity_sha256")
        != source_identity["calibration_identity_sha256"]
        or current_run_provenance.get("implementation_revision")
        != source_identity["execution_revision"]
    ):
        raise ValueError("context compatibility canary calibration differs")
    registry_ref = manifest.get("reference_registry")
    if not isinstance(registry_ref, Mapping) or registry_ref != {
        "artifact_sha256": "1c05166e674a66ed51e3b1991c597ede479657e3195774a2960b1ff1cdd5ba4a",
        "file_sha256": "0a85afe5f21af2599823cb0031087f4701e598acc3f7ca48896769b32b518234",
        "parameter_basis": "production_scaled",
        "path": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
        "predecessor_artifact_sha256": (
            "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b"
        ),
        "required_correction": "osaka_0x15_0x1e_body_scale_v1",
        "required_purpose": "osaka_augmented_core_opcode_submodel",
        "required_recovery_formula": "lab_slope_times_body_scale_to_production_body_v2",
        "required_schema_version": 4,
    }:
        raise ValueError("context reference registry path differs")
    unsigned_registry = dict(source_registry)
    registry_artifact = unsigned_registry.pop("artifact_sha256", None)
    augmentation = source_registry.get("osaka_augmentation")
    if (
        source_registry.get("schema_version") != registry_ref["required_schema_version"]
        or source_registry.get("purpose") != registry_ref["required_purpose"]
        or not isinstance(augmentation, Mapping)
        or augmentation.get("recovery_formula")
        != registry_ref["required_recovery_formula"]
        or augmentation.get("baseline_core_artifact_sha256")
        != "900964c9e64af23c35e71d05f118c519d7b9dfee5263d201af7f850b9de8c964"
        or registry_artifact != registry_ref["artifact_sha256"]
        or registry_artifact != sha256_bytes(canonical_json(unsigned_registry))
        or sha256_bytes(corrected_bytes) != registry_ref["file_sha256"]
        or source_identity.get("corrected_osaka_core_path") != registry_ref["path"]
        or source_identity.get("corrected_osaka_core_artifact_sha256")
        != registry_ref["artifact_sha256"]
        or source_identity.get("corrected_osaka_core_file_sha256")
        != registry_ref["file_sha256"]
    ):
        raise ValueError("context corrected Osaka core source differs")
    evidence_rows, evidence_reports = validate_context_adaptive_evidence(
        manifest, adaptive_evidence, source_identity
    )
    evidence_rows = sorted(
        (copy.deepcopy(row) for row in evidence_rows),
        key=lambda row: (
            row["scenario"],
            row["generator_bound"],
            row["placement"],
            row["count"],
            row["repeat_index"],
            row["lane"],
        ),
    )
    if rows != evidence_rows:
        raise ValueError("context terminal rows differ from adaptive evidence")
    scenario_reports = _fit_any_context_rows(manifest, rows)
    if scenario_reports != evidence_reports:
        raise ValueError("context terminal fit differs from adaptive evidence")
    selected = select_promotable_context_keys(manifest, scenario_reports)
    body_scale = fraction_from_decimal(source_registry.get("body_scale"))
    models = {}
    scenarios_by_key = {}
    for scenario in manifest["scenarios"]:
        key = f"opcode:0x{scenario['opcode']:02x}"
        if key not in selected:
            continue
        report = scenario_reports[scenario["name"]]
        delta = fraction_from_payload(report["delta_lab_exact"])
        reference = load_control_reference(source_registry, scenario["reference_opcode"])
        reference_body = fraction_from_payload(reference["stored_body_per_raw_gas"])
        raw_gas = 3 if scenario["opcode"] == 0x35 else 2
        body = recover_scaled_target_body(
            delta_lab=delta,
            body_scale=body_scale,
            target_raw_gas=raw_gas,
            control_raw_gas=raw_gas,
            stored_control_body=reference_body,
        )
        scenarios_by_key.setdefault(key, []).append((scenario["name"], body, reference))
    consistency = fraction_from_decimal(manifest["consistency_ape_max"])
    for key, scenario_evidence in scenarios_by_key.items():
        bodies = [body for _name, body, _reference in scenario_evidence]
        baseline = bodies[0]
        if baseline <= 0 or any(abs(body - baseline) / baseline > consistency for body in bodies):
            continue
        models[key] = {
            "kind": "static_raw_gas",
            "parameter_basis": "production_scaled_body_excluding_common_dispatch",
            "body_per_raw_gas_exact": fraction_payload(sum(bodies, Fraction()) / len(bodies)),
            "body_scale_exact": fraction_payload(body_scale),
            "required_scenarios": [name for name, _body, _reference in scenario_evidence],
            "reference_models": {
                name: reference for name, _body, reference in scenario_evidence
            },
        }
    files = _sealed_file_bytes(
        manifest,
        rows,
        source_registry,
        compatibility_canary,
        source_identity,
        adaptive_evidence,
    )
    result = {
        "schema_version": 1,
        "purpose": "context_opcode_calibration_result",
        "status": "sealed",
        "promoted_model_keys": sorted(models),
        "candidate_eligible": False,
        "production_registry_modified": False,
        "proposal_validated": False,
        "execution_evidence_authority": {
            "seal_entrypoint": "canonical_create_only_run_and_live_calibration",
            "portable_verification_scope": (
                "integrity_and_exact_fit_replay_not_prover_gas_reauthentication"
            ),
        },
        "parameter_formula": "stored_b_t=(body_scale*delta_lab+r_control*stored_b_control)/r_target",
        "models": models,
        "source_files": {name: sha256_bytes(data) for name, data in sorted(files.items())},
        "manifest": copy.deepcopy(manifest),
        "scenario_reports": copy.deepcopy(scenario_reports),
        "rows": copy.deepcopy(rows),
        "source_registry": copy.deepcopy(source_registry),
        "compatibility_canary": copy.deepcopy(compatibility_canary),
        "source_identity": copy.deepcopy(source_identity),
        "adaptive_evidence": copy.deepcopy(adaptive_evidence),
    }
    identity = sha256_bytes(canonical_json(result))
    result["result_identity_sha256"] = identity
    result["result_id"] = identity[:24]
    result["artifact_sha256"] = sha256_bytes(canonical_json(result))
    return result


def _validate_result_envelope(result: Mapping[str, Any]) -> None:
    artifact = result.get("artifact_sha256")
    unsigned = dict(result)
    unsigned.pop("artifact_sha256", None)
    if artifact != sha256_bytes(canonical_json(unsigned)):
        raise ValueError("context result artifact hash differs")
    identity_value = dict(unsigned)
    result_id = identity_value.pop("result_id", None)
    identity = identity_value.pop("result_identity_sha256", None)
    expected = sha256_bytes(canonical_json(identity_value))
    if identity != expected or result_id != expected[:24]:
        raise ValueError("context result identity hash differs")


def _validated_context_identity_helper(
    source_identity: Mapping[str, Any], campaign_source: Mapping[str, Any]
) -> pathlib.Path:
    _validate_production_campaign_source(campaign_source)
    if campaign_source["calibration_identity_sha256"] != source_identity.get(
        "calibration_identity_sha256"
    ):
        raise ValueError("context production calibration identity differs")
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    launcher = repo_root / str(source_identity.get("launcher_path"))
    if (
        launcher.is_symlink()
        or not launcher.is_file()
        or sha256_bytes(launcher.read_bytes()) != source_identity.get("launcher_sha256")
    ):
        raise ValueError("context native identity helper differs from sealed calibration")
    return launcher


def _replay_context_result_identity(result: Mapping[str, Any]) -> None:
    """Independently replay every unique input through the hash-bound native helper."""
    source_identity = result.get("source_identity")
    evidence = result.get("adaptive_evidence")
    campaign_source = (
        evidence.get("identity", {}).get("source")
        if isinstance(evidence, Mapping)
        else None
    )
    if not isinstance(source_identity, Mapping) or not isinstance(
        campaign_source, Mapping
    ):
        raise ValueError("context production identity source is missing")
    launcher = _validated_context_identity_helper(source_identity, campaign_source)

    cache: dict[str, Mapping[str, Any]] = {}
    with tempfile.TemporaryDirectory(prefix="context-result-identity-") as directory:
        root = pathlib.Path(directory)
        for row in result.get("rows", []):
            fixture = row.get("fixture")
            if not isinstance(fixture, Mapping):
                raise ValueError("context result row fixture differs")
            fixture_sha = sha256_bytes(canonical_json(fixture))
            bundle = cache.get(fixture_sha)
            if bundle is None:
                input_path = root / f"{fixture_sha}.json"
                input_path.write_bytes(canonical_json(fixture) + b"\n")
                bundle = _default_identity_replayer(launcher, input_path)
                cache[fixture_sha] = bundle
            if row.get("identity") != bundle:
                raise ValueError("context persisted identity differs from native replay")
            _admit_executed_context_row(
                result["manifest"], fixture, bundle, row["formal_report"]
            )


def _reject_context_output_overlap(
    output_root: pathlib.Path, protected_inputs: Iterable[pathlib.Path]
) -> None:
    output = pathlib.Path(output_root).resolve(strict=False)
    for supplied in protected_inputs:
        path = pathlib.Path(supplied)
        if path.is_symlink():
            raise ValueError("context seal input must not be a symlink")
        resolved = path.resolve(strict=False)
        if output == resolved or (path.is_dir() and output.is_relative_to(resolved)):
            raise ValueError("context result output overlaps an immutable input")


def _validate_context_canary_run_directory(
    calibration_run: pathlib.Path, canary: Mapping[str, Any]
) -> None:
    """Bind the seal to the create-only canary runner's persisted source ledger."""
    root = calibration_run / "context-compatibility"
    if root.is_symlink() or not root.is_dir():
        raise ValueError("context compatibility source directory is missing")
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ValueError("context compatibility source contains a symlink")
    current = canary.get("current_anchor")
    if not isinstance(current, Mapping):
        raise ValueError("context current anchor source is missing")
    replay = current.get("replay")
    ledger = current.get("source_ledger")
    rows = current.get("rows")
    fit = current.get("fit")
    if (
        not isinstance(replay, Mapping)
        or not isinstance(ledger, Mapping)
        or not isinstance(rows, list)
        or not isinstance(fit, Mapping)
    ):
        raise ValueError("context current anchor source ledger is missing")
    anchor_root = root / "control-anchors"
    fixtures = anchor_root / "fixtures"
    raw_path = anchor_root / "raw.jsonl"
    executor_path = anchor_root / "raw.guest-launcher.jsonl"
    fit_path = anchor_root / "fit.json"
    manifest_path = fixtures / "anchor-probe-manifest.json"
    required = (raw_path, executor_path, fit_path, manifest_path)
    if any(path.is_symlink() or not path.is_file() for path in required):
        raise ValueError("context current anchor persisted source is incomplete")
    expected_inputs = {
        record["guest_input_path"]: canonical_json(record["input"]) + b"\n"
        for record in replay["fixture_inputs"]
    }
    actual_fixture_files = {
        path.relative_to(fixtures).as_posix()
        for path in fixtures.rglob("*")
        if path.is_file()
    }
    if actual_fixture_files != {"anchor-probe-manifest.json", *expected_inputs}:
        raise ValueError("context current anchor fixture inventory differs")
    import opcode_gas

    persisted_manifest, fixture_inventory = (
        opcode_gas._validated_anchor_probe_fixture_manifest(
            fixtures, expected_elf_sha256=canary.get("control_opcode_lab_elf_sha256")
        )
    )
    input_paths = [
        fixtures / pathlib.Path(str(fixture["guest_input_path"]))
        for fixture in fixture_inventory.values()
        for _repeat_index in range(opcode_gas.ANCHOR_PROBE_REPEATS)
    ]
    try:
        executor_reports = [
            json.loads(line) for line in executor_path.read_bytes().splitlines()
        ]
        reconstructed_rows = opcode_gas._anchor_probe_rows_from_reports(
            manifest=persisted_manifest,
            fixture_inventory=fixture_inventory,
            input_paths=input_paths,
            reports=executor_reports,
            launcher_sha256=str(canary.get("launcher_sha256")),
        )
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as error:
        raise ValueError("context current anchor executor source differs") from error
    if reconstructed_rows != rows:
        raise ValueError("context current anchor executor rows differ")
    if (
        raw_path.read_bytes()
        != b"".join(canonical_json(row) + b"\n" for row in rows)
        or fit_path.read_bytes()
        != (json.dumps(fit, indent=2, sort_keys=True) + "\n").encode()
        or manifest_path.read_bytes()
        != canonical_json(replay["fixture_manifest"]) + b"\n"
        or sha256_bytes(executor_path.read_bytes())
        != ledger.get("executor_reports_file_sha256")
        or any(
            fixtures.joinpath(*pathlib.PurePosixPath(relative).parts).read_bytes()
            != data
            for relative, data in expected_inputs.items()
        )
    ):
        raise ValueError("context current anchor persisted source differs")
    _validate_current_anchor_source_ledger(ledger, replay, rows, fit)

    legacy_root = root / "legacy-osaka"
    legacy_replay = canary.get("legacy_osaka_replay")
    if not isinstance(legacy_replay, Mapping):
        raise ValueError("context legacy Osaka persisted source is missing")
    decisions_path = legacy_root / "canary-decisions.json"
    if decisions_path.is_symlink() or not decisions_path.is_file():
        raise ValueError("context legacy Osaka decision source is missing")
    decisions = json.loads(decisions_path.read_bytes())
    if decisions != legacy_replay.get("decisions"):
        raise ValueError("context legacy Osaka decision source differs")
    for embedded, record in zip(
        legacy_replay.get("rounds", []), decisions.get("rounds", [])
    ):
        raw = legacy_root / str(record.get("raw_runs"))
        result = legacy_root / str(record.get("result"))
        if any(path.is_symlink() or not path.is_file() for path in (raw, result)):
            raise ValueError("context legacy Osaka round source is incomplete")
        if (
            [json.loads(line) for line in raw.read_bytes().splitlines()]
            != embedded.get("rows")
            or json.loads(result.read_bytes()) != embedded.get("result")
            or sha256_bytes(raw.read_bytes()) != embedded.get("source_raw_file_sha256")
            or sha256_bytes(result.read_bytes())
            != embedded.get("source_result_file_sha256")
        ):
            raise ValueError("context legacy Osaka round source differs")


def _publish_context_result(
    result: Mapping[str, Any],
    out_root: pathlib.Path,
    *,
    protected_inputs: Mapping[str, pathlib.Path],
) -> pathlib.Path:
    _validate_result_envelope(result)
    if set(protected_inputs) != {
        "calibration_run",
        "run",
        "manifest",
        "compatibility_canary",
        "corrected_core",
        "coverage_v5",
    }:
        raise ValueError("context seal protected-input inventory differs")
    out_root = pathlib.Path(out_root)
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    frozen_inputs = [
        repo_root / "experiments/opcode-gas/manifests/sp1-context-opcode-v1.json",
        repo_root / result["source_identity"]["corrected_osaka_core_path"],
        repo_root / result["source_identity"]["operation_coverage_v5_path"],
        *protected_inputs.values(),
    ]
    _reject_context_output_overlap(out_root, frozen_inputs)
    if out_root.is_symlink():
        raise ValueError("context result output root must not be a symlink")
    out_root.mkdir(parents=True, exist_ok=True)
    destination = out_root / result["result_id"]
    if destination.is_symlink():
        raise ValueError("context result destination conflicts")
    _replay_context_result_identity(result)
    files = _sealed_file_bytes(
        result["manifest"],
        result["rows"],
        result["source_registry"],
        result["compatibility_canary"],
        result["source_identity"],
        result["adaptive_evidence"],
    )
    if result["source_files"] != {name: sha256_bytes(data) for name, data in sorted(files.items())}:
        raise ValueError("context result source hash graph differs")
    files["result.json"] = canonical_json(result) + b"\n"
    if destination.exists():
        if destination.is_symlink() or not destination.is_dir():
            raise ValueError("context result destination conflicts")
        if {entry.name for entry in destination.iterdir()} != CONTEXT_RESULT_INVENTORY:
            raise ValueError("context result destination inventory conflicts")
        resolved_destination = destination.resolve(strict=True)
        for name in CONTEXT_RESULT_INVENTORY:
            path = destination / name
            try:
                mode = path.stat(follow_symlinks=False).st_mode
            except OSError as error:
                raise ValueError("context result destination inventory conflicts") from error
            if (
                path.is_symlink()
                or not stat.S_ISREG(mode)
                or path.resolve(strict=True).parent != resolved_destination
            ):
                raise ValueError("context result destination inventory conflicts")
        if all((destination / name).read_bytes() == data for name, data in files.items()):
            return destination
        raise ValueError("same context result id has conflicting bytes")
    temporary = pathlib.Path(tempfile.mkdtemp(prefix=".context-result-", dir=out_root))
    try:
        for name, data in files.items():
            path = temporary / name
            with path.open("xb") as output:
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
        os.rename(temporary, destination)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)
    return destination


def seal_context_result(
    *,
    manifest_path: pathlib.Path,
    calibration_run: pathlib.Path,
    run: pathlib.Path,
    compatibility_canary_path: pathlib.Path,
    corrected_core_path: pathlib.Path,
    coverage_v5_path: pathlib.Path,
    out_root: pathlib.Path,
) -> pathlib.Path:
    """Seal only a completed create-only runner ledger under its live calibration."""
    import opcode_gas

    calibration_run = pathlib.Path(calibration_run).resolve(strict=True)
    run = pathlib.Path(run).resolve(strict=True)
    if run != calibration_run / "context-campaign":
        raise ValueError("context production run path differs")
    _reject_context_output_overlap(
        pathlib.Path(out_root),
        (
            calibration_run,
            run,
            pathlib.Path(manifest_path),
            pathlib.Path(compatibility_canary_path),
            pathlib.Path(corrected_core_path),
            pathlib.Path(coverage_v5_path),
        ),
    )
    calibration_identity = opcode_gas.validate_calibration_execution_identity(
        calibration_run
    )
    manifest = load_context_manifest(manifest_path)
    adaptive_evidence = load_context_adaptive_evidence(run, manifest)
    campaign_source = adaptive_evidence.get("identity", {}).get("source")
    if (
        not isinstance(campaign_source, Mapping)
        or campaign_source.get("calibration_id") != calibration_run.name
        or campaign_source.get("calibration_identity") != calibration_identity
        or campaign_source.get("calibration_identity_sha256")
        != sha256_bytes(canonical_json(calibration_identity))
    ):
        raise ValueError("context production run differs from live calibration identity")

    repo_root = pathlib.Path(__file__).resolve().parents[2]
    expected_canary = (
        calibration_run / "context-compatibility" / "compatibility-canary.json"
    )
    compatibility_canary_path = pathlib.Path(compatibility_canary_path)
    if (
        compatibility_canary_path.is_symlink()
        or compatibility_canary_path.resolve(strict=True) != expected_canary
    ):
        raise ValueError("context seal requires canonical compatibility canary")
    compatibility_canary = _load_json(compatibility_canary_path)
    if compatibility_canary_path.read_bytes() != (
        json.dumps(compatibility_canary, indent=2, sort_keys=True) + "\n"
    ).encode():
        raise ValueError("context compatibility canary file is noncanonical")
    validate_context_compatibility_canary(compatibility_canary)
    _validate_context_canary_run_directory(calibration_run, compatibility_canary)
    current_replay = compatibility_canary.get("current_anchor", {}).get("replay")
    if (
        not isinstance(current_replay, Mapping)
        or current_replay.get("calibration_identity") != calibration_identity
        or current_replay.get("calibration_identity_sha256")
        != sha256_bytes(canonical_json(calibration_identity))
    ):
        raise ValueError("context compatibility canary calibration differs")
    launcher = repo_root / "target/release/guest-launcher"
    _replay_current_anchor_native_inputs(compatibility_canary, launcher)
    expected_core = repo_root / (
        "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/"
        "core-opcode-submodel.json"
    )
    expected_v5 = repo_root / "experiments/opcode-gas/manifests/operation-coverage-v5.json"
    corrected_core_path = pathlib.Path(corrected_core_path)
    coverage_v5_path = pathlib.Path(coverage_v5_path)
    if (
        corrected_core_path.is_symlink()
        or coverage_v5_path.is_symlink()
        or corrected_core_path.resolve(strict=True) != expected_core.resolve(strict=True)
        or coverage_v5_path.resolve(strict=True) != expected_v5.resolve(strict=True)
    ):
        raise ValueError("context seal requires canonical corrected core and V5 coverage")
    corrected_core = _load_json(corrected_core_path)
    coverage_v5 = _load_json(coverage_v5_path)
    source_identity = {
        "calibration_id": campaign_source["calibration_id"],
        "calibration_identity_sha256": campaign_source[
            "calibration_identity_sha256"
        ],
        "execution_revision": campaign_source["implementation_revision"],
        "elf_sha256": campaign_source["revm_elf_sha256"],
        "elf_path": "crates/guests/elf/sp1_revm_opcode_lab.elf",
        "control_opcode_lab_elf_sha256": campaign_source[
            "control_opcode_lab_elf_sha256"
        ],
        "control_opcode_lab_elf_path": "crates/guests/elf/sp1_opcode_lab.elf",
        "launcher_sha256": campaign_source["launcher_sha256"],
        "launcher_path": "target/release/guest-launcher",
        "corrected_osaka_core_artifact_sha256": corrected_core["artifact_sha256"],
        "corrected_osaka_core_file_sha256": sha256_bytes(
            corrected_core_path.read_bytes()
        ),
        "corrected_osaka_core_path": (
            "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/"
            "core-opcode-submodel.json"
        ),
        "operation_coverage_v5_artifact_sha256": coverage_v5["artifact_sha256"],
        "operation_coverage_v5_file_sha256": sha256_bytes(
            coverage_v5_path.read_bytes()
        ),
        "operation_coverage_v5_path": (
            "experiments/opcode-gas/manifests/operation-coverage-v5.json"
        ),
    }
    result = _build_context_result(
        manifest=manifest,
        rows=adaptive_evidence["terminal"]["rows"],
        source_registry=corrected_core,
        compatibility_canary=compatibility_canary,
        source_identity=source_identity,
        adaptive_evidence=adaptive_evidence,
    )
    return _publish_context_result(
        result,
        out_root,
        protected_inputs={
            "calibration_run": calibration_run,
            "run": run,
            "manifest": pathlib.Path(manifest_path),
            "compatibility_canary": pathlib.Path(compatibility_canary_path),
            "corrected_core": corrected_core_path,
            "coverage_v5": coverage_v5_path,
        },
    )


def verify_context_result(directory: pathlib.Path) -> dict[str, Any]:
    directory = pathlib.Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("context result must be a non-symlink directory")
    if {entry.name for entry in directory.iterdir()} != CONTEXT_RESULT_INVENTORY:
        raise ValueError("context result inventory differs")
    blobs = {}
    for name in CONTEXT_RESULT_INVENTORY:
        path = directory / name
        if path.is_symlink() or not path.is_file():
            raise ValueError("context result inventory contains a non-regular file")
        blobs[name] = path.read_bytes()
    result = json.loads(blobs["result.json"])
    if blobs["result.json"] != canonical_json(result) + b"\n":
        raise ValueError("context result JSON is noncanonical")
    _validate_result_envelope(result)
    if directory.name != result["result_id"]:
        raise ValueError("context result directory identity differs")
    for name, expected in result["source_files"].items():
        if sha256_bytes(blobs[name]) != expected:
            raise ValueError(f"context result {name} hash differs")
    manifest = json.loads(blobs["campaign-manifest.json"])
    registry = json.loads(blobs["source-registry.json"])
    canary = json.loads(blobs["compatibility-canary.json"])
    source_identity = json.loads(blobs["source-identity.json"])
    adaptive_evidence = json.loads(blobs["adaptive-evidence.json"])
    rows = [json.loads(raw) for raw in blobs["rows.jsonl"].splitlines()]
    replay = _build_context_result(
        manifest=manifest,
        rows=rows,
        source_registry=registry,
        compatibility_canary=canary,
        source_identity=source_identity,
        adaptive_evidence=adaptive_evidence,
    )
    if replay != result:
        raise ValueError("context result replay differs")
    _replay_context_result_identity(result)
    return result


def build_operation_coverage_v5(
    coverage_v4: Mapping[str, Any],
    corrected_core: Mapping[str, Any],
    corrected_package: Mapping[str, Any],
) -> dict[str, Any]:
    """Rebind all core-owned rows to the formula-corrected immutable successor."""
    v4_unsigned = dict(coverage_v4)
    v4_artifact = v4_unsigned.pop("artifact_sha256", None)
    core_unsigned = dict(corrected_core)
    core_artifact = core_unsigned.pop("artifact_sha256", None)
    package_unsigned = dict(corrected_package)
    package_artifact = package_unsigned.pop("artifact_sha256", None)
    augmentation = corrected_core.get("osaka_augmentation")
    output_hashes = corrected_package.get("output_hashes")
    if (
        coverage_v4.get("schema_version") != 1
        or coverage_v4.get("purpose") != "operation_coverage_ownership"
        or v4_artifact != sha256_bytes(canonical_json(v4_unsigned))
        or corrected_core.get("schema_version") != 4
        or corrected_core.get("purpose") != "osaka_augmented_core_opcode_submodel"
        or core_artifact != sha256_bytes(canonical_json(core_unsigned))
        or not isinstance(augmentation, Mapping)
        or augmentation.get("recovery_formula")
        != "lab_slope_times_body_scale_to_production_body_v2"
        or package_artifact != sha256_bytes(canonical_json(package_unsigned))
        or corrected_package.get("augmentation_id") != "3fc67063a921182e971e7882"
        or not isinstance(output_hashes, Mapping)
        or output_hashes.get("core_artifact_sha256") != core_artifact
        or output_hashes.get("core_file_sha256")
        != sha256_bytes((json.dumps(corrected_core, indent=2, sort_keys=True) + "\n").encode())
    ):
        raise ValueError("corrected Osaka core or V4 coverage source differs")
    old_source = coverage_v4.get("sources", {}).get("augmented_core_artifact")
    old_artifact = old_source.get("artifact_sha256") if isinstance(old_source, Mapping) else None
    if old_artifact != "b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b":
        raise ValueError("operation coverage V4 core source differs")
    promoted = copy.deepcopy(coverage_v4)
    promoted.pop("artifact_sha256")
    promoted["schema_version"] = 2
    promoted["predecessor_v4_artifact_sha256"] = v4_artifact
    promoted["sources"]["augmented_core_artifact"] = {
        "artifact_sha256": core_artifact,
        "augmentation_artifact_sha256": package_artifact,
        "augmentation_id": corrected_package["augmentation_id"],
        "augmentation_identity_sha256": corrected_package[
            "augmentation_identity_sha256"
        ],
        "file_sha256": output_hashes["core_file_sha256"],
        "modeled_named_opcode_count": corrected_core["modeled_named_opcode_count"],
        "path": "experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json",
        "recovery_formula": "lab_slope_times_body_scale_to_production_body_v2",
    }
    for row in promoted.get("execution_coverage", []):
        artifact_ref = row.get("artifact_ref")
        if isinstance(artifact_ref, dict):
            if artifact_ref.get("artifact_sha256") != old_artifact:
                raise ValueError("operation coverage V4 has an unknown artifact reference")
            artifact_ref["artifact_sha256"] = core_artifact
            artifact_ref["path"] = promoted["sources"]["augmented_core_artifact"]["path"]
        for evidence in row.get("source_evidence", []):
            if evidence.get("kind") in {
                "sealed_registry_model",
                "sealed_registry_unsupported",
            }:
                if evidence.get("artifact_sha256") != old_artifact:
                    raise ValueError("operation coverage V4 has unknown registry provenance")
                evidence["artifact_sha256"] = core_artifact
    promoted["artifact_sha256"] = sha256_bytes(canonical_json(promoted))
    return promoted


def promote_operation_coverage_v6(
    coverage_v5: Mapping[str, Any], result: Mapping[str, Any]
) -> dict[str, Any]:
    required = {f"opcode:0x{opcode:02x}" for opcode in CONTEXT_OPCODES}
    v5_unsigned = dict(coverage_v5)
    v5_artifact = v5_unsigned.pop("artifact_sha256", None)
    core_source = coverage_v5.get("sources", {}).get("augmented_core_artifact")
    if (
        coverage_v5.get("schema_version") != 2
        or coverage_v5.get("purpose") != "operation_coverage_ownership"
        or v5_artifact != sha256_bytes(canonical_json(v5_unsigned))
        or coverage_v5.get("predecessor_v4_artifact_sha256")
        != "2383d9788302f8447522abdcdc99a1f6490cc2b910c376ef9b4e265cc1c978be"
        or not isinstance(core_source, Mapping)
        or core_source.get("artifact_sha256")
        != result.get("source_identity", {}).get(
            "corrected_osaka_core_artifact_sha256"
        )
        or v5_artifact
        != result.get("source_identity", {}).get(
            "operation_coverage_v5_artifact_sha256"
        )
    ):
        raise ValueError("operation coverage V5 source differs from context result")
    promoted_keys = set(result.get("models", {}))
    if (
        result.get("status") != "sealed"
        or result.get("promoted_model_keys") != sorted(promoted_keys)
        or not promoted_keys.issubset(required)
    ):
        raise ValueError("context result promoted-key inventory differs")
    promoted = copy.deepcopy(coverage_v5)
    predecessor = promoted.pop("artifact_sha256", None)
    changed = set()
    seen = set()
    for row in promoted.get("execution_coverage", []):
        key = row.get("key")
        if key not in required:
            continue
        seen.add(key)
        if row.get("classification") != "explicitly_unsupported":
            raise ValueError(f"operation coverage V5 context key is not unsupported: {key}")
        if key not in promoted_keys:
            continue
        changed.add(key)
        row["classification"] = "static_raw_gas"
        row["model_status"] = "measured"
        row.pop("reason", None)
        row["artifact_ref"] = {
            "result_id": result["result_id"],
            "result_identity_sha256": result["result_identity_sha256"],
            "artifact_sha256": result["artifact_sha256"],
            "model_id": key,
            "model_kind": "static_raw_gas",
        }
        row["source_evidence"] = [
            evidence for evidence in row.get("source_evidence", [])
            if evidence.get("kind") != "sealed_registry_unsupported"
        ] + [{"kind": "sealed_context_opcode_model", **row["artifact_ref"]}]
    if seen != required or changed != promoted_keys:
        raise ValueError("operation coverage V5 lacks the complete context key family")
    promoted["schema_version"] = 3
    promoted["predecessor_v5_artifact_sha256"] = predecessor
    promoted["artifact_sha256"] = sha256_bytes(canonical_json(promoted))
    return promoted


def overlay_context_models(
    corrected_registry: Mapping[str, Any], result: Mapping[str, Any]
) -> dict[str, Any]:
    """Overlay only the six context bodies on an already corrected Osaka registry."""
    required = {f"opcode:0x{opcode:02x}" for opcode in CONTEXT_OPCODES}
    promoted_keys = set(result.get("models", {}))
    if (
        result.get("status") != "sealed"
        or result.get("promoted_model_keys") != sorted(promoted_keys)
        or not promoted_keys.issubset(required)
    ):
        raise ValueError("context result promoted-key overlay differs")
    overlaid = copy.deepcopy(corrected_registry)
    models = overlaid.get("models")
    if not isinstance(models, dict):
        raise ValueError("corrected registry model inventory differs")
    for key, context_model in result["models"].items():
        models[key] = {
            "kind": "static_raw_gas",
            "parameters": {
                "body_per_raw_gas": context_model["body_per_raw_gas_exact"]["decimal"]
            },
        }
        opcode = int(key.removeprefix("opcode:0x"), 16)
        slots = overlaid.get("opcode_model_ids")
        if not isinstance(slots, list) or len(slots) != 256:
            raise ValueError("corrected registry opcode slots differ")
        slots[opcode] = key
    return overlaid


def build_context_composite_estimator(
    *,
    base_estimator: Mapping[str, Any],
    corrected_core: Mapping[str, Any],
    coverage_v6: Mapping[str, Any],
    result: Mapping[str, Any],
    context_source_path: str,
) -> dict[str, Any]:
    """Build schema-3 by layering context after corrected core and typed storage."""
    from composite_estimator import (
        CONTEXT_ESTIMATOR_SCHEMA_VERSION,
        validate_estimator_artifact,
    )

    validate_estimator_artifact(base_estimator)
    _validate_result_envelope(result)
    if result.get("status") != "sealed":
        raise ValueError("context composite requires a complete sealed context result")
    required = {f"opcode:0x{opcode:02x}" for opcode in CONTEXT_OPCODES}
    promoted_keys = set(result.get("models", {}))
    if (
        result.get("promoted_model_keys") != sorted(promoted_keys)
        or not promoted_keys.issubset(required)
    ):
        raise ValueError("context composite result model inventory differs")
    v6_unsigned = dict(coverage_v6)
    v6_artifact = v6_unsigned.pop("artifact_sha256", None)
    if (
        coverage_v6.get("schema_version") != 3
        or v6_artifact != sha256_bytes(canonical_json(v6_unsigned))
        or coverage_v6.get("predecessor_v5_artifact_sha256")
        != result["source_identity"]["operation_coverage_v5_artifact_sha256"]
    ):
        raise ValueError("context composite operation coverage V6 differs")
    if (
        corrected_core.get("artifact_sha256")
        != result["source_identity"]["corrected_osaka_core_artifact_sha256"]
        or corrected_core.get("schema_version") != 4
        or corrected_core.get("osaka_augmentation", {}).get("recovery_formula")
        != "lab_slope_times_body_scale_to_production_body_v2"
    ):
        raise ValueError("context composite corrected core differs")
    import opcode_gas

    opcode_gas.assert_generated_paths_only(opcode_gas.git_worktree_status())
    implementation_revision = opcode_gas.git_head()
    if implementation_revision != result["source_identity"]["execution_revision"]:
        raise ValueError(
            "context composite analysis revision differs from execution revision"
        )
    pure = pathlib.PurePosixPath(context_source_path)
    if pure.is_absolute() or ".." in pure.parts or str(pure) != context_source_path:
        raise ValueError("context composite result path differs")

    registry = overlay_context_models(corrected_core["registry"], result)
    base_rows = {
        row["key"]: copy.deepcopy(row) for row in base_estimator["execution_coverage"]
    }
    final_rows = {
        row["key"]: copy.deepcopy(row) for row in coverage_v6["execution_coverage"]
    }
    # Typed storage is layered independently after V6 and retains its sealed source.
    for key in ("opcode:0x54", "opcode:0x55"):
        final_rows[key] = base_rows[key]
    context_models = {key: registry["models"][key] for key in sorted(promoted_keys)}
    result_files = _sealed_file_bytes(
        result["manifest"],
        result["rows"],
        result["source_registry"],
        result["compatibility_canary"],
        result["source_identity"],
        result["adaptive_evidence"],
    )
    result_files["result.json"] = canonical_json(result) + b"\n"
    source_code = dict(base_estimator["source_artifacts"]["source_code_sha256s"])
    repo_root = pathlib.Path(__file__).resolve().parents[2]
    for relative in (
        "experiments/opcode-gas/context_opcode_campaign.py",
        "docs/plans/2026-09-29-zkgas-context-operation-implementation-plan.md",
    ):
        source_code[relative] = sha256_bytes((repo_root / relative).read_bytes())
    artifact = copy.deepcopy(base_estimator)
    artifact.pop("artifact_sha256", None)
    artifact["schema_version"] = CONTEXT_ESTIMATOR_SCHEMA_VERSION
    artifact["implementation_revision"] = implementation_revision
    artifact["registry"] = registry
    artifact["execution_coverage"] = [
        final_rows[row["key"]] for row in coverage_v6["execution_coverage"]
    ]
    artifact["source_artifacts"]["augmented_core"] = {
        "path": result["source_identity"]["corrected_osaka_core_path"],
        "file_sha256": result["source_identity"]["corrected_osaka_core_file_sha256"],
        "artifact_sha256": corrected_core["artifact_sha256"],
    }
    artifact["source_artifacts"]["operation_coverage"] = {
        "path": "experiments/opcode-gas/manifests/operation-coverage-v6.json",
        "file_sha256": sha256_bytes(
            (json.dumps(coverage_v6, indent=2, sort_keys=True) + "\n").encode()
        ),
        "artifact_sha256": coverage_v6["artifact_sha256"],
    }
    artifact["source_artifacts"]["context_operations"] = {
        "path": context_source_path,
        "result_id": result["result_id"],
        "result_identity_sha256": result["result_identity_sha256"],
        "artifact_sha256": result["artifact_sha256"],
        "file_sha256s": {
            name: sha256_bytes(data) for name, data in sorted(result_files.items())
        },
        "models_sha256": sha256_bytes(canonical_json(context_models)),
        "promoted_model_keys": sorted(promoted_keys),
        "execution_revision": result["source_identity"]["execution_revision"],
        "analysis_revision": implementation_revision,
    }
    artifact["source_artifacts"]["source_code_sha256s"] = dict(sorted(source_code.items()))
    artifact["artifact_sha256"] = sha256_bytes(canonical_json(artifact))
    validate_estimator_artifact(artifact)
    return artifact


def predict_context_overlay_event(
    *, common_dispatch: Fraction, raw_gas: int, model: Mapping[str, Any]
) -> Fraction:
    if (
        model.get("kind") != "static_raw_gas"
        or model.get("parameter_basis")
        != "production_scaled_body_excluding_common_dispatch"
    ):
        raise ValueError("context overlay model basis differs")
    return predict_static_event(
        common_dispatch=common_dispatch,
        raw_gas=raw_gas,
        stored_body=fraction_from_payload(model["body_per_raw_gas_exact"]),
    )


def _load_json(path: pathlib.Path) -> Any:
    return json.loads(pathlib.Path(path).read_bytes())


def _load_jsonl(path: pathlib.Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in pathlib.Path(path).read_bytes().splitlines()]


def _write_canonical(path: pathlib.Path, value: Any) -> None:
    pathlib.Path(path).write_bytes(canonical_json(value) + b"\n")


def cmd_generate_fixtures(args: argparse.Namespace) -> None:
    manifest = load_context_manifest(args.manifest)
    output = pathlib.Path(args.out)
    output.mkdir(parents=True, exist_ok=True)
    bound = args.generator_bound
    prefix = ADAPTIVE_PREFIXES[ADAPTIVE_ROUNDS.index(bound)]
    for scenario in manifest["scenarios"]:
        for count in (*prefix, bound):
            for lane in ("control", "target"):
                fixture = generate_context_fixture(
                    manifest, scenario["name"], lane, count, generator_bound=bound
                )
                destination = output / scenario["name"] / str(bound) / str(count) / lane
                destination.mkdir(parents=True, exist_ok=True)
                _write_canonical(destination / "guest-input.json", fixture)


def cmd_run_compatibility_canary(args: argparse.Namespace) -> None:
    canary = run_context_compatibility_canary(
        calibration_run=args.calibration_run,
        controlled_manifest=args.controlled_manifest,
        baseline_derivation=args.baseline_derivation,
        historical_manifest=args.historical_manifest,
        historical_anchor_run=args.historical_anchor_run,
        guest_launcher=args.guest_launcher,
        revm_elf=args.revm_elf,
        control_opcode_lab_elf=args.control_opcode_lab_elf,
        output_root=args.out,
    )
    print(
        "context compatibility canary "
        f"{canary['status']} ({canary['artifact_sha256']})"
    )


def cmd_run_campaign(args: argparse.Namespace) -> None:
    result = run_context_opcode_campaign(
        manifest_path=args.manifest,
        calibration_run=args.calibration_run,
        run=args.run,
        fixtures_root=args.fixtures,
        guest_launcher=args.guest_launcher,
        elf=args.revm_elf,
        control_opcode_lab_elf=args.control_opcode_lab_elf,
    )
    print(
        f"context adaptive campaign {result['status']} "
        f"({len(result['rows'])} terminal rows)"
    )


def cmd_seal_result(args: argparse.Namespace) -> None:
    print(
        seal_context_result(
            manifest_path=args.manifest,
            calibration_run=args.calibration_run,
            run=args.run,
            compatibility_canary_path=args.compatibility_canary,
            corrected_core_path=args.corrected_core,
            coverage_v5_path=args.coverage_v5,
            out_root=args.out_root,
        )
    )


def cmd_verify_result(args: argparse.Namespace) -> None:
    result = verify_context_result(args.result)
    print(f"verified context opcode result {result['result_id']} ({result['status']})")


def cmd_promote_coverage(args: argparse.Namespace) -> None:
    result = verify_context_result(args.result)
    promoted = promote_operation_coverage_v6(_load_json(args.coverage_v5), result)
    _write_canonical(args.out, promoted)


def cmd_build_coverage_v5(args: argparse.Namespace) -> None:
    promoted = build_operation_coverage_v5(
        _load_json(args.coverage_v4),
        _load_json(args.corrected_core),
        _load_json(args.corrected_package),
    )
    pathlib.Path(args.out).write_text(json.dumps(promoted, indent=2, sort_keys=True) + "\n")


def cmd_build_composite_v3(args: argparse.Namespace) -> None:
    result = verify_context_result(args.context_result)
    artifact = build_context_composite_estimator(
        base_estimator=_load_json(args.base_estimator),
        corrected_core=_load_json(args.corrected_core),
        coverage_v6=_load_json(args.coverage_v6),
        result=result,
        context_source_path=args.context_source_path,
    )
    pathlib.Path(args.out).write_text(
        json.dumps(artifact, indent=2, sort_keys=True) + "\n"
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("generate-fixtures")
    generate.add_argument("--manifest", type=pathlib.Path, required=True)
    generate.add_argument("--generator-bound", type=int, choices=ADAPTIVE_ROUNDS, default=8)
    generate.add_argument("--out", type=pathlib.Path, required=True)
    generate.set_defaults(func=cmd_generate_fixtures)
    canary = commands.add_parser("run-compatibility-canary")
    canary.add_argument("--calibration-run", type=pathlib.Path, required=True)
    canary.add_argument("--controlled-manifest", type=pathlib.Path, required=True)
    canary.add_argument("--baseline-derivation", type=pathlib.Path, required=True)
    canary.add_argument("--historical-manifest", type=pathlib.Path, required=True)
    canary.add_argument("--historical-anchor-run", type=pathlib.Path, required=True)
    canary.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    canary.add_argument("--revm-elf", type=pathlib.Path, required=True)
    canary.add_argument(
        "--control-opcode-lab-elf", type=pathlib.Path, required=True
    )
    canary.add_argument("--out", type=pathlib.Path, required=True)
    canary.set_defaults(func=cmd_run_compatibility_canary)
    run = commands.add_parser("run-campaign")
    run.add_argument("--manifest", type=pathlib.Path, required=True)
    run.add_argument("--calibration-run", type=pathlib.Path, required=True)
    run.add_argument("--run", type=pathlib.Path, required=True)
    run.add_argument("--fixtures", type=pathlib.Path, required=True)
    run.add_argument("--guest-launcher", type=pathlib.Path, required=True)
    run.add_argument("--revm-elf", type=pathlib.Path, required=True)
    run.add_argument("--control-opcode-lab-elf", type=pathlib.Path, required=True)
    run.set_defaults(func=cmd_run_campaign)
    seal = commands.add_parser("seal-result")
    seal.add_argument("--manifest", type=pathlib.Path, required=True)
    seal.add_argument("--calibration-run", type=pathlib.Path, required=True)
    seal.add_argument("--run", type=pathlib.Path, required=True)
    seal.add_argument("--corrected-core", type=pathlib.Path, required=True)
    seal.add_argument("--coverage-v5", type=pathlib.Path, required=True)
    seal.add_argument("--compatibility-canary", type=pathlib.Path, required=True)
    seal.add_argument("--out-root", type=pathlib.Path, required=True)
    seal.set_defaults(func=cmd_seal_result)
    verify = commands.add_parser("verify-result")
    verify.add_argument("--result", type=pathlib.Path, required=True)
    verify.set_defaults(func=cmd_verify_result)
    promote = commands.add_parser("promote-coverage-v6")
    promote.add_argument("--coverage-v5", type=pathlib.Path, required=True)
    promote.add_argument("--result", type=pathlib.Path, required=True)
    promote.add_argument("--out", type=pathlib.Path, required=True)
    promote.set_defaults(func=cmd_promote_coverage)
    coverage_v5 = commands.add_parser("build-coverage-v5")
    coverage_v5.add_argument("--coverage-v4", type=pathlib.Path, required=True)
    coverage_v5.add_argument("--corrected-core", type=pathlib.Path, required=True)
    coverage_v5.add_argument("--corrected-package", type=pathlib.Path, required=True)
    coverage_v5.add_argument("--out", type=pathlib.Path, required=True)
    coverage_v5.set_defaults(func=cmd_build_coverage_v5)
    composite = commands.add_parser("build-composite-v3")
    composite.add_argument("--base-estimator", type=pathlib.Path, required=True)
    composite.add_argument("--corrected-core", type=pathlib.Path, required=True)
    composite.add_argument("--coverage-v6", type=pathlib.Path, required=True)
    composite.add_argument("--context-result", type=pathlib.Path, required=True)
    composite.add_argument("--context-source-path", required=True)
    composite.add_argument("--out", type=pathlib.Path, required=True)
    composite.set_defaults(func=cmd_build_composite_v3)
    return parser


def main() -> None:
    args = build_parser().parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
