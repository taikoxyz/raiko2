"""Regression coverage for the committed Boundless quote-model artifact."""

import hashlib
import json
import math
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
REPO_ROOT = ROOT.parents[1]
ARTIFACT_PATH = REPO_ROOT / "crates" / "prover" / "models" / "risc0-zkgas.json"
FIXTURE_PATH = (
    REPO_ROOT
    / "tests"
    / "fixtures"
    / "risc0-zkgas"
    / "2026-08-28-m2-v1"
    / "validation.jsonl"
)
FIXTURE_SHA256 = "dff36c84683011825a7372e43f846b678266f0f062515f44631922e9a7c47767"

EXPECTED = {
    "hoodi": {
        "continuous": {
            "underquote_count": 17,
            "mape_percent": "0.094557",
            "max_absolute_error_percent": "0.279512",
            "max_underquote_percent": "0.279512",
            "over_ten_percent_count": 0,
        },
        "scaled_integer": {
            "underquote_count": 12,
            "mape_percent": "0.093492",
            "max_absolute_error_percent": "0.264550",
            "max_underquote_percent": "0.264550",
            "over_ten_percent_count": 0,
        },
    },
    "mainnet": {
        "continuous": {
            "underquote_count": 19,
            "mape_percent": "5.87",
            "max_underquote_percent": "5.75",
            "overquote_over_ten_percent_count": 1,
            "max_overquote_percent": "21.94",
        },
        "scaled_integer": {
            "underquote_count": 19,
            "mape_percent": "5.8422",
            "max_underquote_percent": "5.7234",
            "overquote_over_ten_percent_count": 1,
            "max_overquote_percent": "21.9679",
        },
    },
}


def parse_audit_decimal(value, label):
    assert isinstance(value, str), f"{label} audit decimal must be a string"
    try:
        parsed = float(value)
    except ValueError as error:
        raise AssertionError(f"{label} audit decimal is malformed") from error
    assert math.isfinite(parsed), f"{label} audit decimal must be finite"
    return parsed


def load_fixture():
    raw = FIXTURE_PATH.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == FIXTURE_SHA256
    rows = [json.loads(line) for line in raw.decode().splitlines()]
    assert len(rows) == 60
    identities = set()
    schema = {
        "network",
        "split",
        "proposal_id",
        "block_count",
        "total_zkgas",
        "actual_mcycles",
    }
    for row in rows:
        assert set(row) == schema
        assert (row["network"], row["split"]) in {
            ("taiko_hoodi", "calibration"),
            ("taiko_mainnet", "evaluation"),
        }
        for field in ("proposal_id", "block_count", "total_zkgas", "actual_mcycles"):
            assert type(row[field]) is int and row[field] > 0
        identity = (row["network"], row["proposal_id"])
        assert identity not in identities
        identities.add(identity)
    assert sum(row["network"] == "taiko_hoodi" for row in rows) == 40
    assert sum(row["network"] == "taiko_mainnet" for row in rows) == 20
    return rows


def continuous_prediction(coefficients, row):
    return (
        coefficients["intercept"]
        + coefficients["total_zkgas"] * row["total_zkgas"]
        + coefficients["block_count"] * row["block_count"]
    )


def scaled_integer_prediction(coefficients, row):
    numerator = (
        coefficients["intercept"]
        + coefficients["total_zkgas"] * row["total_zkgas"]
        + coefficients["block_count"] * row["block_count"]
    )
    assert type(coefficients["scale"]) is int and coefficients["scale"] > 0
    return (numerator + coefficients["scale"] - 1) // coefficients["scale"]


def diagnostics(rows, predict):
    errors = [(predict(row) - row["actual_mcycles"]) * 100 / row["actual_mcycles"] for row in rows]
    return {
        "underquote_count": sum(error < 0 for error in errors),
        "mape_percent": sum(abs(error) for error in errors) / len(errors),
        "max_absolute_error_percent": max(abs(error) for error in errors),
        "max_underquote_percent": max([-error for error in errors if error < 0], default=0.0),
        "over_ten_percent_count": sum(abs(error) > 10 for error in errors),
        "overquote_over_ten_percent_count": sum(error > 10 for error in errors),
        "max_overquote_percent": max([error for error in errors if error > 0], default=0.0),
    }


def assert_diagnostics(actual, artifact, expected):
    assert artifact == expected
    for field, expected_value in artifact.items():
        if isinstance(expected_value, int):
            assert actual[field] == expected_value
            continue
        expected_float = parse_audit_decimal(expected_value, field)
        precision = len(expected_value.partition(".")[2])
        tolerance = 0.5 * 10 ** -precision + 1e-9
        assert abs(actual[field] - expected_float) <= tolerance


def assert_committed_fixture_reproduces_quote_diagnostics():
    artifact = json.loads(ARTIFACT_PATH.read_text())
    rows = load_fixture()
    proposal = artifact["proposal"]
    assert proposal["validation_fixture_sha256"] == FIXTURE_SHA256
    decimal = {
        key: parse_audit_decimal(value, key)
        for key, value in proposal["coefficients"]["decimal"].items()
    }
    scaled = proposal["coefficients"]["scaled"]
    hoodi = [row for row in rows if row["network"] == "taiko_hoodi"]
    mainnet = [row for row in rows if row["network"] == "taiko_mainnet"]

    assert_diagnostics(
        diagnostics(hoodi, lambda row: continuous_prediction(decimal, row)),
        proposal["cohorts"]["hoodi"]["continuous"],
        EXPECTED["hoodi"]["continuous"],
    )
    assert_diagnostics(
        diagnostics(hoodi, lambda row: scaled_integer_prediction(scaled, row)),
        proposal["cohorts"]["hoodi"]["scaled_integer"],
        EXPECTED["hoodi"]["scaled_integer"],
    )
    assert_diagnostics(
        diagnostics(mainnet, lambda row: continuous_prediction(decimal, row)),
        proposal["cohorts"]["mainnet"]["continuous"],
        EXPECTED["mainnet"]["continuous"],
    )
    assert_diagnostics(
        diagnostics(mainnet, lambda row: scaled_integer_prediction(scaled, row)),
        proposal["cohorts"]["mainnet"]["scaled_integer"],
        EXPECTED["mainnet"]["scaled_integer"],
    )


class CommittedFixtureTests(unittest.TestCase):
    def test_audit_decimal_accepts_zero(self):
        self.assertEqual(parse_audit_decimal("0.0", "zero"), 0.0)

    def test_committed_fixture_reproduces_quote_diagnostics(self):
        assert_committed_fixture_reproduces_quote_diagnostics()
