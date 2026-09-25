import pathlib
import sys
import unittest
from decimal import Decimal


ROOT = pathlib.Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "experiments" / "opcode-gas"))

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


if __name__ == "__main__":
    unittest.main()
