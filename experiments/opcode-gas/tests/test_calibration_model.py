import pathlib
import sys
import unittest
from decimal import Decimal, Inexact, ROUND_DOWN, localcontext
from fractions import Fraction


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
import calibration_model
from calibration_model import (
    AffineOpcodeModel,
    BlockCalibrationRow,
    DynamicOpcodeObservation,
    DynamicRelationObservation,
    RelationEquation,
    derive_affine_opcode_model,
    exact_rank,
    fit_block_calibration,
    fit_dynamic_opcode_models,
    validate_dynamic_holdouts,
)


ANCHORS = ("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90")
ANCHOR_Q = dict(zip(ANCHORS, map(Decimal, ("7", "9", "12", "15"))))
ANCHOR_RAW_GAS = dict(zip(ANCHORS, (2, 2, 3, 3)))
FEATURES = ("proposal_startup", "block_base", "tx_base", "native_value_transfer")
OPCODE_FAMILIES = dict(
    zip(("pop_family", "push_family", "dup_family", "swap_family"), ANCHORS)
)


def synthetic_affine_model(*, derived_zero=Decimal("5")):
    derived_keys = ("DERIVED", *(f"DERIVED_{index:02d}" for index in range(97)))
    opcode_keys = (*ANCHORS, *derived_keys)
    anchor_basis = {
        key: {
            anchor: Fraction(int(key == anchor))
            for anchor in ANCHORS
        }
        for key in opcode_keys
    }
    anchor_basis["DERIVED"][ANCHORS[0]] = Fraction(1, 2)
    anchor_basis["DERIVED"][ANCHORS[1]] = Fraction(1, 4)
    return AffineOpcodeModel(
        opcode_keys=opcode_keys,
        anchor_keys=ANCHORS,
        rank=98,
        nullity=4,
        mu_zero={
            **{key: Decimal(0) for key in ANCHORS},
            **{key: derived_zero for key in derived_keys},
        },
        anchor_basis=anchor_basis,
    )


def synthetic_block_rows(
    *,
    body_scale=Decimal("2"),
    common_opcode_overhead_per_operation=Decimal("6"),
    fixed_values=(Decimal("6000"), Decimal("7000"), Decimal("8000"), Decimal("9000")),
    fit_counts=(1, 2, 4, 8, 16),
    holdout_count=32,
    family_startups=None,
    family_slope_offsets=None,
    holdout_offsets=None,
    derived_units_per_count=0,
):
    family_startups = family_startups or {}
    family_slope_offsets = family_slope_offsets or {}
    holdout_offsets = holdout_offsets or {}
    anchor_values = {
        key: (
            body_scale * ANCHOR_Q[key]
            + common_opcode_overhead_per_operation
        )
        / Decimal(ANCHOR_RAW_GAS[key])
        for key in ANCHORS
    }
    derived_multiplier = (
        Decimal("5")
        + anchor_values[ANCHORS[0]] / Decimal(2)
        + anchor_values[ANCHORS[1]] / Decimal(4)
    )
    rows = []
    for family, anchor_key in OPCODE_FAMILIES.items():
        for split, counts in (("fit", fit_counts), ("holdout", (holdout_count,))):
            for count in counts:
                raw_gas = {key: 0 for key in (*ANCHORS, "DERIVED")}
                raw_gas[anchor_key] = ANCHOR_RAW_GAS[anchor_key] * count
                raw_gas["DERIVED"] = derived_units_per_count * count
                features = {key: 0 for key in FEATURES}
                features["proposal_startup"] = 1000
                prover_gas = (
                    Decimal(1000) * fixed_values[0]
                    + Decimal(family_startups.get(family, 0))
                    + Decimal(count)
                    * (
                        Decimal(ANCHOR_RAW_GAS[anchor_key])
                        * anchor_values[anchor_key]
                        + Decimal(derived_units_per_count) * derived_multiplier
                        + Decimal(family_slope_offsets.get(family, 0))
                    )
                )
                if split == "holdout":
                    prover_gas += Decimal(holdout_offsets.get(family, 0))
                rows.append(
                    BlockCalibrationRow(
                        row_id=f"{family}-{split}-{count}",
                        workload_family=family,
                        split=split,
                        prover_gas=prover_gas,
                        raw_gas_by_key=raw_gas,
                        feature_counts=features,
                        workload_count=count,
                    )
                )

    for family_index, family in enumerate(FEATURES):
        for split, counts in (("fit", fit_counts), ("holdout", (holdout_count,))):
            for count in counts:
                features = {key: 0 for key in FEATURES}
                features[family] = count
                prover_gas = Decimal(count) * fixed_values[family_index]
                if split == "holdout":
                    prover_gas += Decimal(holdout_offsets.get(family, 0))
                rows.append(
                    BlockCalibrationRow(
                        row_id=f"{family}-{split}-{count}",
                        workload_family=family,
                        split=split,
                        prover_gas=prover_gas,
                        raw_gas_by_key={key: 0 for key in (*ANCHORS, "DERIVED")},
                        feature_counts=features,
                        workload_count=None,
                    )
                )
    return rows


