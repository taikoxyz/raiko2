"""Exact linear algebra for relative opcode calibration relations."""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Context, Decimal, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from types import MappingProxyType
from typing import Any, Iterable, Mapping, Sequence


_CALIBRATION_DECIMAL_CONTEXT = Context(
    prec=80, rounding=ROUND_HALF_EVEN, traps=[]
)
_ANCHOR_RAW_GAS = MappingProxyType(
    {
        "opcode:0x50": 2,
        "opcode:0x5f": 2,
        "opcode:0x80": 3,
        "opcode:0x90": 3,
    }
)
_TRANSFER_PARAMETER_KEYS = (
    "body_scale",
    "common_opcode_overhead_per_operation",
)
_OPCODE_FAMILY_ANCHORS = MappingProxyType(
    {
        "pop_family": "opcode:0x50",
        "push_family": "opcode:0x5f",
        "dup_family": "opcode:0x80",
        "swap_family": "opcode:0x90",
    }
)


@dataclass(frozen=True)
class RelationEquation:
    relation_id: str
    coefficients: Mapping[str, Fraction]
    slope: Decimal


@dataclass(frozen=True)
class NonnegativeLeastSquaresResult:
    solution: tuple[Decimal, ...]
    active_zero_indices: tuple[int, ...]
    residual: Decimal


@dataclass(frozen=True)
class NonnegativeOpcodeFitEvidence:
    status: str
    opcode_keys: tuple[str, ...]
    anchor_keys: tuple[str, ...]
    lab_body_per_raw_gas: Mapping[str, Decimal]
    active_zero_keys: tuple[str, ...]
    nonzero_relation_mape: Decimal
    nonzero_relation_max_ape: Decimal
    flat_relation_max_normalized_error: Decimal
    residual: Decimal
    predictions: Mapping[str, Mapping[str, Decimal | str]]
    quality_failures: tuple[str, ...]


@dataclass(frozen=True)
class AffineOpcodeModel:
    opcode_keys: tuple[str, ...]
    anchor_keys: tuple[str, ...]
    rank: int
    nullity: int
    mu_zero: Mapping[str, Decimal]
    anchor_basis: Mapping[str, Mapping[str, Fraction]]

    def reconstruct_multipliers(
        self, anchor_values: Mapping[str, Decimal]
    ) -> dict[str, Decimal]:
        expected = set(self.anchor_keys)
        actual = set(anchor_values)
        missing = expected - actual
        if missing:
            raise ValueError(f"missing anchor values: {sorted(missing)!r}")
        extra = actual - expected
        if extra:
            raise ValueError(f"unexpected anchor values: {sorted(extra)!r}")
        for anchor_key in self.anchor_keys:
            value = anchor_values[anchor_key]
            if not isinstance(value, Decimal) or not value.is_finite():
                raise ValueError("anchor value must be a finite Decimal")

        with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
            values = dict(self.mu_zero)
            for opcode_key in self.opcode_keys:
                for anchor_key in self.anchor_keys:
                    coefficient = self.anchor_basis[opcode_key][anchor_key]
                    values[opcode_key] += (
                        Decimal(coefficient.numerator)
                        / Decimal(coefficient.denominator)
                        * anchor_values[anchor_key]
                    )
            return values


def reconstruct_lab_multipliers(
    affine_model: AffineOpcodeModel,
    anchor_body_costs: Mapping[str, Decimal],
) -> dict[str, Decimal]:
    """Reconstruct lab cost per raw-gas unit from per-operation anchor costs."""
    expected = set(affine_model.anchor_keys)
    actual = set(anchor_body_costs)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ValueError(
            "anchor body costs differ from the affine model: "
            f"missing={missing!r}, extra={extra!r}"
        )
    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        anchor_multipliers = {}
        for key in affine_model.anchor_keys:
            if key not in _ANCHOR_RAW_GAS:
                raise ValueError(f"anchor raw gas is not frozen: {key}")
            value = anchor_body_costs[key]
            if (
                not isinstance(value, Decimal)
                or not value.is_finite()
                or value <= 0
            ):
                raise ValueError(
                    f"anchor body cost must be a positive finite Decimal: {key}"
                )
            anchor_multipliers[key] = value / Decimal(_ANCHOR_RAW_GAS[key])
        return affine_model.reconstruct_multipliers(anchor_multipliers)


@dataclass(frozen=True)
class BlockCalibrationRow:
    row_id: str
    workload_family: str
    split: str
    prover_gas: Decimal
    raw_gas_by_key: Mapping[str, int]
    feature_counts: Mapping[str, int]
    workload_count: int | None


@dataclass(frozen=True)
class BlockCalibrationResult:
    transfer_params: Mapping[str, Decimal]
    reconstructed_anchors: Mapping[str, Decimal]
    fixed_costs: Mapping[str, Decimal]
    opcode_multipliers: Mapping[str, Decimal]
    fit_mape: Decimal
    fit_max_ape: Decimal
    holdout_max_ape: Decimal
    status: str
    parameter_order: tuple[str, ...]
    transfer_exact_design_matrix: tuple[tuple[Fraction, ...], ...]
    transfer_exact_rank: int
    transfer_column_scales: tuple[Decimal, ...]
    transfer_solver_residual: Decimal
    fixed_exact_design_matrix: tuple[tuple[Fraction, ...], ...]
    fixed_exact_rank: int
    fixed_column_scales: tuple[Decimal, ...]
    fixed_solver_residual: Decimal
    family_slope_evidence: Mapping[str, Mapping[str, Decimal | str]]
    opcode_holdout_evidence: Mapping[str, Mapping[str, Decimal | int | str]]
    transfer_leave_one_family_out: Mapping[str, Mapping[str, Decimal]]
    predictions: Mapping[str, Mapping[str, Decimal | str]]


@dataclass(frozen=True)
class DynamicRelationObservation:
    dynamic_key: str
    scenario_id: str
    split: str
    equation: RelationEquation


@dataclass(frozen=True)
class DynamicOpcodeObservation:
    dynamic_key: str
    scenario_id: str
    model_split: str
    features: Mapping[str, Fraction]
    target_body_cost: Decimal


@dataclass(frozen=True)
class DynamicOpcodeFitEvidence:
    dynamic_key: str
    status: str
    feature_names: tuple[str, ...]
    exact_rank: int
    parameter_count: int
    observation_count: int
    fit_count: int
    holdout_count: int
    body_coefficients: Mapping[str, Decimal]
    production_coefficients: Mapping[str, Decimal]
    fit_body_mape: Decimal
    fit_body_max_ape: Decimal
    holdout_body_max_ape: Decimal
    fit_production_mape: Decimal
    fit_production_max_ape: Decimal
    holdout_production_max_ape: Decimal
    quality_failures: tuple[str, ...]
    exact_fit_design_matrix: tuple[tuple[Fraction, ...], ...]
    solver_column_scales: tuple[Decimal, ...]
    solver_residual: Decimal
    predictions: Mapping[str, Mapping[str, Decimal | str]]


@dataclass(frozen=True)
class SharedMemoryFitEvidence:
    status: str
    parameter_order: tuple[str, ...]
    exact_rank: int
    parameter_count: int
    observation_count: int
    fit_count: int
    holdout_count: int
    body_coefficients: Mapping[str, Decimal]
    production_coefficients: Mapping[str, Decimal]
    fit_body_mape: Decimal
    fit_body_max_ape: Decimal
    holdout_body_max_ape: Decimal
    fit_production_mape: Decimal
    fit_production_max_ape: Decimal
    holdout_production_max_ape: Decimal
    quality_failures: tuple[str, ...]
    exact_fit_design_matrix: tuple[tuple[Fraction, ...], ...]
    solver_column_scales: tuple[Decimal, ...]
    solver_residual: Decimal
    predictions: Mapping[str, Mapping[str, Decimal | str]]


@dataclass(frozen=True)
class StructuredDynamicOpcodeFitResult:
    status: str
    shared_memory_model: SharedMemoryFitEvidence
    opcode_models: Mapping[str, DynamicOpcodeFitEvidence]
    aggregate_exact_rank: int
    aggregate_parameter_count: int


_STRUCTURED_DYNAMIC_KEYS = (
    "opcode:0x0a",
    "opcode:0x20",
    "opcode:0x51",
    "opcode:0x52",
    "opcode:0x53",
    "opcode:0x5e",
)
_MEMORY_OPCODE_KEYS = ("opcode:0x51", "opcode:0x52", "opcode:0x53")
_MEMORY_FEATURE_NAMES = (
    "memory_growth_event",
    "memory_evm_gas_delta",
    "memory_4k_boundary_event",
)
_SHARED_MEMORY_PARAMETER_ORDER = (
    "opcode:0x51:constant",
    "opcode:0x52:constant",
    "opcode:0x53:constant",
    *_MEMORY_FEATURE_NAMES,
)
_STRUCTURED_FEATURE_NAMES = MappingProxyType(
    {
        "opcode:0x0a": (
            "constant",
            "exponent_bytes",
            "exponent_bytes_squared",
        ),
        "opcode:0x20": (
            "constant",
            "keccak_zero_length_event",
            "keccak_permutations",
            *_MEMORY_FEATURE_NAMES,
        ),
        "opcode:0x51": ("constant", *_MEMORY_FEATURE_NAMES),
        "opcode:0x52": ("constant", *_MEMORY_FEATURE_NAMES),
        "opcode:0x53": ("constant", *_MEMORY_FEATURE_NAMES),
        "opcode:0x5e": ("constant", "copy_words", *_MEMORY_FEATURE_NAMES),
    }
)


