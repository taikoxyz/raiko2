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


@dataclass(frozen=True)
class RelationEquation:
    relation_id: str
    coefficients: Mapping[str, Fraction]
    slope: Decimal


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


@dataclass(frozen=True)
class BlockCalibrationRow:
    row_id: str
    workload_family: str
    split: str
    prover_gas: Decimal
    raw_gas_by_key: Mapping[str, int]
    feature_counts: Mapping[str, int]


@dataclass(frozen=True)
class BlockCalibrationResult:
    anchors: Mapping[str, Decimal]
    fixed_costs: Mapping[str, Decimal]
    opcode_multipliers: Mapping[str, Decimal]
    fit_mape: Decimal
    fit_max_ape: Decimal
    holdout_max_ape: Decimal
    status: str
    parameter_order: tuple[str, ...]
    exact_design_matrix: tuple[tuple[Fraction, ...], ...]
    exact_rank: int
    column_scales: tuple[Decimal, ...]
    solver_residual: Decimal
    predictions: Mapping[str, Mapping[str, Decimal | str]]
    leave_one_family_out: Mapping[str, Mapping[str, Decimal]]


@dataclass(frozen=True)
class DynamicRelationObservation:
    dynamic_key: str
    scenario_id: str
    split: str
    equation: RelationEquation


