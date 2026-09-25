import copy
import pathlib
import sys
import types
import unittest
from decimal import Decimal
from fractions import Fraction


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

import opcode_gas
from hierarchical_model import (
    ModelKind,
    ModelSpec,
    OpcodeEvent,
    OpcodeRegistry,
    predict_opcode_event,
    validate_core_registry,
)


MEMORY_PARAMETERS = {
    "memory_growth_event": Decimal("2"),
    "memory_evm_gas_delta": Decimal("3"),
    "memory_4k_boundary_event": Decimal("7"),
}


def _seal_artifact(payload):
    payload = {
        key: value for key, value in payload.items() if key != "artifact_sha256"
    }
    payload["artifact_sha256"] = opcode_gas.sha256_bytes(
        opcode_gas.canonical_json(payload)
    )
    return payload


def _core_submodel_sources():
    def dynamic_evidence(parameter_order, body, production, *, shared=False):
        parameter_count = len(parameter_order)
        fit_count = parameter_count
        holdout_count = 1
        prediction = {
            "model_split": "fit",
            "actual_body_cost": "1",
            "predicted_body_cost": "1",
            "body_ape": "0",
            "actual_production_cost": "9",
            "predicted_production_cost": "9",
            "production_ape": "0",
        }
        if not shared:
            prediction.update(
                predicted_operation_body_cost="1",
                shared_memory_body_cost="0",
            )
        return {
            "status": "supported",
            "parameter_order": list(parameter_order),
            "exact_fit_rank": parameter_count,
            "parameter_count": parameter_count,
            "observation_count": fit_count + holdout_count,
            "fit_count": fit_count,
            "holdout_count": holdout_count,
            "body_coefficients": body,
            "production_coefficients": production,
            "fit_body_mape": "0",
            "fit_body_max_ape": "0",
            "holdout_body_max_ape": "0",
            "fit_production_mape": "0",
            "fit_production_max_ape": "0",
            "holdout_production_max_ape": "0",
            "quality_failures": [],
            "exact_fit_matrix": [
                ["1" if row == column else "0" for column in range(parameter_count)]
                for row in range(fit_count)
            ],
            "solver_column_scales": ["1"] * parameter_count,
            "solver_residual": "0",
            "predictions": {
                **{
                    f"fit-{index}": dict(prediction)
                    for index in range(fit_count)
                },
                "holdout": {**prediction, "model_split": "holdout"},
            },
        }

    def exp_evidence():
        def prediction(split, body):
            production = body * 2 + 7
            return {
                "model_split": split,
                "actual_body_cost": str(body),
                "predicted_body_cost": str(body),
                "body_ape": "0",
                "actual_production_cost": str(production),
                "predicted_production_cost": str(production),
                "production_ape": "0",
                "predicted_operation_body_cost": str(body),
                "shared_memory_body_cost": "0",
            }

        low_predictions = {}
        for byte_length in (0, 1, 2, 4):
            row = prediction("fit", 20)
            row.pop("predicted_operation_body_cost")
            row.pop("shared_memory_body_cost")
            low_predictions[f"exp-low-{byte_length}"] = {
                **row,
                "exponent_byte_length": byte_length,
                "overprediction": "0",
                "underprediction": "0",
            }
        return {
            "status": "supported",
            "parameter_order": [
                "constant",
                "exponent_bytes",
                "exponent_bytes_squared",
            ],
            "exact_fit_rank": 3,
            "parameter_count": 3,
            "observation_count": 8,
            "fit_count": 3,
            "holdout_count": 1,
            "body_coefficients": {
                "constant": "10",
                "exponent_bytes": "2",
                "exponent_bytes_squared": "0",
            },
            "production_coefficients": {
                "constant": "27",
                "exponent_bytes": "4",
                "exponent_bytes_squared": "0",
            },
            "fit_body_mape": "0",
            "fit_body_max_ape": "0",
            "holdout_body_max_ape": "0",
            "fit_production_mape": "0",
            "fit_production_max_ape": "0",
            "holdout_production_max_ape": "0",
            "quality_failures": [],
            "exact_fit_matrix": [
                ["1", "8", "64"],
                ["1", "16", "256"],
                ["1", "32", "1024"],
            ],
            "solver_column_scales": ["1", "1", "1"],
            "solver_residual": "0",
            "predictions": {
                "exp-large-8": prediction("fit", 26),
                "exp-large-16": prediction("fit", 42),
                "exp-large-32": prediction("fit", 74),
                "exp-large-24": prediction("holdout", 58),
            },
            "approximation_policy": {
                "kind": "conservative_small_exponent_bucket",
                "small_domain_max_exponent_byte_length": 4,
                "expected_low_domain_exponent_byte_lengths": [0, 1, 2, 4],
                "polynomial_domain_min_exponent_byte_length": 5,
                "polynomial_domain_max_exponent_byte_length": 32,
                "large_domain_body_floor": "small_bucket_body",
            },
            "small_bucket_body": "20",
            "low_domain_count": 4,
            "low_domain_exponent_byte_lengths": [0, 1, 2, 4],
            "low_domain_predictions": low_predictions,
        }

    cases = tuple(
        types.SimpleNamespace(
            kind="opcode",
            opcode=opcode,
            template=template,
        )
        for opcode, (_scenario, template, _raw_gas) in sorted(
            opcode_gas.PURE_OPCODE_DEFAULTS.items()
        )
    )
    manifest = types.SimpleNamespace(
        cases=cases,
        opcode_relation_anchors=opcode_gas.OPCODE_RELATION_ANCHORS,
        dynamic_raw_gas_keys=opcode_gas.DYNAMIC_RAW_GAS_KEYS,
    )
    anchor_costs = {
        "opcode:0x50": "4",
        "opcode:0x5f": "4",
        "opcode:0x80": "6",
        "opcode:0x90": "6",
    }
    equations = []
    dynamic_scenarios = {
        "opcode:0x0a": {
            "exponent_byte_length": 1,
            "initial_memory_words": 0,
        },
        "opcode:0x20": {"input_length": 32, "initial_memory_words": 0},
        "opcode:0x51": {
            "highest_touched_offset": 0,
            "initial_memory_words": 0,
        },
        "opcode:0x52": {
            "highest_touched_offset": 0,
            "initial_memory_words": 0,
        },
        "opcode:0x53": {
            "highest_touched_offset": 0,
            "initial_memory_words": 0,
        },
        "opcode:0x5e": {"copy_length": 32, "initial_memory_words": 0},
    }
    for opcode in sorted(opcode_gas.PURE_OPCODE_DEFAULTS):
        key = f"opcode:0x{opcode:02x}"
        if key in opcode_gas.OPCODE_RELATION_ANCHORS:
            continue
        row = {
            "relation_id": f"fit-{key}",
            "signed_raw_gas_by_key": {key: "1"},
            "slope_p": "2",
            "dynamic_key": None,
        }
        if key in dynamic_scenarios:
            row.update(
                dynamic_key=key,
                scenario_id=f"{key}-canonical",
                relation_scenario=dynamic_scenarios[key],
            )
        if key == "opcode:0x0a":
            row.update(
                slope_p="26",
                scenario_id="exp-large-8",
                model_split="fit",
                relation_scenario={
                    "exponent_byte_length": 8,
                    "initial_memory_words": 0,
                },
            )
        equations.append(row)
    dynamic_holdouts = [
        *[
            {
                "relation_id": f"exp-low-{byte_length}",
                "scenario_id": f"exp-low-{byte_length}",
                "dynamic_key": "opcode:0x0a",
                "model_split": "fit",
                "signed_raw_gas_by_key": {"opcode:0x0a": "1"},
                "slope_p": "20",
                "relation_scenario": {
                    "exponent_byte_length": byte_length,
                    "initial_memory_words": 0,
                },
            }
            for byte_length in (0, 1, 2, 4)
        ],
        *[
            {
                "relation_id": f"exp-large-{byte_length}",
                "scenario_id": f"exp-large-{byte_length}",
                "dynamic_key": "opcode:0x0a",
                "model_split": split,
                "signed_raw_gas_by_key": {"opcode:0x0a": "1"},
                "slope_p": str(10 + 2 * byte_length),
                "relation_scenario": {
                    "exponent_byte_length": byte_length,
                    "initial_memory_words": 0,
                },
            }
            for byte_length, split in ((16, "fit"), (24, "holdout"), (32, "fit"))
        ],
        {
            "relation_id": "keccak-zero",
            "scenario_id": "keccak-zero",
            "dynamic_key": "opcode:0x20",
            "model_split": "fit",
            "relation_scenario": {
                "input_length": 0,
                "initial_memory_words": 0,
            },
        },
    ]
    relation_artifact = _seal_artifact(
        {
            "schema_version": 3,
            "purpose": opcode_gas.FORMAL_RELATION_PURPOSE,
            "signal_kind": opcode_gas.FORMAL_RELATION_SIGNAL_KIND,
            "status": "accepted",
            "provenance": {"calibration_id": "literal-fixture"},
            "raw_rows_sha256": "b" * 64,
            "equations": equations,
            "dynamic_holdouts": dynamic_holdouts,
        }
    )
    dynamic_artifact = _seal_artifact(
        {
            "schema_version": 4,
            "purpose": "dynamic_opcode_models",
            "status": "supported",
            "candidate_eligible": False,
            "provenance": {"calibration_id": "literal-fixture"},
            "source_hashes": {
                "relation_artifact_sha256": relation_artifact["artifact_sha256"],
                "relation_raw_rows_sha256": relation_artifact[
                    "raw_rows_sha256"
                ],
                "anchor_probe_primary_sha256": "c" * 64,
                "raw_block_rows_sha256": "d" * 64,
            },
            "anchor_body_cost_metric": "prover_gas",
            "anchor_body_costs": anchor_costs,
            "transfer_params": {
                "body_scale": "2",
                "common_opcode_overhead_per_operation": "7",
            },
            "quality_gates": {
                "fit_production_mape_max": "0.05",
                "fit_production_max_ape_max": "0.10",
                "holdout_production_max_ape_max": "0.10",
            },
            "feature_orders": {
                key: list(opcode_gas.DYNAMIC_OPCODE_FEATURE_ORDERS[key])
                for key in opcode_gas.DYNAMIC_RAW_GAS_KEYS
            },
            "shared_memory_model": dynamic_evidence(
                (
                    "opcode:0x51:constant",
                    "opcode:0x52:constant",
                    "opcode:0x53:constant",
                    "memory_growth_event",
                    "memory_evm_gas_delta",
                    "memory_4k_boundary_event",
                ),
                {
                    "opcode:0x51:constant": "11",
                    "opcode:0x52:constant": "12",
                    "opcode:0x53:constant": "13",
                    "memory_growth_event": "2",
                    "memory_evm_gas_delta": "3",
                    "memory_4k_boundary_event": "5",
                },
                {
                    "opcode:0x51:constant": "29",
                    "opcode:0x52:constant": "31",
                    "opcode:0x53:constant": "33",
                    "memory_growth_event": "4",
                    "memory_evm_gas_delta": "6",
                    "memory_4k_boundary_event": "10",
                },
                shared=True,
            ),
            "aggregate_exact_fit_rank": 14,
            "aggregate_parameter_count": 14,
            "models": {
                "opcode:0x0a": exp_evidence(),
                "opcode:0x20": dynamic_evidence(
                    ("constant", "keccak_zero_length_event", "keccak_permutations"),
                    {
                        "constant": "20",
                        "keccak_zero_length_event": "-5",
                        "keccak_permutations": "3",
                    },
                    {
                        "constant": "47",
                        "keccak_zero_length_event": "-10",
                        "keccak_permutations": "6",
                    },
                ),
                "opcode:0x5e": dynamic_evidence(
                    ("constant", "copy_words"),
                    {"constant": "8", "copy_words": "2"},
                    {"constant": "23", "copy_words": "4"},
                ),
            },
        }
    )
    dynamic_artifact = _seal_artifact(dynamic_artifact)
    return manifest, relation_artifact, dynamic_artifact