def _dynamic_quality_failures(
    fit_mape: Decimal, fit_max_ape: Decimal, holdout_max_ape: Decimal
) -> tuple[str, ...]:
    return tuple(
        name
        for name, failed in (
            ("fit_mape", fit_mape > Decimal("0.05")),
            ("fit_max_ape", fit_max_ape > Decimal("0.10")),
            ("holdout_max_ape", holdout_max_ape > Decimal("0.10")),
        )
        if failed
    )


def _dynamic_prediction_evidence(
    observations: Sequence[DynamicOpcodeObservation],
    predicted_body_costs: Sequence[Decimal],
    *,
    body_scale: Decimal,
    common_overhead: Decimal,
    prediction_ids: Sequence[str],
    extras: Sequence[Mapping[str, Decimal]] | None = None,
) -> tuple[
    Mapping[str, Mapping[str, Decimal | str]],
    Decimal,
    Decimal,
    Decimal,
    Decimal,
    Decimal,
    Decimal,
]:
    if not (
        len(observations) == len(predicted_body_costs) == len(prediction_ids)
    ):
        raise ValueError("dynamic prediction inputs differ in length")
    extra_rows = extras or tuple({} for _ in observations)
    if len(extra_rows) != len(observations):
        raise ValueError("dynamic prediction extras differ in length")
    predictions: dict[str, Mapping[str, Decimal | str]] = {}
    fit_body_apes = []
    holdout_body_apes = []
    fit_production_apes = []
    holdout_production_apes = []
    aggregate_rows = []
    for observation, predicted_body, prediction_id, extra in zip(
        observations, predicted_body_costs, prediction_ids, extra_rows
    ):
        actual_body = observation.target_body_cost
        body_ape = abs(predicted_body - actual_body) / actual_body
        actual_production = actual_body * body_scale + common_overhead
        predicted_production = predicted_body * body_scale + common_overhead
        production_ape = (
            abs(predicted_production - actual_production) / actual_production
        )
        values = (
            predicted_body,
            body_ape,
            actual_production,
            predicted_production,
            production_ape,
            *extra.values(),
        )
        if any(not value.is_finite() for value in values):
            raise ValueError(
                "dynamic prediction produced a nonfinite value: "
                f"{observation.dynamic_key}/{observation.scenario_id}"
            )
        predictions[prediction_id] = MappingProxyType(
            {
                "model_split": observation.model_split,
                "actual_body_cost": actual_body,
                "predicted_body_cost": predicted_body,
                "body_ape": body_ape,
                "actual_production_cost": actual_production,
                "predicted_production_cost": predicted_production,
                "production_ape": production_ape,
                **extra,
            }
        )
        aggregate_rows.append(
            (prediction_id, observation.model_split, body_ape, production_ape)
        )
    for _, model_split, body_ape, production_ape in sorted(aggregate_rows):
        if model_split == "fit":
            fit_body_apes.append(body_ape)
            fit_production_apes.append(production_ape)
        else:
            holdout_body_apes.append(body_ape)
            holdout_production_apes.append(production_ape)
    if not fit_body_apes or not holdout_body_apes:
        raise ValueError("dynamic model requires fit and holdout observations")
    return (
        MappingProxyType(predictions),
        sum(fit_body_apes, Decimal(0)) / Decimal(len(fit_body_apes)),
        max(fit_body_apes),
        max(holdout_body_apes),
        sum(fit_production_apes, Decimal(0))
        / Decimal(len(fit_production_apes)),
        max(fit_production_apes),
        max(holdout_production_apes),
    )


def fit_structured_dynamic_opcode_models(
    observations: Sequence[DynamicOpcodeObservation],
    *,
    body_scale: Decimal,
    common_overhead: Decimal,
) -> StructuredDynamicOpcodeFitResult:
    """Fit EXP plus a shared memory family used by KECCAK256 and MCOPY."""
    if (
        not isinstance(body_scale, Decimal)
        or not body_scale.is_finite()
        or body_scale <= 0
    ):
        raise ValueError("body_scale must be a positive finite Decimal")
    if (
        not isinstance(common_overhead, Decimal)
        or not common_overhead.is_finite()
        or common_overhead < 0
    ):
        raise ValueError("common_overhead must be a nonnegative finite Decimal")
    if any(not isinstance(row, DynamicOpcodeObservation) for row in observations):
        raise ValueError("dynamic observation has an invalid type")
    observed_keys = {row.dynamic_key for row in observations}
    if observed_keys != set(_STRUCTURED_DYNAMIC_KEYS):
        raise ValueError("structured dynamic keys differ from the frozen model")
    for key in _STRUCTURED_DYNAMIC_KEYS:
        key_rows = [row for row in observations if row.dynamic_key == key]
        _validate_unique((row.scenario_id for row in key_rows), "dynamic scenario ID")
        if not any(row.model_split == "fit" for row in key_rows) or not any(
            row.model_split == "holdout" for row in key_rows
        ):
            raise ValueError(f"structured dynamic key {key} requires fit and holdout rows")
        for row in key_rows:
            if row.model_split not in {"fit", "holdout"}:
                raise ValueError("dynamic model_split must be fit or holdout")
            if set(row.features) != set(_STRUCTURED_FEATURE_NAMES[key]):
                raise ValueError(
                    f"structured dynamic feature set differs for {key}/{row.scenario_id}"
                )
            if any(not isinstance(value, Fraction) for value in row.features.values()):
                raise ValueError("dynamic feature values must be Fractions")
            if row.features["constant"] != Fraction(1):
                raise ValueError("dynamic constant feature must be Fraction(1)")
            if (
                not isinstance(row.target_body_cost, Decimal)
                or not row.target_body_cost.is_finite()
                or row.target_body_cost <= 0
            ):
                raise ValueError("dynamic target body cost must be positive and finite")

    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        memory_rows = tuple(
            row for row in observations if row.dynamic_key in _MEMORY_OPCODE_KEYS
        )
        memory_fit_rows = tuple(
            row for row in memory_rows if row.model_split == "fit"
        )
        memory_matrix = []
        for row in memory_fit_rows:
            memory_matrix.append(
                [
                    Fraction(int(row.dynamic_key == key))
                    for key in _MEMORY_OPCODE_KEYS
                ]
                + [row.features[name] for name in _MEMORY_FEATURE_NAMES]
            )
        memory_rank = exact_rank(memory_matrix)
        if memory_rank != len(_SHARED_MEMORY_PARAMETER_ORDER):
            raise ValueError(
                "shared memory fit matrix exact rank must equal parameter count "
                f"{len(_SHARED_MEMORY_PARAMETER_ORDER)}, got {memory_rank}"
            )
        memory_values, memory_scales, memory_residual = (
            _scaled_decimal_least_squares(
                memory_matrix,
                [row.target_body_cost for row in memory_fit_rows],
            )
        )
        memory_body = dict(zip(_SHARED_MEMORY_PARAMETER_ORDER, memory_values))
        memory_production = {
            name: value * body_scale
            + (common_overhead if name.endswith(":constant") else Decimal(0))
            for name, value in memory_body.items()
        }

        def shared_memory_cost(row: DynamicOpcodeObservation) -> Decimal:
            return sum(
                (
                    _decimal_from_fraction(row.features[name])
                    * memory_body[name]
                    for name in _MEMORY_FEATURE_NAMES
                ),
                Decimal(0),
            )

        memory_predictions = [
            memory_body[f"{row.dynamic_key}:constant"] + shared_memory_cost(row)
            for row in memory_rows
        ]
        (
            memory_prediction_rows,
            memory_fit_body_mape,
            memory_fit_body_max,
            memory_holdout_body_max,
            memory_fit_production_mape,
            memory_fit_production_max,
            memory_holdout_production_max,
        ) = _dynamic_prediction_evidence(
            memory_rows,
            memory_predictions,
            body_scale=body_scale,
            common_overhead=common_overhead,
            prediction_ids=tuple(
                f"{row.dynamic_key}/{row.scenario_id}" for row in memory_rows
            ),
        )
        memory_failures = _dynamic_quality_failures(
            memory_fit_production_mape,
            memory_fit_production_max,
            memory_holdout_production_max,
        )
        shared_memory_model = SharedMemoryFitEvidence(
            status="not_supported" if memory_failures else "supported",
            parameter_order=_SHARED_MEMORY_PARAMETER_ORDER,
            exact_rank=memory_rank,
            parameter_count=len(_SHARED_MEMORY_PARAMETER_ORDER),
            observation_count=len(memory_rows),
            fit_count=len(memory_fit_rows),
            holdout_count=len(memory_rows) - len(memory_fit_rows),
            body_coefficients=MappingProxyType(memory_body),
            production_coefficients=MappingProxyType(memory_production),
            fit_body_mape=memory_fit_body_mape,
            fit_body_max_ape=memory_fit_body_max,
            holdout_body_max_ape=memory_holdout_body_max,
            fit_production_mape=memory_fit_production_mape,
            fit_production_max_ape=memory_fit_production_max,
            holdout_production_max_ape=memory_holdout_production_max,
            quality_failures=memory_failures,
            exact_fit_design_matrix=tuple(tuple(row) for row in memory_matrix),
            solver_column_scales=tuple(memory_scales),
            solver_residual=memory_residual,
            predictions=memory_prediction_rows,
        )

        opcode_models: dict[str, DynamicOpcodeFitEvidence] = {}
        for key, feature_names in (
            (
                "opcode:0x0a",
                ("constant", "exponent_bytes", "exponent_bytes_squared"),
            ),
            (
                "opcode:0x20",
                (
                    "constant",
                    "keccak_zero_length_event",
                    "keccak_permutations",
                ),
            ),
            ("opcode:0x5e", ("constant", "copy_words")),
        ):
            key_rows = tuple(row for row in observations if row.dynamic_key == key)
            fit_rows = tuple(row for row in key_rows if row.model_split == "fit")
            matrix = [
                [row.features[name] for name in feature_names] for row in fit_rows
            ]
            rank = exact_rank(matrix)
            if rank != len(feature_names):
                raise ValueError(
                    f"dynamic fit matrix exact rank for {key} must equal "
                    f"parameter count {len(feature_names)}, got {rank}"
                )
            fit_targets = [
                row.target_body_cost
                - (Decimal(0) if key == "opcode:0x0a" else shared_memory_cost(row))
                for row in fit_rows
            ]
            if any(not value.is_finite() for value in fit_targets):
                raise ValueError(
                    f"dynamic operation-specific body cost must be finite: {key}"
                )
            operation_target_failures = (
                ("nonpositive_operation_fit_target",)
                if any(value <= 0 for value in fit_targets)
                else ()
            )
            values, scales, residual = _scaled_decimal_least_squares(
                matrix, fit_targets
            )
            body_coefficients = dict(zip(feature_names, values))
            production_coefficients = {
                name: value * body_scale
                + (common_overhead if name == "constant" else Decimal(0))
                for name, value in body_coefficients.items()
            }
            operation_predictions = [
                sum(
                    (
                        _decimal_from_fraction(row.features[name])
                        * body_coefficients[name]
                        for name in feature_names
                    ),
                    Decimal(0),
                )
                for row in key_rows
            ]
            shared_costs = [
                Decimal(0) if key == "opcode:0x0a" else shared_memory_cost(row)
                for row in key_rows
            ]
            total_predictions = [
                operation + shared
                for operation, shared in zip(operation_predictions, shared_costs)
            ]
            (
                predictions,
                fit_body_mape,
                fit_body_max,
                holdout_body_max,
                fit_production_mape,
                fit_production_max,
                holdout_production_max,
            ) = _dynamic_prediction_evidence(
                key_rows,
                total_predictions,
                body_scale=body_scale,
                common_overhead=common_overhead,
                prediction_ids=tuple(row.scenario_id for row in key_rows),
                extras=tuple(
                    {
                        "predicted_operation_body_cost": operation,
                        "shared_memory_body_cost": shared,
                    }
                    for operation, shared in zip(
                        operation_predictions, shared_costs
                    )
                ),
            )
            failures = tuple(
                dict.fromkeys(
                    (
                        *operation_target_failures,
                        *_dynamic_quality_failures(
                            fit_production_mape,
                            fit_production_max,
                            holdout_production_max,
                        ),
                    )
                )
            )
            opcode_models[key] = DynamicOpcodeFitEvidence(
                dynamic_key=key,
                status="not_supported" if failures else "supported",
                feature_names=feature_names,
                exact_rank=rank,
                parameter_count=len(feature_names),
                observation_count=len(key_rows),
                fit_count=len(fit_rows),
                holdout_count=len(key_rows) - len(fit_rows),
                body_coefficients=MappingProxyType(body_coefficients),
                production_coefficients=MappingProxyType(production_coefficients),
                fit_body_mape=fit_body_mape,
                fit_body_max_ape=fit_body_max,
                holdout_body_max_ape=holdout_body_max,
                fit_production_mape=fit_production_mape,
                fit_production_max_ape=fit_production_max,
                holdout_production_max_ape=holdout_production_max,
                quality_failures=failures,
                exact_fit_design_matrix=tuple(tuple(row) for row in matrix),
                solver_column_scales=tuple(scales),
                solver_residual=residual,
                predictions=predictions,
            )
        aggregate_rank = memory_rank + sum(
            model.exact_rank for model in opcode_models.values()
        )
        aggregate_parameters = len(_SHARED_MEMORY_PARAMETER_ORDER) + sum(
            model.parameter_count for model in opcode_models.values()
        )
        all_models = (shared_memory_model, *opcode_models.values())
        return StructuredDynamicOpcodeFitResult(
            status=(
                "supported"
                if all(model.status == "supported" for model in all_models)
                else "not_supported"
            ),
            shared_memory_model=shared_memory_model,
            opcode_models=MappingProxyType(opcode_models),
            aggregate_exact_rank=aggregate_rank,
            aggregate_parameter_count=aggregate_parameters,
        )


