import pathlib
import sys
import unittest
from decimal import Decimal, Inexact, ROUND_DOWN, localcontext
from fractions import Fraction


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from calibration_model import (
    AffineOpcodeModel,
    BlockCalibrationRow,
    DynamicRelationObservation,
    RelationEquation,
    derive_affine_opcode_model,
    exact_rank,
    fit_block_calibration,
    validate_dynamic_holdouts,
)


ANCHORS = ("A0", "A1", "A2", "A3")
FEATURES = ("proposal_startup", "block_base", "tx_base", "native_value_transfer")


def synthetic_affine_model(*, derived_zero=Decimal("5")):
    opcode_keys = (*ANCHORS, "DERIVED")
    return AffineOpcodeModel(
        opcode_keys=opcode_keys,
        anchor_keys=ANCHORS,
        rank=1,
        nullity=4,
        mu_zero={**{key: Decimal(0) for key in ANCHORS}, "DERIVED": derived_zero},
        anchor_basis={
            key: {
                anchor: Fraction(int(key == anchor))
                for anchor in ANCHORS
            }
            for key in opcode_keys
        },
    )


def synthetic_block_rows(
    *,
    anchor_values=(Decimal("2"), Decimal("3"), Decimal("4"), Decimal("5")),
    fixed_values=(Decimal("6"), Decimal("7"), Decimal("8"), Decimal("9")),
    family_multipliers=(Decimal("1"), Decimal("1")),
    column_scales=(1, 1, 1, 1, 1, 1, 1, 1),
    derived_units=0,
):
    parameters = (*anchor_values, *fixed_values)
    rows = []
    for family_index, family_multiplier in enumerate(family_multipliers):
        for parameter_index, parameter in enumerate(parameters):
            raw_gas = {key: 0 for key in (*ANCHORS, "DERIVED")}
            features = {key: 0 for key in FEATURES}
            scale = column_scales[parameter_index]
            if parameter_index < len(ANCHORS):
                raw_gas[ANCHORS[parameter_index]] = scale
            else:
                features[FEATURES[parameter_index - len(ANCHORS)]] = scale
            raw_gas["DERIVED"] = derived_units
            prover_gas = (
                Decimal(derived_units) * Decimal("5")
                + Decimal(scale) * parameter * family_multiplier
            )
            rows.append(
                BlockCalibrationRow(
                    row_id=f"family-{family_index}-parameter-{parameter_index}",
                    workload_family=f"family-{family_index}",
                    split="fit",
                    prover_gas=prover_gas,
                    raw_gas_by_key=raw_gas,
                    feature_counts=features,
                )
            )
    rows.append(
        BlockCalibrationRow(
            row_id="holdout",
            workload_family="holdout-family",
            split="holdout",
            prover_gas=sum(parameters),
            raw_gas_by_key={**{key: 1 for key in ANCHORS}, "DERIVED": 0},
            feature_counts={key: 1 for key in FEATURES},
        )
    )
    return rows


