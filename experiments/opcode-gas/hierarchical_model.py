"""Typed evaluator for the hierarchical SP1 core-opcode submodel."""

from __future__ import annotations

from dataclasses import dataclass
from decimal import Context, Decimal, ROUND_HALF_EVEN, localcontext
from enum import Enum
from types import MappingProxyType
from typing import Mapping


_DECIMAL_CONTEXT = Context(prec=80, rounding=ROUND_HALF_EVEN, traps=[])
_MEMORY_PARAMETER_KEYS = frozenset(
    {
        "memory_growth_event",
        "memory_evm_gas_delta",
        "memory_4k_boundary_event",
    }
)
_MODEL_PARAMETER_KEYS: Mapping["ModelKind", frozenset[str]]


class ModelKind(str, Enum):
    STATIC_RAW_GAS = "static_raw_gas"
    EXP = "exp"
    KECCAK = "keccak"
    MEMORY_ACCESS = "memory_access"
    MEMORY_COPY = "memory_copy"
    INVALID = "invalid"


_MODEL_PARAMETER_KEYS = MappingProxyType(
    {
        ModelKind.STATIC_RAW_GAS: frozenset({"body_per_raw_gas"}),
        ModelKind.EXP: frozenset(
            {
                "small_bucket_body",
                "constant",
                "exponent_byte",
                "exponent_byte_sq",
            }
        ),
        ModelKind.KECCAK: frozenset(
            {"constant", "zero_length_event", "permutation"}
        ),
        ModelKind.MEMORY_ACCESS: frozenset({"constant"}),
        # This matches the frozen structured-dynamic artifact feature name.
        ModelKind.MEMORY_COPY: frozenset({"constant", "copy_words"}),
        ModelKind.INVALID: frozenset({"constant"}),
    }
)


@dataclass(frozen=True)
class ModelSpec:
    kind: ModelKind
    parameters: Mapping[str, Decimal]

    def __post_init__(self) -> None:
        object.__setattr__(self, "parameters", MappingProxyType(dict(self.parameters)))


@dataclass(frozen=True)
class OpcodeEvent:
    opcode: int
    raw_gas: int | None = None
    exponent_byte_length: int | None = None
    input_length: int | None = None
    copy_words: int | None = None
    memory_growth_event: int = 0
    memory_evm_gas_delta: int = 0
    memory_4k_boundary_event: int = 0

    def __post_init__(self) -> None:
        _validate_opcode(self.opcode)
        for name in (
            "raw_gas",
            "exponent_byte_length",
            "input_length",
            "copy_words",
            "memory_growth_event",
            "memory_evm_gas_delta",
            "memory_4k_boundary_event",
        ):
            _validate_event_number(name, getattr(self, name))


@dataclass(frozen=True)
class OpcodeRegistry:
    common_dispatch: Decimal
    models: Mapping[str, ModelSpec]
    opcode_model_ids: tuple[str | None, ...]
    named_opcodes: frozenset[int]
    invalid_model_id: str
    shared_memory_parameters: Mapping[str, Decimal]

    def __post_init__(self) -> None:
        object.__setattr__(self, "models", MappingProxyType(dict(self.models)))
        object.__setattr__(self, "opcode_model_ids", tuple(self.opcode_model_ids))
        object.__setattr__(self, "named_opcodes", frozenset(self.named_opcodes))
        object.__setattr__(
            self,
            "shared_memory_parameters",
            MappingProxyType(dict(self.shared_memory_parameters)),
        )
        validate_core_registry(self)