def fit_dynamic_opcode_models(
    observations: Sequence[DynamicOpcodeObservation],
    feature_names_by_key: Mapping[str, tuple[str, ...]],
    *,
    body_scale: Decimal,
    common_overhead: Decimal,
) -> Mapping[str, DynamicOpcodeFitEvidence]:
    """Fit independent dynamic-opcode body-cost models with frozen holdouts."""
    if (
        not isinstance(body_scale, Decimal)
        or not body_scale.is_finite()
        or body_scale <= 0
    ):
        raise ValueError("body_scale must be a positive finite Decimal")
    if (
        not isinstance(common_overhead, Decimal)
        or not common_overhead.is_finite()
        or common_overhead < 0
    ):
        raise ValueError("common_overhead must be a nonnegative finite Decimal")
    if not feature_names_by_key:
        raise ValueError("dynamic feature names must not be empty")

    for dynamic_key in feature_names_by_key:
        if not isinstance(dynamic_key, str) or not dynamic_key:
            raise ValueError("dynamic key must be a nonempty string")
    configured_keys = set(feature_names_by_key)
    observed_keys = set()
    for observation in observations:
        if not isinstance(observation, DynamicOpcodeObservation):
            raise ValueError("dynamic observation has an invalid type")
        if not isinstance(observation.dynamic_key, str) or not observation.dynamic_key:
            raise ValueError("dynamic key must be a nonempty string")
        if not isinstance(observation.scenario_id, str) or not observation.scenario_id:
            raise ValueError("dynamic scenario ID must be a nonempty string")
        observed_keys.add(observation.dynamic_key)
    if configured_keys != observed_keys:
        raise ValueError(
            "observed and configured dynamic keys differ: "
            f"observed={sorted(observed_keys)!r}, "
            f"configured={sorted(configured_keys)!r}"
        )

    evidence: dict[str, DynamicOpcodeFitEvidence] = {}
    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        for dynamic_key, feature_names in feature_names_by_key.items():
            if not isinstance(feature_names, tuple) or not feature_names:
                raise ValueError(
                    f"dynamic feature names for {dynamic_key} must be a nonempty tuple"
                )
            if any(not isinstance(name, str) or not name for name in feature_names):
                raise ValueError("dynamic feature name must be a nonempty string")
            _validate_unique(feature_names, "feature name")
            if "constant" not in feature_names:
                raise ValueError(
                    f"dynamic feature names for {dynamic_key} must include constant"
                )

            key_observations = tuple(
                observation
                for observation in observations
                if observation.dynamic_key == dynamic_key
            )
            _validate_unique(
                (observation.scenario_id for observation in key_observations),
                "dynamic scenario ID",
            )
            fit_rows = tuple(
                observation
                for observation in key_observations
                if observation.model_split == "fit"
            )
            holdout_rows = tuple(
                observation
                for observation in key_observations
                if observation.model_split == "holdout"
            )
            if any(
                observation.model_split not in {"fit", "holdout"}
                for observation in key_observations
            ):
                raise ValueError("dynamic model_split must be fit or holdout")
            if not holdout_rows:
                raise ValueError(
                    f"dynamic key {dynamic_key} requires at least one holdout"
                )

            expected_features = set(feature_names)
            for observation in key_observations:
                if set(observation.features) != expected_features:
                    raise ValueError(
                        "dynamic observation feature set differs from frozen "
                        "feature names: "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )
                for value in observation.features.values():
                    if not isinstance(value, Fraction):
                        raise ValueError("dynamic feature values must be Fractions")
                if observation.features["constant"] != Fraction(1):
                    raise ValueError(
                        "dynamic constant feature must be exactly Fraction(1): "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )
                target = observation.target_body_cost
                if (
                    not isinstance(target, Decimal)
                    or not target.is_finite()
                    or target <= 0
                ):
                    raise ValueError(
                        "dynamic target body cost must be a positive finite Decimal: "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )

            fit_matrix = [
                [observation.features[name] for name in feature_names]
                for observation in fit_rows
            ]
            fit_rank = exact_rank(fit_matrix)
            parameter_count = len(feature_names)
            if fit_rank != parameter_count:
                raise ValueError(
                    f"dynamic fit matrix exact rank for {dynamic_key} must equal "
                    f"parameter count {parameter_count}, got {fit_rank}"
                )
            body_values, column_scales, solver_residual = (
                _scaled_decimal_least_squares(
                    fit_matrix,
                    [observation.target_body_cost for observation in fit_rows],
                )
            )
            if any(not value.is_finite() for value in body_values):
                raise ValueError(
                    f"dynamic body fit produced a nonfinite coefficient: {dynamic_key}"
                )
            body_coefficients = dict(zip(feature_names, body_values))
            production_coefficients = {
                name: (
                    value * body_scale
                    + (common_overhead if name == "constant" else Decimal(0))
                )
                for name, value in body_coefficients.items()
            }
            if any(
                not value.is_finite() for value in production_coefficients.values()
            ):
                raise ValueError(
                    "dynamic production conversion produced a nonfinite "
                    f"coefficient: {dynamic_key}"
                )

            predictions: dict[str, dict[str, Decimal | str]] = {}
            fit_body_apes: list[Decimal] = []
            holdout_body_apes: list[Decimal] = []
            fit_production_apes: list[Decimal] = []
            holdout_production_apes: list[Decimal] = []
            for observation in key_observations:
                predicted_body = sum(
                    (
                        _decimal_from_fraction(observation.features[name])
                        * body_coefficients[name]
                        for name in feature_names
                    ),
                    Decimal(0),
                )
                actual_body = observation.target_body_cost
                body_ape = abs(predicted_body - actual_body) / actual_body
                actual_production = actual_body * body_scale + common_overhead
                predicted_production = sum(
                    (
                        _decimal_from_fraction(observation.features[name])
                        * production_coefficients[name]
                        for name in feature_names
                    ),
                    Decimal(0),
                )
                production_ape = (
                    abs(predicted_production - actual_production)
                    / actual_production
                )
                if not all(
                    value.is_finite()
                    for value in (
                        predicted_body,
                        body_ape,
                        actual_production,
                        predicted_production,
                        production_ape,
                    )
                ):
                    raise ValueError(
                        "dynamic prediction produced a nonfinite value: "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )
                predictions[observation.scenario_id] = {
                    "model_split": observation.model_split,
                    "actual_body_cost": actual_body,
                    "predicted_body_cost": predicted_body,
                    "body_ape": body_ape,
                    "actual_production_cost": actual_production,
                    "predicted_production_cost": predicted_production,
                    "production_ape": production_ape,
                }
                if observation.model_split == "fit":
                    fit_body_apes.append(body_ape)
                    fit_production_apes.append(production_ape)
                else:
                    holdout_body_apes.append(body_ape)
                    holdout_production_apes.append(production_ape)

            fit_body_mape = sum(fit_body_apes, Decimal(0)) / Decimal(
                len(fit_body_apes)
            )
            fit_body_max_ape = max(fit_body_apes)
            holdout_body_max_ape = max(holdout_body_apes)
            fit_production_mape = sum(
                fit_production_apes, Decimal(0)
            ) / Decimal(len(fit_production_apes))
            fit_production_max_ape = max(fit_production_apes)
            holdout_production_max_ape = max(holdout_production_apes)
            quality_failures = tuple(
                name
                for name, failed in (
                    ("fit_mape", fit_production_mape > Decimal("0.05")),
                    ("fit_max_ape", fit_production_max_ape > Decimal("0.10")),
                    (
                        "holdout_max_ape",
                        holdout_production_max_ape > Decimal("0.10"),
                    ),
                )
                if failed
            )
            evidence[dynamic_key] = DynamicOpcodeFitEvidence(
                dynamic_key=dynamic_key,
                status="not_supported" if quality_failures else "supported",
                feature_names=feature_names,
                exact_rank=fit_rank,
                parameter_count=parameter_count,
                observation_count=len(key_observations),
                fit_count=len(fit_rows),
                holdout_count=len(holdout_rows),
                body_coefficients=MappingProxyType(body_coefficients),
                production_coefficients=MappingProxyType(
                    production_coefficients
                ),
                fit_body_mape=fit_body_mape,
                fit_body_max_ape=fit_body_max_ape,
                holdout_body_max_ape=holdout_body_max_ape,
                fit_production_mape=fit_production_mape,
                fit_production_max_ape=fit_production_max_ape,
                holdout_production_max_ape=holdout_production_max_ape,
                quality_failures=quality_failures,
                exact_fit_design_matrix=tuple(
                    tuple(row) for row in fit_matrix
                ),
                solver_column_scales=tuple(column_scales),
                solver_residual=solver_residual,
                predictions=MappingProxyType(
                    {
                        scenario_id: MappingProxyType(row)
                        for scenario_id, row in predictions.items()
                    }
                ),
            )
    return MappingProxyType(evidence)