class CalibrationModelTests(unittest.TestCase):
    def test_dynamic_opcode_fit_recovers_body_and_production_coefficients(self):
        observations = (
            DynamicOpcodeObservation(
                dynamic_key="memory_expansion",
                scenario_id="fit-0",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(0)},
                target_body_cost=Decimal("3"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="memory_expansion",
                scenario_id="fit-1",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(1)},
                target_body_cost=Decimal("5"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="memory_expansion",
                scenario_id="holdout-3",
                model_split="holdout",
                features={"constant": Fraction(1), "words": Fraction(3)},
                target_body_cost=Decimal("9"),
            ),
        )

        evidence = fit_dynamic_opcode_models(
            observations,
            {"memory_expansion": ("constant", "words")},
            body_scale=Decimal("4"),
            common_overhead=Decimal("7"),
        )["memory_expansion"]

        self.assertEqual(evidence.status, "supported")
        self.assertEqual(evidence.exact_rank, 2)
        self.assertEqual(evidence.parameter_count, 2)
        self.assertEqual(evidence.observation_count, 3)
        self.assertEqual(evidence.fit_count, 2)
        self.assertEqual(evidence.holdout_count, 1)
        for name, expected in {"constant": Decimal("3"), "words": Decimal("2")}.items():
            self.assertLessEqual(
                abs(evidence.body_coefficients[name] - expected), Decimal("1e-60")
            )
        for name, expected in {
            "constant": Decimal("19"),
            "words": Decimal("8"),
        }.items():
            self.assertLessEqual(
                abs(evidence.production_coefficients[name] - expected), Decimal("1e-60")
            )
        self.assertLessEqual(evidence.fit_body_mape, Decimal("1e-60"))
        self.assertLessEqual(evidence.fit_body_max_ape, Decimal("1e-60"))
        self.assertLessEqual(evidence.holdout_body_max_ape, Decimal("1e-60"))
        self.assertLessEqual(evidence.fit_production_max_ape, Decimal("1e-60"))
        self.assertLessEqual(evidence.holdout_production_max_ape, Decimal("1e-60"))
        self.assertEqual(evidence.quality_failures, ())
        prediction = evidence.predictions["holdout-3"]
        self.assertEqual(prediction["model_split"], "holdout")
        self.assertEqual(prediction["actual_body_cost"], Decimal("9"))
        self.assertLessEqual(
            abs(prediction["predicted_body_cost"] - Decimal("9")), Decimal("1e-60")
        )
        self.assertLessEqual(prediction["body_ape"], Decimal("1e-60"))
        self.assertEqual(prediction["actual_production_cost"], Decimal("43"))
        self.assertLessEqual(
            abs(prediction["predicted_production_cost"] - Decimal("43")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(prediction["production_ape"], Decimal("1e-60"))

    def test_dynamic_opcode_fit_returns_not_supported_with_quality_evidence(self):
        observations = (
            DynamicOpcodeObservation(
                dynamic_key="storage",
                scenario_id="fit-0",
                model_split="fit",
                features={"constant": Fraction(1), "slots": Fraction(0)},
                target_body_cost=Decimal("10"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="storage",
                scenario_id="fit-1",
                model_split="fit",
                features={"constant": Fraction(1), "slots": Fraction(1)},
                target_body_cost=Decimal("20"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="storage",
                scenario_id="fit-2",
                model_split="fit",
                features={"constant": Fraction(1), "slots": Fraction(2)},
                target_body_cost=Decimal("100"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="storage",
                scenario_id="holdout",
                model_split="holdout",
                features={"constant": Fraction(1), "slots": Fraction(3)},
                target_body_cost=Decimal("1"),
            ),
        )

        evidence = fit_dynamic_opcode_models(
            observations,
            {"storage": ("constant", "slots")},
            body_scale=Decimal("2"),
            common_overhead=Decimal("1"),
        )["storage"]

        self.assertEqual(evidence.status, "not_supported")
        expected_constant = Decimal(
            "-1.6666666666666666666666666666666666666666666666666666666666666666666666666666667"
        )
        self.assertLessEqual(
            abs(evidence.body_coefficients["constant"] - expected_constant),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(evidence.body_coefficients["slots"] - Decimal("45")),
            Decimal("1e-60"),
        )
        self.assertGreater(evidence.fit_body_mape, Decimal("0.05"))
        self.assertGreater(evidence.fit_body_max_ape, Decimal("0.10"))
        self.assertGreater(evidence.holdout_body_max_ape, Decimal("0.10"))
        self.assertEqual(
            evidence.quality_failures,
            ("fit_mape", "fit_max_ape", "holdout_max_ape"),
        )
        self.assertEqual(
            set(evidence.predictions),
            {row.scenario_id for row in observations},
        )

    def test_dynamic_opcode_quality_gates_use_production_units(self):
        observations = (
            DynamicOpcodeObservation(
                dynamic_key="memory",
                scenario_id="fit-0",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(0)},
                target_body_cost=Decimal("1"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="memory",
                scenario_id="fit-1",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(1)},
                target_body_cost=Decimal("2"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="memory",
                scenario_id="fit-2",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(2)},
                target_body_cost=Decimal("4"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="memory",
                scenario_id="holdout",
                model_split="holdout",
                features={"constant": Fraction(1), "words": Fraction(3)},
                target_body_cost=Decimal("5"),
            ),
        )

        evidence = fit_dynamic_opcode_models(
            observations,
            {"memory": ("constant", "words")},
            body_scale=Decimal("1"),
            common_overhead=Decimal("1000"),
        )["memory"]

        self.assertGreater(evidence.fit_body_mape, Decimal("0.05"))
        self.assertLess(evidence.fit_production_mape, Decimal("0.05"))
        self.assertEqual(evidence.status, "supported")
        self.assertEqual(evidence.quality_failures, ())

    def test_dynamic_opcode_fit_rejects_structural_and_provenance_failures(self):
        valid = (
            DynamicOpcodeObservation(
                dynamic_key="copy",
                scenario_id="fit-0",
                model_split="fit",
                features={"constant": Fraction(1), "bytes": Fraction(0)},
                target_body_cost=Decimal("1"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="copy",
                scenario_id="fit-1",
                model_split="fit",
                features={"constant": Fraction(1), "bytes": Fraction(1)},
                target_body_cost=Decimal("2"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="copy",
                scenario_id="holdout",
                model_split="holdout",
                features={"constant": Fraction(1), "bytes": Fraction(2)},
                target_body_cost=Decimal("3"),
            ),
        )

        cases = (
            (
                (*valid, DynamicOpcodeObservation(**valid[0].__dict__)),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "duplicate dynamic scenario ID",
            ),
            (
                tuple(row for row in valid if row.model_split == "fit"),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "at least one holdout",
            ),
            (
                (
                    valid[0],
                    DynamicOpcodeObservation(
                        **{**valid[1].__dict__, "features": {"constant": Fraction(1)}}
                    ),
                    valid[2],
                ),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "feature set",
            ),
            (
                (
                    DynamicOpcodeObservation(
                        **{
                            **valid[0].__dict__,
                            "features": {"constant": Fraction(2), "bytes": Fraction(0)},
                        }
                    ),
                    valid[1],
                    valid[2],
                ),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "constant.*Fraction\\(1",
            ),
            (
                (
                    valid[0],
                    DynamicOpcodeObservation(
                        **{
                            **valid[1].__dict__,
                            "features": {"constant": Fraction(1), "bytes": Fraction(0)},
                        }
                    ),
                    valid[2],
                ),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "exact rank",
            ),
            (
                (
                    DynamicOpcodeObservation(
                        **{**valid[0].__dict__, "target_body_cost": Decimal("NaN")}
                    ),
                    valid[1],
                    valid[2],
                ),
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("1"),
                "target body cost.*positive finite Decimal",
            ),
            (
                valid,
                {"copy": ("constant", "bytes")},
                Decimal("0"),
                Decimal("1"),
                "body_scale.*positive finite Decimal",
            ),
            (
                valid,
                {"copy": ("constant", "bytes")},
                Decimal("2"),
                Decimal("-1"),
                "common_overhead.*nonnegative finite Decimal",
            ),
        )
        for rows, feature_names, body_scale, common_overhead, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(
                ValueError, message
            ):
                fit_dynamic_opcode_models(
                    rows,
                    feature_names,
                    body_scale=body_scale,
                    common_overhead=common_overhead,
                )

    def test_dynamic_opcode_fit_rejects_non_fraction_features_and_invalid_keys(self):
        base = (
            DynamicOpcodeObservation(
                dynamic_key="hash",
                scenario_id="fit-0",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(0)},
                target_body_cost=Decimal("2"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="hash",
                scenario_id="fit-1",
                model_split="fit",
                features={"constant": Fraction(1), "words": Fraction(1)},
                target_body_cost=Decimal("3"),
            ),
            DynamicOpcodeObservation(
                dynamic_key="hash",
                scenario_id="holdout",
                model_split="holdout",
                features={"constant": Fraction(1), "words": Fraction(2)},
                target_body_cost=Decimal("4"),
            ),
        )
        invalid_feature = DynamicOpcodeObservation(
            **{**base[0].__dict__, "features": {"constant": Fraction(1), "words": 0}}
        )
        invalid_split = DynamicOpcodeObservation(
            **{**base[0].__dict__, "model_split": "training"}
        )
        invalid_scenario_id = DynamicOpcodeObservation(
            **{**base[0].__dict__, "scenario_id": 7}
        )

        for observations, feature_names, message in (
            ((invalid_feature, *base[1:]), {"hash": ("constant", "words")}, "Fraction"),
            (
                (invalid_split, *base[1:]),
                {"hash": ("constant", "words")},
                "fit or holdout",
            ),
            (
                (invalid_scenario_id, *base[1:]),
                {"hash": ("constant", "words")},
                "scenario ID.*nonempty string",
            ),
            (base, {"other": ("constant", "words")}, "dynamic keys"),
            (base, {"hash": ("words",)}, "constant"),
            (base, {"hash": ("constant", "constant")}, "duplicate feature name"),
        ):
            with self.subTest(message=message), self.assertRaisesRegex(
                ValueError, message
            ):
                fit_dynamic_opcode_models(
                    observations,
                    feature_names,
                    body_scale=Decimal("2"),
                    common_overhead=Decimal("1"),
                )

    def test_dynamic_opcode_fit_uses_isolated_decimal_context(self):
        observations = (
            DynamicOpcodeObservation(
                "log", "fit-0", "fit",
                {"constant": Fraction(1), "topics": Fraction(0)}, Decimal("1")
            ),
            DynamicOpcodeObservation(
                "log", "fit-1", "fit",
                {"constant": Fraction(1), "topics": Fraction(1)}, Decimal("3")
            ),
            DynamicOpcodeObservation(
                "log", "holdout", "holdout",
                {"constant": Fraction(1), "topics": Fraction(2)}, Decimal("5")
            ),
        )

        with localcontext() as hostile:
            hostile.prec = 7
            hostile.rounding = ROUND_DOWN
            hostile.traps[Inexact] = True

            evidence = fit_dynamic_opcode_models(
                observations,
                {"log": ("constant", "topics")},
                body_scale=Decimal("2"),
                common_overhead=Decimal("1"),
            )["log"]

            self.assertEqual(evidence.status, "supported")
            self.assertEqual(hostile.prec, 7)
            self.assertEqual(hostile.rounding, ROUND_DOWN)
            self.assertTrue(hostile.traps[Inexact])

    def test_block_fit_recovers_staged_transfer_parameters_and_multipliers(self):
        model = synthetic_affine_model()
        expected_anchors = dict(zip(ANCHORS, map(Decimal, ("10", "12", "10", "12"))))
        expected_fixed = dict(
            zip(FEATURES, map(Decimal, ("6000", "7000", "8000", "9000")))
        )

        result = fit_block_calibration(
            model, synthetic_block_rows(), FEATURES, ANCHOR_Q
        )

        self.assertLessEqual(
            abs(result.transfer_params["body_scale"] - Decimal("2")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(
                result.transfer_params["common_opcode_overhead_per_operation"]
                - Decimal("6")
            ),
            Decimal("1e-60"),
        )
        for key, expected in (*expected_anchors.items(), *expected_fixed.items()):
            actual = result.reconstructed_anchors.get(key, result.fixed_costs.get(key))
            self.assertLessEqual(abs(actual - expected), Decimal("1e-60"))
        self.assertEqual(result.opcode_multipliers["DERIVED"], Decimal("13"))
        self.assertEqual(len(result.opcode_multipliers), 102)
        self.assertTrue(all(value > 0 for value in result.opcode_multipliers.values()))
        self.assertEqual(result.transfer_exact_rank, 2)
        self.assertEqual(result.fixed_exact_rank, 4)
        self.assertEqual(len(result.fixed_exact_design_matrix), 40)
        self.assertEqual(
            result.transfer_exact_design_matrix[0],
            (Fraction(7), Fraction(1)),
        )
        self.assertTrue(
            all(
                isinstance(value, Fraction)
                for row in result.transfer_exact_design_matrix
                for value in row
            )
        )
        self.assertEqual(set(result.family_slope_evidence), set(OPCODE_FAMILIES))
        self.assertEqual(set(result.opcode_holdout_evidence), set(OPCODE_FAMILIES))
        self.assertEqual(
            set(result.transfer_leave_one_family_out), set(OPCODE_FAMILIES)
        )
        self.assertTrue(
            all(
                row["slope_ape"] <= Decimal("1e-60")
                for row in result.family_slope_evidence.values()
            )
        )
        self.assertEqual(
            result.parameter_order,
            ("body_scale", "common_opcode_overhead_per_operation", *FEATURES),
        )
        self.assertEqual(result.status, "accepted")

    def test_block_fit_subtracts_mu_zero_family_slope_before_transfer(self):
        result = fit_block_calibration(
            synthetic_affine_model(),
            synthetic_block_rows(derived_units_per_count=3),
            FEATURES,
            ANCHOR_Q,
        )

        self.assertLessEqual(
            abs(result.transfer_params["body_scale"] - Decimal("2")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(
                result.transfer_params["common_opcode_overhead_per_operation"]
                - Decimal("6")
            ),
            Decimal("1e-60"),
        )
        self.assertEqual(
            result.family_slope_evidence["pop_family"]["mu_zero_slope"],
            Decimal("15"),
        )

    def test_block_fit_uses_isolated_precision_without_mutating_caller_context(self):
        with localcontext() as hostile:
            hostile.prec = 7
            hostile.rounding = ROUND_DOWN
            hostile.traps[Inexact] = True

            result = fit_block_calibration(
                synthetic_affine_model(), synthetic_block_rows(), FEATURES, ANCHOR_Q
            )

            self.assertLessEqual(
                abs(result.transfer_params["body_scale"] - Decimal("2")),
                Decimal("1e-60"),
            )
            self.assertEqual(hostile.prec, 7)
            self.assertEqual(hostile.rounding, ROUND_DOWN)
            self.assertTrue(hostile.traps[Inexact])

    def test_block_fit_rejects_transfer_rank_one_and_fixed_rank_three(self):
        with self.assertRaisesRegex(ValueError, "transfer.*rank.*two"):
            fit_block_calibration(
                synthetic_affine_model(),
                synthetic_block_rows(),
                FEATURES,
                {key: Decimal("7") for key in ANCHORS},
            )

        rows = synthetic_block_rows()
        rank_three = []
        for row in rows:
            features = dict(row.feature_counts)
            features[FEATURES[-1]] = 0
            if row.workload_family == FEATURES[-1]:
                features[FEATURES[0]] = row.feature_counts[FEATURES[-1]]
                prover_gas = Decimal(features[FEATURES[0]]) * Decimal("6000")
            else:
                prover_gas = row.prover_gas
            rank_three.append(
                BlockCalibrationRow(
                    **{
                        **row.__dict__,
                        "feature_counts": features,
                        "prover_gas": prover_gas,
                    }
                )
            )
        with self.assertRaisesRegex(ValueError, "fixed.*rank.*four"):
            fit_block_calibration(
                synthetic_affine_model(), rank_three, FEATURES, ANCHOR_Q
            )

    def test_block_fit_rejects_negative_reconstructed_multiplier(self):
        with self.assertRaisesRegex(ValueError, "opcode multiplier must be positive"):
            fit_block_calibration(
                synthetic_affine_model(derived_zero=Decimal("-20")),
                synthetic_block_rows(),
                FEATURES,
                ANCHOR_Q,
            )

    def test_block_fit_rejects_invalid_anchor_q_and_transfer_parameters(self):
        invalid_q_cases = (
            ({key: value for key, value in ANCHOR_Q.items() if key != ANCHORS[-1]}, "exactly"),
            ({**ANCHOR_Q, "extra": Decimal("1")}, "exactly"),
            ({**ANCHOR_Q, ANCHORS[0]: Decimal("-1")}, "positive finite"),
            ({**ANCHOR_Q, ANCHORS[0]: Decimal("NaN")}, "positive finite"),
        )
        for anchor_q, message in invalid_q_cases:
            with self.subTest(anchor_q=anchor_q), self.assertRaisesRegex(
                ValueError, message
            ):
                fit_block_calibration(
                    synthetic_affine_model(), synthetic_block_rows(), FEATURES, anchor_q
                )

        with self.assertRaisesRegex(ValueError, "body_scale.*positive"):
            fit_block_calibration(
                synthetic_affine_model(),
                synthetic_block_rows(
                    body_scale=Decimal("-1"),
                    common_opcode_overhead_per_operation=Decimal("20"),
                ),
                FEATURES,
                ANCHOR_Q,
            )
        with self.assertRaisesRegex(ValueError, "common_opcode_overhead.*nonnegative"):
            fit_block_calibration(
                synthetic_affine_model(),
                synthetic_block_rows(
                    common_opcode_overhead_per_operation=Decimal("-1")
                ),
                FEATURES,
                ANCHOR_Q,
            )

        zero_common = fit_block_calibration(
            synthetic_affine_model(),
            synthetic_block_rows(common_opcode_overhead_per_operation=Decimal("0")),
            FEATURES,
            ANCHOR_Q,
        )
        self.assertEqual(
            zero_common.transfer_params["common_opcode_overhead_per_operation"],
            Decimal("0"),
        )

    def test_block_fit_rejects_invalid_workload_counts(self):
        rows = synthetic_block_rows()
        opcode_index = next(
            index for index, row in enumerate(rows) if row.workload_family in OPCODE_FAMILIES
        )
        rows[opcode_index] = BlockCalibrationRow(
            **{**rows[opcode_index].__dict__, "workload_count": None}
        )
        with self.assertRaisesRegex(ValueError, "opcode family.*positive workload_count"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

        rows = synthetic_block_rows()
        base_index = next(
            index for index, row in enumerate(rows) if row.workload_family in FEATURES
        )
        rows[base_index] = BlockCalibrationRow(
            **{**rows[base_index].__dict__, "workload_count": 1}
        )
        with self.assertRaisesRegex(ValueError, "base family.*workload_count.*None"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

        rows = synthetic_block_rows()
        opcode_index = next(
            index for index, row in enumerate(rows) if row.workload_family == "pop_family"
        )
        changed_features = dict(rows[opcode_index].feature_counts)
        changed_features[FEATURES[0]] += 1
        rows[opcode_index] = BlockCalibrationRow(
            **{**rows[opcode_index].__dict__, "feature_counts": changed_features}
        )
        with self.assertRaisesRegex(ValueError, "fixed/base features must remain constant"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_block_fit_rejects_family_slope_error(self):
        rows = synthetic_block_rows(family_slope_offsets={"pop_family": 10})
        with self.assertRaisesRegex(ValueError, "family slope APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_block_fit_rejects_opcode_holdout_delta_signal_error(self):
        rows = synthetic_block_rows(holdout_offsets={"pop_family": 1000})
        with self.assertRaisesRegex(ValueError, "opcode holdout delta-signal APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_block_fit_rejects_fit_mape_and_max_ape_gates(self):
        rows = synthetic_block_rows(family_startups={"pop_family": 3000000})
        with self.assertRaisesRegex(ValueError, "fit (MAPE|maximum APE)"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_block_fit_rejects_holdout_max_ape_gate(self):
        rows = synthetic_block_rows()
        rows[-1] = BlockCalibrationRow(
            **{**rows[-1].__dict__, "prover_gas": rows[-1].prover_gas * 2}
        )
        with self.assertRaisesRegex(ValueError, "holdout maximum APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_block_fit_rejects_nonpositive_or_nonfinite_actuals(self):
        for invalid in (Decimal("0"), Decimal("-1"), Decimal("NaN")):
            rows = synthetic_block_rows()
            rows[0] = BlockCalibrationRow(
                **{**rows[0].__dict__, "prover_gas": invalid}
            )
            with self.subTest(invalid=invalid), self.assertRaisesRegex(
                ValueError, "positive finite"
            ):
                fit_block_calibration(
                    synthetic_affine_model(), rows, FEATURES, ANCHOR_Q
                )

    def test_block_fit_rejects_transfer_leave_one_family_out_slope_error(self):
        rows = synthetic_block_rows(family_slope_offsets={"pop_family": 3})
        with self.assertRaisesRegex(ValueError, "leave-one-family-out.*slope APE"):
            fit_block_calibration(synthetic_affine_model(), rows, FEATURES, ANCHOR_Q)

    def test_staged_fit_avoids_joint_startup_compensation_with_negative_common(self):
        startups = {
            "pop_family": 500000,
            "push_family": -500000,
            "dup_family": -500000,
            "swap_family": 500000,
        }
        rows = synthetic_block_rows(family_startups=startups)
        fit_rows = [row for row in rows if row.split == "fit"]
        joint_matrix, offsets, actuals = calibration_model._block_design(
            synthetic_affine_model(), fit_rows, FEATURES, ANCHOR_Q
        )
        joint, _scales, _residual = calibration_model._scaled_decimal_least_squares(
            joint_matrix,
            [actual - offset for actual, offset in zip(actuals, offsets)],
        )
        self.assertLess(joint[1], 0)

        result = fit_block_calibration(
            synthetic_affine_model(), rows, FEATURES, ANCHOR_Q
        )

        self.assertLessEqual(
            abs(result.transfer_params["body_scale"] - Decimal("2")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(
                result.transfer_params["common_opcode_overhead_per_operation"]
                - Decimal("6")
            ),
            Decimal("1e-60"),
        )

    def test_dynamic_holdouts_return_evidence_without_refitting(self):
        model = synthetic_affine_model()
        multipliers = model.reconstruct_multipliers(
            dict(zip(ANCHORS, map(Decimal, ("2", "3", "4", "5"))))
        )
        observations = tuple(
            DynamicRelationObservation(
                dynamic_key=ANCHORS[0],
                scenario_id=scenario,
                split=split,
                equation=RelationEquation(
                    scenario,
                    {ANCHORS[0]: Fraction(raw), ANCHORS[1]: Fraction(-1)},
                    Decimal(raw * 2 - 3),
                ),
            )
            for scenario, split, raw in (
                ("canonical", "canonical", 1),
                ("medium", "dynamic_holdout", 2),
                ("large", "dynamic_holdout", 4),
                ("larger", "dynamic_holdout", 8),
            )
        )

        evidence = validate_dynamic_holdouts(
            model, multipliers, observations, (ANCHORS[0],)
        )

        self.assertEqual(evidence[ANCHORS[0]]["status"], "accepted")
        self.assertEqual(multipliers[ANCHORS[0]], Decimal("2"))
        self.assertEqual(
            [
                row["implied_multiplier"]
                for row in evidence[ANCHORS[0]]["observations"]
            ],
            [Decimal("2"), Decimal("2"), Decimal("2"), Decimal("2")],
        )

    def test_dynamic_holdouts_reject_shape_sign_ape_and_consistency_failures(self):
        model = synthetic_affine_model()
        multipliers = model.reconstruct_multipliers(
            dict(zip(ANCHORS, map(Decimal, ("2", "3", "4", "5"))))
        )

        def observations(slopes=(Decimal("-1"), Decimal("1"), Decimal("5"))):
            return tuple(
                DynamicRelationObservation(
                    dynamic_key=ANCHORS[0],
                    scenario_id=scenario,
                    split=split,
                    equation=RelationEquation(
                        scenario,
                        {ANCHORS[0]: Fraction(raw), ANCHORS[1]: Fraction(-1)},
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
                dynamic_key=ANCHORS[0],
                scenario_id=scenario,
                split=split,
                equation=RelationEquation(
                    scenario,
                    {ANCHORS[0]: Fraction(raw), ANCHORS[1]: Fraction(-100)},
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
            (observations()[:1], "one canonical and at least one"),
            (observations((Decimal("1"), Decimal("1"), Decimal("5"))), "predicted sign"),
            (observations((Decimal("-1"), Decimal("2"), Decimal("5"))), "relation APE"),
            (inconsistent, "implied multiplier consistency"),
        )
        for rows, message in cases:
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                validate_dynamic_holdouts(model, multipliers, rows, (ANCHORS[0],))

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