def validate_core_registry(registry: OpcodeRegistry) -> None:
    """Reject registries that cannot preserve the one-owner model contract."""
    if not isinstance(registry, OpcodeRegistry):
        raise TypeError("registry must be an OpcodeRegistry")
    _validate_decimal("common_dispatch", registry.common_dispatch)
    _validate_nonnegative("common_dispatch", registry.common_dispatch)

    if not isinstance(registry.invalid_model_id, str) or not registry.invalid_model_id:
        raise ValueError("invalid_model_id must be a nonempty string")
    if registry.invalid_model_id not in registry.models:
        raise ValueError("invalid model is missing from models")
    if len(registry.opcode_model_ids) != 256:
        raise ValueError("opcode_model_ids must contain exactly 256 slots")

    if set(registry.shared_memory_parameters) != _MEMORY_PARAMETER_KEYS:
        raise ValueError("shared memory parameters differ from the required schema")
    for key, value in registry.shared_memory_parameters.items():
        _validate_decimal(f"shared memory parameter {key}", value)
        _validate_nonnegative(f"shared memory parameter {key}", value)

    for opcode in registry.named_opcodes:
        _validate_opcode(opcode)

    for model_id, spec in registry.models.items():
        if not isinstance(model_id, str) or not model_id:
            raise ValueError("model ids must be nonempty strings")
        _validate_model_spec(model_id, spec)

    invalid_spec = registry.models[registry.invalid_model_id]
    if invalid_spec.kind is not ModelKind.INVALID:
        raise ValueError("invalid_model_id must reference the INVALID model")

    for model_id, spec in registry.models.items():
        if model_id != registry.invalid_model_id and spec.kind is ModelKind.INVALID:
            raise ValueError("only invalid_model_id may reference an INVALID model")

    for opcode, model_id in enumerate(registry.opcode_model_ids):
        if model_id is not None and not isinstance(model_id, str):
            raise ValueError(f"opcode slot 0x{opcode:02x} is not a model id or None")
        if opcode in registry.named_opcodes:
            if model_id == registry.invalid_model_id:
                raise ValueError(
                    f"named opcode 0x{opcode:02x} must be modeled or unsupported"
                )
            if model_id is not None and model_id not in registry.models:
                raise ValueError(
                    f"opcode slot 0x{opcode:02x} references unknown model {model_id!r}"
                )
        else:
            if model_id != registry.invalid_model_id:
                raise ValueError(
                    f"undefined opcode 0x{opcode:02x} must reference invalid_model_id"
                )


def predict_opcode_event(registry: OpcodeRegistry, event: OpcodeEvent) -> Decimal:
    """Evaluate one opcode event with its single top-level model."""
    if not isinstance(registry, OpcodeRegistry):
        raise TypeError("registry must be an OpcodeRegistry")
    if not isinstance(event, OpcodeEvent):
        raise TypeError("event must be an OpcodeEvent")

    model_id = registry.opcode_model_ids[event.opcode]
    if model_id is None:
        raise ValueError(f"unsupported opcode 0x{event.opcode:02x}")
    model = registry.models[model_id]
    _validate_features(model.kind, event)

    with localcontext(_DECIMAL_CONTEXT):
        prediction = registry.common_dispatch + _predict_body(
            model, event, registry.shared_memory_parameters
        )
    if not prediction.is_finite() or prediction < 0:
        raise ValueError("negative opcode prediction")
    return prediction


def _validate_model_spec(model_id: str, spec: ModelSpec) -> None:
    if not isinstance(spec, ModelSpec):
        raise ValueError(f"model {model_id!r} is not a ModelSpec")
    if not isinstance(spec.kind, ModelKind):
        raise ValueError(f"model {model_id!r} has an unknown model kind")
    expected_keys = _MODEL_PARAMETER_KEYS[spec.kind]
    actual_keys = set(spec.parameters)
    if actual_keys != expected_keys:
        raise ValueError(
            f"model {model_id!r} parameters differ from the required schema: "
            f"missing={sorted(expected_keys - actual_keys)!r}, "
            f"extra={sorted(actual_keys - expected_keys)!r}"
        )
    for key, value in spec.parameters.items():
        _validate_decimal(f"model {model_id!r} parameter {key}", value)
        if (
            spec.kind is ModelKind.KECCAK and key == "zero_length_event"
        ) or (
            spec.kind is ModelKind.EXP
            and key in {"constant", "exponent_byte", "exponent_byte_sq"}
        ):
            continue
        _validate_nonnegative(
            f"model {model_id!r} parameter {key}", value
        )