def fit_block_calibration(
    affine_model: AffineOpcodeModel,
    rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
    anchor_q: Mapping[str, Decimal],
) -> BlockCalibrationResult:
    """Fit transfer slopes first, then fixed costs with reconstructed multipliers."""
    _validate_unique(feature_keys, "feature key")
    if set(feature_keys) & set(_TRANSFER_PARAMETER_KEYS):
        raise ValueError("transfer and feature parameter keys must be disjoint")
    if (
        set(affine_model.anchor_keys) != set(_ANCHOR_RAW_GAS)
        or len(feature_keys) != 4
    ):
        raise ValueError(
            "block calibration requires four natural anchors and four fixed costs"
        )
    if set(anchor_q) != set(affine_model.anchor_keys):
        raise ValueError("anchor_q must cover affine model anchor keys exactly")
    for key in affine_model.anchor_keys:
        value = anchor_q[key]
        if (
            not isinstance(value, Decimal)
            or not value.is_finite()
            or value <= 0
        ):
            raise ValueError(f"anchor_q must be a positive finite Decimal: {key}")
    parameter_order = (*_TRANSFER_PARAMETER_KEYS, *feature_keys)
    if len(parameter_order) != 6:
        raise ValueError("block calibration requires exactly six staged parameters")
    _validate_unique((row.row_id for row in rows), "block calibration row ID")

    fit_rows = tuple(row for row in rows if row.split == "fit")
    holdout_rows = tuple(row for row in rows if row.split == "holdout")
    if len(fit_rows) != 40:
        raise ValueError(
            f"block calibration requires exactly 40 fit rows, got {len(fit_rows)}"
        )
    if len(holdout_rows) != 8:
        raise ValueError(
            f"block calibration requires exactly 8 holdout rows, got {len(holdout_rows)}"
        )
    if any(row.split not in {"fit", "holdout"} for row in rows):
        raise ValueError("block calibration row split must be fit or holdout")
    _validate_block_workload_shape(rows, feature_keys)

    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        fit_joint_matrix, fit_offsets, fit_actuals = _block_design(
            affine_model, fit_rows, feature_keys, anchor_q
        )
        _holdout_matrix, _holdout_offsets, _holdout_actuals = _block_design(
            affine_model, holdout_rows, feature_keys, anchor_q
        )
        transfer_matrix = []
        transfer_targets = []
        fit_lines: dict[str, tuple[Decimal, Decimal]] = {}
        family_components: dict[
            str, tuple[Decimal, Decimal, Fraction, Fraction]
        ] = {}
        for family in _OPCODE_FAMILY_ANCHORS:
            indices = [
                index
                for index, row in enumerate(fit_rows)
                if row.workload_family == family
            ]
            counts = [fit_rows[index].workload_count for index in indices]
            if any(count is None for count in counts):
                raise ValueError("opcode family requires positive workload_count")
            exact_counts = [int(count) for count in counts]
            observed_intercept, observed_slope = _decimal_line_fit(
                exact_counts, [fit_actuals[index] for index in indices]
            )
            _offset_intercept, offset_slope = _decimal_line_fit(
                exact_counts, [fit_offsets[index] for index in indices]
            )
            body_slope = _fraction_line_slope(
                exact_counts, [fit_joint_matrix[index][0] for index in indices]
            )
            common_slope = _fraction_line_slope(
                exact_counts, [fit_joint_matrix[index][1] for index in indices]
            )
            transfer_matrix.append([body_slope, common_slope])
            transfer_targets.append(observed_slope - offset_slope)
            fit_lines[family] = (observed_intercept, observed_slope)
            family_components[family] = (
                observed_slope,
                offset_slope,
                body_slope,
                common_slope,
            )

        transfer_rank = exact_rank(transfer_matrix)
        if transfer_rank != 2:
            raise ValueError(
                "block calibration transfer family slope matrix exact rank must be "
                f"two, got {transfer_rank}"
            )
        transfer_values, transfer_scales, transfer_residual = (
            _scaled_decimal_least_squares(transfer_matrix, transfer_targets)
        )
        transfer_zero_tolerance = max(
            Decimal(1), *(abs(value) for value in transfer_values)
        ) * Decimal("1e-60")
        transfer_values = [
            Decimal(0) if abs(value) <= transfer_zero_tolerance else value
            for value in transfer_values
        ]
        transfer_residual = _vector_norm(
            [
                sum(
                    (
                        _decimal_from_fraction(coefficient) * value
                        for coefficient, value in zip(row, transfer_values)
                    ),
                    Decimal(0),
                )
                - target
                for row, target in zip(transfer_matrix, transfer_targets)
            ]
        )
        transfer_params = dict(zip(_TRANSFER_PARAMETER_KEYS, transfer_values))
        body_scale = transfer_params["body_scale"]
        common_overhead = transfer_params["common_opcode_overhead_per_operation"]
        if not body_scale.is_finite() or body_scale <= 0:
            raise ValueError("fitted body_scale must be positive and finite")
        if not common_overhead.is_finite() or common_overhead < 0:
            raise ValueError(
                "fitted common_opcode_overhead_per_operation must be nonnegative and finite"
            )
        reconstructed_anchors = {
            key: (
                body_scale * anchor_q[key] + common_overhead
            )
            / Decimal(_ANCHOR_RAW_GAS[key])
            for key in affine_model.anchor_keys
        }
        opcode_multipliers = affine_model.reconstruct_multipliers(
            reconstructed_anchors
        )
        for key, value in opcode_multipliers.items():
            if not value.is_finite() or value <= 0:
                raise ValueError(f"opcode multiplier must be positive and finite: {key}")

        family_slope_evidence = _family_slope_evidence(
            family_components, body_scale, common_overhead
        )
        transfer_lofo = _transfer_leave_one_family_out(
            family_components, transfer_matrix, transfer_targets
        )
        opcode_holdout_evidence = _opcode_holdout_evidence(
            holdout_rows, fit_rows, fit_lines
        )

        fixed_matrix = [row[2:] for row in fit_joint_matrix]
        fixed_rank = exact_rank(fixed_matrix)
        if fixed_rank != 4:
            raise ValueError(
                "block calibration fixed feature fit matrix exact rank must be "
                f"four, got {fixed_rank}"
            )
        fixed_targets = [
            actual - _opcode_contribution(row, opcode_multipliers)
            for row, actual in zip(fit_rows, fit_actuals)
        ]
        fixed_values, fixed_scales, fixed_residual = _scaled_decimal_least_squares(
            fixed_matrix, fixed_targets
        )
        fixed_costs = dict(zip(feature_keys, fixed_values))
        for key, value in fixed_costs.items():
            if not value.is_finite() or value <= 0:
                raise ValueError(f"fitted fixed cost must be positive and finite: {key}")

        fit_predictions, fit_apes = _predict_staged_block_rows(
            fit_rows, opcode_multipliers, feature_keys, fixed_costs
        )
        holdout_predictions, holdout_apes = _predict_staged_block_rows(
            holdout_rows, opcode_multipliers, feature_keys, fixed_costs
        )
        fit_mape = sum(fit_apes, Decimal(0)) / Decimal(len(fit_apes))
        fit_max_ape = max(fit_apes)
        holdout_max_ape = max(holdout_apes)
        if fit_mape > Decimal("0.05"):
            raise ValueError(f"block calibration fit MAPE exceeds 0.05: {fit_mape}")
        if fit_max_ape > Decimal("0.10"):
            raise ValueError(
                f"block calibration fit maximum APE exceeds 0.10: {fit_max_ape}"
            )
        if holdout_max_ape > Decimal("0.10"):
            raise ValueError(
                "block calibration holdout maximum APE exceeds 0.10: "
                f"{holdout_max_ape}"
            )

        predictions = {**fit_predictions, **holdout_predictions}
        return BlockCalibrationResult(
            transfer_params=MappingProxyType(transfer_params),
            reconstructed_anchors=MappingProxyType(reconstructed_anchors),
            fixed_costs=MappingProxyType(fixed_costs),
            opcode_multipliers=MappingProxyType(opcode_multipliers),
            fit_mape=fit_mape,
            fit_max_ape=fit_max_ape,
            holdout_max_ape=holdout_max_ape,
            status="accepted",
            parameter_order=parameter_order,
            transfer_exact_design_matrix=tuple(
                tuple(row) for row in transfer_matrix
            ),
            transfer_exact_rank=transfer_rank,
            transfer_column_scales=tuple(transfer_scales),
            transfer_solver_residual=transfer_residual,
            fixed_exact_design_matrix=tuple(tuple(row) for row in fixed_matrix),
            fixed_exact_rank=fixed_rank,
            fixed_column_scales=tuple(fixed_scales),
            fixed_solver_residual=fixed_residual,
            family_slope_evidence=_freeze_nested_mapping(family_slope_evidence),
            opcode_holdout_evidence=_freeze_nested_mapping(
                opcode_holdout_evidence
            ),
            transfer_leave_one_family_out=_freeze_nested_mapping(transfer_lofo),
            predictions=MappingProxyType(
                {key: MappingProxyType(value) for key, value in predictions.items()}
            ),
        )