class CalibrationModelTests(unittest.TestCase):
    def test_block_fit_recovers_rank_eight_parameters_and_multipliers(self):
        model = synthetic_affine_model()
        expected_anchors = dict(zip(ANCHORS, map(Decimal, ("2", "3", "4", "5"))))
        expected_fixed = dict(zip(FEATURES, map(Decimal, ("6", "7", "8", "9"))))

        result = fit_block_calibration(model, synthetic_block_rows(), FEATURES)

        for key, expected in (*expected_anchors.items(), *expected_fixed.items()):
            actual = result.anchors.get(key, result.fixed_costs.get(key))
            self.assertLessEqual(abs(actual - expected), Decimal("1e-60"))
        self.assertEqual(result.opcode_multipliers["DERIVED"], Decimal("5"))
        self.assertEqual(result.exact_rank, 8)
        self.assertEqual(result.status, "accepted")

    def test_block_fit_column_scaling_recovers_ill_scaled_full_rank_matrix(self):
        scales = (1, 10**8, 10**16, 10**24, 10**32, 10**40, 10**48, 10**56)

        result = fit_block_calibration(
            synthetic_affine_model(),
            synthetic_block_rows(column_scales=scales),
            FEATURES,
        )

        expected = tuple(map(Decimal, ("2", "3", "4", "5", "6", "7", "8", "9")))
        actual = tuple(result.anchors[key] for key in ANCHORS) + tuple(
            result.fixed_costs[key] for key in FEATURES
        )
        self.assertTrue(all(abs(left - right) <= Decimal("1e-50") for left, right in zip(actual, expected)))
        with localcontext() as expected_context:
            expected_context.prec = 80
            expected_scale = Decimal(10**56) * Decimal(2).sqrt()
        self.assertEqual(result.column_scales[-1], expected_scale)

    def test_block_fit_uses_isolated_precision_without_mutating_caller_context(self):
        with localcontext() as hostile:
            hostile.prec = 7
            hostile.rounding = ROUND_DOWN
            hostile.traps[Inexact] = True

            result = fit_block_calibration(
                synthetic_affine_model(), synthetic_block_rows(), FEATURES
            )

            self.assertLessEqual(
                abs(result.anchors["A0"] - Decimal("2")), Decimal("1e-60")
            )
            self.assertEqual(hostile.prec, 7)
            self.assertEqual(hostile.rounding, ROUND_DOWN)
            self.assertTrue(hostile.traps[Inexact])

    def test_block_fit_rejects_rank_seven(self):
        rows = [
            row
            for row in synthetic_block_rows()
            if not row.row_id.endswith("parameter-7")
        ]
        with self.assertRaisesRegex(ValueError, "rank.*eight"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

    def test_block_fit_rejects_negative_reconstructed_multiplier(self):
        with self.assertRaisesRegex(ValueError, "opcode multiplier must be positive"):
            fit_block_calibration(
                synthetic_affine_model(derived_zero=Decimal("-1")),
                synthetic_block_rows(),
                FEATURES,
            )

    def test_block_fit_rejects_fit_mape_and_max_ape_gates(self):
        rows = synthetic_block_rows()
        rows[0] = BlockCalibrationRow(
            **{**rows[0].__dict__, "prover_gas": Decimal("20")}
        )
        with self.assertRaisesRegex(ValueError, "fit MAPE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

        rows = synthetic_block_rows()
        rows[0] = BlockCalibrationRow(
            **{**rows[0].__dict__, "prover_gas": Decimal("2.5")}
        )
        with self.assertRaisesRegex(ValueError, "fit maximum APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

    def test_block_fit_rejects_holdout_max_ape_gate(self):
        rows = synthetic_block_rows()
        rows[-1] = BlockCalibrationRow(
            **{**rows[-1].__dict__, "prover_gas": Decimal("100")}
        )
        with self.assertRaisesRegex(ValueError, "holdout maximum APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

    def test_block_fit_rejects_nonpositive_or_nonfinite_actuals(self):
        for invalid in (Decimal("0"), Decimal("-1"), Decimal("NaN")):
            rows = synthetic_block_rows()
            rows[0] = BlockCalibrationRow(
                **{**rows[0].__dict__, "prover_gas": invalid}
            )
            with self.subTest(invalid=invalid), self.assertRaisesRegex(
                ValueError, "positive finite"
            ):
                fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

    def test_block_fit_rejects_leave_one_family_out_drift(self):
        rows = synthetic_block_rows(
            family_multipliers=(Decimal("1"), Decimal("1.12")),
            derived_units=100,
        )
        with self.assertRaisesRegex(ValueError, "leave-one-family-out"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES)

    def test_dynamic_holdouts_return_evidence_without_refitting(self):
        model = synthetic_affine_model()
        multipliers = model.reconstruct_multipliers(
            dict(zip(ANCHORS, map(Decimal, ("2", "3", "4", "5"))))
        )
        observations = tuple(
            DynamicRelationObservation(
                dynamic_key="A0",
                scenario_id=scenario,
                split=split,
                equation=RelationEquation(
                    scenario,
                    {"A0": Fraction(raw), "A1": Fraction(-1)},
                    Decimal(raw * 2 - 3),
                ),
            )
            for scenario, split, raw in (
                ("canonical", "canonical", 1),
                ("medium", "dynamic_holdout", 2),
                ("large", "dynamic_holdout", 4),
            )
        )

        evidence = validate_dynamic_holdouts(
            model, multipliers, observations, ("A0",)
        )

        self.assertEqual(evidence["A0"]["status"], "accepted")
        self.assertEqual(multipliers["A0"], Decimal("2"))
        self.assertEqual(
            [row["implied_multiplier"] for row in evidence["A0"]["observations"]],
            [Decimal("2"), Decimal("2"), Decimal("2")],
        )

    def test_dynamic_holdouts_reject_shape_sign_ape_and_consistency_failures(self):
        model = synthetic_affine_model()
        multipliers = model.reconstruct_multipliers(
            dict(zip(ANCHORS, map(Decimal, ("2", "3", "4", "5"))))
        )

        def observations(slopes=(Decimal("-1"), Decimal("1"), Decimal("5"))):
            return tuple(
                DynamicRelationObservation(
                    dynamic_key="A0",
                    scenario_id=scenario,
                    split=split,
                    equation=RelationEquation(
                        scenario,
                        {"A0": Fraction(raw), "A1": Fraction(-1)},
                        slope,
                    ),
                )
                for (scenario, split, raw), slope in zip(
                    (
                        ("canonical", "canonical", 1),
                        ("medium", "dynamic_holdout", 2),
                        ("large", "dynamic_holdout", 4),
                    ),
                    slopes,
                )
            )

        inconsistent = tuple(
            DynamicRelationObservation(
                dynamic_key="A0",
                scenario_id=scenario,
                split=split,
                equation=RelationEquation(
                    scenario,
                    {"A0": Fraction(raw), "A1": Fraction(-100)},
                    slope,
                ),
            )
            for (scenario, split, raw), slope in zip(
                (
                    ("canonical", "canonical", 1),
                    ("medium", "dynamic_holdout", 2),
                    ("large", "dynamic_holdout", 4),
                ),
                (Decimal("-298"), Decimal("-295.76"), Decimal("-291.52")),
            )
        )
        cases = (
            (observations()[:2], "one canonical and two"),
            (observations((Decimal("1"), Decimal("1"), Decimal("5"))), "predicted sign"),
            (observations((Decimal("-1"), Decimal("2"), Decimal("5"))), "relation APE"),
            (inconsistent, "implied multiplier consistency"),
        )
        for rows, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                validate_dynamic_holdouts(model, multipliers, rows, ("A0",))

    def test_exact_rank_and_affine_reconstruction(self):
        equations = (
            RelationEquation(
                "add-pop", {"ADD": Fraction(3), "POP": Fraction(-2)}, Decimal("50")
            ),
            RelationEquation(
                "mul-pop", {"MUL": Fraction(5), "POP": Fraction(-2)}, Decimal("170")
            ),
        )

        model = derive_affine_opcode_model(
            opcode_keys=("ADD", "MUL", "POP"),
            equations=equations,
            anchor_keys=("POP",),
        )

        self.assertEqual(model.rank, 2)
        self.assertEqual(model.nullity, 1)
        self.assertEqual(
            model.reconstruct_multipliers({"POP": Decimal("20")}),
            {"ADD": Decimal("30"), "MUL": Decimal("42"), "POP": Decimal("20")},
        )

    def test_rejects_wrong_anchor_basis(self):
        equations = (
            RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),
        )

        with self.assertRaisesRegex(
            ValueError, "anchor columns do not leave a full-rank system"
        ):
            derive_affine_opcode_model(("A", "B"), equations, ())

    def test_solves_full_rank_system_without_anchors(self):
        model = derive_affine_opcode_model(
            ("A",),
            (RelationEquation("a", {"A": Fraction(2)}, Decimal("10")),),
            (),
        )

        self.assertEqual(model.nullity, 0)
        self.assertEqual(model.reconstruct_multipliers({}), {"A": Decimal("5")})

    def test_rejects_duplicate_opcode_anchor_and_relation_identities(self):
        equation = RelationEquation("same", {"A": Fraction(1)}, Decimal("1"))

        with self.assertRaisesRegex(ValueError, "duplicate opcode key"):
            derive_affine_opcode_model(("A", "A"), (equation,), ())
        with self.assertRaisesRegex(ValueError, "duplicate anchor key"):
            derive_affine_opcode_model(("A", "B"), (equation,), ("B", "B"))
        with self.assertRaisesRegex(ValueError, "duplicate relation ID"):
            derive_affine_opcode_model(("A",), (equation, equation), ())

    def test_rejects_empty_or_unknown_coefficient_maps(self):
        with self.assertRaisesRegex(ValueError, "empty coefficient map"):
            derive_affine_opcode_model(
                ("A",), (RelationEquation("empty", {}, Decimal("1")),), ()
            )
        with self.assertRaisesRegex(ValueError, "unknown opcode key"):
            derive_affine_opcode_model(
                ("A",),
                (RelationEquation("unknown", {"B": Fraction(1)}, Decimal("1")),),
                (),
            )

    def test_rejects_non_finite_relation_slopes(self):
        for slope in (Decimal("NaN"), Decimal("Infinity"), Decimal("-Infinity")):
            with self.subTest(slope=slope), self.assertRaisesRegex(
                ValueError, "relation slope must be finite"
            ):
                derive_affine_opcode_model(
                    ("A",), (RelationEquation("bad", {"A": Fraction(1)}, slope),), ()
                )

    def test_rejects_rank_anchor_count_and_non_square_failures(self):
        dependent = (
            RelationEquation("first", {"A": Fraction(1), "B": Fraction(1)}, Decimal("1")),
            RelationEquation("second", {"A": Fraction(2), "B": Fraction(2)}, Decimal("2")),
        )
        with self.assertRaisesRegex(ValueError, "relation matrix rank"):
            derive_affine_opcode_model(("A", "B"), dependent, ())

        equation = RelationEquation("a", {"A": Fraction(1)}, Decimal("1"))
        with self.assertRaisesRegex(ValueError, "anchor count does not equal nullity"):
            derive_affine_opcode_model(("A", "B"), (equation,), ("A", "B"))
        with self.assertRaisesRegex(ValueError, "non-anchor system must be square"):
            derive_affine_opcode_model(("A", "B", "C"), (equation,), ("C",))

    def test_reconstruction_requires_exact_anchor_values(self):
        model = derive_affine_opcode_model(
            ("A", "B"),
            (RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),),
            ("B",),
        )

        with self.assertRaisesRegex(ValueError, "missing anchor values"):
            model.reconstruct_multipliers({})
        with self.assertRaisesRegex(ValueError, "unexpected anchor values"):
            model.reconstruct_multipliers({"B": Decimal("1"), "C": Decimal("2")})

    def test_derived_model_mappings_cannot_be_mutated(self):
        model = derive_affine_opcode_model(
            ("A", "B"),
            (RelationEquation("a-b", {"A": Fraction(1), "B": Fraction(-1)}, Decimal("1")),),
            ("B",),
        )

        with self.assertRaises(TypeError):
            model.mu_zero["A"] = Decimal("999")
        with self.assertRaises(TypeError):
            model.anchor_basis["A"]["B"] = Fraction(7)
        self.assertEqual(
            model.reconstruct_multipliers({"B": Decimal("2")}),
            {"A": Decimal("3"), "B": Decimal("2")},
        )

    def test_decimal_operations_ignore_hostile_caller_context(self):
        with localcontext() as hostile:
            hostile.prec = 7
            hostile.rounding = ROUND_DOWN
            hostile.traps[Inexact] = True
            model = derive_affine_opcode_model(
                ("A", "B"),
                (
                    RelationEquation(
                        "three-a-two-b",
                        {"A": Fraction(3), "B": Fraction(-2)},
                        Decimal("50"),
                    ),
                ),
                ("B",),
            )

            self.assertEqual(hostile.prec, 7)
            self.assertEqual(hostile.rounding, ROUND_DOWN)
            self.assertTrue(hostile.traps[Inexact])
            self.assertEqual(
                model.reconstruct_multipliers({"B": Decimal("2")}),
                {"A": Decimal("18"), "B": Decimal("2")},
            )

    def test_exact_rank_normalizes_large_integer_cells_without_float_rounding(self):
        large = 2**54

        self.assertEqual(exact_rank(((large, large + 1), (large + 1, large + 2))), 2)

    def test_current_matched_controls_have_rank_98_with_natural_anchors(self):
        cases = tuple(
            opcode_gas.CaseSpec(
                name=f"opcode:0x{opcode:02x}",
                opcode=opcode,
                scenario=scenario,
                template=template,
                target_raw_gas=raw_gas,
            )
            for opcode, (scenario, template, raw_gas) in sorted(
                opcode_gas.PURE_OPCODE_DEFAULTS.items()
            )
        )
        opcode_keys = tuple(case.name for case in cases)
        equations = []
        coefficient_rows = []
        for case in cases:
            spec = opcode_gas.matched_control_spec(case)
            target_raw_gas_by_key = {case.name: case.target_raw_gas}
            if spec.compound:
                control_raw_gas_by_key = {
                    f"opcode:0x{opcode:02x}": opcode_gas.PURE_OPCODE_DEFAULTS[opcode][2]
                    * count
                    for opcode, count in spec.reference_opcode_counts
                }
                self.assertEqual(
                    sum(control_raw_gas_by_key.values()), spec.reference_raw_gas_total
                )
            else:
                control_raw_gas_by_key = {
                    f"opcode:0x{spec.reference_opcode:02x}": spec.reference_raw_gas
                }

            coefficients = {
                opcode_key: Fraction(
                    target_raw_gas_by_key.get(opcode_key, 0)
                    - control_raw_gas_by_key.get(opcode_key, 0)
                )
                for opcode_key in target_raw_gas_by_key.keys()
                | control_raw_gas_by_key.keys()
            }
            coefficients = {
                opcode_key: coefficient
                for opcode_key, coefficient in coefficients.items()
                if coefficient
            }
            if coefficients:
                equations.append(RelationEquation(case.name, coefficients, Decimal("0")))
                coefficient_rows.append(
                    [coefficients.get(opcode_key, Fraction(0)) for opcode_key in opcode_keys]
                )

        model = derive_affine_opcode_model(
            opcode_keys=opcode_keys,
            equations=tuple(equations),
            anchor_keys=("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90"),
        )

        self.assertEqual(len(opcode_keys), 102)
        self.assertEqual(len(equations), 98)
        self.assertEqual(exact_rank(coefficient_rows), 98)
        self.assertEqual(
            model.anchor_keys,
            ("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90"),
        )


if __name__ == "__main__":
    unittest.main()
