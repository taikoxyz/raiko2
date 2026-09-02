# Model Scope

This schema-v3 fixture republishes the proposal model from the 2026-08-31 global-cap fixture with
the same 140 proposal rows, coefficients, and `500,000,000` total-zkGas operating cap. Schema v3
removes aggregation child-count calibration as a runtime availability gate.

Estimated aggregation uses the configured `180` mcycles per child for every structurally valid,
non-empty aggregation input. The aggregation provenance in `config.json` is audit metadata only; it
does not select child counts or compare the running guest image. Arithmetic overflow is the only
aggregation-estimator condition that falls back to local execution.

The proposal measurements predate the proposal ELF rebuilt by raiko2 #242 and do not use the v0.6.0
release guest. The model identity records what was measured; it is intentionally not a runtime ELF
gate. A release that enables `estimated` with another guest accepts unmeasured quote-price and
timeout drift. Use `evaluated` when the exact cycle count for the running guest is required.

## Proposal measured ranges

Across all 140 proposal rows the samples span `block_count` 155-192 and `total_zkgas`
216314230-562107601, at 1.16-2.93 million zkGas per block. Runtime proposal availability is
deliberately wider than these ranges and uses the explicit global cap rather than the observed
sample rectangle.