def validate_dynamic_holdouts(
    affine_model: AffineOpcodeModel,
    opcode_multipliers: Mapping[str, Decimal],
    dynamic_relations: Sequence[DynamicRelationObservation],
    dynamic_keys: tuple[str, ...],
) -> dict[str, Any]:
    """Validate frozen dynamic scenarios without changing fitted multipliers."""
    _validate_unique(dynamic_keys, "dynamic key")
    if set(opcode_multipliers) != set(affine_model.opcode_keys):
        raise ValueError("opcode multiplier keys differ from the affine model")
    for key, value in opcode_multipliers.items():
        if not isinstance(value, Decimal) or not value.is_finite() or value <= 0:
            raise ValueError(f"opcode multiplier must be positive and finite: {key}")

    evidence: dict[str, Any] = {}
    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        for dynamic_key in dynamic_keys:
            observations = [
                item for item in dynamic_relations if item.dynamic_key == dynamic_key
            ]
            splits = [item.split for item in observations]
            if (
                len(observations) < 2
                or splits.count("canonical") != 1
                or splits.count("dynamic_holdout") != len(observations) - 1
            ):
                raise ValueError(
                    f"dynamic key {dynamic_key} requires one canonical and at least one dynamic holdout"
                )
            _validate_unique(
                (item.scenario_id for item in observations), "dynamic scenario ID"
            )
            rows = []
            implied_values = []
            for observation in observations:
                equation = observation.equation
                _validate_equation(equation, set(affine_model.opcode_keys))
                observed = equation.slope
                if observed == 0:
                    raise ValueError("dynamic relation observed slope must be nonzero")
                coefficient = equation.coefficients.get(dynamic_key, Fraction(0))
                if coefficient == 0:
                    raise ValueError("dynamic relation declared key coefficient must be nonzero")
                predicted = sum(
                    (
                        _decimal_from_fraction(value) * opcode_multipliers[key]
                        for key, value in equation.coefficients.items()
                    ),
                    Decimal(0),
                )
                if predicted == 0 or predicted.is_signed() != observed.is_signed():
                    raise ValueError(
                        "dynamic relation predicted sign differs from observed sign: "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )
                relation_ape = abs(predicted - observed) / abs(observed)
                if relation_ape > Decimal("0.10"):
                    raise ValueError(
                        "dynamic relation APE exceeds 0.10 for "
                        f"{dynamic_key}/{observation.scenario_id}: {relation_ape}"
                    )
                reference = sum(
                    (
                        _decimal_from_fraction(value) * opcode_multipliers[key]
                        for key, value in equation.coefficients.items()
                        if key != dynamic_key
                    ),
                    Decimal(0),
                )
                implied = (observed - reference) / _decimal_from_fraction(coefficient)
                if not implied.is_finite() or implied <= 0:
                    raise ValueError(
                        "dynamic relation implied multiplier must be positive: "
                        f"{dynamic_key}/{observation.scenario_id}"
                    )
                implied_values.append(implied)
                rows.append(
                    {
                        "scenario_id": observation.scenario_id,
                        "split": observation.split,
                        "observed_slope": observed,
                        "predicted_slope": predicted,
                        "relation_ape": relation_ape,
                        "implied_multiplier": implied,
                    }
                )
            consistency = max(implied_values) / min(implied_values) - Decimal(1)
            if consistency > Decimal("0.05"):
                raise ValueError(
                    "dynamic implied multiplier consistency exceeds 0.05 for "
                    f"{dynamic_key}: {consistency}"
                )
            evidence[dynamic_key] = {
                "status": "accepted",
                "consistency": consistency,
                "observations": rows,
            }
    extras = set(item.dynamic_key for item in dynamic_relations) - set(dynamic_keys)
    if extras:
        raise ValueError(f"unexpected dynamic keys: {sorted(extras)!r}")
    return evidence


def _validate_block_workload_shape(
    rows: Sequence[BlockCalibrationRow], feature_keys: tuple[str, ...]
) -> None:
    opcode_families = set(_OPCODE_FAMILY_ANCHORS)
    base_families = set(feature_keys)
    if opcode_families & base_families:
        raise ValueError("opcode and base workload families must be disjoint")
    expected = opcode_families | base_families
    actual = {row.workload_family for row in rows}
    if actual != expected:
        raise ValueError(
            "block calibration workload families differ from the staged model: "
            f"missing={sorted(expected - actual)!r}, extra={sorted(actual - expected)!r}"
        )
    for row in rows:
        if row.workload_family in opcode_families:
            if type(row.workload_count) is not int or row.workload_count <= 0:
                raise ValueError(
                    "opcode family row requires a positive workload_count"
                )
        elif row.workload_count is not None:
            raise ValueError("base family row workload_count must be None")

    for family in expected:
        family_fit = [
            row for row in rows if row.workload_family == family and row.split == "fit"
        ]
        family_holdout = [
            row
            for row in rows
            if row.workload_family == family and row.split == "holdout"
        ]
        if len(family_fit) != 5 or len(family_holdout) != 1:
            raise ValueError(
                f"workload family {family} requires five fit rows and exactly one holdout"
            )
        if family in opcode_families:
            counts = [row.workload_count for row in family_fit]
            if len(set(counts)) < 2:
                raise ValueError(
                    f"opcode family {family} requires at least two distinct fit counts"
                )
            if int(family_holdout[0].workload_count) <= max(int(count) for count in counts):
                raise ValueError(
                    f"opcode family {family} holdout count must exceed all fit counts"
                )
            feature_vectors = {
                tuple(row.feature_counts[key] for key in feature_keys)
                for row in (*family_fit, *family_holdout)
            }
            if len(feature_vectors) != 1:
                raise ValueError(
                    f"opcode family {family} fixed/base features must remain constant"
                )


def _decimal_line_fit(
    counts: Sequence[int], values: Sequence[Decimal]
) -> tuple[Decimal, Decimal]:
    if len(counts) != len(values) or len(counts) < 2:
        raise ValueError("family line fit requires at least two paired observations")
    count_values = [Decimal(value) for value in counts]
    count_mean = sum(count_values, Decimal(0)) / Decimal(len(count_values))
    value_mean = sum(values, Decimal(0)) / Decimal(len(values))
    denominator = sum(
        ((value - count_mean) * (value - count_mean) for value in count_values),
        Decimal(0),
    )
    if denominator == 0:
        raise ValueError("family line fit workload counts have zero variance")
    slope = sum(
        (
            (count - count_mean) * (value - value_mean)
            for count, value in zip(count_values, values)
        ),
        Decimal(0),
    ) / denominator
    return value_mean - slope * count_mean, slope