def _validate_features(kind: ModelKind, event: OpcodeEvent) -> None:
    if kind is ModelKind.STATIC_RAW_GAS:
        _require(event, "raw_gas")
        _reject_optional(event, "exponent_byte_length")
        _reject_optional(event, "input_length")
        _reject_optional(event, "copy_words")
        _reject_memory(event)
    elif kind is ModelKind.EXP:
        _require(event, "exponent_byte_length")
        if event.exponent_byte_length > 32:
            raise ValueError("exponent_byte_length must be in range 0..=32")
        _reject_optional(event, "raw_gas")
        _reject_optional(event, "input_length")
        _reject_optional(event, "copy_words")
        _reject_memory(event)
    elif kind is ModelKind.KECCAK:
        _require(event, "input_length")
        _reject_optional(event, "raw_gas")
        _reject_optional(event, "exponent_byte_length")
        _reject_optional(event, "copy_words")
    elif kind is ModelKind.MEMORY_ACCESS:
        _reject_optional(event, "raw_gas")
        _reject_optional(event, "exponent_byte_length")
        _reject_optional(event, "input_length")
        _reject_optional(event, "copy_words")
    elif kind is ModelKind.MEMORY_COPY:
        _require(event, "copy_words")
        _reject_optional(event, "raw_gas")
        _reject_optional(event, "exponent_byte_length")
        _reject_optional(event, "input_length")
    elif kind is ModelKind.INVALID:
        _reject_optional(event, "raw_gas")
        _reject_optional(event, "exponent_byte_length")
        _reject_optional(event, "input_length")
        _reject_optional(event, "copy_words")
        _reject_memory(event)
    else:
        raise ValueError(f"unsupported model kind: {kind!r}")


def _predict_body(
    model: ModelSpec,
    event: OpcodeEvent,
    shared_memory_parameters: Mapping[str, Decimal],
) -> Decimal:
    parameters = model.parameters
    if model.kind is ModelKind.STATIC_RAW_GAS:
        return parameters["body_per_raw_gas"] * Decimal(event.raw_gas)
    if model.kind is ModelKind.EXP:
        exponent_bytes = Decimal(event.exponent_byte_length)
        if event.exponent_byte_length <= 4:
            return parameters["small_bucket_body"]
        polynomial = (
            parameters["constant"]
            + parameters["exponent_byte"] * exponent_bytes
            + parameters["exponent_byte_sq"] * exponent_bytes * exponent_bytes
        )
        return max(parameters["small_bucket_body"], polynomial)
    if model.kind is ModelKind.KECCAK:
        permutations = (
            0
            if event.input_length == 0
            else event.input_length // 136 + 1
        )
        return (
            parameters["constant"]
            + (
                parameters["zero_length_event"]
                if event.input_length == 0
                else Decimal(0)
            )
            + parameters["permutation"] * Decimal(permutations)
            + _predict_shared_memory(event, shared_memory_parameters)
        )
    if model.kind is ModelKind.MEMORY_ACCESS:
        return parameters["constant"] + _predict_shared_memory(
            event, shared_memory_parameters
        )
    if model.kind is ModelKind.MEMORY_COPY:
        return (
            parameters["constant"]
            + parameters["copy_words"] * Decimal(event.copy_words)
            + _predict_shared_memory(event, shared_memory_parameters)
        )
    if model.kind is ModelKind.INVALID:
        return parameters["constant"]
    raise ValueError(f"unsupported model kind: {model.kind!r}")


def _predict_shared_memory(
    event: OpcodeEvent, parameters: Mapping[str, Decimal]
) -> Decimal:
    return (
        parameters["memory_growth_event"] * Decimal(event.memory_growth_event)
        + parameters["memory_evm_gas_delta"]
        * Decimal(event.memory_evm_gas_delta)
        + parameters["memory_4k_boundary_event"]
        * Decimal(event.memory_4k_boundary_event)
    )


def _require(event: OpcodeEvent, feature: str) -> None:
    if getattr(event, feature) is None:
        raise ValueError(f"missing required feature: {feature}")


def _reject_optional(event: OpcodeEvent, feature: str) -> None:
    if getattr(event, feature) is not None:
        raise ValueError(f"irrelevant feature: {feature}")


def _reject_memory(event: OpcodeEvent) -> None:
    for feature in _MEMORY_PARAMETER_KEYS:
        if getattr(event, feature) != 0:
            raise ValueError(f"irrelevant feature: {feature}")


def _validate_opcode(opcode: int) -> None:
    if isinstance(opcode, bool) or not isinstance(opcode, int):
        raise TypeError("opcode must be an int")
    if not 0 <= opcode <= 0xFF:
        raise ValueError("opcode must be in range 0x00..0xff")


def _validate_event_number(name: str, value: int | None) -> None:
    if value is None:
        return
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be a nonnegative int")
    if value < 0:
        raise ValueError(f"{name} must be a nonnegative int")


def _validate_decimal(name: str, value: Decimal) -> None:
    if not isinstance(value, Decimal) or not value.is_finite():
        raise ValueError(f"{name} must be a finite Decimal")


def _validate_nonnegative(name: str, value: Decimal) -> None:
    if value < 0:
        raise ValueError(f"negative opcode prediction: {name} must be nonnegative")