def registry_with(
    *,
    common_dispatch,
    opcode,
    kind,
    params,
    named_opcodes=None,
    unsupported_opcodes=(),
    invalid_constant="0",
    shared_memory_parameters=None,
):
    model_id = "modeled_opcode"
    invalid_model_id = "invalid"
    slots = [invalid_model_id] * 256
    slots[opcode] = model_id
    for unsupported_opcode in unsupported_opcodes:
        slots[unsupported_opcode] = None
    if named_opcodes is None:
        named_opcodes = frozenset({opcode})
    return OpcodeRegistry(
        common_dispatch=Decimal(common_dispatch),
        models={
            invalid_model_id: ModelSpec(
                kind=ModelKind.INVALID,
                parameters={"constant": Decimal(invalid_constant)},
            ),
            model_id: ModelSpec(
                kind=ModelKind(kind),
                parameters={key: Decimal(value) for key, value in params.items()},
            ),
        },
        opcode_model_ids=tuple(slots),
        named_opcodes=frozenset(named_opcodes),
        invalid_model_id=invalid_model_id,
        shared_memory_parameters={
            key: Decimal(value)
            for key, value in (
                shared_memory_parameters or MEMORY_PARAMETERS
            ).items()
        },
    )


def registry_with_keccak(*, common_dispatch, params):
    return registry_with(
        common_dispatch=common_dispatch,
        opcode=0x20,
        kind="keccak",
        params=params,
    )