def _fraction_line_slope(
    counts: Sequence[int], values: Sequence[Fraction]
) -> Fraction:
    if len(counts) != len(values) or len(counts) < 2:
        raise ValueError("family exact slope requires at least two paired observations")
    count_values = [Fraction(value) for value in counts]
    count_mean = sum(count_values, Fraction(0)) / len(count_values)
    value_mean = sum(values, Fraction(0)) / len(values)
    denominator = sum(
        ((value - count_mean) * (value - count_mean) for value in count_values),
        Fraction(0),
    )
    if denominator == 0:
        raise ValueError("family exact slope workload counts have zero variance")
    return sum(
        (
            (count - count_mean) * (value - value_mean)
            for count, value in zip(count_values, values)
        ),
        Fraction(0),
    ) / denominator


def _ape_or_reject_zero_signal(
    predicted: Decimal, observed: Decimal, *, label: str
) -> Decimal:
    if observed == 0:
        if predicted == 0:
            return Decimal(0)
        raise ValueError(f"{label} has zero observed signal but nonzero prediction")
    return abs(predicted - observed) / abs(observed)


def _family_slope_evidence(
    components: Mapping[str, tuple[Decimal, Decimal, Fraction, Fraction]],
    body_scale: Decimal,
    common_overhead: Decimal,
) -> dict[str, dict[str, Decimal | str]]:
    evidence: dict[str, dict[str, Decimal | str]] = {}
    for family, (observed, offset, body, common) in components.items():
        predicted = (
            offset
            + _decimal_from_fraction(body) * body_scale
            + _decimal_from_fraction(common) * common_overhead
        )
        ape = _ape_or_reject_zero_signal(
            predicted, observed, label=f"family slope {family}"
        )
        if ape > Decimal("0.10"):
            raise ValueError(f"family slope APE exceeds 0.10 for {family}: {ape}")
        evidence[family] = {
            "anchor_key": _OPCODE_FAMILY_ANCHORS[family],
            "observed_full_slope": observed,
            "mu_zero_slope": offset,
            "body_column_slope": _decimal_from_fraction(body),
            "common_column_slope": _decimal_from_fraction(common),
            "predicted_full_slope": predicted,
            "slope_ape": ape,
        }
    return evidence


def _transfer_leave_one_family_out(
    components: Mapping[str, tuple[Decimal, Decimal, Fraction, Fraction]],
    matrix: Sequence[Sequence[Fraction]],
    targets: Sequence[Decimal],
) -> dict[str, dict[str, Decimal]]:
    families = tuple(components)
    evidence: dict[str, dict[str, Decimal]] = {}
    for omitted_index, family in enumerate(families):
        reduced_matrix = [
            row for index, row in enumerate(matrix) if index != omitted_index
        ]
        reduced_targets = [
            value for index, value in enumerate(targets) if index != omitted_index
        ]
        if exact_rank(reduced_matrix) != 2:
            raise ValueError(
                f"transfer leave-one-family-out matrix for {family} does not retain exact rank two"
            )
        alternate, _scales, _residual = _scaled_decimal_least_squares(
            reduced_matrix, reduced_targets
        )
        observed, offset, body, common = components[family]
        predicted = (
            offset
            + _decimal_from_fraction(body) * alternate[0]
            + _decimal_from_fraction(common) * alternate[1]
        )
        ape = _ape_or_reject_zero_signal(
            predicted,
            observed,
            label=f"transfer leave-one-family-out {family} slope",
        )
        if ape > Decimal("0.10"):
            raise ValueError(
                "transfer leave-one-family-out omitted slope APE exceeds 0.10 "
                f"for {family}: {ape}"
            )
        evidence[family] = {
            "observed_full_slope": observed,
            "predicted_full_slope": predicted,
            "omitted_slope_ape": ape,
        }
    return evidence


def _opcode_holdout_evidence(
    holdout_rows: Sequence[BlockCalibrationRow],
    fit_rows: Sequence[BlockCalibrationRow],
    fit_lines: Mapping[str, tuple[Decimal, Decimal]],
) -> dict[str, dict[str, Decimal | int | str]]:
    evidence: dict[str, dict[str, Decimal | int | str]] = {}
    for family, (intercept, slope) in fit_lines.items():
        fit_family = [row for row in fit_rows if row.workload_family == family]
        holdout = next(
            row for row in holdout_rows if row.workload_family == family
        )
        first_count = min(int(row.workload_count) for row in fit_family)
        holdout_count = int(holdout.workload_count)
        first_fit_line = intercept + slope * Decimal(first_count)
        predicted = intercept + slope * Decimal(holdout_count)
        delta_signal = holdout.prover_gas - first_fit_line
        if delta_signal != 0:
            delta_ape = abs(predicted - holdout.prover_gas) / abs(delta_signal)
        elif predicted == holdout.prover_gas:
            delta_ape = Decimal(0)
        else:
            raise ValueError(
                f"opcode holdout {family} has zero delta signal but nonzero residual"
            )
        if delta_ape > Decimal("0.10"):
            raise ValueError(
                "opcode holdout delta-signal APE exceeds 0.10 for "
                f"{family}: {delta_ape}"
            )
        evidence[family] = {
            "status": "accepted",
            "fit_intercept": intercept,
            "fit_slope": slope,
            "first_fit_count": first_count,
            "holdout_count": holdout_count,
            "fit_line_at_first_count": first_fit_line,
            "predicted_holdout": predicted,
            "actual_holdout": holdout.prover_gas,
            "delta_signal": delta_signal,
            "delta_signal_ape": delta_ape,
        }
    return evidence


def _opcode_contribution(
    row: BlockCalibrationRow, opcode_multipliers: Mapping[str, Decimal]
) -> Decimal:
    return sum(
        (
            Decimal(units) * opcode_multipliers[key]
            for key, units in row.raw_gas_by_key.items()
        ),
        Decimal(0),
    )


def _predict_staged_block_rows(
    rows: Sequence[BlockCalibrationRow],
    opcode_multipliers: Mapping[str, Decimal],
    feature_keys: tuple[str, ...],
    fixed_costs: Mapping[str, Decimal],
) -> tuple[dict[str, dict[str, Decimal | str]], list[Decimal]]:
    predictions: dict[str, dict[str, Decimal | str]] = {}
    apes = []
    for row in rows:
        predicted = _opcode_contribution(row, opcode_multipliers) + sum(
            (
                Decimal(row.feature_counts[key]) * fixed_costs[key]
                for key in feature_keys
            ),
            Decimal(0),
        )
        ape = abs(predicted - row.prover_gas) / row.prover_gas
        predictions[row.row_id] = {
            "split": row.split,
            "actual_prover_gas": row.prover_gas,
            "predicted_prover_gas": predicted,
            "ape": ape,
        }
        apes.append(ape)
    return predictions, apes


def _freeze_nested_mapping(
    values: Mapping[str, Mapping[str, Any]],
) -> Mapping[str, Mapping[str, Any]]:
    return MappingProxyType(
        {key: MappingProxyType(dict(value)) for key, value in values.items()}
    )


def _block_design(
    affine_model: AffineOpcodeModel,
    rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
    anchor_q: Mapping[str, Decimal],
) -> tuple[list[list[Fraction]], list[Decimal], list[Decimal]]:
    matrix: list[list[Fraction]] = []
    offsets: list[Decimal] = []
    actuals: list[Decimal] = []
    opcode_keys = set(affine_model.opcode_keys)
    for row in rows:
        if not row.row_id or not row.workload_family:
            raise ValueError("block calibration row identity and family are required")
        if (
            not isinstance(row.prover_gas, Decimal)
            or not row.prover_gas.is_finite()
            or row.prover_gas <= 0
        ):
            raise ValueError("block calibration prover gas must be a positive finite Decimal")
        unknown = set(row.raw_gas_by_key) - opcode_keys
        if unknown:
            raise ValueError(f"block calibration contains unknown opcode keys: {sorted(unknown)!r}")
        if set(row.feature_counts) != set(feature_keys):
            raise ValueError("block calibration feature counts differ from feature keys")
        for value in (*row.raw_gas_by_key.values(), *row.feature_counts.values()):
            if type(value) is not int or value < 0:
                raise ValueError("block calibration counts must be nonnegative integers")

        projected: dict[str, Fraction] = {}
        for anchor_key in affine_model.anchor_keys:
            projected[anchor_key] = sum(
                (
                    Fraction(units) * affine_model.anchor_basis[key][anchor_key]
                    for key, units in row.raw_gas_by_key.items()
                ),
                Fraction(0),
            )
        body_scale = sum(
            (
                value
                * Fraction(anchor_q[anchor_key])
                / _ANCHOR_RAW_GAS[anchor_key]
                for anchor_key, value in projected.items()
            ),
            Fraction(0),
        )
        common_overhead = sum(
            (
                value / _ANCHOR_RAW_GAS[anchor_key]
                for anchor_key, value in projected.items()
            ),
            Fraction(0),
        )
        fixed = [Fraction(row.feature_counts[key]) for key in feature_keys]
        matrix.append([body_scale, common_overhead, *fixed])
        offsets.append(
            sum(
                (
                    Decimal(units) * affine_model.mu_zero[key]
                    for key, units in row.raw_gas_by_key.items()
                ),
                Decimal(0),
            )
        )
        actuals.append(row.prover_gas)
    return matrix, offsets, actuals


