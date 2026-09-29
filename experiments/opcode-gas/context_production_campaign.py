#!/usr/bin/env python3
"""Frozen contract for production-guest context-opcode calibration."""

from __future__ import annotations

import hashlib
import json
import pathlib
import re
import stat
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Mapping

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
        "model_selection_order": list(MODEL_SELECTION_ORDER),
        "model_candidates": json.loads(canonical_json(MODEL_CANDIDATES)),
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
    model_selection_order: tuple[str, ...]
    model_candidates: tuple[ProductionContextModelCandidate, ...]
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

    @classmethod
    def from_mapping(cls, value: Mapping[str, Any]) -> "ProductionContextManifest":
        if not isinstance(value, Mapping):
            raise ValueError("production context manifest must be an object")
        _reject_binary_floats(value)
        expected = canonical_production_context_manifest_payload()
        if canonical_json(value) != canonical_json(expected):
            raise ValueError("production context manifest differs from the frozen contract")

        for source_name, source in value["sources"].items():
            _require_relative_path(source["path"], f"sources.{source_name}.path")
            if "directory_identity_path" in source:
                _require_relative_path(source["directory_identity_path"], f"sources.{source_name}.directory_identity_path")
        for field in ("launcher_path", "production_elf_path", "production_vk_path", "trace_schema_source"):
            _require_relative_path(value["execution"][field], f"execution.{field}")

        candidates = tuple(ProductionContextModelCandidate.from_mapping(row) for row in value["model_candidates"])
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
            model_selection_order=tuple(value["model_selection_order"]),
            model_candidates=candidates,
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


def validate_production_context_sources(manifest: ProductionContextManifest, repo_root: pathlib.Path) -> dict[str, str]:
    validated: dict[str, str] = {}
    coverage_source = manifest.sources["operation_coverage_v5"]
    _, coverage = _load_pinned_json(repo_root, coverage_source, "operation_coverage_v5")
    _validate_content_address(coverage, coverage_source["artifact_sha256"], "operation_coverage_v5")
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
