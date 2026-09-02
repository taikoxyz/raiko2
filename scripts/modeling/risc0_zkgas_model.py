#!/usr/bin/env python3
"""Package and verify the production RISC0 zkGas cycle-estimation model."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import pathlib
import sys
import tempfile
from decimal import Decimal, ROUND_HALF_UP
from typing import Any, Iterable, Mapping, Sequence


ROOT = pathlib.Path(__file__).resolve().parents[2]
DEFAULT_FIXTURE_DIR = (
    ROOT
    / "tests"
    / "fixtures"
    / "risc0-zkgas"
    / "2026-09-02-m2-aggregation-direct-v3"
)
DEFAULT_MODEL = ROOT / "crates" / "prover" / "models" / "risc0-zkgas.json"
MODEL_ID_PREFIX = "risc0-zkgas-m2-"
AUTO_MODEL_ID = f"{MODEL_ID_PREFIX}auto"
CONTENT_ID_HEX_LENGTH = 16
MAX_DIAGNOSTIC_DECIMAL_PLACES = 18
UNZEN_TIMESTAMPS = {
    "taiko_hoodi": 1_781_787_600,
    "taiko_mainnet": 1_786_021_200,
}
COHORT_INVARIANT_FIELDS = (
    "source_revision",
    "image_id",
    "risc0_image_id",
    "risc0_version",
    "execution_po2",
    "artifact_hashes",
)
ARTIFACT_HASH_FIELDS = {
    "cargo_lock_sha256",
    "chain_spec_sha256",
    "collector_script_sha256",
    "guest_launcher_binary_sha256",
    "preflight_binary_sha256",
    "proposal_elf_sha256",
    "stress_discovery_script_sha256",
}
SHARED_ARTIFACT_HASH_FIELDS = ARTIFACT_HASH_FIELDS - {"chain_spec_sha256"}
COMPACT_FIELDS = (
    "network",
    "split",
    "proposal_id",
    "block_count",
    "total_zkgas",
    "actual_mcycles",
)


class ModelError(ValueError):
    """The configured model cannot be reproduced safely."""


def read_json(path: pathlib.Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ModelError(f"cannot read {path}: {error}") from error


def read_jsonl(path: pathlib.Path) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise ModelError(f"cannot read {path}: {error}") from error
    rows = []
    for line_number, line in enumerate(lines, 1):
        if not line:
            raise ModelError(f"{path}:{line_number}: empty JSONL row")
        try:
            row = json.loads(line)
        except json.JSONDecodeError as error:
            raise ModelError(f"{path}:{line_number}: invalid JSON: {error}") from error
        if not isinstance(row, dict):
            raise ModelError(f"{path}:{line_number}: row must be an object")
        rows.append(row)
    return rows


def positive_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise ModelError(f"{label} must be a positive integer")
    return value


def rust_uint(value: Any, bits: int, label: str, *, nonzero: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise ModelError(f"{label} must be a u{bits} integer")
    if value < 0 or value > (1 << bits) - 1:
        raise ModelError(f"{label} must fit u{bits}")
    if nonzero and value == 0:
        raise ModelError(f"{label} must be non-zero")
    return value


def require_keys(value: Any, expected: set[str], label: str) -> Mapping[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise ModelError(f"{label} has an unsupported schema")
    return value


def is_lower_hex(value: Any, length: int) -> bool:
    return (
        isinstance(value, str)
        and len(value) == length
        and all(character in "0123456789abcdef" for character in value)
    )


def validate_config(config: Mapping[str, Any]) -> None:
    require_keys(
        config,
        {
            "schema_version",
            "model_id",
            "originating_experiment_model",
            "raw_input_rows_hash",
            "proposal",
            "aggregation",
        },
        "model config",
    )
    schema_version = rust_uint(config.get("schema_version"), 32, "schema_version")
    if schema_version != 3:
        raise ModelError("unsupported schema_version")
    model_id = config.get("model_id")
    if (
        not isinstance(model_id, str)
        or not model_id.startswith(MODEL_ID_PREFIX)
        or len(model_id) == len(MODEL_ID_PREFIX)
    ):
        raise ModelError("unsupported model_id family")
    if model_id != AUTO_MODEL_ID and not is_lower_hex(
        model_id[len(MODEL_ID_PREFIX) :], CONTENT_ID_HEX_LENGTH
    ):
        raise ModelError(
            "new model_id must be content-addressed or use risc0-zkgas-m2-auto"
        )
    if config.get("originating_experiment_model") != "M2":
        raise ModelError("unsupported originating experiment model")

    proposal = require_keys(
        config["proposal"],
        {
            "provenance",
            "collector_artifact_hashes",
            "coefficients",
            "max_total_zkgas",
            "cohorts",
            "error_budget_percent",
        },
        "proposal config",
    )
    provenance = require_keys(
        proposal["provenance"],
        {
            "source_revision",
            "image_id",
            "elf_sha256",
            "risc0_version",
            "min_execution_po2",
        },
        "proposal provenance",
    )
    if not is_lower_hex(provenance.get("source_revision"), 40):
        raise ModelError("proposal source_revision must be a 40-character SHA-1")
    image_id = provenance.get("image_id")
    if not (
        isinstance(image_id, str)
        and image_id.startswith("0x")
        and is_lower_hex(image_id[2:], 64)
    ):
        raise ModelError("proposal image_id must be a 32-byte hexadecimal value")
    if not is_lower_hex(provenance.get("elf_sha256"), 64):
        raise ModelError("proposal elf_sha256 must be a SHA-256 digest")
    version = provenance.get("risc0_version")
    if not (
        isinstance(version, str)
        and len(version.split(".")) == 3
        and all(
            part and part.isascii() and part.isdigit()
            for part in version.split(".")
        )
    ):
        raise ModelError("proposal risc0_version must be a semantic version")
    rust_uint(
        provenance.get("min_execution_po2"),
        32,
        "proposal min_execution_po2",
        nonzero=True,
    )

    artifact_config = require_keys(
        proposal["collector_artifact_hashes"],
        {"shared", "chain_spec_sha256_by_network"},
        "collector artifact hashes",
    )
    shared = require_keys(
        artifact_config["shared"],
        SHARED_ARTIFACT_HASH_FIELDS,
        "collector shared artifact hashes",
    )
    for field, value in shared.items():
        if not is_lower_hex(value, 64):
            raise ModelError(f"collector {field} must be a SHA-256 digest")
    if shared["proposal_elf_sha256"] != provenance["elf_sha256"]:
        raise ModelError("collector proposal ELF must match proposal provenance")
    chain_specs = require_keys(
        artifact_config["chain_spec_sha256_by_network"],
        {"taiko_hoodi", "taiko_mainnet"},
        "collector chain-spec hashes",
    )
    for network, value in chain_specs.items():
        if not is_lower_hex(value, 64):
            raise ModelError(f"collector {network} chain_spec_sha256 must be a SHA-256 digest")

    coefficients = require_keys(
        proposal["coefficients"], {"scale"}, "proposal coefficient config"
    )
    rust_uint(coefficients["scale"], 64, "coefficient scale", nonzero=True)
    rust_uint(
        proposal.get("max_total_zkgas"),
        64,
        "proposal max_total_zkgas",
        nonzero=True,
    )

    cohorts = require_keys(
        proposal["cohorts"], {"hoodi", "mainnet"}, "proposal cohorts"
    )
    hoodi = require_keys(
        cohorts["hoodi"],
        {"fit_count", "calibration_count", "diagnostic_decimal_places"},
        "Hoodi cohort",
    )
    mainnet = require_keys(
        cohorts["mainnet"],
        {
            "evaluation_count",
            "influenced_model_selection",
            "untouched_holdout",
            "diagnostic_decimal_places",
        },
        "Mainnet cohort",
    )
    for label, value in (
        ("Hoodi fit_count", hoodi["fit_count"]),
        ("Hoodi calibration_count", hoodi["calibration_count"]),
        ("Mainnet evaluation_count", mainnet["evaluation_count"]),
    ):
        rust_uint(value, 32, label, nonzero=True)
    for network, cohort in (("Hoodi", hoodi), ("Mainnet", mainnet)):
        places = require_keys(
            cohort["diagnostic_decimal_places"],
            {"continuous", "scaled_integer"},
            f"{network} diagnostic decimal places",
        )
        continuous_places = rust_uint(
            places["continuous"], 32, f"{network} continuous decimal places"
        )
        integer_places = rust_uint(
            places["scaled_integer"],
            32,
            f"{network} scaled integer decimal places",
        )
        if max(continuous_places, integer_places) > MAX_DIAGNOSTIC_DECIMAL_PLACES:
            raise ModelError(
                f"{network} diagnostic decimal places must not exceed "
                f"{MAX_DIAGNOSTIC_DECIMAL_PLACES}"
            )
    if not isinstance(mainnet["influenced_model_selection"], bool) or not isinstance(
        mainnet["untouched_holdout"], bool
    ):
        raise ModelError("Mainnet cohort provenance flags must be booleans")
    if not mainnet["influenced_model_selection"] or mainnet["untouched_holdout"]:
        raise ModelError("Mainnet evaluation provenance is inconsistent")
    error_budget = proposal["error_budget_percent"]
    if isinstance(error_budget, bool) or not isinstance(error_budget, int) or error_budget != 10:
        raise ModelError("proposal error_budget_percent must be exactly 10")

    validate_aggregation(config["aggregation"])


def validate_aggregation(aggregation: Mapping[str, Any]) -> None:
    aggregation = require_keys(
        aggregation,
        {
            "per_child_mcycles",
            "provenance",
        },
        "aggregation config",
    )
    per_child_mcycles = aggregation.get("per_child_mcycles")
    rust_uint(
        per_child_mcycles,
        64,
        "aggregation per_child_mcycles",
        nonzero=True,
    )
    provenance = require_keys(
        aggregation["provenance"],
        {"image_id", "elf_sha256", "execution_po2"},
        "aggregation provenance",
    )
    image_id = provenance["image_id"]
    if image_id is not None and not (
        isinstance(image_id, str)
        and image_id.startswith("0x")
        and is_lower_hex(image_id[2:], 64)
    ):
        raise ModelError("aggregation image_id must be a 32-byte hexadecimal value")
    if not is_lower_hex(provenance.get("elf_sha256"), 64):
        raise ModelError("aggregation elf_sha256 must be a SHA-256 digest")
    rust_uint(
        provenance.get("execution_po2"),
        32,
        "aggregation execution_po2",
        nonzero=True,
    )


def successful_rows(
    rows: Sequence[Mapping[str, Any]], label: str
) -> list[Mapping[str, Any]]:
    if any("status" not in row for row in rows):
        raise ModelError(f"{label} contains a row missing status")
    successful = [row for row in rows if row.get("status") == "success"]
    if not successful:
        raise ModelError(f"{label} contains no successful observations")
    return successful


def validate_collector_cohort(
    rows: Sequence[Mapping[str, Any]],
    label: str,
    expected_network: str,
    config: Mapping[str, Any],
) -> Mapping[str, Any]:
    successful = successful_rows(rows, label)
    sample_keys: set[str] = set()
    expected_splits = (
        {"fit", "calibration"}
        if expected_network == "taiko_hoodi"
        else {"holdout"}
    )
    for index, row in enumerate(successful, 1):
        row_label = f"{label} successful row {index}"
        row_schema = rust_uint(
            row.get("schema_version"), 32, f"{row_label} schema_version"
        )
        if row_schema != 1:
            raise ModelError(f"{row_label} has unsupported schema_version")
        sample_key = row.get("sample_key")
        if not isinstance(sample_key, str) or not sample_key:
            raise ModelError(f"{row_label} sample_key must be present")
        expected_sample_key = (
            f"{expected_network}:{row.get('proposal_id')}:{row.get('image_id')}"
        )
        if sample_key != expected_sample_key:
            raise ModelError(f"{row_label} sample_key does not match collector identity")
        if sample_key in sample_keys:
            raise ModelError(f"{label} contains duplicate sample_key {sample_key}")
        sample_keys.add(sample_key)
        if row.get("network") != expected_network:
            raise ModelError(f"{label} contains an observation for the wrong network")
        if row.get("split") not in expected_splits:
            raise ModelError(f"{label} contains an unsupported collector split")
        if "evaluated_mcycles_count" not in row:
            raise ModelError(f"{row_label} is missing evaluated_mcycles_count")
        evaluated_mcycles = positive_int(
            row["evaluated_mcycles_count"],
            f"{row_label} evaluated_mcycles_count",
        )
        user_cycles = positive_int(
            row.get("risc0_user_cycles"), f"{row_label} risc0_user_cycles"
        )
        if (user_cycles + 999_999) // 1_000_000 != evaluated_mcycles:
            raise ModelError(
                f"{row_label} evaluated_mcycles_count does not match risc0_user_cycles"
            )
        positive_int(row.get("attempt"), f"{row_label} attempt")
        if row.get("unzen_timestamp") != UNZEN_TIMESTAMPS[expected_network]:
            raise ModelError(f"{row_label} has an invalid unzen_timestamp")
        if "stratum" not in row:
            raise ModelError(f"{row_label} is missing stratum")
        stratum = row["stratum"]
        if stratum is not None and (not isinstance(stratum, str) or not stratum):
            raise ModelError(f"{row_label} stratum must be null or a non-empty string")
        if "actual_mcycles" in row:
            raise ModelError(f"{row_label} must not provide compact actual_mcycles")
    cohort: dict[str, Any] = {}
    for field in COHORT_INVARIANT_FIELDS:
        values = [row.get(field) for row in successful]
        if any(value is None for value in values):
            raise ModelError(f"{label} is missing cohort invariant {field}")
        encoded = {
            json.dumps(value, sort_keys=True, separators=(",", ":"))
            for value in values
        }
        if len(encoded) != 1:
            raise ModelError(f"{label} crosses cohort invariant {field}")
        cohort[field] = values[0]

    artifact_hashes = cohort["artifact_hashes"]
    if (
        not isinstance(artifact_hashes, dict)
        or set(artifact_hashes) != ARTIFACT_HASH_FIELDS
    ):
        raise ModelError(f"{label} artifact_hashes has an unsupported schema")
    for field, value in artifact_hashes.items():
        if not is_lower_hex(value, 64):
            raise ModelError(f"{label} artifact_hashes.{field} must be a SHA-256 digest")

    provenance = config["proposal"]["provenance"]
    for field in ("source_revision", "image_id", "risc0_version"):
        if cohort[field] != provenance[field]:
            raise ModelError(f"{label} {field} does not match config provenance")
    if cohort["execution_po2"] != provenance["min_execution_po2"]:
        raise ModelError(f"{label} execution_po2 does not match config provenance")
    if cohort["risc0_image_id"] != provenance["image_id"]:
        raise ModelError(f"{label} risc0_image_id does not match config provenance")
    if artifact_hashes["proposal_elf_sha256"] != provenance["elf_sha256"]:
        raise ModelError(f"{label} proposal ELF does not match config provenance")
    approved = config["proposal"]["collector_artifact_hashes"]
    expected_hashes = {
        **approved["shared"],
        "chain_spec_sha256": approved["chain_spec_sha256_by_network"][
            expected_network
        ],
    }
    for field, expected in expected_hashes.items():
        if artifact_hashes[field] != expected:
            raise ModelError(f"{label} {field} does not match config")
    return cohort


def validate_shared_collector_artifacts(
    hoodi: Mapping[str, Any], mainnet: Mapping[str, Any]
) -> None:
    for field in SHARED_ARTIFACT_HASH_FIELDS:
        if hoodi["artifact_hashes"][field] != mainnet["artifact_hashes"][field]:
            raise ModelError(f"collector cohorts disagree on artifact_hashes.{field}")


def compact_row(row: Mapping[str, Any], label: str) -> dict[str, Any]:
    actual = row.get("actual_mcycles", row.get("evaluated_mcycles_count"))
    compact = {
        "network": row.get("network"),
        "split": row.get("split"),
        "proposal_id": rust_uint(
            row.get("proposal_id"), 32, f"{label} proposal_id", nonzero=True
        ),
        "block_count": rust_uint(
            row.get("block_count"), 32, f"{label} block_count", nonzero=True
        ),
        "total_zkgas": rust_uint(
            row.get("total_zkgas"), 32, f"{label} total_zkgas", nonzero=True
        ),
        "actual_mcycles": rust_uint(
            actual, 32, f"{label} actual_mcycles", nonzero=True
        ),
    }
    if compact["network"] not in {"taiko_hoodi", "taiko_mainnet"}:
        raise ModelError(f"{label} has unsupported network {compact['network']!r}")
    if compact["split"] not in {"fit", "calibration", "evaluation"}:
        raise ModelError(f"{label} has unsupported split {compact['split']!r}")
    return compact


def project_rows(
    rows: Iterable[Mapping[str, Any]],
    label: str,
    split_overrides: Mapping[str, str] | None = None,
) -> list[dict[str, Any]]:
    projected = []
    identities: set[tuple[str, int]] = set()
    for index, row in enumerate(rows, 1):
        if "status" in row and row.get("status") != "success":
            continue
        source = dict(row)
        if split_overrides and source.get("split") in split_overrides:
            source["split"] = split_overrides[source["split"]]
        compact = compact_row(source, f"{label} row {index}")
        identity = (compact["network"], compact["proposal_id"])
        if identity in identities:
            raise ModelError(f"{label} contains duplicate observation {identity}")
        identities.add(identity)
        projected.append(compact)
    return projected


def jsonl_bytes(rows: Sequence[Mapping[str, Any]]) -> bytes:
    return b"".join(
        (json.dumps(dict(row), separators=(",", ":")) + "\n").encode()
        for row in rows
    )


def artifact_bytes(artifact: Mapping[str, Any]) -> bytes:
    return (json.dumps(artifact, indent=2, sort_keys=True) + "\n").encode()


def canonical_json_bytes(value: Mapping[str, Any]) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def generator_config_sha256(config: Mapping[str, Any]) -> str:
    identity_config = copy.deepcopy(config)
    identity_config.pop("model_id")
    return hashlib.sha256(canonical_json_bytes(identity_config)).hexdigest()


def resolve_model_id(artifact: dict[str, Any], configured_model_id: str) -> None:
    identity = copy.deepcopy(artifact)
    identity.pop("model_id")
    digest = hashlib.sha256(canonical_json_bytes(identity)).hexdigest()
    expected = f"{MODEL_ID_PREFIX}{digest[:CONTENT_ID_HEX_LENGTH]}"
    if configured_model_id == AUTO_MODEL_ID:
        artifact["model_id"] = expected
    elif configured_model_id != expected:
        raise ModelError(
            f"model_id {configured_model_id} does not match model content; use {expected}"
        )


def solve_linear_system(matrix: list[list[float]], vector: list[float]) -> list[float]:
    size = len(vector)
    augmented = [matrix[row][:] + [vector[row]] for row in range(size)]
    for column in range(size):
        pivot = max(range(column, size), key=lambda row: abs(augmented[row][column]))
        if abs(augmented[pivot][column]) < 1e-12:
            raise ModelError("Hoodi fit feature matrix is singular")
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        augmented[column] = [value / divisor for value in augmented[column]]
        for row in range(size):
            if row == column:
                continue
            factor = augmented[row][column]
            augmented[row] = [
                current - factor * pivot_value
                for current, pivot_value in zip(augmented[row], augmented[column])
            ]
    return [augmented[row][-1] for row in range(size)]


def left_fold_sum(values: Iterable[float], start: float = 0.0) -> float:
    total = start
    for value in values:
        total += value
    return total


def left_fold_mean(values: Sequence[float]) -> float:
    if not values:
        raise ModelError("cannot average an empty fit column")
    return left_fold_sum(values) / len(values)


def fit_m2(rows: Sequence[Mapping[str, Any]]) -> dict[str, Decimal]:
    features = ("total_zkgas", "block_count")
    columns = [[float(row[feature]) for row in rows] for feature in features]
    target = [float(row["actual_mcycles"]) for row in rows]
    means = [left_fold_mean(column) for column in columns]
    scales = [
        max(abs(value - mean) for value in column)
        for column, mean in zip(columns, means)
    ]
    if any(scale == 0 for scale in scales):
        raise ModelError("Hoodi fit feature matrix is singular")
    normalized = [
        [(value - mean) / scale for value in column]
        for column, mean, scale in zip(columns, means, scales)
    ]
    target_mean = left_fold_mean(target)
    centered_target = [value - target_mean for value in target]
    gram = [
        [
            left_fold_sum(
                left * right for left, right in zip(normalized[i], normalized[j])
            )
            for j in range(len(features))
        ]
        for i in range(len(features))
    ]
    rhs = [
        left_fold_sum(
            value * target_value
            for value, target_value in zip(column, centered_target)
        )
        for column in normalized
    ]
    normalized_coefficients = solve_linear_system(gram, rhs)
    slopes = [value / scale for value, scale in zip(normalized_coefficients, scales)]
    intercept = target_mean - left_fold_sum(
        slope * mean for slope, mean in zip(slopes, means)
    )
    fitted = {"intercept": intercept, **dict(zip(features, slopes))}
    return {name: Decimal(str(value)) for name, value in fitted.items()}


def scaled_coefficients(
    config: Mapping[str, Any], decimal: Mapping[str, Decimal]
) -> dict[str, int]:
    scale = rust_uint(
        config["proposal"]["coefficients"].get("scale"),
        64,
        "coefficient scale",
        nonzero=True,
    )
    scaled = {
        "scale": scale,
        **{
            name: int((value * scale).to_integral_value(rounding=ROUND_HALF_UP))
            for name, value in decimal.items()
        },
    }
    for name in ("intercept", "total_zkgas", "block_count"):
        rust_uint(
            scaled[name],
            64,
            f"scaled {name} coefficient",
            nonzero=True,
        )
    return scaled


def continuous_prediction(
    coefficients: Mapping[str, Decimal], row: Mapping[str, Any]
) -> Decimal:
    return (
        coefficients["intercept"]
        + coefficients["total_zkgas"] * row["total_zkgas"]
        + coefficients["block_count"] * row["block_count"]
    )


def integer_prediction(
    coefficients: Mapping[str, int], row: Mapping[str, Any]
) -> Decimal:
    numerator = (
        coefficients["intercept"]
        + coefficients["total_zkgas"] * row["total_zkgas"]
        + coefficients["block_count"] * row["block_count"]
    )
    return Decimal((numerator + coefficients["scale"] - 1) // coefficients["scale"])


def errors(rows: Sequence[Mapping[str, Any]], predict) -> list[Decimal]:
    return [
        (predict(row) - row["actual_mcycles"]) * 100 / row["actual_mcycles"]
        for row in rows
    ]


def formatted(value: Decimal, places: int) -> str:
    quantum = Decimal(1).scaleb(-places)
    return format(value.quantize(quantum, rounding=ROUND_HALF_UP), f".{places}f")


def hoodi_diagnostics(values: Sequence[Decimal], places: int) -> dict[str, Any]:
    return {
        "underquote_count": sum(value < 0 for value in values),
        "mape_percent": formatted(sum(map(abs, values)) / len(values), places),
        "max_absolute_error_percent": formatted(max(map(abs, values)), places),
        "max_underquote_percent": formatted(
            max((-value for value in values if value < 0), default=Decimal(0)),
            places,
        ),
        "over_ten_percent_count": sum(abs(value) > 10 for value in values),
    }


def mainnet_diagnostics(values: Sequence[Decimal], places: int) -> dict[str, Any]:
    return {
        "underquote_count": sum(value < 0 for value in values),
        "mape_percent": formatted(sum(map(abs, values)) / len(values), places),
        "max_underquote_percent": formatted(
            max((-value for value in values if value < 0), default=Decimal(0)),
            places,
        ),
        "overquote_over_ten_percent_count": sum(value > 10 for value in values),
        "max_overquote_percent": formatted(
            max((value for value in values if value > 0), default=Decimal(0)),
            places,
        ),
    }


def require_count(rows: Sequence[Any], expected: int, label: str) -> None:
    if len(rows) != expected:
        raise ModelError(f"expected {expected} {label} rows, found {len(rows)}")


def validate_operating_policy(
    max_total_zkgas: int,
    observations: Sequence[Mapping[str, Any]],
    scaled: Mapping[str, int],
    error_budget: Decimal,
) -> None:
    admitted = [
        row for row in observations if row["total_zkgas"] <= max_total_zkgas
    ]
    if not admitted:
        raise ModelError("proposal max_total_zkgas admits no observations")
    # The per-network domains this replaced each required an admitted observation, so a cap could
    # never silently exclude a whole cohort. Keep that guarantee: the fit cohort in particular must
    # still contribute evidence to the budget check.
    admitted_networks = {row["network"] for row in admitted}
    for network in sorted({row["network"] for row in observations}):
        if network not in admitted_networks:
            raise ModelError(
                f"proposal max_total_zkgas admits no {network} observations"
            )
    admitted_errors = errors(
        admitted, lambda item: integer_prediction(scaled, item)
    )
    for row, error in zip(admitted, admitted_errors):
        if abs(error) > error_budget:
            raise ModelError(
                f"{row['network']} proposal {row['proposal_id']} exceeds the configured "
                f"{formatted(error_budget, 0)}% error budget ({formatted(abs(error), 4)}%)"
            )


def generate_artifact(
    config: Mapping[str, Any],
    fit_rows: Sequence[Mapping[str, Any]],
    validation_rows: Sequence[Mapping[str, Any]],
    fit_payload: bytes,
    validation_payload: bytes,
) -> dict[str, Any]:
    validate_config(config)
    if config.get("raw_input_rows_hash") != {
        "algorithm": "sha256",
        "inputs": ["hoodi-fit.jsonl", "validation.jsonl"],
    }:
        raise ModelError(
            "raw_input_rows_hash must describe SHA-256 over fit then validation bytes"
        )
    proposal_config = config["proposal"]
    cohort_config = proposal_config["cohorts"]
    hoodi_fit = [
        row
        for row in fit_rows
        if (row["network"], row["split"]) == ("taiko_hoodi", "fit")
    ]
    hoodi_calibration = [
        row
        for row in validation_rows
        if (row["network"], row["split"])
        == ("taiko_hoodi", "calibration")
    ]
    mainnet = [
        row
        for row in validation_rows
        if (row["network"], row["split"])
        == ("taiko_mainnet", "evaluation")
    ]
    require_count(hoodi_fit, cohort_config["hoodi"]["fit_count"], "Hoodi fit")
    require_count(
        hoodi_calibration,
        cohort_config["hoodi"]["calibration_count"],
        "Hoodi calibration",
    )
    require_count(
        mainnet,
        cohort_config["mainnet"]["evaluation_count"],
        "Mainnet evaluation",
    )
    if len(hoodi_fit) + len(hoodi_calibration) + len(mainnet) != len(
        fit_rows
    ) + len(validation_rows):
        raise ModelError("fixtures contain an unsupported network or split")
    if fit_payload != jsonl_bytes(hoodi_fit):
        raise ModelError("hoodi-fit.jsonl must use canonical compact JSONL bytes")
    if validation_payload != jsonl_bytes(validation_rows):
        raise ModelError("validation.jsonl must use canonical compact JSONL bytes")

    decimal = fit_m2(hoodi_fit)
    scaled = scaled_coefficients(config, decimal)
    error_budget = Decimal(str(proposal_config["error_budget_percent"]))
    validate_operating_policy(
        proposal_config["max_total_zkgas"],
        [*hoodi_fit, *hoodi_calibration, *mainnet],
        scaled,
        error_budget,
    )
    hoodi_places = cohort_config["hoodi"]["diagnostic_decimal_places"]
    mainnet_places = cohort_config["mainnet"]["diagnostic_decimal_places"]
    hoodi_continuous = errors(
        hoodi_calibration, lambda row: continuous_prediction(decimal, row)
    )
    hoodi_integer = errors(
        hoodi_calibration, lambda row: integer_prediction(scaled, row)
    )
    mainnet_continuous = errors(
        mainnet, lambda row: continuous_prediction(decimal, row)
    )
    mainnet_integer = errors(
        mainnet, lambda row: integer_prediction(scaled, row)
    )

    artifact = {
        "schema_version": config["schema_version"],
        "model_id": config["model_id"],
        "originating_experiment_model": config["originating_experiment_model"],
        "proposal": {
            "provenance": proposal_config["provenance"],
            "generator_config_sha256": generator_config_sha256(config),
            "raw_input_rows_sha256": hashlib.sha256(
                fit_payload + validation_payload
            ).hexdigest(),
            "validation_fixture_sha256": hashlib.sha256(validation_payload).hexdigest(),
            "coefficients": {
                "decimal": {
                    name: format(value, "f") for name, value in decimal.items()
                },
                "scaled": scaled,
            },
            "max_total_zkgas": proposal_config["max_total_zkgas"],
            "cohorts": {
                "hoodi": {
                    "fit_count": cohort_config["hoodi"]["fit_count"],
                    "calibration_count": cohort_config["hoodi"]["calibration_count"],
                    "continuous": hoodi_diagnostics(
                        hoodi_continuous, hoodi_places["continuous"]
                    ),
                    "scaled_integer": hoodi_diagnostics(
                        hoodi_integer, hoodi_places["scaled_integer"]
                    ),
                },
                "mainnet": {
                    "evaluation_count": cohort_config["mainnet"]["evaluation_count"],
                    "influenced_model_selection": cohort_config["mainnet"][
                        "influenced_model_selection"
                    ],
                    "untouched_holdout": cohort_config["mainnet"][
                        "untouched_holdout"
                    ],
                    "continuous": mainnet_diagnostics(
                        mainnet_continuous, mainnet_places["continuous"]
                    ),
                    "scaled_integer": mainnet_diagnostics(
                        mainnet_integer, mainnet_places["scaled_integer"]
                    ),
                },
            },
        },
        "aggregation": config["aggregation"],
    }
    resolve_model_id(artifact, config["model_id"])
    return artifact


def resolve_paths(
    args: argparse.Namespace,
) -> tuple[pathlib.Path, pathlib.Path, pathlib.Path, pathlib.Path]:
    fixture_dir = args.fixture_dir.resolve()
    config = (args.config.resolve() if args.config else fixture_dir / "config.json")
    return fixture_dir, config, fixture_dir / "hoodi-fit.jsonl", fixture_dir / "validation.jsonl"


def check(args: argparse.Namespace) -> int:
    _, config_path, fit_path, validation_path = resolve_paths(args)
    config = read_json(config_path)
    fit_payload = fit_path.read_bytes()
    fit_rows = project_rows(read_jsonl(fit_path), "fit fixture")
    validation_payload = validation_path.read_bytes()
    validation_rows = project_rows(read_jsonl(validation_path), "validation fixture")
    expected = artifact_bytes(
        generate_artifact(
            config, fit_rows, validation_rows, fit_payload, validation_payload
        )
    )
    try:
        actual = args.model.read_bytes()
    except OSError as error:
        raise ModelError(f"cannot read runtime model {args.model}: {error}") from error
    if actual != expected:
        raise ModelError(
            "runtime model differs from generated artifact; "
            "run update-risc0-zkgas-model"
        )
    print("runtime model matches generated artifact")
    return 0


def reject_model_identity_reuse(
    candidate: Mapping[str, Any], paths: Iterable[pathlib.Path]
) -> None:
    seen: set[pathlib.Path] = set()
    for path in paths:
        resolved = path.resolve()
        if resolved in seen or not resolved.exists():
            continue
        seen.add(resolved)
        existing = read_json(resolved)
        if (
            existing.get("model_id") == candidate["model_id"]
            and existing != candidate
        ):
            raise ModelError(
                f"model_id {candidate['model_id']} already identifies different "
                f"model bytes in {resolved}; choose a new model_id and fixture directory"
            )


def existing_fixture_artifact(fixture_dir: pathlib.Path) -> Mapping[str, Any] | None:
    config_path = fixture_dir / "config.json"
    fit_path = fixture_dir / "hoodi-fit.jsonl"
    validation_path = fixture_dir / "validation.jsonl"
    data_paths = (fit_path, validation_path)
    if not any(path.exists() for path in data_paths):
        return None
    if not config_path.exists():
        raise ModelError(
            f"existing fixture data requires config.json in {fixture_dir}"
        )
    if not all(path.exists() for path in data_paths):
        return None
    config = read_json(config_path)
    fit_rows = project_rows(read_jsonl(fit_path), "existing fit fixture")
    validation_rows = project_rows(
        read_jsonl(validation_path), "existing validation fixture"
    )
    fit_payload = jsonl_bytes(fit_rows)
    validation_payload = jsonl_bytes(validation_rows)
    return generate_artifact(
        config, fit_rows, validation_rows, fit_payload, validation_payload
    )


def atomic_write(path: pathlib.Path, payload: bytes) -> None:
    descriptor, temporary_name = tempfile.mkstemp(
        dir=path.parent, prefix=f".{path.name}.", suffix=".tmp"
    )
    os.close(descriptor)
    temporary_path = pathlib.Path(temporary_name)
    try:
        temporary_path.write_bytes(payload)
        temporary_path.chmod(0o644)
        os.replace(temporary_path, path)
    finally:
        temporary_path.unlink(missing_ok=True)


def update(args: argparse.Namespace) -> int:
    fixture_dir, config_path, _, _ = resolve_paths(args)
    config = read_json(config_path)
    validate_config(config)
    hoodi_raw_rows = read_jsonl(args.hoodi_samples)
    mainnet_raw_rows = read_jsonl(args.mainnet_samples)
    hoodi_cohort = validate_collector_cohort(
        hoodi_raw_rows, "Hoodi collector", "taiko_hoodi", config
    )
    mainnet_cohort = validate_collector_cohort(
        mainnet_raw_rows, "Mainnet collector", "taiko_mainnet", config
    )
    validate_shared_collector_artifacts(hoodi_cohort, mainnet_cohort)
    hoodi_rows = project_rows(hoodi_raw_rows, "Hoodi collector")
    mainnet_rows = project_rows(
        mainnet_raw_rows,
        "Mainnet collector",
        {"holdout": "evaluation"},
    )
    if any(row["network"] != "taiko_hoodi" for row in hoodi_rows):
        raise ModelError("Hoodi collector contains a non-Hoodi observation")
    if any(
        row["network"] != "taiko_mainnet" or row["split"] != "evaluation"
        for row in mainnet_rows
    ):
        raise ModelError("Mainnet collector must contain only Mainnet holdout observations")
    fit_rows = [row for row in hoodi_rows if row["split"] == "fit"]
    calibration = [row for row in hoodi_rows if row["split"] == "calibration"]
    validation_rows = [*calibration, *mainnet_rows]
    fit_payload = jsonl_bytes(fit_rows)
    validation_payload = jsonl_bytes(validation_rows)
    artifact = generate_artifact(
        config, fit_rows, validation_rows, fit_payload, validation_payload
    )
    artifact_payload = artifact_bytes(artifact)
    output_config = copy.deepcopy(config)
    output_config["model_id"] = artifact["model_id"]
    config_payload = (
        artifact_bytes(output_config)
        if config["model_id"] == AUTO_MODEL_ID
        else config_path.read_bytes()
    )
    reject_model_identity_reuse(artifact, (DEFAULT_MODEL, args.model))
    existing_fixture = existing_fixture_artifact(fixture_dir)
    if (
        existing_fixture is not None
        and existing_fixture.get("model_id") == artifact["model_id"]
        and existing_fixture != artifact
    ):
        raise ModelError(
            f"model_id {artifact['model_id']} already identifies different model bytes "
            f"in {fixture_dir}; choose a new model_id and fixture directory"
        )
    existing_config_path = fixture_dir / "config.json"
    if existing_config_path.exists():
        existing_config = read_json(existing_config_path)
        existing_model_id = existing_config.get("model_id")
        auto_template = existing_model_id == AUTO_MODEL_ID and existing_fixture is None
        if existing_model_id != output_config["model_id"] and not auto_template:
            raise ModelError(
                "fixture directory already belongs to a different model_id"
            )
    fixture_dir.mkdir(parents=True, exist_ok=True)
    atomic_write(fixture_dir / "config.json", config_payload)
    atomic_write(fixture_dir / "hoodi-fit.jsonl", fit_payload)
    atomic_write(fixture_dir / "validation.jsonl", validation_payload)
    args.model.parent.mkdir(parents=True, exist_ok=True)
    atomic_write(args.model, artifact_payload)
    print(f"updated {args.model}")
    return 0


def add_common(parser: argparse.ArgumentParser, *, update: bool = False) -> None:
    parser.add_argument(
        "--fixture-dir",
        type=pathlib.Path,
        required=update,
        default=None if update else DEFAULT_FIXTURE_DIR,
    )
    parser.add_argument("--config", type=pathlib.Path, required=update)
    parser.add_argument("--model", type=pathlib.Path, default=DEFAULT_MODEL)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    check_parser = subparsers.add_parser(
        "check", help="verify committed fixture and runtime model bytes"
    )
    add_common(check_parser)
    check_parser.set_defaults(func=check)
    update_parser = subparsers.add_parser(
        "update", help="project collector rows and regenerate the runtime model"
    )
    add_common(update_parser, update=True)
    update_parser.add_argument("--hoodi-samples", type=pathlib.Path, required=True)
    update_parser.add_argument("--mainnet-samples", type=pathlib.Path, required=True)
    update_parser.set_defaults(func=update)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except (KeyError, TypeError, OSError, ModelError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
