# Model Scope

This schema-v3 fixture republishes the proposal model from the 2026-08-31 global-cap fixture with
the same 140 proposal rows, coefficients, and `500,000,000` total-zkGas operating cap. Schema v3
removes aggregation child-count calibration as a runtime availability gate.

Estimated aggregation uses the configured `180` mcycles per child for every structurally valid,
non-empty aggregation input. The aggregation provenance in `config.json` is audit metadata only; it
does not select child counts or compare the running guest image. The shipped aggregation ELF has not
been measured successfully. Its SHA-256 is
`fd56481a38855c3d85488cc267653ae390633c16ba1612fcf2d4891f5b30d924`, but its
`disable-dev-mode` build rejects the development receipts used by the observation probe, and its
artifact provenance therefore has `image_id: null`. Historical
different-cohort observations are about 175 mcycles at one child and 817-818 at five children, so
the five-child 900-mcycle quote overestimates them by 10.02-10.16%; extending that trend would
approach roughly 12% overquote at larger counts, but does not establish the running guest's error
direction. The current v4 API admits 1-1024 proposals. At the committed scalar, checked numeric
overflow is unreachable within that range, so the estimator itself has no numeric fallback for a
valid v4 input. Quote preparation can still fall back if a separately configured `mcycles_offset`
addition overflows; the documented aggregation offset is zero.

The proposal measurements predate the proposal ELF rebuilt by raiko2 #242 and do not use the v0.6.0
release guest. The model identity records what was measured; it is intentionally not a runtime ELF
gate. A release that enables `estimated` with another guest accepts unmeasured quote-price and
timeout drift. Use `evaluated` when the exact cycle count for the running guest is required.

## Proposal measured ranges

Across all 140 proposal rows the samples span `block_count` 155-192 and `total_zkgas`
216314230-562107601, at 1.16-2.93 million zkGas per block. Runtime proposal availability is
deliberately wider than these ranges and uses the explicit global cap rather than the observed
sample rectangle.