def registry_with_exp(*, common_dispatch, params):
    return registry_with(
        common_dispatch=common_dispatch,
        opcode=0x0A,
        kind="exp",
        params=params,
    )


class HierarchicalModelTests(unittest.TestCase):
    def test_static_opcode_adds_dispatch_exactly_once(self):
        registry = registry_with(
            common_dispatch="7",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
        )

        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0x01, raw_gas=5)),
            Decimal("22"),
        )
        self.assertIsNone(validate_core_registry(registry))

    def test_declared_dispatch_only_opcodes_add_common_dispatch_exactly_once(self):
        for opcode in (0x19, 0x5B):
            with self.subTest(opcode=opcode):
                registry = registry_with(
                    common_dispatch="7",
                    opcode=opcode,
                    kind="static_raw_gas",
                    params={"body_per_raw_gas": "0"},
                )

                self.assertEqual(
                    predict_opcode_event(
                        registry, OpcodeEvent(opcode=opcode, raw_gas=3)
                    ),
                    Decimal("7"),
                )

    def test_exp_uses_small_bucket_then_floored_polynomial_with_one_dispatch(self):
        registry = registry_with_exp(
            common_dispatch="7",
            params={
                "small_bucket_body": "20",
                "constant": "-10",
                "exponent_byte": "2",
                "exponent_byte_sq": "0",
            },
        )

        for exponent_bytes in (0, 1, 2, 4, 5):
            with self.subTest(exponent_bytes=exponent_bytes):
                self.assertEqual(
                    predict_opcode_event(
                        registry,
                        OpcodeEvent(
                            opcode=0x0A,
                            exponent_byte_length=exponent_bytes,
                        ),
                    ),
                    Decimal("27"),
                )

    def test_exp_rejects_exponent_byte_length_outside_evm_domain(self):
        registry = registry_with_exp(
            common_dispatch="7",
            params={
                "small_bucket_body": "20",
                "constant": "0",
                "exponent_byte": "2",
                "exponent_byte_sq": "1",
            },
        )

        with self.assertRaisesRegex(ValueError, "0..=32"):
            predict_opcode_event(
                registry,
                OpcodeEvent(opcode=0x0A, exponent_byte_length=33),
            )

    def test_keccak_signed_empty_adjustment_must_leave_nonnegative_prediction(self):
        registry = registry_with_keccak(
            common_dispatch="5",
            params={
                "constant": "20",
                "zero_length_event": "-10",
                "permutation": "30",
            },
        )

        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0x20, input_length=0)),
            Decimal("15"),
        )

    def test_registry_rejects_negative_in_domain_event_prediction(self):
        registry = registry_with_keccak(
            common_dispatch="0",
            params={
                "constant": "5",
                "zero_length_event": "-10",
                "permutation": "30",
            },
        )

        with self.assertRaisesRegex(ValueError, "negative opcode prediction"):
            predict_opcode_event(
                registry,
                OpcodeEvent(opcode=0x20, input_length=0),
            )

    def test_memory_access_adds_shared_memory_basis_once(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x51,
            kind="memory_access",
            params={"constant": "11"},
        )

        self.assertEqual(
            predict_opcode_event(
                registry,
                OpcodeEvent(
                    opcode=0x51,
                    memory_growth_event=2,
                    memory_evm_gas_delta=4,
                    memory_4k_boundary_event=1,
                ),
            ),
            Decimal("39"),
        )

    def test_memory_copy_charges_copy_words_and_shared_memory_once(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x5E,
            kind="memory_copy",
            params={"constant": "11", "copy_words": "13"},
        )

        self.assertEqual(
            predict_opcode_event(
                registry,
                OpcodeEvent(
                    opcode=0x5E,
                    copy_words=2,
                    memory_growth_event=2,
                    memory_evm_gas_delta=4,
                    memory_4k_boundary_event=1,
                ),
            ),
            Decimal("65"),
        )

    def test_keccak_permutations_are_zero_for_empty_and_step_after_136_bytes(self):
        registry = registry_with_keccak(
            common_dispatch="5",
            params={
                "constant": "20",
                "zero_length_event": "-10",
                "permutation": "30",
            },
        )

        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0x20, input_length=135)),
            Decimal("55"),
        )
        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0x20, input_length=136)),
            Decimal("85"),
        )
        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0x20, input_length=272)),
            Decimal("115"),
        )

    def test_undefined_byte_resolves_to_the_one_invalid_model(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
            invalid_constant="9",
        )

        self.assertEqual(registry.opcode_model_ids[0xFE], "invalid")
        self.assertEqual(
            predict_opcode_event(registry, OpcodeEvent(opcode=0xFE)),
            Decimal("14"),
        )

    def test_named_but_unmodeled_opcode_is_not_treated_as_invalid(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
            named_opcodes=frozenset({0x01, 0x02}),
            unsupported_opcodes=(0x02,),
        )

        with self.assertRaisesRegex(ValueError, "unsupported opcode 0x02"):
            predict_opcode_event(registry, OpcodeEvent(opcode=0x02))

    def test_registry_rejects_non_decimal_or_extra_model_parameters(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
        )
        models = dict(registry.models)
        models["modeled_opcode"] = ModelSpec(
            kind=ModelKind.STATIC_RAW_GAS,
            parameters={"body_per_raw_gas": Decimal("3"), "extra": Decimal("1")},
        )
        with self.assertRaisesRegex(ValueError, "parameters differ"):
            OpcodeRegistry(
                common_dispatch=registry.common_dispatch,
                models=models,
                opcode_model_ids=registry.opcode_model_ids,
                named_opcodes=registry.named_opcodes,
                invalid_model_id=registry.invalid_model_id,
                shared_memory_parameters=registry.shared_memory_parameters,
            )

        models["modeled_opcode"] = ModelSpec(
            kind=ModelKind.STATIC_RAW_GAS,
            parameters={"body_per_raw_gas": 3},
        )
        with self.assertRaisesRegex(ValueError, "finite Decimal"):
            OpcodeRegistry(
                common_dispatch=registry.common_dispatch,
                models=models,
                opcode_model_ids=registry.opcode_model_ids,
                named_opcodes=registry.named_opcodes,
                invalid_model_id=registry.invalid_model_id,
                shared_memory_parameters=registry.shared_memory_parameters,
            )

    def test_registry_rejects_unknown_model_references_and_invalid_slot_count(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
        )
        slots = list(registry.opcode_model_ids)
        slots[0x01] = "missing"
        with self.assertRaisesRegex(ValueError, "unknown model"):
            OpcodeRegistry(
                common_dispatch=registry.common_dispatch,
                models=registry.models,
                opcode_model_ids=tuple(slots),
                named_opcodes=registry.named_opcodes,
                invalid_model_id=registry.invalid_model_id,
                shared_memory_parameters=registry.shared_memory_parameters,
            )

        with self.assertRaisesRegex(ValueError, "256"):
            OpcodeRegistry(
                common_dispatch=registry.common_dispatch,
                models=registry.models,
                opcode_model_ids=registry.opcode_model_ids[:-1],
                named_opcodes=registry.named_opcodes,
                invalid_model_id=registry.invalid_model_id,
                shared_memory_parameters=registry.shared_memory_parameters,
            )

    def test_predict_rejects_missing_and_irrelevant_features(self):
        registry = registry_with(
            common_dispatch="5",
            opcode=0x01,
            kind="static_raw_gas",
            params={"body_per_raw_gas": "3"},
        )

        with self.assertRaisesRegex(ValueError, "missing required feature: raw_gas"):
            predict_opcode_event(registry, OpcodeEvent(opcode=0x01))
        with self.assertRaisesRegex(ValueError, "irrelevant feature: input_length"):
            predict_opcode_event(
                registry,
                OpcodeEvent(opcode=0x01, raw_gas=5, input_length=0),
            )

    def test_event_rejects_bool_negative_and_out_of_range_numeric_values(self):
        with self.assertRaisesRegex(ValueError, "opcode"):
            OpcodeEvent(opcode=256)
        with self.assertRaisesRegex(TypeError, "raw_gas"):
            OpcodeEvent(opcode=0x01, raw_gas=True)
        with self.assertRaisesRegex(ValueError, "input_length"):
            OpcodeEvent(opcode=0x20, input_length=-1)

    def test_predict_rejects_a_non_registry_input(self):
        with self.assertRaisesRegex(TypeError, "OpcodeRegistry"):
            predict_opcode_event("not a registry", OpcodeEvent(opcode=0x00))


