"""Exact linear algebra for relative opcode calibration relations."""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Context, Decimal, ROUND_HALF_EVEN, localcontext
from fractions import Fraction
from types import MappingProxyType
from typing import Iterable, Mapping, Sequence


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
