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
    fit_nonnegative_opcode_bodies,
    fit_structured_dynamic_opcode_models,
    nonnegative_decimal_least_squares,
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
    def _structured_dynamic_observations(self):
        memory_coefficients = {
            "memory_growth_event": Decimal("2"),
            "memory_evm_gas_delta": Decimal("3"),
            "memory_4k_boundary_event": Decimal("5"),
        }
        constants = {
            "opcode:0x51": Decimal("10"),
            "opcode:0x52": Decimal("20"),
            "opcode:0x53": Decimal("30"),
        }

        def memory_target(key, growth_event, gas_delta, boundary_event):
            return (
                constants[key]
                + Decimal(growth_event) * memory_coefficients["memory_growth_event"]
                + Decimal(gas_delta) * memory_coefficients["memory_evm_gas_delta"]
                + Decimal(boundary_event)
                * memory_coefficients["memory_4k_boundary_event"]
            )

        observations = []
        memory_fit = ((0, 0, 0), (1, 3, 0), (1, 100, 1), (1, 200, 1))
        for key in constants:
            for index, (growth_event, gas_delta, boundary_event) in enumerate(
                memory_fit
            ):
                observations.append(
                    DynamicOpcodeObservation(
                        key,
                        f"{key}-fit-{index}",
                        "fit",
                        {
                            "constant": Fraction(1),
                            "memory_growth_event": Fraction(growth_event),
                            "memory_evm_gas_delta": Fraction(gas_delta),
                            "memory_4k_boundary_event": Fraction(boundary_event),
                        },
                        memory_target(key, growth_event, gas_delta, boundary_event),
                    )
                )
            observations.append(
                DynamicOpcodeObservation(
                    key,
                    f"{key}-holdout",
                    "holdout",
                    {
                        "constant": Fraction(1),
                        "memory_growth_event": Fraction(1),
                        "memory_evm_gas_delta": Fraction(300),
                        "memory_4k_boundary_event": Fraction(1),
                    },
                    memory_target(key, 1, 300, 1),
                )
            )

        for key, word_name, constant, word_coefficient in (
            (
                "opcode:0x20",
                "keccak_permutations",
                Decimal("40"),
                Decimal("4"),
            ),
            ("opcode:0x5e", "copy_words", Decimal("50"), Decimal("6")),
        ):
            for index, (
                split,
                words,
                growth_event,
                gas_delta,
                boundary_event,
            ) in enumerate(
                (
                    ("fit", 0, 0, 0, 0),
                    ("fit", 8, 1, 21, 0),
                    ("fit", 32, 1, 101, 1),
                    ("holdout", 64, 1, 205, 1),
                )
            ):
                shared = (
                    Decimal(growth_event) * Decimal("2")
                    + Decimal(gas_delta) * Decimal("3")
                    + Decimal(boundary_event) * Decimal("5")
                )
                zero_length_event = int(key == "opcode:0x20" and words == 0)
                zero_length_adjustment = Decimal("-5")
                features = {
                    "constant": Fraction(1),
                    word_name: Fraction(words),
                    "memory_growth_event": Fraction(growth_event),
                    "memory_evm_gas_delta": Fraction(gas_delta),
                    "memory_4k_boundary_event": Fraction(boundary_event),
                }
                if key == "opcode:0x20":
                    features["keccak_zero_length_event"] = Fraction(
                        zero_length_event
                    )
                observations.append(
                    DynamicOpcodeObservation(
                        key,
                        f"{key}-{index}",
                        split,
                        features,
                        constant
                        + Decimal(words) * word_coefficient
                        + Decimal(zero_length_event) * zero_length_adjustment
                        + shared,
                    )
                )

        exp_rows = (
            ("fit", 0, Decimal("40")),
            ("fit", 1, Decimal("43")),
            ("fit", 2, Decimal("50")),
            ("fit", 4, Decimal("60")),
            ("fit", 8, Decimal(7 + 2 * 8 + 3 * 8**2)),
            ("fit", 16, Decimal(7 + 2 * 16 + 3 * 16**2)),
            ("holdout", 24, Decimal(7 + 2 * 24 + 3 * 24**2)),
            ("fit", 32, Decimal(7 + 2 * 32 + 3 * 32**2)),
        )
        for index, (split, exponent_bytes, target) in enumerate(exp_rows):
            observations.append(
                DynamicOpcodeObservation(
                    "opcode:0x0a",
                    f"opcode:0x0a-{index}",
                    split,
                    {
                        "constant": Fraction(1),
                        "exponent_bytes": Fraction(exponent_bytes),
                        "exponent_bytes_squared": Fraction(exponent_bytes**2),
                    },
                    target,
                )
            )
        return tuple(observations)

    def test_structured_dynamic_fit_shares_memory_coefficients_and_overhead(self):
        result = fit_structured_dynamic_opcode_models(
            self._structured_dynamic_observations(),
            body_scale=Decimal("2"),
            common_overhead=Decimal("11"),
        )

        self.assertEqual(result.status, "supported")
        self.assertEqual(result.aggregate_exact_rank, 14)
        self.assertEqual(result.aggregate_parameter_count, 14)
        keccak = result.opcode_models["opcode:0x20"]
        self.assertEqual(keccak.exact_rank, 3)
        self.assertEqual(
            keccak.exact_fit_design_matrix[0],
            (Fraction(1), Fraction(1), Fraction(0)),
        )
        self.assertLessEqual(
            abs(keccak.body_coefficients["constant"] - Decimal("40")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(keccak.body_coefficients["keccak_permutations"] - Decimal("4")),
            Decimal("1e-60"),
        )
        self.assertLessEqual(
            abs(
                keccak.body_coefficients["keccak_zero_length_event"]
                - Decimal("-5")
            ),
            Decimal("1e-60"),
        )
        memory = result.shared_memory_model
        self.assertEqual(memory.exact_rank, 6)
        self.assertEqual(
            memory.parameter_order,
            (
                "opcode:0x51:constant",
                "opcode:0x52:constant",
                "opcode:0x53:constant",
                "memory_growth_event",
                "memory_evm_gas_delta",
                "memory_4k_boundary_event",
            ),
        )
        for name, expected in {
            "opcode:0x51:constant": Decimal("10"),
            "opcode:0x52:constant": Decimal("20"),
            "opcode:0x53:constant": Decimal("30"),
            "memory_growth_event": Decimal("2"),
            "memory_evm_gas_delta": Decimal("3"),
            "memory_4k_boundary_event": Decimal("5"),
        }.items():
            self.assertLessEqual(
                abs(memory.body_coefficients[name] - expected), Decimal("1e-60")
            )
        for name, expected in {
            "opcode:0x51:constant": Decimal("31"),
            "opcode:0x52:constant": Decimal("51"),
            "opcode:0x53:constant": Decimal("71"),
            "memory_growth_event": Decimal("4"),
            "memory_evm_gas_delta": Decimal("6"),
            "memory_4k_boundary_event": Decimal("10"),
        }.items():
            self.assertLessEqual(
                abs(memory.production_coefficients[name] - expected),
                Decimal("1e-60"),
            )

        exp = result.opcode_models["opcode:0x0a"]
        self.assertEqual(exp.small_bucket_body, Decimal("60"))
        self.assertEqual(exp.low_domain_byte_lengths, (0, 1, 2, 4))
        self.assertEqual(exp.fit_count, 3)
        self.assertEqual(exp.holdout_count, 1)
        self.assertEqual(exp.low_domain_count, 4)
        self.assertEqual(
            {
                prediction["predicted_body_cost"]
                for prediction in exp.low_domain_predictions.values()
            },
            {Decimal("60")},
        )

    def test_structured_exp_large_holdout_failure_still_blocks_support(self):
        observations = list(self._structured_dynamic_observations())
        index = next(
            index
            for index, row in enumerate(observations)
            if row.dynamic_key == "opcode:0x0a" and row.model_split == "holdout"
        )
        observations[index] = DynamicOpcodeObservation(
            **{
                **observations[index].__dict__,
                "target_body_cost": observations[index].target_body_cost
                * Decimal("2"),
            }
        )

        result = fit_structured_dynamic_opcode_models(
            observations,
            body_scale=Decimal("2"),
            common_overhead=Decimal("11"),
        )

        exp = result.opcode_models["opcode:0x0a"]
        self.assertEqual(exp.status, "not_supported")
        self.assertIn("holdout_max_ape", exp.quality_failures)

    def test_structured_exp_requires_exact_low_domain(self):
        observations = tuple(
            row
            for row in self._structured_dynamic_observations()
            if not (
                row.dynamic_key == "opcode:0x0a"
                and row.features["exponent_bytes"] == Fraction(4)
            )
        )

        with self.assertRaisesRegex(ValueError, "low-domain.*0, 1, 2, 4"):
            fit_structured_dynamic_opcode_models(
                observations,
                body_scale=Decimal("2"),
                common_overhead=Decimal("11"),
            )

    def test_dynamic_prediction_aggregates_use_prediction_id_order(self):
        observations = (
            DynamicOpcodeObservation(
                "opcode:0x0a",
                "large",
                "fit",
                {"constant": Fraction(1)},
                Decimal(1),
            ),
            DynamicOpcodeObservation(
                "opcode:0x0a",
                "small-a",
                "fit",
                {"constant": Fraction(1)},
                Decimal(1),
            ),
            DynamicOpcodeObservation(
                "opcode:0x0a",
                "small-b",
                "fit",
                {"constant": Fraction(1)},
                Decimal(1),
            ),
            DynamicOpcodeObservation(
                "opcode:0x0a",
                "holdout",
                "holdout",
                {"constant": Fraction(1)},
                Decimal(1),
            ),
        )
        predicted_body_costs = (
            Decimal("1e80"),
            Decimal(5),
            Decimal(5),
            Decimal(1),
        )
        prediction_ids = ("z-large", "a-small", "b-small", "h-holdout")
        reordered = (1, 2, 0, 3)

        with localcontext(calibration_model._CALIBRATION_DECIMAL_CONTEXT):
            original = calibration_model._dynamic_prediction_evidence(
                observations,
                predicted_body_costs,
                body_scale=Decimal(1),
                common_overhead=Decimal(0),
                prediction_ids=prediction_ids,
            )
            permuted = calibration_model._dynamic_prediction_evidence(
                tuple(observations[index] for index in reordered),
                tuple(predicted_body_costs[index] for index in reordered),
                body_scale=Decimal(1),
                common_overhead=Decimal(0),
                prediction_ids=tuple(
                    prediction_ids[index] for index in reordered
                ),
            )

        self.assertEqual(original[1:], permuted[1:])

    def test_structured_dynamic_fit_rejects_rank_deficient_shared_memory(self):
        observations = tuple(
            DynamicOpcodeObservation(
                row.dynamic_key,
                row.scenario_id,
                row.model_split,
                {
                    **row.features,
                    "memory_4k_boundary_event": Fraction(0),
                },
                row.target_body_cost,
            )
            if row.dynamic_key in {"opcode:0x51", "opcode:0x52", "opcode:0x53"}
            else row
            for row in self._structured_dynamic_observations()
        )

        with self.assertRaisesRegex(ValueError, "shared memory.*exact rank"):
            fit_structured_dynamic_opcode_models(
                observations,
                body_scale=Decimal("2"),
                common_overhead=Decimal("11"),
            )

    def test_structured_dynamic_fit_rejects_invalid_observation_type(self):
        with self.assertRaisesRegex(ValueError, "invalid type"):
            fit_structured_dynamic_opcode_models(
                (*self._structured_dynamic_observations(), object()),
                body_scale=Decimal("2"),
                common_overhead=Decimal("11"),
            )

    def test_structured_dynamic_holdout_failure_keeps_evidence(self):
        observations = list(self._structured_dynamic_observations())
        index = next(
            index
            for index, row in enumerate(observations)
            if row.dynamic_key == "opcode:0x51" and row.model_split == "holdout"
        )
        observations[index] = DynamicOpcodeObservation(
            **{
                **observations[index].__dict__,
                "target_body_cost": observations[index].target_body_cost * Decimal("2"),
            }
        )

        result = fit_structured_dynamic_opcode_models(
            observations,
            body_scale=Decimal("2"),
            common_overhead=Decimal("11"),
        )

        self.assertEqual(result.status, "not_supported")
        self.assertEqual(result.shared_memory_model.status, "not_supported")
        self.assertIn("holdout_max_ape", result.shared_memory_model.quality_failures)
        self.assertIn(
            "opcode:0x51/opcode:0x51-holdout",
            result.shared_memory_model.predictions,
        )

    def test_nonpositive_operation_fit_target_is_not_supported_evidence(self):
        observations = list(self._structured_dynamic_observations())
        index = next(
            index
            for index, row in enumerate(observations)
            if row.dynamic_key == "opcode:0x20"
            and row.model_split == "fit"
            and row.features["memory_growth_event"] == Fraction(1)
        )
        observations[index] = DynamicOpcodeObservation(
            **{
                **observations[index].__dict__,
                "target_body_cost": Decimal("1"),
            }
        )

        result = fit_structured_dynamic_opcode_models(
            observations,
            body_scale=Decimal("2"),
            common_overhead=Decimal("11"),
        )

        model = result.opcode_models["opcode:0x20"]
        self.assertEqual(result.status, "not_supported")
        self.assertEqual(model.status, "not_supported")
        self.assertIn("nonpositive_operation_fit_target", model.quality_failures)
        self.assertEqual(
            set(model.predictions),
            {
                row.scenario_id
                for row in observations
                if row.dynamic_key == "opcode:0x20"
            },
        )
        self.assertTrue(
            all(
                value.is_finite()
                for prediction in model.predictions.values()
                for name, value in prediction.items()
                if name != "model_split"
            )
        )

    def test_structured_keccak_and_mcopy_predictions_reuse_shared_memory(self):
        result = fit_structured_dynamic_opcode_models(
            self._structured_dynamic_observations(),
            body_scale=Decimal("2"),
            common_overhead=Decimal("11"),
        )

        for key in ("opcode:0x20", "opcode:0x5e"):
            model = result.opcode_models[key]
            prediction = next(
                row for row in model.predictions.values()
                if row["model_split"] == "holdout"
            )
            self.assertGreater(prediction["shared_memory_body_cost"], Decimal(0))
            self.assertLessEqual(
                abs(
                    prediction["predicted_body_cost"]
                    - prediction["predicted_operation_body_cost"]
                    - prediction["shared_memory_body_cost"]
                ),
                Decimal("1e-60"),
            )
            self.assertLessEqual(prediction["body_ape"], Decimal("1e-60"))

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

    def test_nonnegative_least_squares_refits_after_activating_zero_bound(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(1), Fraction(0)),
                (Fraction(0), Fraction(1)),
                (Fraction(1), Fraction(1)),
            ),
            (Decimal("1"), Decimal("-1"), Decimal("0")),
        )

        self.assertEqual(result.solution, (Decimal("0.5"), Decimal("0")))
        self.assertEqual(result.active_zero_indices, (1,))

    def test_nonnegative_least_squares_removes_inexact_boundary_coordinate(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(1), Fraction(0)),
                (Fraction(-4), Fraction(-3)),
            ),
            (Decimal("-3"), Decimal("-4")),
        )

        self.assertEqual(
            result.solution,
            (
                Decimal("0"),
                Decimal("1.33333333333333333333333333333333333333333333333333333333333"),
            ),
        )
        self.assertEqual(result.active_zero_indices, (0,))

    def test_nonnegative_least_squares_ignores_flat_kkt_dual_roundoff(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(3), Fraction(1)),
                (Fraction(0), Fraction(0)),
                (Fraction(0), Fraction(1)),
            ),
            (Decimal("1"), Decimal("1"), Decimal("0")),
        )

        self.assertEqual(
            result.solution,
            (
                Decimal("0.333333333333333333333333333333333333333333333333333333333333"),
                Decimal("0"),
            ),
        )
        self.assertEqual(result.active_zero_indices, (1,))

    def test_nonnegative_least_squares_uses_passive_kkt_roundoff_bound(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(4), Fraction(3)),
                (Fraction(4), Fraction(5)),
            ),
            (Decimal("9"), Decimal("9")),
        )

        self.assertEqual(result.solution, (Decimal("2.25"), Decimal("0")))
        self.assertEqual(result.active_zero_indices, (1,))

    def test_nonnegative_least_squares_keeps_precision_edge_positive_dual(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(3), Fraction(1)),
                (Fraction(0), Fraction(1)),
            ),
            (Decimal("1"), Decimal("1e-79")),
        )

        self.assertEqual(result.solution[1], Decimal("1e-79"))
        self.assertEqual(result.active_zero_indices, ())

    def test_nonnegative_least_squares_terminates_repeated_decimal_state(self):
        edge_target = Decimal("6." + "0" * 78 + "2")
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(5), Fraction(3)),
                (Fraction(2), Fraction(4)),
            ),
            (edge_target, Decimal("8." + "0" * 78 + "2")),
        )

        self.assertEqual(result.solution, (Decimal("0"), Decimal("2")))
        self.assertEqual(result.active_zero_indices, (0,))

    def test_nonnegative_least_squares_restarts_after_empty_boundary(self):
        result = nonnegative_decimal_least_squares(
            (
                (Fraction(-3),),
                (Fraction(8),),
                (Fraction(-7),),
                (Fraction(4),),
            ),
            (
                Decimal("-7." + "9" * 79),
                Decimal("-2." + "9" * 79),
                Decimal("4." + "0" * 78 + "2"),
                Decimal("7." + "0" * 78 + "5"),
            ),
        )

        self.assertEqual(result.solution, (Decimal("0"),))
        self.assertEqual(result.active_zero_indices, (0,))

    def test_nonnegative_least_squares_residual_matches_reported_solution(self):
        result = nonnegative_decimal_least_squares(
            ((Fraction(3),),), (Decimal("1"),)
        )

        with localcontext(calibration_model._CALIBRATION_DECIMAL_CONTEXT):
            self.assertEqual(
                result.residual,
                abs(Decimal(3) * result.solution[0] - Decimal(1)),
            )

    def test_nonnegative_opcode_fit_preserves_fixed_anchor_body_cost(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="target-minus-anchor",
                    coefficients={
                        "opcode:0x01": Fraction(3),
                        "opcode:0x50": Fraction(-2),
                    },
                    slope=Decimal("9"),
                ),
            ),
            opcode_keys=("opcode:0x01", "opcode:0x50"),
            anchor_body_costs={"opcode:0x50": Decimal("6")},
        )

        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x50"], Decimal("3"))
        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x01"], Decimal("5"))

    def test_nonnegative_opcode_fit_accepts_exact_flat_signed_relation(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="dup2-minus-dup1",
                    coefficients={
                        "opcode:0x80": Fraction(-3),
                        "opcode:0x81": Fraction(3),
                    },
                    slope=Decimal("0"),
                ),
            ),
            opcode_keys=("opcode:0x80", "opcode:0x81"),
            anchor_body_costs={"opcode:0x80": Decimal("9")},
        )

        self.assertEqual(result.status, "supported")
        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x80"], Decimal("3"))
        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x81"], Decimal("3"))
        self.assertEqual(result.flat_relation_max_normalized_error, Decimal("0"))

    def test_nonnegative_opcode_fit_rejects_inconsistent_fixed_anchor_flat_relation(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="push0-minus-pop",
                    coefficients={
                        "opcode:0x50": Fraction(-2),
                        "opcode:0x5f": Fraction(2),
                    },
                    slope=Decimal("0"),
                ),
            ),
            opcode_keys=("opcode:0x50", "opcode:0x5f"),
            anchor_body_costs={
                "opcode:0x50": Decimal("4"),
                "opcode:0x5f": Decimal("10"),
            },
        )

        self.assertEqual(result.status, "not_supported")
        self.assertIn("flat_relation_max_normalized_error", result.quality_failures)

    def test_declared_dispatch_only_body_is_fixed_zero_and_residual_is_diagnostic(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="not",
                    coefficients={"opcode:0x19": Fraction(1)},
                    slope=Decimal("1000"),
                ),
                RelationEquation(
                    relation_id="add",
                    coefficients={"opcode:0x01": Fraction(1)},
                    slope=Decimal("2"),
                ),
            ),
            opcode_keys=("opcode:0x01", "opcode:0x19"),
            anchor_body_costs={},
            dispatch_only_keys=("opcode:0x19",),
        )

        self.assertEqual(result.status, "supported")
        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x19"], Decimal(0))
        self.assertEqual(result.dispatch_only_keys, ("opcode:0x19",))
        self.assertEqual(result.approximation_relation_ids, ("not",))
        self.assertEqual(result.approximation_relation_count, 1)
        self.assertEqual(
            result.maximum_absolute_approximation_residual, Decimal("1000")
        )
        self.assertEqual(
            result.predictions["not"]["gate"],
            "declared_approximation",
        )
        self.assertEqual(
            result.predictions["not"]["absolute_residual"],
            Decimal("1000"),
        )

    def test_dispatch_only_mixed_relation_cannot_transfer_cost_into_target_fit(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="target-clean",
                    coefficients={"opcode:0x01": Fraction(1)},
                    slope=Decimal("2"),
                ),
                RelationEquation(
                    relation_id="target-minus-not",
                    coefficients={
                        "opcode:0x01": Fraction(1),
                        "opcode:0x19": Fraction(-1),
                    },
                    slope=Decimal("100"),
                ),
            ),
            opcode_keys=("opcode:0x01", "opcode:0x19"),
            anchor_body_costs={},
            dispatch_only_keys=("opcode:0x19",),
        )

        self.assertEqual(result.status, "supported")
        self.assertEqual(result.lab_body_per_raw_gas["opcode:0x01"], Decimal("2"))
        self.assertEqual(
            result.predictions["target-minus-not"]["gate"],
            "declared_approximation",
        )
        self.assertEqual(
            result.predictions["target-minus-not"]["absolute_residual"],
            Decimal("98"),
        )

    def test_ordinary_relation_residual_still_blocks_support(self):
        result = fit_nonnegative_opcode_bodies(
            equations=(
                RelationEquation(
                    relation_id="ordinary-a",
                    coefficients={"opcode:0x01": Fraction(1)},
                    slope=Decimal("1000"),
                ),
                RelationEquation(
                    relation_id="ordinary-b",
                    coefficients={"opcode:0x01": Fraction(1)},
                    slope=Decimal("2"),
                ),
                RelationEquation(
                    relation_id="not",
                    coefficients={"opcode:0x19": Fraction(1)},
                    slope=Decimal("1"),
                ),
            ),
            opcode_keys=("opcode:0x01", "opcode:0x19"),
            anchor_body_costs={},
            dispatch_only_keys=("opcode:0x19",),
        )

        self.assertEqual(result.status, "not_supported")
        self.assertIn("nonzero_relation_max_ape", result.quality_failures)

    def test_rejects_malformed_dispatch_only_policy(self):
        equation = RelationEquation(
            relation_id="add",
            coefficients={"opcode:0x01": Fraction(1)},
            slope=Decimal("2"),
        )
        cases = (
            (("opcode:0xff",), {}, "unknown dispatch-only"),
            (("opcode:0x50",), {"opcode:0x50": Decimal("4")}, "anchor"),
            (("opcode:0x19", "opcode:0x19"), {}, "duplicate dispatch-only"),
            (("opcode:0x19",), {}, "absent dispatch-only"),
        )
        for dispatch_only_keys, anchors, message in cases:
            with self.subTest(
                dispatch_only_keys=dispatch_only_keys
            ), self.assertRaisesRegex(ValueError, message):
                fit_nonnegative_opcode_bodies(
                    equations=(equation,),
                    opcode_keys=("opcode:0x01", "opcode:0x19", "opcode:0x50"),
                    anchor_body_costs=anchors,
                    dispatch_only_keys=dispatch_only_keys,
                )

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

    def test_current_matched_controls_have_rank_99_with_natural_anchors(self):
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

        self.assertEqual(len(opcode_keys), 103)
        self.assertEqual(len(equations), 99)
        self.assertEqual(exact_rank(coefficient_rows), 99)
        self.assertEqual(
            model.anchor_keys,
            ("opcode:0x50", "opcode:0x5f", "opcode:0x80", "opcode:0x90"),
        )


if __name__ == "__main__":
    unittest.main()