class CoreOpcodeSubmodelArtifactTests(unittest.TestCase):
    def test_dynamic_dispatch_only_control_uses_declared_zero_body(self):
        target_body = opcode_gas._dynamic_target_body_cost_from_relation(
            row={"slope_p": "11"},
            dynamic_key="opcode:0x51",
            coefficients={
                "opcode:0x51": Fraction(3),
                "opcode:0x19": Fraction(-3),
            },
            dynamic_keys={"opcode:0x51"},
            lab_multipliers={"opcode:0x19": Decimal("7")},
            dispatch_only_keys=("opcode:0x19",),
        )

        self.assertEqual(target_body, Decimal("11"))

    def test_builds_102_of_150_named_unzen_opcodes_and_replays_exactly(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()

        artifact = opcode_gas.build_core_opcode_submodel_artifact(
            manifest=manifest,
            relation_artifact=relation_artifact,
            dynamic_artifact=dynamic_artifact,
        )

        self.assertEqual(artifact["purpose"], "core_opcode_submodel")
        self.assertEqual(artifact["schema_version"], 3)
        self.assertEqual(artifact["status"], "supported_core_submodel")
        self.assertFalse(artifact["candidate_eligible"])
        self.assertEqual(artifact["named_opcode_count"], 150)
        self.assertEqual(artifact["modeled_named_opcode_count"], 102)
        self.assertEqual(artifact["unsupported_named_opcode_count"], 48)
        self.assertIn("opcode:0x1e", artifact["unsupported_named_opcode_keys"])
        self.assertEqual(
            artifact["registry"]["models"]["opcode:0x01"]["parameters"],
            {"body_per_raw_gas": "4"},
        )
        self.assertEqual(artifact["registry"]["common_dispatch"], "7")
        self.assertEqual(
            artifact["approximation_policy"]["dispatch_only_opcode_keys"],
            ["opcode:0x19", "opcode:0x5b"],
        )
        self.assertEqual(
            artifact["fit_evidence"]["dispatch_only_keys"],
            ["opcode:0x19", "opcode:0x5b"],
        )
        for key in ("opcode:0x19", "opcode:0x5b"):
            self.assertEqual(
                artifact["registry"]["models"][key]["parameters"],
                {"body_per_raw_gas": "0"},
            )
        self.assertEqual(
            artifact["registry"]["models"]["opcode:0x0a"],
            {
                "kind": "exp",
                "parameters": {
                    "small_bucket_body": "40",
                    "constant": "20",
                    "exponent_byte": "4",
                    "exponent_byte_sq": "0",
                },
            },
        )
        self.assertEqual(
            artifact["registry"]["models"]["opcode:0x20"]["parameters"],
            {
                "constant": "40",
                "zero_length_event": "-10",
                "permutation": "6",
            },
        )
        self.assertEqual(
            artifact["registry"]["models"]["opcode:0x51"]["parameters"],
            {"constant": "22"},
        )
        self.assertEqual(
            artifact["registry"]["models"]["opcode:0x5e"]["parameters"],
            {"constant": "16", "copy_words": "4"},
        )
        for dynamic_key in opcode_gas.DYNAMIC_RAW_GAS_KEYS:
            self.assertNotEqual(
                artifact["registry"]["models"][dynamic_key]["kind"],
                "static_raw_gas",
            )
        self.assertEqual(
            opcode_gas.validate_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact, artifact
            ),
            artifact,
        )

    def test_relation_predictions_replay_from_final_typed_registry(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        mload_relation = next(
            row
            for row in relation_artifact["equations"]
            if row["relation_id"] == "fit-opcode:0x51"
        )
        mload_relation.update(
            signed_raw_gas_by_key={
                "opcode:0x51": "3",
                "opcode:0x19": "-3",
            },
            slope_p="2",
        )
        relation_artifact = _seal_artifact(relation_artifact)
        dynamic_artifact["source_hashes"]["relation_artifact_sha256"] = (
            relation_artifact["artifact_sha256"]
        )
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        artifact = opcode_gas.build_core_opcode_submodel_artifact(
            manifest, relation_artifact, dynamic_artifact
        )

        predictions = artifact["fit_evidence"]["predictions"]
        self.assertEqual(
            set(predictions),
            {row["relation_id"] for row in relation_artifact["equations"]},
        )
        self.assertEqual(
            predictions["fit-opcode:0x51"],
            {
                "basis": "final_typed_registry_lab_body",
                "gate": "declared_approximation",
                "observed_slope": "2",
                "predicted_slope": "22",
                "absolute_residual": "20",
                "ape": "10",
            },
        )

    def test_builder_marks_dispatch_only_dependent_static_key_unmeasured(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        static_relation = next(
            row
            for row in relation_artifact["equations"]
            if row["relation_id"] == "fit-opcode:0x15"
        )
        static_relation["signed_raw_gas_by_key"] = {
            "opcode:0x15": "1",
            "opcode:0x19": "-1",
        }
        relation_artifact = _seal_artifact(relation_artifact)
        dynamic_artifact["source_hashes"]["relation_artifact_sha256"] = (
            relation_artifact["artifact_sha256"]
        )
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        artifact = opcode_gas.build_core_opcode_submodel_artifact(
            manifest, relation_artifact, dynamic_artifact
        )

        self.assertEqual(artifact["modeled_named_opcode_count"], 101)
        self.assertEqual(artifact["unsupported_named_opcode_count"], 49)
        self.assertEqual(
            artifact["unsupported_opcode_reasons"]["opcode:0x15"],
            "only_dispatch_dependent_evidence",
        )
        self.assertEqual(
            artifact["fit_evidence"]["predictions"]["fit-opcode:0x15"],
            {
                "basis": "final_typed_registry_lab_body",
                "outcome": "unmeasured_unsupported",
                "observed_slope": "2",
                "unsupported_opcode_keys": ["opcode:0x15"],
                "unsupported_reason": "only_dispatch_dependent_evidence",
            },
        )

    def test_builder_fails_closed_when_identifiable_static_columns_lose_rank(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        for relation_id in ("fit-opcode:0x02", "fit-opcode:0x03"):
            relation = next(
                row
                for row in relation_artifact["equations"]
                if row["relation_id"] == relation_id
            )
            relation["signed_raw_gas_by_key"] = {
                "opcode:0x02": "1",
                "opcode:0x03": "1",
            }
        with self.assertRaisesRegex(
            ValueError, "core static relation basis is rank-deficient"
        ):
            opcode_gas._core_static_fit_projection(
                opcode_gas._core_opcode_keys(manifest),
                opcode_gas._core_relation_equations(
                    relation_artifact,
                    opcode_gas._core_opcode_keys(manifest),
                ),
            )

    def test_replay_rejects_mutated_sources_registry_and_digest(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        artifact = opcode_gas.build_core_opcode_submodel_artifact(
            manifest, relation_artifact, dynamic_artifact
        )
        mutations = []

        changed_relation = copy.deepcopy(artifact)
        changed_relation["source_hashes"]["relation_artifact_sha256"] = "e" * 64
        mutations.append(("relation source hash", _seal_artifact(changed_relation)))

        changed_dynamic = copy.deepcopy(artifact)
        changed_dynamic["source_hashes"]["dynamic_artifact_sha256"] = "f" * 64
        mutations.append(("dynamic source hash", _seal_artifact(changed_dynamic)))

        negative_static = copy.deepcopy(artifact)
        negative_static["registry"]["models"]["opcode:0x01"]["parameters"][
            "body_per_raw_gas"
        ] = "-1"
        mutations.append(("negative static body", _seal_artifact(negative_static)))

        changed_dispatch = copy.deepcopy(artifact)
        changed_dispatch["registry"]["common_dispatch"] = "8"
        mutations.append(("changed common dispatch", _seal_artifact(changed_dispatch)))

        named_invalid = copy.deepcopy(artifact)
        named_invalid["registry"]["opcode_model_ids"][0x01] = "invalid"
        mutations.append(("named opcode mapped to INVALID", _seal_artifact(named_invalid)))

        changed_bucket = copy.deepcopy(artifact)
        changed_bucket["registry"]["models"]["opcode:0x0a"]["parameters"][
            "small_bucket_body"
        ] = "41"
        mutations.append(("EXP bucket", _seal_artifact(changed_bucket)))

        nonzero_dispatch_only = copy.deepcopy(artifact)
        nonzero_dispatch_only["registry"]["models"]["opcode:0x19"][
            "parameters"
        ]["body_per_raw_gas"] = "1"
        mutations.append(
            ("nonzero dispatch-only body", _seal_artifact(nonzero_dispatch_only))
        )

        changed_policy = copy.deepcopy(artifact)
        changed_policy["approximation_policy"]["dispatch_only_opcode_keys"] = [
            "opcode:0x19"
        ]
        mutations.append(("approximation policy", _seal_artifact(changed_policy)))

        old_schema = copy.deepcopy(artifact)
        old_schema["schema_version"] = 1
        mutations.append(("old core schema", _seal_artifact(old_schema)))

        for label, mutated in mutations:
            with self.subTest(mutation=label), self.assertRaisesRegex(
                ValueError, "exact source replay"
            ):
                opcode_gas.validate_core_opcode_submodel_artifact(
                    manifest,
                    relation_artifact,
                    dynamic_artifact,
                    mutated,
                )

        bad_digest = copy.deepcopy(artifact)
        bad_digest["artifact_sha256"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "content hash"):
            opcode_gas.validate_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact, bad_digest
            )

    def test_builder_rejects_dynamic_source_disagreement(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        dynamic_artifact["source_hashes"]["relation_artifact_sha256"] = "e" * 64
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(ValueError, "source.*relation|relation.*source"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )

    def test_builder_rejects_incomplete_schema_4_dynamic_evidence(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        del dynamic_artifact["aggregate_exact_fit_rank"]
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(ValueError, "schema-4|schema"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )

    def test_builder_rejects_schema_3_dynamic_evidence(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        dynamic_artifact["schema_version"] = 3
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(ValueError, "schema-4|schema"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )

    def test_builder_rejects_mutated_exp_bucket_policy_and_underprediction(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        mutations = []

        changed_bucket = copy.deepcopy(dynamic_artifact)
        changed_bucket["models"]["opcode:0x0a"]["small_bucket_body"] = "19"
        mutations.append(("bucket", _seal_artifact(changed_bucket), "bucket"))

        changed_policy = copy.deepcopy(dynamic_artifact)
        changed_policy["models"]["opcode:0x0a"]["approximation_policy"][
            "small_domain_max_exponent_byte_length"
        ] = 3
        mutations.append(("policy", _seal_artifact(changed_policy), "policy"))

        underprediction = copy.deepcopy(dynamic_artifact)
        row = underprediction["models"]["opcode:0x0a"][
            "low_domain_predictions"
        ]["exp-low-2"]
        row.update(
            actual_body_cost="21",
            actual_production_cost="49",
            body_ape="0.047619047619047619047619047619047619047619047619047619047619047619047619047619048",
            production_ape="0.040816326530612244897959183673469387755102040816326530612244897959183673469387755",
            underprediction="1",
        )
        mutations.append(
            ("underprediction", _seal_artifact(underprediction), "underprediction")
        )

        for label, mutated, message in mutations:
            with self.subTest(mutation=label), self.assertRaisesRegex(
                ValueError, message
            ):
                opcode_gas.build_core_opcode_submodel_artifact(
                    manifest, relation_artifact, mutated
                )

    def test_builder_rejects_coherently_resealed_exp_low_domain_evidence(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        exp_evidence = dynamic_artifact["models"]["opcode:0x0a"]
        exp_evidence["small_bucket_body"] = "21"
        for prediction in exp_evidence["low_domain_predictions"].values():
            prediction.update(
                actual_body_cost="21",
                predicted_body_cost="21",
                body_ape="0",
                actual_production_cost="49",
                predicted_production_cost="49",
                production_ape="0",
                overprediction="0",
                underprediction="0",
            )
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(
            ValueError, "actual body cost differs from relation-derived source"
        ):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )

    def test_builder_independently_checks_dynamic_quality_rank_and_costs(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        mutations = []

        failed_gate = copy.deepcopy(dynamic_artifact)
        failed_gate["models"]["opcode:0x0a"]["fit_production_mape"] = "0.051"
        mutations.append(("quality gate", _seal_artifact(failed_gate), "quality gate"))

        rank_deficient = copy.deepcopy(dynamic_artifact)
        matrix = rank_deficient["models"]["opcode:0x0a"]["exact_fit_matrix"]
        matrix[1] = list(matrix[0])
        mutations.append(("exact rank", _seal_artifact(rank_deficient), "exact rank"))

        negative_cost = copy.deepcopy(dynamic_artifact)
        negative_cost["models"]["opcode:0x0a"]["predictions"]["exp-large-8"][
            "predicted_operation_body_cost"
        ] = "-1"
        mutations.append(("negative cost", _seal_artifact(negative_cost), "negative"))

        for label, mutated, message in mutations:
            with self.subTest(mutation=label), self.assertRaisesRegex(
                ValueError, message
            ):
                opcode_gas.build_core_opcode_submodel_artifact(
                    manifest, relation_artifact, mutated
                )

    def test_builder_recomputes_dynamic_prediction_arithmetic_and_aggregates(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        mutations = []

        stale_apes = copy.deepcopy(dynamic_artifact)
        row = stale_apes["models"]["opcode:0x0a"]["predictions"]["exp-large-8"]
        row.update(
            actual_body_cost="100",
            predicted_body_cost="1",
            predicted_operation_body_cost="1",
            actual_production_cost="207",
            predicted_production_cost="9",
        )
        mutations.append(("row APE", _seal_artifact(stale_apes), "body APE"))

        zero_actual = copy.deepcopy(dynamic_artifact)
        zero_actual_row = zero_actual["models"]["opcode:0x0a"]["predictions"][
            "exp-large-8"
        ]
        zero_actual_row.update(
            actual_body_cost="0",
            actual_production_cost="7",
        )
        mutations.append(
            (
                "zero actual denominator",
                _seal_artifact(zero_actual),
                "must be positive",
            )
        )

        stale_production_ape = copy.deepcopy(dynamic_artifact)
        stale_production_ape["models"]["opcode:0x0a"]["predictions"]["exp-large-8"][
            "production_ape"
        ] = "0.01"
        mutations.append(
            (
                "production APE",
                _seal_artifact(stale_production_ape),
                "production APE",
            )
        )

        bad_production = copy.deepcopy(dynamic_artifact)
        bad_production["models"]["opcode:0x0a"]["predictions"]["exp-large-8"][
            "actual_production_cost"
        ] = "10"
        mutations.append(
            (
                "production conversion",
                _seal_artifact(bad_production),
                "production cost",
            )
        )

        bad_operation_sum = copy.deepcopy(dynamic_artifact)
        bad_operation_sum["models"]["opcode:0x0a"]["predictions"]["exp-large-8"][
            "predicted_operation_body_cost"
        ] = "2"
        mutations.append(
            (
                "operation plus shared",
                _seal_artifact(bad_operation_sum),
                "operation.*shared",
            )
        )

        for field in (
            "fit_body_mape",
            "fit_body_max_ape",
            "holdout_body_max_ape",
            "fit_production_mape",
            "fit_production_max_ape",
            "holdout_production_max_ape",
        ):
            aggregate_drift = copy.deepcopy(dynamic_artifact)
            aggregate_drift["models"]["opcode:0x0a"][field] = "0.01"
            mutations.append(
                (f"aggregate {field}", _seal_artifact(aggregate_drift), "aggregate")
            )

        for label, mutated, message in mutations:
            with self.subTest(mutation=label), self.assertRaisesRegex(
                ValueError, message
            ):
                opcode_gas.build_core_opcode_submodel_artifact(
                    manifest, relation_artifact, mutated
                )

    def test_builder_binds_exp_source_rows_to_model_predictions(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()

        missing = copy.deepcopy(dynamic_artifact)
        predictions = missing["models"]["opcode:0x0a"]["low_domain_predictions"]
        predictions["different-zero"] = predictions.pop("exp-low-0")
        missing = _seal_artifact(missing)
        with self.assertRaisesRegex(ValueError, "EXP source prediction"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, missing
            )

        wrong_split = copy.deepcopy(dynamic_artifact)
        predictions = wrong_split["models"]["opcode:0x0a"][
            "low_domain_predictions"
        ]
        predictions["exp-low-0"]["model_split"] = "holdout"
        wrong_split = _seal_artifact(wrong_split)
        with self.assertRaisesRegex(ValueError, "EXP prediction model_split"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, wrong_split
            )

    def test_builder_requires_zero_exp_evidence_from_relation_source(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        relation_artifact["dynamic_holdouts"] = [
            row
            for row in relation_artifact["dynamic_holdouts"]
            if row.get("scenario_id") != "exp-low-0"
        ]
        relation_artifact = _seal_artifact(relation_artifact)
        dynamic_artifact["source_hashes"]["relation_artifact_sha256"] = (
            relation_artifact["artifact_sha256"]
        )
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(ValueError, "EXP source domain"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )

    def test_builder_rejects_negative_dynamic_prediction_in_source_domain(self):
        manifest, relation_artifact, dynamic_artifact = _core_submodel_sources()
        dynamic_artifact["models"]["opcode:0x20"]["body_coefficients"][
            "keccak_zero_length_event"
        ] = "-100"
        dynamic_artifact["models"]["opcode:0x20"]["production_coefficients"][
            "keccak_zero_length_event"
        ] = "-200"
        dynamic_artifact = _seal_artifact(dynamic_artifact)

        with self.assertRaisesRegex(ValueError, "negative opcode prediction"):
            opcode_gas.build_core_opcode_submodel_artifact(
                manifest, relation_artifact, dynamic_artifact
            )


if __name__ == "__main__":
    unittest.main()