def _scaled_decimal_least_squares(
    exact_matrix: Sequence[Sequence[Fraction]], targets: Sequence[Decimal]
) -> tuple[list[Decimal], list[Decimal], Decimal]:
    """Solve a full-rank least-squares problem using scaled, reorthogonalized MGS."""
    if not exact_matrix or len(exact_matrix) != len(targets):
        raise ValueError("least-squares matrix and target dimensions differ")
    width = len(exact_matrix[0])
    if width == 0 or any(len(row) != width for row in exact_matrix):
        raise ValueError("least-squares matrix has invalid dimensions")
    matrix = [
        [_decimal_from_fraction(value) for value in row] for row in exact_matrix
    ]
    columns = [
        [matrix[row][column] for row in range(len(matrix))]
        for column in range(width)
    ]
    scales = [_vector_norm(column) for column in columns]
    if any(scale == 0 for scale in scales):
        raise ValueError("least-squares design contains a zero column norm")
    scaled_columns = [
        [value / scale for value in column]
        for column, scale in zip(columns, scales)
    ]

    q_columns: list[list[Decimal]] = []
    upper = [[Decimal(0) for _ in range(width)] for _ in range(width)]
    for column_index, source in enumerate(scaled_columns):
        vector = list(source)
        for _pass in range(2):
            for previous_index, previous in enumerate(q_columns):
                projection = _dot(previous, vector)
                upper[previous_index][column_index] += projection
                vector = [
                    value - projection * basis
                    for value, basis in zip(vector, previous)
                ]
        diagonal = _vector_norm(vector)
        if diagonal == 0:
            raise ValueError("least-squares QR has a zero diagonal")
        upper[column_index][column_index] = diagonal
        q_columns.append([value / diagonal for value in vector])

    rhs = [_dot(column, targets) for column in q_columns]
    scaled_solution = [Decimal(0) for _ in range(width)]
    for row in range(width - 1, -1, -1):
        diagonal = upper[row][row]
        if diagonal == 0:
            raise ValueError("least-squares QR has a zero diagonal")
        remainder = sum(
            (
                upper[row][column] * scaled_solution[column]
                for column in range(row + 1, width)
            ),
            Decimal(0),
        )
        scaled_solution[row] = (rhs[row] - remainder) / diagonal
    solution = [value / scale for value, scale in zip(scaled_solution, scales)]
    residual = _vector_norm(
        [
            sum((cell * value for cell, value in zip(row, solution)), Decimal(0))
            - target
            for row, target in zip(matrix, targets)
        ]
    )
    return solution, scales, residual


def nonnegative_decimal_least_squares(
    exact_matrix: Sequence[Sequence[Fraction]], targets: Sequence[Decimal]
) -> NonnegativeLeastSquaresResult:
    """Fit a full-rank Decimal least-squares system subject to ``x >= 0``."""
    if not exact_matrix or len(exact_matrix) != len(targets):
        raise ValueError("least-squares matrix and target dimensions differ")
    width = len(exact_matrix[0])
    if width == 0 or any(len(row) != width for row in exact_matrix):
        raise ValueError("least-squares matrix has invalid dimensions")
    matrix = [
        [_as_exact_fraction(value) for value in row] for row in exact_matrix
    ]
    if exact_rank(matrix) != width:
        raise ValueError("nonnegative least-squares matrix is rank-deficient")
    if any(
        not isinstance(target, Decimal) or not target.is_finite()
        for target in targets
    ):
        raise ValueError("nonnegative least-squares targets must be finite Decimals")

    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        decimal_matrix = [
            [_decimal_from_fraction(value) for value in row] for row in matrix
        ]
        solution = [Decimal(0) for _ in range(width)]
        passive: set[int] = set()
        attempted_entries: set[tuple[tuple[int, ...], int]] = set()
        maximum_iterations = 30 * width
        iterations = 0

        while True:
            passive_columns = tuple(sorted(passive))
            residual_vector = [
                target
                - sum(
                    (cell * value for cell, value in zip(row, solution)),
                    Decimal(0),
                )
                for row, target in zip(decimal_matrix, targets)
            ]
            dual = [
                _dot(
                    [row[column] for row in decimal_matrix], residual_vector
                )
                for column in range(width)
            ]
            entering = [
                column
                for column in range(width)
                if (
                    column not in passive
                    and (passive_columns, column) not in attempted_entries
                    and dual[column] > 0
                )
            ]
            if not entering:
                reported_solution = tuple(
                    _round_decimal_significant(value, 60) for value in solution
                )
                residual = _vector_norm(
                    [
                        sum(
                            (
                                cell * value
                                for cell, value in zip(row, reported_solution)
                            ),
                            Decimal(0),
                        )
                        - target
                        for row, target in zip(decimal_matrix, targets)
                    ]
                )
                return NonnegativeLeastSquaresResult(
                    solution=reported_solution,
                    active_zero_indices=tuple(
                        column
                        for column, value in enumerate(reported_solution)
                        if value == 0
                    ),
                    residual=residual,
                )

            if iterations >= maximum_iterations:
                raise ValueError("nonnegative least-squares iteration cap exceeded")
            entering_column = max(
                entering, key=lambda column: dual[column]
            )
            # A revisited passive set has the same Decimal QR solution. Retrying
            # an earlier entry cannot make progress, even if its raw dual remains
            # a Decimal-roundoff positive value.
            attempted_entries.add((passive_columns, entering_column))
            source_passive_columns = passive_columns
            passive.add(entering_column)
            iterations += 1

            while True:
                passive_columns = tuple(sorted(passive))
                passive_matrix = [
                    [row[column] for column in passive_columns] for row in matrix
                ]
                passive_solution, _scales, _residual = _scaled_decimal_least_squares(
                    passive_matrix, targets
                )
                candidate = [Decimal(0) for _ in range(width)]
                for column, value in zip(passive_columns, passive_solution):
                    candidate[column] = value
                if all(candidate[column] > 0 for column in passive_columns):
                    solution = candidate
                    break

                boundary_steps = tuple(
                    (
                        (
                            Decimal(0)
                            if solution[column] == 0
                            else solution[column]
                            / (solution[column] - candidate[column])
                        ),
                        column,
                    )
                    for column in passive_columns
                    if candidate[column] <= 0
                )
                step = min(value for value, _column in boundary_steps)
                blocking_columns = tuple(
                    column
                    for value, column in boundary_steps
                    if value - step <= step.next_plus() - step
                )
                solution = [
                    current + step * (proposed - current)
                    for current, proposed in zip(solution, candidate)
                ]
                for column in blocking_columns:
                    solution[column] = Decimal(0)
                    passive.remove(column)
                iterations += 1
                if iterations > maximum_iterations:
                    raise ValueError("nonnegative least-squares iteration cap exceeded")
                if not passive or tuple(sorted(passive)) == source_passive_columns:
                    break


def fit_nonnegative_opcode_bodies(
    equations: Sequence[RelationEquation],
    opcode_keys: tuple[str, ...],
    anchor_body_costs: Mapping[str, Decimal],
) -> NonnegativeOpcodeFitEvidence:
    """Fit absolute opcode bodies while preserving signed relative relations."""
    _validate_unique(opcode_keys, "opcode key")
    if not opcode_keys:
        raise ValueError("opcode keys must not be empty")
    if not equations:
        raise ValueError("relation matrix must not be empty")
    _validate_unique((equation.relation_id for equation in equations), "relation ID")

    opcode_set = set(opcode_keys)
    unknown_anchors = set(anchor_body_costs) - opcode_set
    if unknown_anchors:
        raise ValueError(f"unknown anchor key: {sorted(unknown_anchors)!r}")
    unsupported_anchors = set(anchor_body_costs) - set(_ANCHOR_RAW_GAS)
    if unsupported_anchors:
        raise ValueError(
            "anchor body cost is not a natural anchor: "
            f"{sorted(unsupported_anchors)!r}"
        )
    for equation in equations:
        _validate_equation(equation, opcode_set)

    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        anchor_keys = tuple(
            key for key in opcode_keys if key in anchor_body_costs
        )
        lab_body_per_raw_gas: dict[str, Decimal] = {}
        for key in anchor_keys:
            value = anchor_body_costs[key]
            if (
                not isinstance(value, Decimal)
                or not value.is_finite()
                or value <= 0
            ):
                raise ValueError(
                    f"anchor body cost must be a positive finite Decimal: {key}"
                )
            lab_body_per_raw_gas[key] = value / Decimal(_ANCHOR_RAW_GAS[key])

        non_anchor_keys = tuple(
            key for key in opcode_keys if key not in anchor_body_costs
        )
        adjusted_targets = []
        matrix = []
        for equation in equations:
            fixed_contribution = sum(
                (
                    _decimal_from_fraction(equation.coefficients.get(key, Fraction(0)))
                    * lab_body_per_raw_gas[key]
                    for key in anchor_keys
                ),
                Decimal(0),
            )
            adjusted_targets.append(equation.slope - fixed_contribution)
            matrix.append(
                [
                    equation.coefficients.get(key, Fraction(0))
                    for key in non_anchor_keys
                ]
            )

        if non_anchor_keys:
            fit = nonnegative_decimal_least_squares(matrix, adjusted_targets)
            for key, value in zip(non_anchor_keys, fit.solution):
                lab_body_per_raw_gas[key] = value
            active_zero_keys = tuple(
                non_anchor_keys[index] for index in fit.active_zero_indices
            )
        else:
            active_zero_keys = ()

        predictions: dict[str, Mapping[str, Decimal | str]] = {}
        nonzero_apes = []
        flat_errors = []
        residual_terms = []
        for equation in equations:
            terms = [
                _decimal_from_fraction(coefficient) * lab_body_per_raw_gas[key]
                for key, coefficient in equation.coefficients.items()
            ]
            predicted = sum(terms, Decimal(0))
            residual_terms.append(predicted - equation.slope)
            if equation.slope == 0:
                normalized_error = abs(predicted) / max(
                    sum((abs(term) for term in terms), Decimal(0)), Decimal(1)
                )
                flat_errors.append(normalized_error)
                predictions[equation.relation_id] = MappingProxyType(
                    {
                        "gate": "exact_flat",
                        "observed_slope": equation.slope,
                        "predicted_slope": predicted,
                        "normalized_error": normalized_error,
                    }
                )
            else:
                ape = abs(predicted - equation.slope) / abs(equation.slope)
                nonzero_apes.append(ape)
                predictions[equation.relation_id] = MappingProxyType(
                    {
                        "gate": "nonzero",
                        "observed_slope": equation.slope,
                        "predicted_slope": predicted,
                        "ape": ape,
                    }
                )

        nonzero_relation_mape = (
            sum(nonzero_apes, Decimal(0)) / Decimal(len(nonzero_apes))
            if nonzero_apes
            else Decimal(0)
        )
        nonzero_relation_max_ape = max(nonzero_apes, default=Decimal(0))
        flat_relation_max_normalized_error = max(
            flat_errors, default=Decimal(0)
        )
        quality_failures = tuple(
            name
            for name, failed in (
                ("nonzero_relation_mape", nonzero_relation_mape > Decimal("0.05")),
                (
                    "nonzero_relation_max_ape",
                    nonzero_relation_max_ape > Decimal("0.10"),
                ),
                (
                    "flat_relation_max_normalized_error",
                    flat_relation_max_normalized_error > Decimal("1e-30"),
                ),
            )
            if failed
        )
        return NonnegativeOpcodeFitEvidence(
            status="not_supported" if quality_failures else "supported",
            opcode_keys=opcode_keys,
            anchor_keys=anchor_keys,
            lab_body_per_raw_gas=MappingProxyType(
                {key: lab_body_per_raw_gas[key] for key in opcode_keys}
            ),
            active_zero_keys=active_zero_keys,
            nonzero_relation_mape=nonzero_relation_mape,
            nonzero_relation_max_ape=nonzero_relation_max_ape,
            flat_relation_max_normalized_error=flat_relation_max_normalized_error,
            residual=_vector_norm(residual_terms),
            predictions=MappingProxyType(predictions),
            quality_failures=quality_failures,
        )