def fit_block_calibration(
    affine_model: AffineOpcodeModel,
    rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
) -> BlockCalibrationResult:
    """Fit opcode anchors and fixed costs from controlled block rows."""
    _validate_unique(feature_keys, "feature key")
    if set(feature_keys) & set(affine_model.anchor_keys):
        raise ValueError("anchor and feature parameter keys must be disjoint")
    if len(affine_model.anchor_keys) != 4 or len(feature_keys) != 4:
        raise ValueError("block calibration requires four anchors and four fixed costs")
    parameter_order = (*affine_model.anchor_keys, *feature_keys)
    if len(parameter_order) != 8:
        raise ValueError("block calibration requires exactly eight parameters")
    _validate_unique((row.row_id for row in rows), "block calibration row ID")

    fit_rows = tuple(row for row in rows if row.split == "fit")
    holdout_rows = tuple(row for row in rows if row.split == "holdout")
    if not fit_rows:
        raise ValueError("block calibration requires fit rows")
    if not holdout_rows:
        raise ValueError("block calibration requires holdout rows")
    if len(fit_rows) < len(parameter_order):
        raise ValueError("block calibration exact fit matrix rank must be eight")
    if any(row.split not in {"fit", "holdout"} for row in rows):
        raise ValueError("block calibration row split must be fit or holdout")

    with localcontext(_CALIBRATION_DECIMAL_CONTEXT):
        fit_matrix, fit_offsets, fit_actuals = _block_design(
            affine_model, fit_rows, feature_keys
        )
        rank = exact_rank(fit_matrix)
        if rank != len(parameter_order):
            raise ValueError(
                f"block calibration exact fit matrix rank must be eight, got {rank}"
            )
        fit_targets = [
            actual - offset for actual, offset in zip(fit_actuals, fit_offsets)
        ]
        parameters, scales, solver_residual = _scaled_decimal_least_squares(
            fit_matrix, fit_targets
        )
        anchors = dict(
            zip(
                affine_model.anchor_keys,
                parameters[: len(affine_model.anchor_keys)],
            )
        )
        fixed_costs = dict(
            zip(feature_keys, parameters[len(affine_model.anchor_keys) :])
        )
        for key, value in (*anchors.items(), *fixed_costs.items()):
            if not value.is_finite():
                raise ValueError(f"fitted parameter must be finite: {key}")
            if value <= 0:
                raise ValueError(f"fitted parameter must be positive: {key}")

        opcode_multipliers = affine_model.reconstruct_multipliers(anchors)
        for key, value in opcode_multipliers.items():
            if not value.is_finite() or value <= 0:
                raise ValueError(f"opcode multiplier must be positive and finite: {key}")

        fit_predictions, fit_apes = _predict_block_rows(
            fit_rows, fit_matrix, fit_offsets, parameters, fit_actuals
        )
        holdout_matrix, holdout_offsets, holdout_actuals = _block_design(
            affine_model, holdout_rows, feature_keys
        )
        holdout_predictions, holdout_apes = _predict_block_rows(
            holdout_rows,
            holdout_matrix,
            holdout_offsets,
            parameters,
            holdout_actuals,
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

        lofo = _leave_one_family_out(
            affine_model,
            fit_rows,
            feature_keys,
            parameter_order,
            parameters,
        )
        predictions = {**fit_predictions, **holdout_predictions}
        return BlockCalibrationResult(
            anchors=MappingProxyType(anchors),
            fixed_costs=MappingProxyType(fixed_costs),
            opcode_multipliers=MappingProxyType(opcode_multipliers),
            fit_mape=fit_mape,
            fit_max_ape=fit_max_ape,
            holdout_max_ape=holdout_max_ape,
            status="accepted",
            parameter_order=parameter_order,
            exact_design_matrix=tuple(tuple(row) for row in fit_matrix),
            exact_rank=rank,
            column_scales=tuple(scales),
            solver_residual=solver_residual,
            predictions=MappingProxyType(
                {key: MappingProxyType(value) for key, value in predictions.items()}
            ),
            leave_one_family_out=MappingProxyType(
                {key: MappingProxyType(value) for key, value in lofo.items()}
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
                len(observations) != 3
                or splits.count("canonical") != 1
                or splits.count("dynamic_holdout") != 2
            ):
                raise ValueError(
                    f"dynamic key {dynamic_key} requires one canonical and two dynamic holdouts"
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
                    raise ValueError("dynamic relation predicted sign differs from observed sign")
                relation_ape = abs(predicted - observed) / abs(observed)
                if relation_ape > Decimal("0.10"):
                    raise ValueError(
                        f"dynamic relation APE exceeds 0.10: {relation_ape}"
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
                    raise ValueError("dynamic relation implied multiplier must be positive")
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
                    "dynamic implied multiplier consistency exceeds 0.05: "
                    f"{consistency}"
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


def _block_design(
    affine_model: AffineOpcodeModel,
    rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
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

        projected = []
        for anchor_key in affine_model.anchor_keys:
            projected.append(
                sum(
                    (
                        Fraction(units) * affine_model.anchor_basis[key][anchor_key]
                        for key, units in row.raw_gas_by_key.items()
                    ),
                    Fraction(0),
                )
            )
        fixed = [Fraction(row.feature_counts[key]) for key in feature_keys]
        matrix.append([*projected, *fixed])
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


def _predict_block_rows(
    rows: Sequence[BlockCalibrationRow],
    matrix: Sequence[Sequence[Fraction]],
    offsets: Sequence[Decimal],
    parameters: Sequence[Decimal],
    actuals: Sequence[Decimal],
) -> tuple[dict[str, dict[str, Decimal | str]], list[Decimal]]:
    predictions = {}
    apes = []
    for row, exact_design, offset, actual in zip(rows, matrix, offsets, actuals):
        predicted = offset + sum(
            (
                _decimal_from_fraction(coefficient) * parameter
                for coefficient, parameter in zip(exact_design, parameters)
            ),
            Decimal(0),
        )
        ape = abs(predicted - actual) / actual
        predictions[row.row_id] = {
            "split": row.split,
            "actual_prover_gas": actual,
            "predicted_prover_gas": predicted,
            "ape": ape,
        }
        apes.append(ape)
    return predictions, apes


def _leave_one_family_out(
    affine_model: AffineOpcodeModel,
    fit_rows: Sequence[BlockCalibrationRow],
    feature_keys: tuple[str, ...],
    parameter_order: tuple[str, ...],
    full_parameters: Sequence[Decimal],
) -> dict[str, dict[str, Decimal]]:
    evidence = {}
    families = tuple(dict.fromkeys(row.workload_family for row in fit_rows))
    if len(families) < 2:
        raise ValueError("leave-one-family-out requires at least two workload families")
    for family in families:
        reduced_rows = tuple(row for row in fit_rows if row.workload_family != family)
        matrix, offsets, actuals = _block_design(
            affine_model, reduced_rows, feature_keys
        )
        rank = exact_rank(matrix)
        if rank != len(parameter_order):
            raise ValueError(
                f"leave-one-family-out matrix for {family} does not retain exact rank eight"
            )
        targets = [actual - offset for actual, offset in zip(actuals, offsets)]
        reduced, _scales, _residual = _scaled_decimal_least_squares(matrix, targets)
        drifts = {}
        for key, full, alternate in zip(parameter_order, full_parameters, reduced):
            if full == 0:
                raise ValueError("leave-one-family-out cannot compare a zero coefficient")
            drift = abs(alternate - full) / abs(full)
            if drift > Decimal("0.05"):
                raise ValueError(
                    f"leave-one-family-out coefficient drift exceeds 0.05 for {family}/{key}: {drift}"
                )
            drifts[key] = drift
        evidence[family] = drifts
    return evidence


def _dot(left: Sequence[Decimal], right: Sequence[Decimal]) -> Decimal:
    return sum((a * b for a, b in zip(left, right)), Decimal(0))


def _vector_norm(vector: Sequence[Decimal]) -> Decimal:
    return _dot(vector, vector).sqrt()


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