def _dot(left: Sequence[Decimal], right: Sequence[Decimal]) -> Decimal:
    return sum((a * b for a, b in zip(left, right)), Decimal(0))


def _vector_norm(vector: Sequence[Decimal]) -> Decimal:
    return _dot(vector, vector).sqrt()


def _round_decimal_significant(value: Decimal, digits: int) -> Decimal:
    if value == 0:
        return Decimal(0)
    return value.quantize(Decimal(1).scaleb(value.adjusted() - digits + 1))


def exact_rank(matrix: Sequence[Sequence[Fraction]]) -> int:
    """Return a matrix rank using only exact rational arithmetic."""
    if not matrix:
        return 0
    width = len(matrix[0])
    if any(len(row) != width for row in matrix):
        raise ValueError("matrix rows must have a consistent width")

    reduced = [[_as_exact_fraction(value) for value in row] for row in matrix]
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
        pivot_value = reduced[pivot_row][column]
        reduced[pivot_row] = [value / pivot_value for value in reduced[pivot_row]]
        for row in range(len(reduced)):
            if row == pivot_row or reduced[row][column] == 0:
                continue
            factor = reduced[row][column]
            reduced[row] = [
                value - factor * pivot_value
                for value, pivot_value in zip(reduced[row], reduced[pivot_row])
            ]
        pivot_row += 1
        if pivot_row == len(reduced):
            break
    return pivot_row


def derive_affine_opcode_model(
    opcode_keys: tuple[str, ...],
    equations: Sequence[RelationEquation],
    anchor_keys: tuple[str, ...],
) -> AffineOpcodeModel:
    """Solve ``A * mu = d`` as ``mu_zero + B * anchor_values`` exactly."""
    _validate_unique(opcode_keys, "opcode key")
    _validate_unique(anchor_keys, "anchor key")
    if not opcode_keys:
        raise ValueError("opcode keys must not be empty")
    unknown_anchors = set(anchor_keys) - set(opcode_keys)
    if unknown_anchors:
        raise ValueError(f"unknown anchor key: {sorted(unknown_anchors)!r}")
    if not equations:
        raise ValueError("relation matrix must not be empty")
    _validate_unique((equation.relation_id for equation in equations), "relation ID")

    opcode_set = set(opcode_keys)
    rows: list[list[Fraction]] = []
    slopes: list[Decimal] = []
    for equation in equations:
        _validate_equation(equation, opcode_set)
        rows.append([equation.coefficients.get(opcode_key, Fraction(0)) for opcode_key in opcode_keys])
        slopes.append(equation.slope)

    rank = exact_rank(rows)
    if rank != len(rows):
        raise ValueError("relation matrix rank does not equal equation count")
    nullity = len(opcode_keys) - rank
    if len(anchor_keys) > nullity:
        raise ValueError("anchor count does not equal nullity")

    non_anchor_keys = tuple(key for key in opcode_keys if key not in set(anchor_keys))
    if len(anchor_keys) == 0 and nullity != 0:
        raise ValueError("anchor columns do not leave a full-rank system")
    if len(anchor_keys) < nullity:
        raise ValueError("non-anchor system must be square")
    if len(anchor_keys) != nullity:
        raise ValueError("anchor count does not equal nullity")
    if len(rows) != len(non_anchor_keys):
        raise ValueError("non-anchor system must be square")

    non_anchor_columns = [opcode_keys.index(key) for key in non_anchor_keys]
    anchor_columns = [opcode_keys.index(key) for key in anchor_keys]
    system = [[row[column] for column in non_anchor_columns] for row in rows]
    if exact_rank(system) != len(non_anchor_keys):
        raise ValueError("anchor columns do not leave a full-rank system")
    anchor_matrix = [[row[column] for column in anchor_columns] for row in rows]

    reduced, solved_slopes = _gauss_jordan_with_decimal_rhs(
        system, anchor_matrix, slopes
    )
    mu_zero = {opcode_key: Decimal(0) for opcode_key in opcode_keys}
    anchor_basis = {
        opcode_key: {anchor_key: Fraction(0) for anchor_key in anchor_keys}
        for opcode_key in opcode_keys
    }
    for anchor_key in anchor_keys:
        anchor_basis[anchor_key][anchor_key] = Fraction(1)
    for row, opcode_key in enumerate(non_anchor_keys):
        mu_zero[opcode_key] = solved_slopes[row]
        for column, anchor_key in enumerate(anchor_keys):
            anchor_basis[opcode_key][anchor_key] = -reduced[row][column]

    return AffineOpcodeModel(
        opcode_keys=opcode_keys,
        anchor_keys=anchor_keys,
        rank=rank,
        nullity=nullity,
        mu_zero=MappingProxyType(mu_zero),
        anchor_basis=MappingProxyType(
            {
                opcode_key: MappingProxyType(coefficients)
                for opcode_key, coefficients in anchor_basis.items()
            }
        ),
    )


def _validate_unique(values: Iterable[str], label: str) -> None:
    seen: set[str] = set()
    for value in values:
        if value in seen:
            raise ValueError(f"duplicate {label}: {value!r}")
        seen.add(value)


def _validate_equation(equation: RelationEquation, opcode_keys: set[str]) -> None:
    if not equation.coefficients or not any(equation.coefficients.values()):
        raise ValueError("empty coefficient map")
    unknown = set(equation.coefficients) - opcode_keys
    if unknown:
        raise ValueError(f"unknown opcode key: {sorted(unknown)!r}")
    for coefficient in equation.coefficients.values():
        if not isinstance(coefficient, Fraction):
            raise ValueError("relation coefficient must be a Fraction")
    if not isinstance(equation.slope, Decimal) or not equation.slope.is_finite():
        raise ValueError("relation slope must be finite")


def _gauss_jordan_with_decimal_rhs(
    system: Sequence[Sequence[Fraction]],
    anchor_matrix: Sequence[Sequence[Fraction]],
    slopes: Sequence[Decimal],
) -> tuple[list[list[Fraction]], list[Decimal]]:
    """Reduce a square rational system while applying its row operations to Decimal d."""
    width = len(system)
    augmented = [
        list(row) + list(anchor_row)
        for row, anchor_row in zip(system, anchor_matrix)
    ]
    values = list(slopes)
    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        for column in range(width):
            pivot = next(
                (row for row in range(column, width) if augmented[row][column] != 0),
                None,
            )
            if pivot is None:
                raise ValueError("anchor columns do not leave a full-rank system")
            augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
            values[column], values[pivot] = values[pivot], values[column]
            pivot_value = augmented[column][column]
            augmented[column] = [value / pivot_value for value in augmented[column]]
            values[column] /= _decimal_from_fraction(pivot_value)
            for row in range(width):
                if row == column or augmented[row][column] == 0:
                    continue
                factor = augmented[row][column]
                augmented[row] = [
                    value - factor * pivot_value
                    for value, pivot_value in zip(augmented[row], augmented[column])
                ]
                values[row] -= _decimal_from_fraction(factor) * values[column]
    return [row[width:] for row in augmented], values


def _decimal_from_fraction(value: Fraction) -> Decimal:
    return Decimal(value.numerator) / Decimal(value.denominator)


def _as_exact_fraction(value: Fraction | int) -> Fraction:
    if isinstance(value, Fraction):
        return value
    if isinstance(value, int):
        return Fraction(value)
    raise ValueError("matrix cell must be a Fraction or int")
